//! Session-level wrapper around one codex thread's turn lifecycle.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::anyhow;
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc};

use crate::foundation::codex::process::CodexProcess;

/// A single codex thread, owning the [`CodexProcess`] that hosts it.
///
/// Dropping the session drops the process, which signals shutdown and tears the child
/// down — the per-session teardown path. There is at most one in-flight turn per
/// session.
pub struct AgentSession {
    id: String,
    /// The child process this thread runs on. Owned exclusively, so dropping the
    /// session closes the process; turns are driven on its connection.
    process: CodexProcess,
    /// Wrapped so [`SessionRun`] can take the receiver for the duration of a turn
    /// without re-creating the channel on every call. There is at most one in-flight
    /// turn per session.
    rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<Value>>>>,
    /// The turn currently running, so [`cancel`](Self::cancel) can name it. Codex
    /// interrupts a `(threadId, turnId)` pair, where ACP cancelled a whole session.
    current_turn: Arc<Mutex<Option<String>>>,
    data_dir: PathBuf,
}

/// Why a turn stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Completed,
    Interrupted,
    Failed,
}

impl StopReason {
    fn from_status(status: Option<&str>) -> Self {
        match status {
            Some("interrupted") => StopReason::Interrupted,
            Some("failed") => StopReason::Failed,
            _ => StopReason::Completed,
        }
    }
}

/// One streaming variant we surface to callers.
#[derive(Debug, Clone)]
pub enum SessionUpdate {
    /// A chunk of agent text. Concatenate to reconstruct the message.
    Text(String),
    /// A chunk of agent internal reasoning. Routers may or may not care.
    Thought(String),
    /// **The notification itself, verbatim.** Emitted for every notification without
    /// exception, including the ones [`Text`](Self::Text) and [`Thought`](Self::Thought)
    /// also project.
    ///
    /// Those two are conveniences for the speech path, which genuinely wants text
    /// concatenated; this is the record. Earlier there was no record — a tool call became
    /// a stub naming its variant, so `arguments`, `result`, `status` and `kind` were all
    /// dropped on the floor. That is exactly what verification reads and what a replayed
    /// session is made of. Codex's item vocabulary will keep growing, so "a notification
    /// we do not know" is a permanent condition, not a gap to close.
    ///
    /// See `docs/arch/foundation.md#full-frames-not-modelled-events`.
    Frame(Value),
}

impl SessionUpdate {
    /// Every notification, verbatim — plus a text projection where one applies.
    ///
    /// The frame comes first and is never conditional: a method this build has never
    /// heard of still arrives whole, because recording does not require understanding.
    fn from_notification(note: &Value) -> Vec<SessionUpdate> {
        let mut out = vec![SessionUpdate::Frame(note.clone())];
        let delta = || {
            note.get("params")
                .and_then(|p| p.get("delta"))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        match note.get("method").and_then(Value::as_str) {
            Some("item/agentMessage/delta") => {
                if let Some(text) = delta() {
                    out.push(SessionUpdate::Text(text));
                }
            }
            Some("item/reasoning/summaryTextDelta") => {
                if let Some(text) = delta() {
                    out.push(SessionUpdate::Thought(text));
                }
            }
            _ => {}
        }
        out
    }
}

/// Result returned from [`SessionRun::wait`].
#[derive(Debug, Clone)]
pub struct PromptResult {
    pub stop_reason: StopReason,
    /// All text chunks concatenated, in the order they arrived. Provided as a
    /// convenience for callers that only want the final string.
    pub text: String,
}

/// Handle for a single in-flight turn. Stream updates via [`next_update`], or block to
/// completion with [`wait`].
///
/// **A turn ends on the stream, not on the response.** `turn/start` answers immediately
/// with the turn object — it means "accepted", not "done" — so the terminal event is the
/// `turn/completed` notification. That is the one structural difference from the ACP
/// path, where the `session/prompt` response *was* the end of the turn.
pub struct SessionRun {
    rx_slot: Arc<Mutex<Option<mpsc::UnboundedReceiver<Value>>>>,
    rx: Option<mpsc::UnboundedReceiver<Value>>,
    current_turn: Arc<Mutex<Option<String>>>,
    turn_id: String,
    /// Updates projected from the notification we most recently pulled, not yet handed
    /// to the caller. One notification can yield a frame *and* a text projection.
    queue: VecDeque<SessionUpdate>,
    /// Message item ids that arrived as deltas, so a completed item is not spoken twice.
    /// See the completion fallback in [`absorb`](Self::absorb).
    streamed_items: HashSet<String>,
    /// Set once `turn/completed` for this turn has been seen. Its frame is still
    /// delivered; nothing is read from the stream afterwards.
    stop_reason: Option<StopReason>,
    /// The failure a turn reported, if it reported one — from `turn/completed`'s error
    /// payload or a mid-turn `error` notification. Captured rather than dropped so
    /// `wait()` can surface the true cause (a gateway 402/429, a transport reset) to the
    /// reaction's classifier instead of a generic placeholder.
    error: Option<anyhow::Error>,
    /// The managed-account state lives outside this thread. Keeping the data dir here
    /// lets the common wait boundary raise a single process-wide Pause for every LLM
    /// role, instead of relying on each rung to classify 402 independently.
    data_dir: PathBuf,
    /// Text chunks observed so far, so `wait()` can return a completed assembly without
    /// forcing the caller to also pull updates.
    text_buf: String,
}

impl SessionRun {
    /// Pull the next streamed [`SessionUpdate`]. Returns `None` when the turn has
    /// finished and every queued update has been drained.
    pub async fn next_update(&mut self) -> Option<SessionUpdate> {
        loop {
            if let Some(update) = self.queue.pop_front() {
                if let SessionUpdate::Text(text) = &update {
                    self.text_buf.push_str(text);
                }
                return Some(update);
            }
            if self.stop_reason.is_some() {
                return None;
            }

            let rx = self.rx.as_mut()?;
            let Some(note) = rx.recv().await else {
                // The connection died mid-turn. Treat it as a failure rather than a
                // quiet completion — a caller that cannot tell those apart will report
                // an empty reply as success.
                self.stop_reason = Some(StopReason::Failed);
                if self.error.is_none() {
                    self.error = Some(anyhow!("codex connection closed mid-turn"));
                }
                return None;
            };
            self.absorb(&note);
        }
    }

