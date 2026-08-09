//! Observatory — structured visibility into the agent session lifecycle.
//!
//! agent sessions are otherwise invisible: the persistent reaction session,
//! ephemeral worker sessions (each on its own subprocess), in-flight prompts,
//! session lifecycle events all live only as scattered `tracing`
//! lines. The observatory is an additive, cloneable handle (like [`Memory`] or
//! [`TextBus`]) that the reaction, workers and heartbeat feed as
//! those things happen. It keeps two things:
//!
//! - a **live mirror** — the voice's current state (reaction session, context
//!   budget, last turn), for `GET /api/sessions`;
//! - an **event history** — a bounded ring of lifecycle [`SessionEvent`]s plus a
//!   live `broadcast`, streamed verbatim over SSE on `GET /api/sessions/events`,
//!   and best-effort appended to `<data_dir>/sessions.jsonl` for durable replay.
//!
//! Recording an event mutates the mirror, pushes to the ring, appends to the
//! journal and broadcasts it — all under one lock so an SSE subscriber that
//! snapshots-then-subscribes can neither miss an event nor see a duplicate.
//!
//! [`Memory`]: crate::mind::memory::Memory
//! [`TextBus`]: crate::foundation::server::TextBus

use std::collections::VecDeque;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::{Mutex, RwLock, broadcast};


/// How many recent events the in-memory ring retains for SSE replay-on-connect.
const HISTORY_CAP: usize = 1000;
/// Broadcast backlog; a subscriber that lags past this misses events (logged on
/// the wire as a gap, never blocks the producer).
const BROADCAST_CAP: usize = 512;

/// Which rung an agent session is: the four of the
/// [ladder](../../../docs/arch/arch.md) that hold sessions, plus the workers below
/// them. It mirrors [`crate::foundation::agent::SessionRole`] and must keep doing
/// so — a rung with no variant here cannot be told apart from a worker anywhere in
/// the observatory, which is exactly what happened to Deliberation. (A `Summarizer`
/// variant outlived the session swap that produced it and is gone with it.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Reaction,
    Deliberation,
    Worker,
    Reflection,
    Cognition,
}

/// The mirror, made the compiler's problem rather than a convention. Adding a rung to
/// [`SessionRole`] now fails to compile until it has a kind here, which is the check
/// that was missing when Deliberation was added and the observatory kept calling it a
/// worker.
impl From<crate::foundation::agent::SessionRole> for SessionKind {
    fn from(role: crate::foundation::agent::SessionRole) -> Self {
        use crate::foundation::agent::SessionRole as R;
        match role {
            R::Reaction => Self::Reaction,
            R::Deliberation => Self::Deliberation,
            R::Worker => Self::Worker,
            R::Reflection => Self::Reflection,
            R::Cognition => Self::Cognition,
        }
    }
}

/// Live state of one agent session in the mirror.
#[derive(Debug, Clone, Serialize)]
pub struct SessionView {
    pub id: String,
    pub kind: SessionKind,
    pub opened_at: DateTime<Utc>,
    /// True while a prompt is mid-flight on this session.
    pub in_flight: bool,
    /// Completed turns/prompts driven on this session.
    pub turns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState {
    Running,
    Done,
    Failed,
}

/// The most recent turn on the reaction session.
#[derive(Debug, Clone, Serialize)]
pub struct TurnView {
    pub turn: u64,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub stop_reason: Option<String>,
    pub reply_chars: Option<usize>,
}

/// The full live picture of the agent's voice, served by `GET /api/sessions`.
///
/// **Voice-shaped state only.** There is deliberately no `workers` here: a working
/// session belongs to whoever created it, and worker lifecycle is carried by the event
/// log ([`EventKind::WorkerSpawned`] and friends), which is keyed by session and does
/// not have to lie about where the work lives.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentView {
    pub reaction_session: Option<SessionView>,
    /// Accumulated prompt+reply chars since the live session was last opened. Reported
    /// only — nothing thresholds on it. Bounding a session's context is the underlying
    /// agent's job (see [`crate::body::reaction::heartbeat`]), so there is no ceiling here
    /// to render it against.
    pub budget_chars: usize,
    pub last_turn: Option<TurnView>,
    pub turns_total: u64,
}

/// One lifecycle event — the unit of the SSE stream, the ring, and `sessions.jsonl`.
#[derive(Debug, Clone, Serialize)]
pub struct SessionEvent {
    /// Monotonic, gap-free sequence number assigned at record time.
    pub seq: u64,
    pub ts: DateTime<Utc>,
    #[serde(flatten)]
    pub kind: EventKind,
}

