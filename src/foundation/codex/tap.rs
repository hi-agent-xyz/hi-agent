//! Raw wire tap — a business-logic-agnostic mirror of the JSON-RPC frames flowing
//! between hi-agent and every session's `codex app-server` subprocess.
//!
//! The [`Observatory`](crate::foundation::observatory::Observatory) renders the *reaction's*
//! view of a session (turns, context budget, hot-swaps). This is the
//! opposite: the rawest possible window, knowing nothing about the reaction. It
//! taps the one place every frame transits — the reader and writer tasks of the
//! connection (see [`crate::foundation::codex::process`]) — and records each line verbatim,
//! tagged with a per-connection id (one subprocess hosts one thread, so this
//! groups a session's frames together), the rung's role, the direction, and whatever
//! `threadId`/`method`/`id` can be parsed out of the JSON.
//!
//! It mirrors the observatory's ring+broadcast shape so the SSE handler reads it
//! the same way (replay-then-live), but its [`record`](WireTap::record) is
//! **synchronous**, so it can be called from the I/O tasks without an await point
//! in the middle of a line. A `std::sync::Mutex` guards a short, IO-free critical
//! section.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};

/// How many recent frames the in-memory ring retains for SSE replay-on-connect.
/// Larger than the observatory's event ring: frames are higher-frequency (every
/// chunk of a streamed reply is a notification) and this is a debug surface.
const RING_CAP: usize = 4000;
/// Broadcast backlog; a subscriber that lags past this misses frames (surfaced as
/// a gap, never blocks the producer).
const BROADCAST_CAP: usize = 1024;

/// Direction of a raw frame on the wire, from hi-agent's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    /// Sent to the subprocess (requests we issue, responses to its requests).
    Send,
    /// Received from the subprocess (its responses, its notifications/requests).
    Recv,
    /// A line the subprocess wrote to stderr.
    Stderr,
}

/// One raw JSON-RPC line, verbatim, plus the little we parse out for grouping.
#[derive(Debug, Clone, Serialize)]
pub struct RawFrame {
    /// Monotonic, gap-free sequence number assigned at record time.
    pub seq: u64,
    pub ts: DateTime<Utc>,
    /// Which subprocess/connection emitted this. One subprocess hosts exactly one
    /// session, so the inspector groups a session's frames by this — including the
    /// `initialize`/`session/new` frames that precede (and so carry no) `sessionId`.
    pub conn: u64,
    /// hi-agent's own session id for that connection, when it has one.
    ///
    /// Distinct from [`thread_id`](Self::thread_id), which is the *protocol's* id
    /// parsed off the line and absent during the handshake. This one is minted by the
    /// host before the subprocess starts, so it names every frame including the first —
    /// which is what makes a durable per-session file possible at all.
    pub agent_session: Option<u64>,
    /// The rung this subprocess hosts (`reaction`, `cognition`, `worker`, …).
    pub role: String,
    pub dir: Dir,
    /// `threadId` parsed from `params`/`result`, when present. The `initialize`
    /// handshake and the `thread/start` request carry `None` (the id doesn't exist
    /// yet); they still group with the session via `conn`.
    pub thread_id: Option<String>,
    /// The JSON-RPC `method`, for requests and notifications.
    pub method: Option<String>,
    /// The JSON-RPC `id`, for request/response correlation (number or string).
    pub id: Option<Value>,
    /// The line exactly as it crossed the wire.
    pub raw: String,
}

/// Cloneable handle over the shared raw-frame ring + live broadcast.
#[derive(Clone)]
pub struct WireTap {
    inner: Arc<Inner>,
}

struct Inner {
    state: Mutex<State>,
    tx: broadcast::Sender<RawFrame>,
    /// Durable sink. `None` when the tap is purely an inspector window (tests), which
    /// is why it is an option rather than a path: a tap with nowhere to write is a
    /// legitimate configuration, a tap that *pretends* to write is not.
    ///
    /// Unbounded on purpose. The ring above and the broadcast beside it are debug
    /// surfaces and may drop; this is the record, and a record that quietly loses
    /// frames under load is worse than no record — it is a record you would trust.
    durable: Option<mpsc::UnboundedSender<RawFrame>>,
}