    /// Fold one notification into the queue, and notice the ones that end the turn.
    fn absorb(&mut self, note: &Value) {
        let method = note.get("method").and_then(Value::as_str).unwrap_or_default();
        let params = note.get("params");

        // Remember which message items streamed, so the completion fallback below can
        // tell "already delivered in pieces" from "arrived whole".
        if method == "item/agentMessage/delta"
            && let Some(id) = params.and_then(|p| p.get("itemId")).and_then(Value::as_str)
        {
            self.streamed_items.insert(id.to_string());
        }

        match method {
            "turn/completed" => {
                let turn = params.and_then(|p| p.get("turn"));
                let same_turn = turn
                    .and_then(|t| t.get("id"))
                    .and_then(Value::as_str)
                    .is_none_or(|id| id == self.turn_id);
                if same_turn {
                    let status = turn.and_then(|t| t.get("status")).and_then(Value::as_str);
                    self.stop_reason = Some(StopReason::from_status(status));
                    if let Some(err) = turn.and_then(|t| t.get("error")).filter(|e| !e.is_null()) {
                        self.note_error(err);
                    }
                }
            }
            // Emitted for upstream failures (model errors, quota) and may precede the
            // terminal notification; we keep the cause and let the turn end normally.
            "error" => {
                if let Some(err) = params.and_then(|p| p.get("error")).or(params) {
                    self.note_error(err);
                }
            }
            _ => {}
        }

        self.queue.extend(SessionUpdate::from_notification(note));

        // A message that never streamed still has to be *said*. Deltas are the usual
        // path, but they are not guaranteed — a provider that returns the assistant
        // message as one completed item produces `item/completed` and no deltas at all,
        // and projecting only from deltas made the voice go silent while the turn
        // reported `Completed`. Caught in a live run against a non-streaming upstream:
        // `reply_chars=0` on a turn that had plainly succeeded.
        if method == "item/completed"
            && let Some(item) = params.and_then(|p| p.get("item"))
            && item.get("type").and_then(Value::as_str) == Some("agentMessage")
            && let Some(text) = item.get("text").and_then(Value::as_str)
            && !text.is_empty()
        {
            let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
            if !self.streamed_items.contains(id) {
                self.queue.push_back(SessionUpdate::Text(text.to_string()));
            }
        }
    }

    /// Record a turn's failure, preferring the first one seen — the earliest error is
    /// the cause; later ones are usually its consequences.
    fn note_error(&mut self, err: &Value) {
        if self.error.is_some() {
            return;
        }
        let message = err
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| err.to_string());
        // `codexErrorInfo` carries the upstream status structurally (e.g.
        // `HttpConnectionFailed { httpStatusCode: 402 }`). Append it to the text so the
        // energy classifier, which matches on the message, sees the status even when the
        // human-readable half omits it.
        let detail = err.get("codexErrorInfo").filter(|v| !v.is_null());
        self.error = Some(match detail {
            Some(info) => anyhow!("{message} ({info})"),
            None => anyhow!("{message}"),
        });
    }