/// The shape of each lifecycle event. Serialized with an `"event"` tag so the
/// wire form is `{ "seq", "ts", "event": "...", ...fields }`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EventKind {
    SessionOpened { kind: SessionKind, id: String },
    SessionClosed { kind: SessionKind, id: String },
    /// `input` is the human-readable incoming message(s) for this turn — the new
    /// signals batch (human utterances, worker reports, pulses), not the
    /// full seeded prompt.
    TurnStarted { turn: u64, input: String },
    /// `reply` is the agent's spoken text for this turn (markers stripped).
    TurnFinished { turn: u64, stop_reason: Option<String>, reply_chars: usize, reply: String },
    WorkerSpawned { id: u64, task: String },
    /// A warm (finished-but-idle) worker was handed a follow-up task and is running
    /// again on the same session.
    WorkerResumed { id: u64, task: String },
    WorkerFinished { id: u64, state: WorkerState, summary_chars: usize },
    /// One agent-to-agent edge: the one verb crossing, and what became of it.
    ///
    /// Recorded for **both** directions of host mediation. `from: Some(id)` is one agent
    /// reaching another (`send_message`); `from: None` is the host putting something in
    /// a mailbox on nobody's behalf — a finished worker's report handed up to its owner,
    /// or a follow-up merged into a warm session. That is exactly the meaning `from`
    /// carries on [`crate::foundation::registry::Message`], mirrored rather than
    /// reinterpreted.
    ///
    /// `to` is a session id, which is now the only kind of address there is — so an edge
    /// names both its ends without anything having to be resolved after the fact.
    ///
    /// The full `message` travels, like `TurnStarted { input }` and
    /// `TurnFinished { reply }`. An edge you can see the existence but not the content
    /// of does not answer the question you opened the inspector to ask.
    MessageSent {
        from: Option<u64>,
        to: u64,
        delivery: crate::foundation::registry::Delivery,
        message: String,
    },
}

/// Cloneable handle over the shared observatory state.
#[derive(Clone)]
pub struct Observatory {
    inner: Arc<Inner>,
}

struct Inner {
    agent: RwLock<AgentView>,
    history: Mutex<History>,
    tx: broadcast::Sender<SessionEvent>,
    /// Where to append the durable event log, or `None` to skip persistence.
    jsonl: Option<PathBuf>,
}

struct History {
    seq: u64,
    ring: VecDeque<SessionEvent>,
}

