//! Observatory — structured visibility into the ACP session lifecycle.
//!
//! ACP sessions are otherwise invisible: a scene's persistent reactor session,
//! ephemeral worker sessions (each on its own subprocess), in-flight prompts,
//! heartbeat hot-swaps and self-alarms all live only as scattered `tracing`
//! lines. The observatory is an additive, cloneable handle (like [`Memory`] or
//! [`TextBus`]) that the reactor, workers and heartbeat feed as
//! those things happen. It keeps two things:
//!
//! - a **live mirror** — the current state per scene (reactor session, workers,
//!   context budget, pending alarms, last turn), for `GET /api/sessions`;
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

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::{Mutex, RwLock, broadcast};

use crate::types::Scene;

/// How many recent events the in-memory ring retains for SSE replay-on-connect.
const HISTORY_CAP: usize = 1000;
/// Broadcast backlog; a subscriber that lags past this misses events (logged on
/// the wire as a gap, never blocks the producer).
const BROADCAST_CAP: usize = 512;

/// Which kind of ACP session this is — the reactor's persistent mind, an
/// ephemeral worker, the throwaway summarizer a hot-swap briefs from, or the
/// reflection ("sleep") pass that consolidates raw into episodes/facets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Reactor,
    Worker,
    Summarizer,
    Reflection,
}

/// Live state of one ACP session in the mirror.
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

/// A pending self-alarm the mind scheduled, shown until it fires.
#[derive(Debug, Clone, Serialize)]
pub struct AlarmView {
    pub note: String,
    pub fires_at: DateTime<Utc>,
}

/// The most recent turn on a scene's reactor session.
#[derive(Debug, Clone, Serialize)]
pub struct TurnView {
    pub turn: u64,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub stop_reason: Option<String>,
    pub reply_chars: Option<usize>,
}

/// The full live picture of one scene, served by `GET /api/sessions`.
///
/// **Scene-shaped state only.** There is deliberately no `workers` here: a working
/// session belongs to whoever created it, and the sceneless rungs are precisely the
/// ones that create them, so a per-scene list could only ever hold the subset that
/// happened to be hosted in that scene — a number with no meaning. Worker lifecycle is
/// carried by the event log ([`EventKind::WorkerSpawned`] and friends), which is keyed
/// by session and does not have to lie about where the work lives.
#[derive(Debug, Clone, Serialize)]
pub struct SceneView {
    pub scene: Scene,
    pub reactor_session: Option<SessionView>,
    /// Accumulated prompt+reply chars since the live session was last opened/swapped.
    pub budget_chars: usize,
    /// The soft ceiling at which the heartbeat hot-swaps (mirrors `SWAP_AFTER_CHARS`).
    pub swap_after_chars: usize,
    pub swap_count: u64,
    pub last_swap_at: Option<DateTime<Utc>>,
    pub pending_alarms: Vec<AlarmView>,
    pub last_turn: Option<TurnView>,
    pub turns_total: u64,
}

impl SceneView {
    fn new(scene: Scene, swap_after_chars: usize) -> Self {
        Self {
            scene,
            reactor_session: None,
            budget_chars: 0,
            swap_after_chars,
            swap_count: 0,
            last_swap_at: None,
            pending_alarms: Vec::new(),
            last_turn: None,
            turns_total: 0,
        }
    }
}

/// One lifecycle event — the unit of the SSE stream, the ring, and `sessions.jsonl`.
#[derive(Debug, Clone, Serialize)]
pub struct SessionEvent {
    /// Monotonic, gap-free sequence number assigned at record time.
    pub seq: u64,
    pub ts: DateTime<Utc>,
    /// The conversation this happened in, or `None` when it happened outside every
    /// conversation — Cognition and Reflection have no scene, and inventing one for
    /// them is not free: the mirror keys on scene, so a sentinel would render as a
    /// conversation in the dashboard that nobody is having.
    pub scene: Option<Scene>,
    #[serde(flatten)]
    pub kind: EventKind,
}