    /// Drain the stream to completion and return the final result. Consumes the handle.
    pub async fn wait(mut self) -> anyhow::Result<PromptResult> {
        while self.next_update().await.is_some() {}

        // Park the receiver back *first* so a subsequent turn on the same session can
        // pick up where we left off. This must precede the error check: a session is
        // reused turn after turn, and a turn that ended badly would otherwise leave the
        // slot empty forever, wedging every later turn with "already has an in-flight
        // turn".
        if let Some(rx) = self.rx.take() {
            *self.rx_slot.lock().await = Some(rx);
        }
        *self.current_turn.lock().await = None;

        let stop_reason = self.stop_reason.unwrap_or(StopReason::Failed);
        if stop_reason == StopReason::Failed {
            let err = self
                .error
                .take()
                .unwrap_or_else(|| anyhow!("turn failed without saying why"));
            crate::foundation::energy_state::note_402_error(&self.data_dir, &err);
            return Err(err);
        }

        Ok(PromptResult { stop_reason, text: std::mem::take(&mut self.text_buf) })
    }
}

impl AgentSession {
    pub(crate) fn new(
        id: String,
        process: CodexProcess,
        rx: mpsc::UnboundedReceiver<Value>,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            id,
            process,
            rx: Arc::new(Mutex::new(Some(rx))),
            current_turn: Arc::new(Mutex::new(None)),
            data_dir,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Start a turn with `text` and return a streaming handle.
    pub async fn prompt(&self, text: String) -> anyhow::Result<SessionRun> {
        let rx = {
            let mut slot = self.rx.lock().await;
            slot.take()
                .ok_or_else(|| anyhow!("session already has an in-flight turn"))?
        };

        // Restore the receiver if starting the turn fails, so one refused `turn/start`
        // does not wedge the session for good.
        let started = self
            .process
            .request(
                "turn/start",
                json!({
                    "threadId": self.id,
                    "input": [{ "type": "text", "text": text, "text_elements": [] }],
                }),
            )
            .await;
        let started = match started {
            Ok(value) => value,
            Err(err) => {
                *self.rx.lock().await = Some(rx);
                return Err(err);
            }
        };

        let turn_id = started
            .get("turn")
            .and_then(|t| t.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("turn/start returned no turn id: {started}"))?
            .to_string();
        *self.current_turn.lock().await = Some(turn_id.clone());

        Ok(SessionRun {
            rx_slot: self.rx.clone(),
            rx: Some(rx),
            current_turn: self.current_turn.clone(),
            turn_id,
            queue: VecDeque::new(),
            streamed_items: HashSet::new(),
            stop_reason: None,
            error: None,
            data_dir: self.data_dir.clone(),
            text_buf: String::new(),
        })
    }

    /// Interrupt the turn currently running. The in-flight [`SessionRun`] resolves with
    /// [`StopReason::Interrupted`]. A no-op when no turn is running.
    pub async fn cancel(&self) -> anyhow::Result<()> {
        let Some(turn_id) = self.current_turn.lock().await.clone() else {
            return Ok(());
        };
        self.process
            .request("turn/interrupt", json!({ "threadId": self.id, "turnId": turn_id }))
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(turn_id: &str) -> SessionRun {
        let (_tx, rx) = mpsc::unbounded_channel();
        SessionRun {
            rx_slot: Arc::new(Mutex::new(None)),
            rx: Some(rx),
            current_turn: Arc::new(Mutex::new(None)),
            turn_id: turn_id.to_string(),
            queue: VecDeque::new(),
            streamed_items: HashSet::new(),
            stop_reason: None,
            error: None,
            data_dir: PathBuf::from("/nonexistent"),
            text_buf: String::new(),
        }
    }

    #[test]
    fn every_notification_becomes_a_frame_verbatim() {
        // The point of the frame contract: an item type this build has never modelled
        // still arrives whole, payload and all.
        let note = json!({
            "method": "item/completed",
            "params": { "item": {
                "type": "mcpToolCall", "id": "i1", "server": "hi-agent", "tool": "look",
                "status": "completed",
                "result": { "content": [{ "type": "image", "mimeType": "image/png" }] }
            }}
        });
        let updates = SessionUpdate::from_notification(&note);
        assert_eq!(updates.len(), 1, "no text projection applies to a tool call");
        let SessionUpdate::Frame(frame) = &updates[0] else { panic!("expected a frame") };
        assert_eq!(frame["params"]["item"]["result"]["content"][0]["type"], "image");
    }

    #[test]
    fn text_and_thought_are_projected_beside_the_frame() {
        let updates = SessionUpdate::from_notification(&json!({
            "method": "item/agentMessage/delta",
            "params": { "itemId": "i1", "delta": "hello" }
        }));
        assert!(matches!(updates.as_slice(), [SessionUpdate::Frame(_), SessionUpdate::Text(t)] if t == "hello"));

        let updates = SessionUpdate::from_notification(&json!({
            "method": "item/reasoning/summaryTextDelta",
            "params": { "itemId": "i2", "delta": "thinking" }
        }));
        assert!(matches!(updates.as_slice(), [SessionUpdate::Frame(_), SessionUpdate::Thought(t)] if t == "thinking"));
    }

    #[tokio::test]
    async fn a_completed_turn_ends_the_stream_and_assembles_its_text() {
        let mut r = run("turn_1");
        r.absorb(&json!({ "method": "turn/started", "params": { "turn": { "id": "turn_1" } } }));
        r.absorb(&json!({ "method": "item/agentMessage/delta", "params": { "delta": "on " } }));
        r.absorb(&json!({ "method": "item/agentMessage/delta", "params": { "delta": "it" } }));
        r.absorb(&json!({
            "method": "turn/completed",
            "params": { "turn": { "id": "turn_1", "status": "completed" } }
        }));

        while r.next_update().await.is_some() {}
        assert_eq!(r.stop_reason, Some(StopReason::Completed));
        assert_eq!(r.text_buf, "on it");
    }

    /// A provider that does not stream returns the whole message as one completed item.
    /// The voice must still say it — this exact case shipped silent once.
    #[tokio::test]
    async fn a_message_that_never_streamed_is_still_spoken() {
        let mut r = run("turn_1");
        r.absorb(&json!({
            "method": "item/completed",
            "params": { "item": { "type": "agentMessage", "id": "m1", "text": "hello there" } }
        }));
        r.absorb(&json!({
            "method": "turn/completed",
            "params": { "turn": { "id": "turn_1", "status": "completed" } }
        }));
        while r.next_update().await.is_some() {}
        assert_eq!(r.text_buf, "hello there");
    }

    /// ...and the usual streaming path must not then say it a second time.
    #[tokio::test]
    async fn a_streamed_message_is_not_repeated_on_completion() {
        let mut r = run("turn_1");
        r.absorb(&json!({
            "method": "item/agentMessage/delta",
            "params": { "itemId": "m1", "delta": "hello " }
        }));
        r.absorb(&json!({
            "method": "item/agentMessage/delta",
            "params": { "itemId": "m1", "delta": "there" }
        }));
        r.absorb(&json!({
            "method": "item/completed",
            "params": { "item": { "type": "agentMessage", "id": "m1", "text": "hello there" } }
        }));
        r.absorb(&json!({
            "method": "turn/completed",
            "params": { "turn": { "id": "turn_1", "status": "completed" } }
        }));
        while r.next_update().await.is_some() {}
        assert_eq!(r.text_buf, "hello there");
    }

    #[tokio::test]
    async fn another_turns_completion_does_not_end_this_one() {
        let mut r = run("turn_1");
        r.absorb(&json!({
            "method": "turn/completed",
            "params": { "turn": { "id": "turn_2", "status": "completed" } }
        }));
        assert_eq!(r.stop_reason, None, "a stray turn's end must not end ours");
    }

    #[tokio::test]
    async fn a_failed_turn_keeps_the_upstream_status_in_its_error() {
        // The energy gate classifies the managed 402 by matching the message text, so
        // the status has to survive into the error even when it only appears in
        // `codexErrorInfo`.
        let mut r = run("turn_1");
        r.absorb(&json!({
            "method": "turn/completed",
            "params": { "turn": { "id": "turn_1", "status": "failed", "error": {
                "message": "upstream request failed",
                "codexErrorInfo": { "HttpConnectionFailed": { "httpStatusCode": 402 } }
            }}}
        }));
        while r.next_update().await.is_some() {}
        let text = r.error.as_ref().unwrap().to_string();
        assert!(text.contains("402"), "402 must survive into the error text: {text}");
    }

    #[tokio::test]
    async fn a_mid_turn_error_wins_over_a_later_one() {
        let mut r = run("turn_1");
        r.absorb(&json!({ "method": "error", "params": { "error": { "message": "the real cause" } } }));
        r.absorb(&json!({
            "method": "turn/completed",
            "params": { "turn": { "id": "turn_1", "status": "failed", "error": { "message": "a consequence" } } }
        }));
        while r.next_update().await.is_some() {}
        assert!(r.error.as_ref().unwrap().to_string().contains("the real cause"));
    }

    #[tokio::test]
    async fn a_dead_connection_mid_turn_is_a_failure_not_a_quiet_success() {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut r = run("turn_1");
        r.rx = Some(rx);
        drop(tx);
        assert!(r.next_update().await.is_none());
        assert_eq!(r.stop_reason, Some(StopReason::Failed));
    }
}