impl Observatory {
    /// Build an observatory. `jsonl` is where to append durable events (created
    /// lazily on first append); pass `None` to keep history in-memory only.
    pub fn new(jsonl: Option<PathBuf>) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAP);
        Self {
            inner: Arc::new(Inner {
                agent: RwLock::new(AgentView::default()),
                history: Mutex::new(History { seq: 0, ring: VecDeque::new() }),
                tx,
                jsonl,
            }),
        }
    }

    /// Record one lifecycle event: assign a seq, mutate the live mirror, push to
    /// the ring, append to the journal, and broadcast — the ring push and the
    /// broadcast both happen under the history lock so a concurrent
    /// [`subscribe`](Self::subscribe) sees a consistent, dup-free cut.
    ///
    /// History takes everything; the mirror takes only what describes the voice —
    /// a worker spawning is real history, but it is not the state of the mouth.
    pub async fn record(&self, kind: EventKind) {
        // Mirror first (its own lock), so a snapshot taken right after the event
        // lands reflects it.
        self.apply_to_mirror(&kind).await;

        let mut hist = self.inner.history.lock().await;
        hist.seq += 1;
        let event = SessionEvent {
            seq: hist.seq,
            ts: Utc::now(),
            kind,
        };
        hist.ring.push_back(event.clone());
        while hist.ring.len() > HISTORY_CAP {
            hist.ring.pop_front();
        }
        if let Some(path) = &self.inner.jsonl {
            append_jsonl(path, &event).await;
        }
        // Ignore the error: no subscribers is fine. Held under the history lock
        // so `subscribe` cannot interleave between the ring push and this send.
        let _ = self.inner.tx.send(event);
    }

    /// A live snapshot of the voice's state.
    pub async fn snapshot(&self) -> AgentView {
        self.inner.agent.read().await.clone()
    }

    /// Snapshot the event ring and subscribe to the live feed atomically. The
    /// returned `Vec` is everything recorded so far; the receiver yields every
    /// event recorded after this call. Because [`record`](Self::record)
    /// broadcasts under the same lock we hold here, the two never overlap — no
    /// event is both replayed and live.
    pub async fn subscribe(&self) -> (Vec<SessionEvent>, broadcast::Receiver<SessionEvent>) {
        let hist = self.inner.history.lock().await;
        let rx = self.inner.tx.subscribe();
        let replay = hist.ring.iter().cloned().collect();
        (replay, rx)
    }

    /// Update the accumulated context budget (mirror-only; not an event — it changes
    /// every turn and matters as state, not as history).
    pub async fn set_budget(&self, chars: usize) {
        self.inner.agent.write().await.budget_chars = chars;
    }

    /// Fold an event into the live mirror. Pure state transition; no I/O.
    async fn apply_to_mirror(&self, kind: &EventKind) {
        let now = Utc::now();
        let mut view = self.inner.agent.write().await;
        let view = &mut *view;

        match kind {
            EventKind::SessionOpened { kind, id } => match kind {
                SessionKind::Reaction => {
                    view.reaction_session = Some(SessionView {
                        id: id.clone(),
                        kind: SessionKind::Reaction,
                        opened_at: now,
                        in_flight: false,
                        turns: 0,
                    });
                }
                // Worker open is mirrored by WorkerSpawned; a reflection pass is a
                // throwaway we don't surface as a standing session. Deliberation and
                // Cognition are not the voice, so they are history-only — which is
                // where a rung nobody is listening to honestly belongs. They are
                // *recorded* either way: the event log is what names a rung, and
                // until it did, Deliberation was indistinguishable from a worker.
                SessionKind::Worker
                | SessionKind::Deliberation
                | SessionKind::Reflection
                | SessionKind::Cognition => {}
            },
            EventKind::SessionClosed { kind: SessionKind::Reaction, id } => {
                if view.reaction_session.as_ref().map(|session| session.id.as_str())
                    == Some(id.as_str())
                {
                    view.reaction_session = None;
                }
            }
            // Worker open/close is history-only; summarizer, Reflection, and
            // Cognition sessions are not represented as standing voice state.
            EventKind::SessionClosed { .. } => {}
            EventKind::TurnStarted { turn, .. } => {
                if let Some(s) = view.reaction_session.as_mut() {
                    s.in_flight = true;
                }
                view.last_turn = Some(TurnView {
                    turn: *turn,
                    started_at: now,
                    finished_at: None,
                    stop_reason: None,
                    reply_chars: None,
                });
            }
            EventKind::TurnFinished { turn, stop_reason, reply_chars, .. } => {
                if let Some(s) = view.reaction_session.as_mut() {
                    s.in_flight = false;
                    s.turns += 1;
                }
                view.turns_total += 1;
                view.last_turn = Some(TurnView {
                    turn: *turn,
                    started_at: view
                        .last_turn
                        .as_ref()
                        .filter(|t| t.turn == *turn)
                        .map(|t| t.started_at)
                        .unwrap_or(now),
                    finished_at: Some(now),
                    stop_reason: stop_reason.clone(),
                    reply_chars: Some(*reply_chars),
                });
            }
            // Worker lifecycle is history, not voice state — see [`AgentView`]. A
            // working session is keyed by its own id and owned by whoever asked for it,
            // so folding it into the mouth's state would answer a question nobody
            // asked. Read these off the event log.
            EventKind::WorkerSpawned { .. }
            | EventKind::WorkerResumed { .. }
            | EventKind::WorkerFinished { .. } => {}
            // Also history-only, and deliberately not summarized onto the voice. An
            // edge has two ends and at most one of them is the voice, so any single
            // slot would have to pick one and pretend. `last_question` was that
            // mistake — one slot fed by two different events, each overwriting the
            // other — and `docs/arch/foundation.md#debug-surfaces` forbids it.
            EventKind::MessageSent { .. } => {}
        }
    }

    /// Test/util: total events recorded so far.
    #[cfg(test)]
    pub async fn event_count(&self) -> u64 {
        self.inner.history.lock().await.seq
    }
}

/// Best-effort append of one event as a JSON line. Failures are logged and
/// swallowed — the durable log is a convenience, never load-bearing.
async fn append_jsonl(path: &PathBuf, event: &SessionEvent) {
    use tokio::io::AsyncWriteExt;
    let mut line = match serde_json::to_string(event) {
        Ok(s) => s,
        Err(err) => {
            tracing::warn!(%err, "observatory: serialize event failed");
            return;
        }
    };
    line.push('\n');
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await;
    match file {
        Ok(mut f) => {
            if let Err(err) = f.write_all(line.as_bytes()).await {
                tracing::warn!(%err, "observatory: append to sessions.jsonl failed");
            }
        }
        Err(err) => {
            tracing::warn!(%err, path = %path.display(), "observatory: open sessions.jsonl failed");
        }
    }
}