struct State {
    seq: u64,
    ring: VecDeque<RawFrame>,
}

impl WireTap {
    /// An inspector-only tap: the in-memory ring and the live broadcast, nothing on
    /// disk. For tests and for anything that has no data dir yet.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAP);
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State { seq: 0, ring: VecDeque::new() }),
                tx,
                durable: None,
            }),
        }
    }

    /// A tap that also **keeps** what it sees, under
    /// [`session_frames_path`](crate::mind::memory::layout::session_frames_path).
    ///
    /// Spawns one writer task; [`record`](Self::record) hands frames to it and never
    /// touches the filesystem itself, because it runs on the subprocess's own I/O path —
    /// a blocking write there would stall the agent mid-turn.
    pub fn with_durable_log(data_dir: PathBuf) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAP);
        let (dtx, drx) = mpsc::unbounded_channel();
        tokio::spawn(write_frames(data_dir, drx));
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State { seq: 0, ring: VecDeque::new() }),
                tx,
                durable: Some(dtx),
            }),
        }
    }

    /// Record one raw line. Synchronous and non-blocking: assigns a seq, pushes
    /// to the bounded ring, and broadcasts. Safe to call from the `with_debug`
    /// hook (no await, no IO under the lock). A poisoned lock is ignored — the
    /// tap is a convenience, never load-bearing. `conn` identifies the emitting
    /// subprocess so the inspector can group one session's frames together.
    pub fn record(&self, conn: u64, agent_session: Option<u64>, role: &str, dir: Dir, line: &str) {
        let (thread_id, method, id) = parse_meta(line);
        let mut state = match self.inner.state.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.seq += 1;
        let frame = RawFrame {
            seq: state.seq,
            ts: Utc::now(),
            conn,
            agent_session,
            role: role.to_string(),
            dir,
            thread_id,
            method,
            id,
            raw: line.to_string(),
        };
        // Only frames that name a session are kept. A connection with no agent session
        // id has nothing durable to be filed as, and inventing a bucket for it would put
        // frames somewhere no reader will look.
        if let Some(durable) = &self.inner.durable
            && frame.agent_session.is_some()
            && durable.send(frame.clone()).is_err()
        {
            tracing::warn!(seq = frame.seq, "frame log writer is gone; frame not kept");
        }
        state.ring.push_back(frame.clone());
        while state.ring.len() > RING_CAP {
            state.ring.pop_front();
        }
        drop(state);
        // No subscribers is fine.
        let _ = self.inner.tx.send(frame);
    }

    /// Snapshot the ring and subscribe to the live feed atomically, mirroring
    /// [`Observatory::subscribe`](crate::foundation::observatory::Observatory::subscribe):
    /// the replay `Vec` is every frame so far, the receiver yields frames
    /// recorded after this call, with no overlap (both happen under the lock).
    pub fn subscribe(&self) -> (Vec<RawFrame>, broadcast::Receiver<RawFrame>) {
        let state = match self.inner.state.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let rx = self.inner.tx.subscribe();
        let replay = state.ring.iter().cloned().collect();
        (replay, rx)
    }
}

impl Default for WireTap {
    fn default() -> Self {
        Self::new()
    }
}