/// The shape of each lifecycle event. Serialized with an `"event"` tag so the
/// wire form is `{ "seq", "ts", "scene", "event": "...", ...fields }`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EventKind {
    SessionOpened { kind: SessionKind, id: String },
    SessionClosed { kind: SessionKind, id: String },
    /// `input` is the human-readable incoming message(s) for this turn — the new
    /// signals batch (human utterances, worker reports, fired alarms), not the
    /// full seeded prompt.
    TurnStarted { turn: u64, input: String },
    /// `reply` is the agent's spoken text for this turn (markers stripped).
    TurnFinished { turn: u64, stop_reason: Option<String>, reply_chars: usize, reply: String },
    HotSwap { old_id: String, new_id: String, briefing_chars: usize },
    WorkerSpawned { id: u64, task: String },
    /// A warm (finished-but-idle) worker was handed a follow-up task and is running
    /// again on the same session.
    WorkerResumed { id: u64, task: String },
    WorkerFinished { id: u64, state: WorkerState, summary_chars: usize },
    AlarmScheduled { note: String, delay_s: u64 },
    AlarmFired { note: String },
}

/// Cloneable handle over the shared observatory state.
#[derive(Clone)]
pub struct Observatory {
    inner: Arc<Inner>,
}

struct Inner {
    scenes: RwLock<HashMap<Scene, SceneView>>,
    history: Mutex<History>,
    tx: broadcast::Sender<SessionEvent>,
    /// Where to append the durable event log, or `None` to skip persistence.
    jsonl: Option<PathBuf>,
    swap_after_chars: usize,
}

struct History {
    seq: u64,
    ring: VecDeque<SessionEvent>,
}