/// Convenience for the SSE handler: turn a broadcast receiver into a stream of
/// events, skipping lag gaps and ending on close.
pub fn event_stream(
    rx: broadcast::Receiver<SessionEvent>,
) -> impl futures::Stream<Item = Result<SessionEvent, Infallible>> {
    futures::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(ev) => return Some((Ok(ev), rx)),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "observatory: SSE subscriber lagged");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The observatory's whole job is answering "what is the machine doing", and a
    /// rung it cannot name is one it answers wrongly rather than not at all. Each
    /// role's kind also has to serialize to the role's own word, because the wire
    /// name is what the inspector prints.
    #[test]
    fn every_rung_has_a_kind_and_keeps_its_name() {
        use crate::foundation::agent::SessionRole as R;
        for (role, word) in [
            (R::Reaction, "reaction"),
            (R::Deliberation, "deliberation"),
            (R::Worker, "worker"),
            (R::Reflection, "reflection"),
            (R::Cognition, "cognition"),
        ] {
            let kind = SessionKind::from(role);
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{word}\""), "{role:?} serializes as {json}");
        }
    }

    #[tokio::test]
    async fn mirrors_reaction_session_and_turn() {
        let obs = Observatory::new(None);
        obs.record(EventKind::SessionOpened { kind: SessionKind::Reaction, id: "sess-1".into() })
            .await;
        obs.record(EventKind::TurnStarted { turn: 0, input: "hi".into() }).await;

        let snap = obs.snapshot().await;
        let rs = snap.reaction_session.as_ref().unwrap();
        assert_eq!(rs.id, "sess-1");
        assert!(rs.in_flight, "turn in flight");

        obs.record(EventKind::TurnFinished {
            turn: 0,
            stop_reason: Some("end_turn".into()),
            reply_chars: 42,
            reply: "hello there".into(),
        })
        .await;
        let v = obs.snapshot().await;
        assert!(!v.reaction_session.as_ref().unwrap().in_flight);
        assert_eq!(v.turns_total, 1);
        assert_eq!(v.last_turn.as_ref().unwrap().reply_chars, Some(42));
    }

    #[tokio::test]
    async fn closing_a_reaction_session_clears_only_that_live_session() {
        let obs = Observatory::new(None);
        obs.record(EventKind::SessionOpened { kind: SessionKind::Reaction, id: "sess-1".into() })
            .await;

        obs.record(EventKind::SessionClosed { kind: SessionKind::Reaction, id: "older".into() })
            .await;
        assert_eq!(
            obs.snapshot().await.reaction_session.as_ref().map(|s| s.id.as_str()),
            Some("sess-1")
        );

        obs.record(EventKind::SessionClosed { kind: SessionKind::Reaction, id: "sess-1".into() })
            .await;
        assert!(obs.snapshot().await.reaction_session.is_none());
    }

    /// Worker lifecycle is history, and *only* history — a working session is keyed by
    /// its own id and owned by whoever asked for it, so it is not the state of the mouth.
    #[tokio::test]
    async fn worker_lifecycle_is_history_not_voice_state() {
        let obs = Observatory::new(None);
        obs.record(EventKind::WorkerSpawned { id: 1, task: "research X".into() }).await;
        obs.record(EventKind::WorkerFinished {
            id: 1,
            state: WorkerState::Done,
            summary_chars: 120,
        })
        .await;

        let (replay, _rx) = obs.subscribe().await;
        assert_eq!(replay.len(), 2, "both events are in history");
        assert!(obs.snapshot().await.reaction_session.is_none(), "the mouth is untouched");
        assert_eq!(obs.event_count().await, 2);
    }

    /// A rung that is not the voice is real history and must not be mirrored as the
    /// mouth's state — Reflection opening a pass says nothing about whether anyone is
    /// being spoken to.
    #[tokio::test]
    async fn a_non_voice_event_is_recorded_and_mirrors_nothing() {
        let obs = Observatory::new(None);
        obs.record(EventKind::SessionOpened {
            kind: SessionKind::Reflection,
            id: "refl-1".into(),
        })
        .await;

        assert!(obs.snapshot().await.reaction_session.is_none(), "no voice was invented");
        let (replay, _rx) = obs.subscribe().await;
        assert_eq!(replay.len(), 1, "but it is still history");
    }

    #[tokio::test]
    async fn subscribe_replays_then_streams_live_without_dup() {
        let obs = Observatory::new(None);
        obs.record(EventKind::SessionOpened { kind: SessionKind::Reaction, id: "sess-1".into() })
            .await;
        let (replay, mut rx) = obs.subscribe().await;
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].seq, 1);

        obs.record(EventKind::SessionClosed { kind: SessionKind::Reaction, id: "sess-1".into() })
            .await;
        let live = rx.recv().await.unwrap();
        assert_eq!(live.seq, 2, "live event follows replay with no gap or dup");
    }
}