/// Pull the grouping metadata out of a raw JSON-RPC line. Best-effort: a line
/// that doesn't parse as JSON (rare; stderr spew) yields all-`None`, which the
/// inspector still shows verbatim.
fn parse_meta(line: &str) -> (Option<String>, Option<String>, Option<Value>) {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return (None, None, None);
    };
    // The thread id rides `params` (requests and notifications), `result.thread.id`
    // (the `thread/start` response), or `params.thread.id` (`thread/started`).
    let thread_id = v
        .get("params")
        .and_then(|p| p.get("threadId"))
        .or_else(|| v.get("params").and_then(|p| p.get("thread")).and_then(|t| t.get("id")))
        .or_else(|| v.get("result").and_then(|r| r.get("thread")).and_then(|t| t.get("id")))
        .and_then(|s| s.as_str())
        .map(str::to_string);
    let method = v.get("method").and_then(|m| m.as_str()).map(str::to_string);
    let id = v.get("id").cloned();
    (thread_id, method, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point: what crossed the wire is still there afterwards, byte for byte.
    ///
    /// Asserts on a **tool-call payload**, because that is precisely what the old
    /// modelling threw away — a call's arguments and result were reduced to the string
    /// `"tool_call"` — and it is what verification has to read.
    #[tokio::test]
    async fn frames_are_kept_verbatim_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let tap = WireTap::with_durable_log(dir.path().to_path_buf());

        let tool_call = r#"{"method":"item/completed","params":{"threadId":"s1","item":{"type":"mcpToolCall","id":"tc-1","server":"hi-agent","tool":"read","status":"completed","arguments":{"path":"/etc/hosts"},"result":{"content":"127.0.0.1"}}}}"#;
        tap.record(1, Some(42), "reaction", Dir::Recv, tool_call);
        tap.record(1, Some(42), "reaction", Dir::Stderr, "a warning from the subprocess");
        // A different session must not land in the same file.
        tap.record(2, Some(43), "reaction", Dir::Recv, r#"{"jsonrpc":"2.0","method":"initialize"}"#);

        // Let the writer task drain.
        let run = crate::foundation::run::id();
        let path = crate::mind::memory::layout::session_frames_path(dir.path(), run, 42);
        let mut body = String::new();
        for _ in 0..100 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            if let Ok(text) = std::fs::read_to_string(&path)
                && text.lines().count() >= 2
            {
                body = text;
                break;
            }
        }
        assert!(!body.is_empty(), "nothing was written to {}", path.display());

        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2, "both directions are kept, not just the interesting one");

        let first: Value = serde_json::from_str(lines[0]).expect("a json object per line");
        assert_eq!(first["role"], "reaction");
        assert_eq!(first["dir"], "recv");
        assert_eq!(first["thread_id"], "s1", "grouping metadata is parsed out beside the line");
        // ...and the line itself is untouched, payload and all.
        assert_eq!(first["raw"].as_str().unwrap(), tool_call);
        let inner: Value = serde_json::from_str(first["raw"].as_str().unwrap()).unwrap();
        assert_eq!(inner["params"]["item"]["arguments"]["path"], "/etc/hosts");
        assert_eq!(inner["params"]["item"]["result"]["content"], "127.0.0.1");

        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["dir"], "stderr");

        // Session 43's frame went to session 43's file, not into 42's.
        let other = crate::mind::memory::layout::session_frames_path(dir.path(), run, 43);
        let other_body = std::fs::read_to_string(&other).expect("session 43 has its own file");
        assert_eq!(other_body.lines().count(), 1);
        assert!(other_body.contains("initialize"), "the handshake is kept too: {other_body}");
    }

    /// A connection with no agent session id has nothing durable to be filed as. It must
    /// still reach the inspector, and must not invent a bucket on disk.
    #[tokio::test]
    async fn a_frame_with_no_session_is_seen_but_not_filed() {
        let dir = tempfile::tempdir().unwrap();
        let tap = WireTap::with_durable_log(dir.path().to_path_buf());
        tap.record(9, None, "reaction", Dir::Recv, r#"{"jsonrpc":"2.0"}"#);
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;

        let (backlog, _live) = tap.subscribe();
        assert_eq!(backlog.len(), 1, "the inspector still sees it");
        let sessions = crate::mind::memory::layout::raw_root(dir.path()).join("sessions");
        assert!(!sessions.exists(), "nothing was invented on disk");
    }

    /// A tap with nowhere to write must not pretend, and must not panic.
    #[test]
    fn an_inspector_only_tap_keeps_nothing() {
        let tap = WireTap::new();
        tap.record(1, Some(1), "reaction", Dir::Recv, r#"{"jsonrpc":"2.0"}"#);
        let (backlog, _live) = tap.subscribe();
        assert_eq!(backlog.len(), 1, "still an inspector window");
    }

    #[test]
    fn parses_thread_id_from_params() {
        let line = r#"{"id":3,"method":"turn/start","params":{"threadId":"thr-abc","input":[]}}"#;
        let (tid, method, id) = parse_meta(line);
        assert_eq!(tid.as_deref(), Some("thr-abc"));
        assert_eq!(method.as_deref(), Some("turn/start"));
        assert_eq!(id, Some(serde_json::json!(3)));
    }

    /// The id first exists in `thread/start`'s response, where it is nested under
    /// `thread` rather than sitting flat like the request's `threadId`.
    #[test]
    fn parses_thread_id_from_the_thread_start_result() {
        let line = r#"{"id":1,"result":{"thread":{"id":"thr-xyz","ephemeral":true}}}"#;
        let (tid, method, _) = parse_meta(line);
        assert_eq!(tid.as_deref(), Some("thr-xyz"));
        assert_eq!(method, None, "responses carry no method");
    }

    #[test]
    fn parses_thread_id_from_a_thread_started_notification() {
        let line = r#"{"method":"thread/started","params":{"thread":{"id":"thr-n"}}}"#;
        let (tid, method, _) = parse_meta(line);
        assert_eq!(tid.as_deref(), Some("thr-n"));
        assert_eq!(method.as_deref(), Some("thread/started"));
    }

    #[test]
    fn non_json_line_is_all_none_but_still_recordable() {
        let (tid, method, id) = parse_meta("thread 'main' panicked at whatever");
        assert!(tid.is_none() && method.is_none() && id.is_none());
    }

    #[tokio::test]
    async fn subscribe_replays_then_streams_live() {
        let tap = WireTap::new();
        tap.record(0, Some(7), "cognition", Dir::Send, r#"{"method":"initialize","id":0}"#);
        let (replay, mut rx) = tap.subscribe();
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].seq, 1);
        assert_eq!(replay[0].conn, 0);
        assert_eq!(replay[0].dir, Dir::Send);

        tap.record(0, Some(7), "cognition", Dir::Recv, r#"{"id":0,"result":{}}"#);
        let live = rx.recv().await.unwrap();
        assert_eq!(live.seq, 2, "live frame follows replay with no gap or dup");
        assert_eq!(live.dir, Dir::Recv);
    }
}

/// Append every frame to its **session's** file, verbatim, one JSON object per line.
///
/// Batches whatever is already queued and groups it by session, so a busy turn costs one
/// open-and-write per session rather than one per line. Frames arrive in order and a
/// session's frames are contiguous in practice, but grouping does not assume that — a
/// batch spanning two sessions writes each to its own file.
///
/// Failures are logged and the loop continues. Losing the log must never take the agent
/// down with it, and a disk that has stopped accepting writes is not something a retry
/// here can fix.
async fn write_frames(data_dir: PathBuf, mut rx: mpsc::UnboundedReceiver<RawFrame>) {
    use std::collections::BTreeMap;
    use tokio::io::AsyncWriteExt as _;

    let run = crate::foundation::run::id();
    let mut batch: Vec<RawFrame> = Vec::new();
    while let Some(first) = rx.recv().await {
        batch.push(first);
        while let Ok(more) = rx.try_recv() {
            batch.push(more);
        }

        let mut by_session: BTreeMap<u64, String> = BTreeMap::new();
        for frame in batch.drain(..) {
            let Some(session) = frame.agent_session else { continue };
            match serde_json::to_string(&frame) {
                Ok(line) => {
                    let buf = by_session.entry(session).or_default();
                    buf.push_str(&line);
                    buf.push('\n');
                }
                Err(err) => {
                    tracing::error!(error = %err, seq = frame.seq, "frame would not serialize");
                }
            }
        }

        for (session, buf) in by_session {
            let path = crate::mind::memory::layout::session_frames_path(&data_dir, run, session);
            if let Some(parent) = path.parent()
                && let Err(err) = tokio::fs::create_dir_all(parent).await
            {
                tracing::error!(error = %err, path = %parent.display(), "cannot make the session frame dir");
                continue;
            }
            match tokio::fs::OpenOptions::new().create(true).append(true).open(&path).await {
                Ok(mut f) => {
                    if let Err(err) = f.write_all(buf.as_bytes()).await {
                        tracing::error!(error = %err, path = %path.display(), "session frame write failed");
                    }
                }
                Err(err) => {
                    tracing::error!(error = %err, path = %path.display(), "cannot open the session frame log");
                }
            }
        }
    }
}