impl Observatory {
    /// Build an observatory. `jsonl` is where to append durable events (created
    /// lazily on first append); pass `None` to keep history in-memory only.
    /// `swap_after_chars` is the heartbeat's swap ceiling, surfaced per scene so
    /// the dashboard can render the budget as a fraction.
    pub fn new(jsonl: Option<PathBuf>, swap_after_chars: usize) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAP);
        Self {
            inner: Arc::new(Inner {
                scenes: RwLock::new(HashMap::new()),
                history: Mutex::new(History { seq: 0, ring: VecDeque::new() }),
                tx,
                jsonl,
                swap_after_chars,
            }),
        }
    }

    /// Record one lifecycle event: assign a seq, mutate the live mirror, push to
    /// the ring, append to the journal, and broadcast — the ring push and the
    /// broadcast both happen under the history lock so a concurrent
    /// [`subscribe`](Self::subscribe) sees a consistent, dup-free cut.
    ///
    /// `scene: None` records the event in history without touching the per-scene
    /// mirror — for the sceneless rungs, whose events are real history but describe no
    /// conversation. History takes everything; the mirror only takes what it can key.
    pub async fn record(&self, scene: Option<&Scene>, kind: EventKind) {
        // Mirror first (its own lock), so a snapshot taken right after the event
        // lands reflects it.
        self.apply_to_mirror(scene, &kind).await;

        let mut hist = self.inner.history.lock().await;
        hist.seq += 1;
        let event = SessionEvent {
            seq: hist.seq,
            ts: Utc::now(),
            scene: scene.cloned(),
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

    /// A live snapshot of every scene, newest-process-first is not meaningful —
    /// scenes are returned in arbitrary map order; the dashboard sorts by name.
    pub async fn snapshot(&self) -> Vec<SceneView> {
        self.inner.scenes.read().await.values().cloned().collect()
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

    /// Update a scene's accumulated context budget (mirror-only; not an event —
    /// it changes every turn and matters as state, not as history).
    pub async fn set_budget(&self, scene: &Scene, chars: usize) {
        let mut scenes = self.inner.scenes.write().await;
        scenes.entry(scene.clone()).or_insert_with(|| self.fresh(scene)).budget_chars = chars;
    }

    fn fresh(&self, scene: &Scene) -> SceneView {
        SceneView::new(scene.clone(), self.inner.swap_after_chars)
    }

    /// Fold an event into the live mirror. Pure state transition; no I/O.
    ///
    /// A sceneless event folds into nothing — and returns **before** the map entry is
    /// touched, which is the whole point: `entry().or_insert_with()` materializes a
    /// `SceneView` whether or not any arm below uses it, so reaching this function with
    /// a placeholder scene is enough to put a conversation on the dashboard that does
    /// not exist.
    async fn apply_to_mirror(&self, scene: Option<&Scene>, kind: &EventKind) {
        let Some(scene) = scene else { return };
        let now = Utc::now();
        let mut scenes = self.inner.scenes.write().await;
        let view = scenes
            .entry(scene.clone())
            .or_insert_with(|| SceneView::new(scene.clone(), self.inner.swap_after_chars));

        match kind {
            EventKind::SessionOpened { kind, id } => match kind {
                SessionKind::Reactor => {
                    view.reactor_session = Some(SessionView {
                        id: id.clone(),
                        kind: SessionKind::Reactor,
                        opened_at: now,
                        in_flight: false,
                        turns: 0,
                    });
                }
                // Worker open is mirrored by WorkerSpawned; the summarizer and
                // reflection passes are throwaways we don't surface as standing
                // sessions.
                SessionKind::Worker | SessionKind::Summarizer | SessionKind::Reflection => {}
            },
            EventKind::SessionClosed { .. } => {
                // Reactor close is rare (only error teardown); workers are removed
                // on WorkerFinished. Nothing to do for the summarizer.
            }
            EventKind::TurnStarted { turn, .. } => {
                if let Some(s) = view.reactor_session.as_mut() {
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
                if let Some(s) = view.reactor_session.as_mut() {
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
            EventKind::HotSwap { new_id, .. } => {
                view.swap_count += 1;
                view.last_swap_at = Some(now);
                view.budget_chars = 0;
                view.reactor_session = Some(SessionView {
                    id: new_id.clone(),
                    kind: SessionKind::Reactor,
                    opened_at: now,
                    in_flight: false,
                    turns: 0,
                });
            }
            // Worker lifecycle is history, not scene state — see [`SceneView`]. A
            // working session is keyed by its own id and owned by whoever asked for it,
            // so folding it into whichever scene happened to record the event would
            // answer a question nobody asked. Read these off the event log.
            EventKind::WorkerSpawned { .. }
            | EventKind::WorkerResumed { .. }
            | EventKind::WorkerFinished { .. } => {}
            EventKind::AlarmScheduled { note, delay_s } => {
                view.pending_alarms.push(AlarmView {
                    note: note.clone(),
                    fires_at: now + chrono::Duration::seconds(*delay_s as i64),
                });
            }
            EventKind::AlarmFired { note } => {
                // Drop the earliest pending alarm matching this note.
                if let Some(idx) = view.pending_alarms.iter().position(|a| &a.note == note) {
                    view.pending_alarms.remove(idx);
                }
            }
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

    fn scene() -> Scene {
        Scene("alice@phone".to_string())
    }

    #[tokio::test]
    async fn mirrors_reactor_session_and_turn() {
        let obs = Observatory::new(None, 48_000);
        let s = scene();
        obs.record(
            Some(&s),
            EventKind::SessionOpened { kind: SessionKind::Reactor, id: "sess-1".into() },
        )
        .await;
        obs.record(Some(&s), EventKind::TurnStarted { turn: 0, input: "hi".into() }).await;

        let snap = obs.snapshot().await;
        assert_eq!(snap.len(), 1);
        let v = &snap[0];
        let rs = v.reactor_session.as_ref().unwrap();
        assert_eq!(rs.id, "sess-1");
        assert!(rs.in_flight, "turn in flight");

        obs.record(
            Some(&s),
            EventKind::TurnFinished {
                turn: 0,
                stop_reason: Some("end_turn".into()),
                reply_chars: 42,
                reply: "hello there".into(),
            },
        )
        .await;
        let v = &obs.snapshot().await[0];
        assert!(!v.reactor_session.as_ref().unwrap().in_flight);
        assert_eq!(v.turns_total, 1);
        assert_eq!(v.last_turn.as_ref().unwrap().reply_chars, Some(42));
    }

    #[tokio::test]
    async fn hot_swap_resets_budget_and_bumps_count() {
        let obs = Observatory::new(None, 48_000);
        let s = scene();
        obs.record(
            Some(&s),
            EventKind::SessionOpened { kind: SessionKind::Reactor, id: "old".into() },
        )
        .await;
        obs.set_budget(&s, 50_000).await;
        obs.record(
            Some(&s),
            EventKind::HotSwap { old_id: "old".into(), new_id: "new".into(), briefing_chars: 800 },
        )
        .await;
        let v = &obs.snapshot().await[0];
        assert_eq!(v.swap_count, 1);
        assert_eq!(v.budget_chars, 0);
        assert_eq!(v.reactor_session.as_ref().unwrap().id, "new");
    }

    /// Worker lifecycle is history, and *only* history. It used to fold into a
    /// per-scene `workers` vec — a list that could only ever hold the workers hosted
    /// in that scene, which after the pool moved process-wide meant none of them.
    #[tokio::test]
    async fn worker_lifecycle_is_history_not_scene_state() {
        let obs = Observatory::new(None, 48_000);
        let s = scene();
        obs.record(Some(&s), EventKind::WorkerSpawned { id: 1, task: "research X".into() }).await;
        obs.record(
            Some(&s),
            EventKind::WorkerFinished { id: 1, state: WorkerState::Done, summary_chars: 120 },
        )
        .await;

        let (replay, _rx) = obs.subscribe().await;
        assert_eq!(replay.len(), 2, "both events are in history");
        assert_eq!(replay[0].scene.as_ref(), Some(&s));
        assert_eq!(obs.event_count().await, 2);
    }

    /// A sceneless rung's events are real history and must not invent a conversation.
    /// The trap is `apply_to_mirror`'s `entry().or_insert_with()`: it materializes a
    /// `SceneView` before any arm runs, so merely *passing* a placeholder scene put a
    /// row on the dashboard — no arm had to use it.
    #[tokio::test]
    async fn a_sceneless_event_is_recorded_and_mirrors_nothing() {
        let obs = Observatory::new(None, 48_000);
        obs.record(
            None,
            EventKind::SessionOpened { kind: SessionKind::Reflection, id: "refl-1".into() },
        )
        .await;

        assert!(obs.snapshot().await.is_empty(), "no scene was invented");
        let (replay, _rx) = obs.subscribe().await;
        assert_eq!(replay.len(), 1, "but it is still history");
        assert!(replay[0].scene.is_none());
    }

    #[tokio::test]
    async fn subscribe_replays_then_streams_live_without_dup() {
        let obs = Observatory::new(None, 48_000);
        let s = scene();
        obs.record(
            Some(&s),
            EventKind::SessionOpened { kind: SessionKind::Reactor, id: "sess-1".into() },
        )
        .await;
        let (replay, mut rx) = obs.subscribe().await;
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].seq, 1);

        obs.record(
            Some(&s),
            EventKind::SessionClosed { kind: SessionKind::Reactor, id: "sess-1".into() },
        )
        .await;
        let live = rx.recv().await.unwrap();
        assert_eq!(live.seq, 2, "live event follows replay with no gap or dup");
    }

    #[tokio::test]
    async fn alarms_scheduled_and_fired() {
        let obs = Observatory::new(None, 48_000);
        let s = scene();
        obs.record(Some(&s), EventKind::AlarmScheduled { note: "wake them".into(), delay_s: 60 })
            .await;
        assert_eq!(obs.snapshot().await[0].pending_alarms.len(), 1);
        obs.record(Some(&s), EventKind::AlarmFired { note: "wake them".into() }).await;
        assert!(obs.snapshot().await[0].pending_alarms.is_empty());
    }
}
