//! Reactor — the *mind*. Per-scene queues + one persistent session per scene.
//!
//! One mpsc per scene, one task per scene; turns run serially against a single
//! ACP session that is opened on the scene's first turn and reused forever as
//! the scene's continuous mind. Deliberation is delegated to that session; the
//! reactor never blocks on it.
//!
//! ## Turn-taking lives here, not in the client
//!
//! The client is a dumb face: it streams the mic and renders what arrives. It
//! does not decide *when* the agent speaks — the mind does, and these are the
//! two rules:
//!
//! 1. **Commit-after-quiet.** A finalized utterance does not immediately make
//!    the agent reply. The human often speaks in bursts; each burst arrives as
//!    its own inbound signal (one segmented utterance over `/api/in/audio`), and the mind
//!    waits until no new signal has landed for a short settle before it
//!    responds, absorbing every burst in the meantime into one consolidated
//!    prompt. The cost is a little latency; the win is that the agent doesn't
//!    answer a half-finished thought, and nothing the human says is lost.
//!    Because the reply only starts once things have gone quiet, its output can
//!    stream straight to the client — no holding, no turn-tagging on the wire;
//!    superseded drafts are *never generated* rather than generated-then-discarded.
//! 2. **Fix-forward, no reflexive cancel.** A new signal never cancels the
//!    in-flight prompt. The per-scene loop is serial — it runs one turn to
//!    completion before draining the next batch — so a signal that lands during
//!    generation simply queues and is folded into the next turn. The warm
//!    session remembers fragments it chose not to act on yet, so a thought spread
//!    across several bursts reassembles across turns; the mind corrects course
//!    rather than being cut off. (The client mutes its own speaker reflexively the
//!    instant its mic goes hot, so an interruption feels instant regardless.)
//!    A voice barge-in — the human talking over the agent's playback — is no
//!    exception: the client ducks on its own, the words buffer like any other
//!    signal, and the mind merely learns afterwards what went unheard. See
//!    [`interrupts`].
//!
//! ## Heavy work goes to a working session, not onto the floor
//!
//! The mind keeps a single voice, so it must never block the floor on slow
//! work. When a turn needs research, multi-step tool use, or anything
//! long-running, the mind calls the `delegate` tool with the task; the reactor
//! spawns a channel-mute [`workers`] session for it and keeps talking. The worker
//! runs with the same substrate (memory, tools) but no voice of its own, and
//! posts its result — or a question, if it gets stuck — back into this scene's
//! queue, where it lands as just another input the next turn folds into what the
//! mind says.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

mod heartbeat;
mod interleave;
mod interrupts;
pub mod outbound;
mod sequencer;
mod tools;
mod workers;

pub use interrupts::InterruptRegistry;
pub use outbound::OutboundSignal;
pub use tools::{SceneControl, ToolRegistry, ToolSink};

/// The heartbeat's soft context-budget ceiling, surfaced so the observatory can
/// render each scene's budget as a fraction of where a hot-swap kicks in.
pub fn swap_budget_chars() -> usize {
    heartbeat::swap_after_chars()
}

use chrono::Utc;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::time::{Instant, sleep_until, timeout};

use crate::foundation::acp::{AcpSession, SessionOpts, SessionUpdate};
use crate::foundation::agent::{AgentLayer, SessionRole};
use crate::foundation::config;
use crate::foundation::registry;
use crate::foundation::shutdown::Shutdown;
use crate::mind::memory::{Memory, layout, snapshot};
use crate::foundation::observatory::{EventKind, Observatory, SessionKind};
use crate::types::{Channel, Geometry, JournalEntry, Origin, Scene, Signal, ViewEnvelope, ViewOp};
use bytes::Bytes;
use uuid::Uuid;

/// How long the floor must stay quiet after the last finalized utterance before
/// the mind commits to replying. The human-interface tradeoff knob: higher =
/// more patient (never talks over a multi-burst thought) but more latency;
/// lower = snappier but more likely to answer a half-finished thought. Paired
/// with the client VAD's `endSilenceMs`, which governs how fast an utterance is
/// *finalized* (and POSTed); this governs how long we wait to see if another one
/// follows.
const RESPONSE_SETTLE: Duration = Duration::from_millis(700);

/// Ceiling on a between-turns hot-swap. The swap prompts the *live* session for a
/// self-briefing with unbounded awaits beneath it; if that session is wedged (a
/// pathological turn can leave the subprocess unresponsive), an un-capped swap
/// blocks the scene loop forever — signals keep queueing but no turn ever runs,
/// and the scene goes deaf until a restart. On expiry the session is discarded:
/// it ignored a prompt for this whole window, so the journal cold-open path is
/// strictly better than waiting.
const SWAP_TIMEOUT: Duration = Duration::from_secs(180);

/// Default idle interval between host pulses — the scene's recurring moment of
/// self-attention. A pulse is not a schedule of work: it injects bare situational
/// facts ("nothing new for 30m") and core.md tells the mind what such a moment is
/// for (read down its open tasks, glance at setups it owns); most pulses should
/// conclude with nothing to do or say. Override via `pulse`; `0`/`off`
/// disables. Boot is not a special case — the first pulse after the host starts
/// simply carries that fact.
const DEFAULT_PULSE: Duration = Duration::from_secs(1800);

/// Resolve the pulse interval from the stored `pulse` tunable in alarm-delay grammar
/// if set (`None` for `0`/`off` — pulses disabled), else [`DEFAULT_PULSE`].
fn pulse_interval() -> Option<Duration> {
    duration_tunable(config::tunables::get(config::KEY_PULSE), DEFAULT_PULSE)
}

/// Whether the reflection ("sleep") pass runs at all. On unless the stored `reflect`
/// tunable is `off` — a master escape hatch to disable consolidation without
/// touching the cadence (see [`reflect_interval`]).
fn reflect_enabled() -> bool {
    !config::tunables::get(config::KEY_REFLECT)
        .map(|v| v.eq_ignore_ascii_case("off"))
        .unwrap_or(false)
}

/// Default base reflection cadence — how often a scene with fresh input
/// consolidates ([`reflect_interval`]). The idle backoff grows from here.
const DEFAULT_REFLECT_EVERY: Duration = Duration::from_secs(60);
/// Default ceiling on the idle backoff ([`reflect_max_interval`]): a long-quiet
/// scene re-checks at most this often.
const DEFAULT_REFLECT_MAX: Duration = Duration::from_secs(8 * 3600);

/// Resolve a stored duration tunable in alarm-delay grammar (`90s`/`30m`/`1h`; bare
/// integer = seconds): `None` for `off`/`0` (disabled), the parsed value, or
/// `default` when unset / unparseable. (The value is already trimmed / non-empty by
/// [`config::tunables::get`].)
fn duration_tunable(value: Option<String>, default: Duration) -> Option<Duration> {
    match value {
        None => Some(default),
        Some(v) if v.eq_ignore_ascii_case("off") => None,
        Some(v) => match parse_delay(&v) {
            Some(d) if d.is_zero() => None,
            Some(d) => Some(d),
            None => Some(default),
        },
    }
}

/// The base reflection cadence, or `None` if reflection is off
/// (`reflect=off`) or `reflect_every` is `0`/`off`. A scene with
/// fresh input consolidates this often; once it goes quiet the gap backs off from
/// here up to [`reflect_max_interval`].
fn reflect_interval() -> Option<Duration> {
    reflect_enabled()
        .then(|| duration_tunable(config::tunables::get(config::KEY_REFLECT_EVERY), DEFAULT_REFLECT_EVERY))
        .flatten()
}

/// The ceiling on the idle backoff: a caught-up, quiet scene doubles its gap from
/// the base each pass but never past this. Always returns a value (no `off`); a
/// `0`/blank `reflect_max` falls back to the default.
fn reflect_max_interval() -> Duration {
    duration_tunable(config::tunables::get(config::KEY_REFLECT_MAX), DEFAULT_REFLECT_MAX)
        .unwrap_or(DEFAULT_REFLECT_MAX)
}

/// Default base gap for a transient-outage retry (429 / generic). The gap doubles
/// on each failed retry toward [`BACKOFF_CAP`]; 30s is unobtrusive and won't hammer
/// a throttled gateway.
const DEFAULT_VENDOR_PROBE: Duration = Duration::from_secs(30);
/// Default consecutive *generic* terminal failures before flipping to an informed
/// backoff. Each terminal failure is already up to 3 model calls, so 2 = a real
/// outage, not a one-off blip. (402/429 bypass this — they flip immediately.)
const DEFAULT_VENDOR_DOWN_AFTER: u32 = 2;
/// A transient-outage retry never waits longer than this — the 1h ceiling.
const BACKOFF_CAP: Duration = Duration::from_secs(3600);

/// The base transient-outage retry gap. `vendor_probe` in alarm-delay grammar;
/// `off`/`0`/unset/unparseable → default. (Kept under the historical config key.)
fn backoff_base() -> Duration {
    duration_tunable(config::tunables::get(config::KEY_VENDOR_PROBE), DEFAULT_VENDOR_PROBE)
        .unwrap_or(DEFAULT_VENDOR_PROBE)
}

/// The consecutive generic-failure count that flips the reactor into an informed
/// backoff. `vendor_down_after`; `0`/unparseable → default.
fn vendor_down_after() -> u32 {
    config::tunables::get(config::KEY_VENDOR_DOWN_AFTER)
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_VENDOR_DOWN_AFTER)
}

/// How a scene loop should treat the vendor right now — the read side of [`Vendor`].
#[derive(Clone, Copy, Debug)]
enum SceneGate {
    /// Reachable: drive turns normally.
    Go,
    /// Transient outage (429 / generic): hold mail, and drive a catch-up turn once
    /// `at` (the current backoff deadline) passes. A failed retry grows the gap.
    Retry { at: Instant },
}

/// The vendor's reachability and, when down, how to recover from it.
#[derive(Clone, Copy, Debug)]
enum VendorState {
    Up,
    /// Transient backoff (429 / generic). `try_at` is the next retry deadline;
    /// `attempt` grows the gap toward [`BACKOFF_CAP`]; `silent` suppresses the user
    /// notice for a pure rate-limit (429), which the user needn't hear about.
    Backoff { try_at: Instant, attempt: u32, silent: bool },
}

/// Shared, process-wide view of the upstream LLM vendor and how to recover from an
/// outage. Every scene loop reads it (via [`Vendor::scene_gate`]) to decide whether
/// and when to drive a turn; `run_turn`'s terminal path writes it. The vendor is a
/// shared resource, so one scene detecting an outage steers all of them.
///
/// The `note_*` writers return whether the transition warrants a *one-time* user
/// notice (so the reactor announces "can't reach the model" exactly once),
/// mirroring the old flip-once contract.
struct Vendor {
    state: std::sync::Mutex<VendorState>,
    /// Consecutive *generic* (Unreachable) failures, to absorb a blip before an
    /// informed backoff. Accessed only under `state`'s lock, so effectively part of
    /// the same critical section. Reset on success.
    generic_failures: AtomicU32,
    down_after: u32,
    /// The transient-outage retry base; the gap is `base · 2^attempt`, capped at 1h.
    base: Duration,
}

impl Vendor {
    fn new(down_after: u32, base: Duration) -> Self {
        Self {
            state: std::sync::Mutex::new(VendorState::Up),
            generic_failures: AtomicU32::new(0),
            down_after,
            base,
        }
    }

    fn is_down(&self) -> bool {
        !matches!(*self.state.lock().unwrap(), VendorState::Up)
    }

    /// The retry gap for the given attempt: `base · 2^attempt`, capped at 1h.
    fn backoff(&self, attempt: u32) -> Duration {
        let base = self.base.as_secs().max(1);
        let secs = base.saturating_mul(1u64 << attempt.min(20));
        Duration::from_secs(secs.min(BACKOFF_CAP.as_secs()))
    }

    /// The scene loop's scheduling read: drive now (Go) or retry at a deadline (Retry).
    fn scene_gate(&self) -> SceneGate {
        match *self.state.lock().unwrap() {
            VendorState::Up => SceneGate::Go,
            VendorState::Backoff { try_at, .. } => SceneGate::Retry { at: try_at },
        }
    }

    /// Terminal generic outage. Absorb one blip via `down_after`, then flip to an
    /// *informed* backoff. Returns `true` exactly on that flip (announce once);
    /// `false` while still absorbing or already backing off.
    fn note_unreachable(&self) -> bool {
        let mut st = self.state.lock().unwrap();
        match *st {
            // Already backing off — a failed retry just grows the gap.
            VendorState::Backoff { attempt, silent, .. } => {
                let a = attempt.saturating_add(1);
                *st = VendorState::Backoff { try_at: Instant::now() + self.backoff(a), attempt: a, silent };
                false
            }
            VendorState::Up => {
                let n = self.generic_failures.fetch_add(1, Ordering::Relaxed) + 1;
                if n >= self.down_after {
                    *st = VendorState::Backoff { try_at: Instant::now() + self.backoff(0), attempt: 0, silent: false };
                    true
                } else {
                    false
                }
            }
        }
    }

    /// A turn (or retry) succeeded. Flip Up and reset the blip counter. Returns
    /// `true` if this ended an outage (so the caller logs the recovery).
    fn note_success(&self) -> bool {
        let mut st = self.state.lock().unwrap();
        self.generic_failures.store(0, Ordering::Relaxed);
        let was_down = !matches!(*st, VendorState::Up);
        *st = VendorState::Up;
        was_down
    }
}

/// A short human phrase for when the balance refills ("大约 3 小时后", "约 20 分钟后"),
/// or a vague fallback when the reset time is unknown.
pub(crate) fn humanize_until_reset(resets_at: &str) -> String {
    let Ok(reset) = chrono::DateTime::parse_from_rfc3339(resets_at.trim()) else {
        return "过一会儿".to_string();
    };
    let mins = (reset.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_minutes();
    if mins <= 1 {
        "很快就".to_string()
    } else if mins < 60 {
        format!("约 {mins} 分钟后")
    } else {
        let hours = (mins as f64 / 60.0).round() as i64;
        format!("大约 {hours} 小时后")
    }
}

#[cfg(test)]
mod vendor_tests {
    use super::*;

    fn fresh() -> Vendor {
        Vendor::new(2, Duration::from_secs(30))
    }

    #[test]
    fn starts_up() {
        assert!(!fresh().is_down());
    }

    #[test]
    fn generic_outage_absorbs_a_blip_then_informs_once() {
        let v = fresh();
        assert!(!v.note_unreachable(), "first generic failure is absorbed (down_after = 2)");
        assert!(!v.is_down(), "still reachable after one blip");
        assert!(v.note_unreachable(), "the second flips to an informed backoff, announced once");
        assert!(v.is_down());
        assert!(matches!(v.scene_gate(), SceneGate::Retry { .. }));
        assert!(!v.note_unreachable(), "a failed retry grows the backoff without re-announcing");
    }

    #[test]
    fn success_resets_the_blip_counter() {
        let v = fresh();
        v.note_unreachable(); // one blip, still up
        v.note_success(); // resets the counter
        assert!(!v.note_unreachable(), "one blip after a reset must not flip");
        assert!(!v.is_down());
    }

    #[test]
    fn threshold_one_flips_on_first_generic_failure() {
        let v = Vendor::new(1, Duration::from_secs(30));
        assert!(v.note_unreachable());
        assert!(v.is_down());
    }

    #[test]
    fn backoff_grows_and_caps_at_one_hour() {
        let v = fresh(); // base 30s
        assert_eq!(v.backoff(0), Duration::from_secs(30));
        assert_eq!(v.backoff(1), Duration::from_secs(60));
        assert_eq!(v.backoff(2), Duration::from_secs(120));
        assert_eq!(v.backoff(100), BACKOFF_CAP, "never exceeds the 1h cap");
    }
}

/// The soonest a reflection may fire for a scene, or `None` when reflection is
/// disabled (`base` is `None`). One adaptive clock, anchored on the **last
/// reflection** (or `loop_started` before the first) so a never-idle scene still
/// fires every `base`:
/// - **fresh input** since the anchor (`last_activity > anchor`) → fire `base`
///   after the anchor — the active ~1/`base` cadence;
/// - **caught up and quiet** (`last_activity <= anchor`) → fire `backoff_gap`
///   after the anchor, where `backoff_gap` has been doubling toward the cap.
///
/// `backoff_gap` is the loop's running idle gap (reset to `base` whenever a pass
/// runs with fresh input, doubled toward the cap when one runs while quiet); this
/// function just reads it. Activity after a long idle re-anchors on the old
/// reflection, so the next pass is due immediately — fine, it's a detached session
/// and an under-`MIN_REFLECT_SIGNALS` frontier no-ops cheaply.
fn next_reflection_at(
    loop_started: Instant,
    last_activity: Instant,
    last_reflection: Option<Instant>,
    base: Option<Duration>,
    backoff_gap: Duration,
) -> Option<Instant> {
    let base = base?;
    let anchor = last_reflection.unwrap_or(loop_started);
    let gap = if last_activity > anchor { base } else { backoff_gap };
    Some(anchor + gap)
}

#[cfg(test)]
mod reflection_schedule_tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn fresh_input_fires_at_base_after_the_anchor() {
        let t0 = Instant::now();
        // Never reflected; a turn landed at t0+30s. Anchor is loop_start (t0), and
        // fresh input since then → fire base (60s) after the anchor.
        let at = next_reflection_at(t0, t0 + secs(30), None, Some(secs(60)), secs(60));
        assert_eq!(at, Some(t0 + secs(60)));
    }

    #[test]
    fn busy_scene_fires_base_after_the_last_reflection() {
        let t0 = Instant::now();
        // Reflected at t0+60s; a later turn keeps activity ahead of the anchor, so
        // the next pass is base after the *reflection*, not pushed out by activity.
        let last_reflection = t0 + secs(60);
        let at = next_reflection_at(t0, t0 + secs(90), Some(last_reflection), Some(secs(60)), secs(60));
        assert_eq!(at, Some(last_reflection + secs(60)));
    }

    #[test]
    fn quiet_scene_uses_the_backed_off_gap() {
        let t0 = Instant::now();
        // Reflected at t0+60s, nothing since (activity at t0 < anchor) → fire the
        // backoff gap (already doubled to 240s) after the anchor.
        let last_reflection = t0 + secs(60);
        let at = next_reflection_at(t0, t0, Some(last_reflection), Some(secs(60)), secs(240));
        assert_eq!(at, Some(last_reflection + secs(240)));
    }

    #[test]
    fn new_input_after_long_idle_is_due_immediately() {
        let t0 = Instant::now();
        // Long idle: anchor is an hour-old reflection, gap backed off to 8h. A turn
        // just landed → fresh input → due `base` after the *old* anchor, i.e. in the
        // past, so the loop fires it on the next tick.
        let last_reflection = t0;
        let at = next_reflection_at(t0, t0 + secs(3600), Some(last_reflection), Some(secs(60)), secs(8 * 3600));
        assert_eq!(at, Some(last_reflection + secs(60)));
        assert!(at.unwrap() < t0 + secs(3600));
    }

    #[test]
    fn disabled_when_base_is_off() {
        let t0 = Instant::now();
        assert_eq!(next_reflection_at(t0, t0, None, None, secs(60)), None);
    }
}

/// How far back a scene's raw memory may date and still count as "recently active"
/// for the consolidated reflection pass. (Re-warming at startup uses the tighter
/// [`REWARM_MAX_IDLE`] gate instead — see [`scenes_to_rewarm`].)
const REWARM_WINDOW: Duration = Duration::from_secs(7 * 24 * 3600);

const SCENE_QUEUE_CAPACITY: usize = 64;

/// One item in a scene's turn queue. Both a human utterance and a worker's
/// report drive a reactor turn; they differ only in source. A human signal comes
/// through [`Reactor::deliver_to_scene`]; a worker report is posted straight into
/// the queue by the worker's drive task. Neither interrupts live speech — both
/// wait their turn and are settled into one batch.
enum LoopInput {
    Human(Signal),
    Worker(workers::WorkerReport),
    /// A self-scheduled wake firing. The mind asked for it earlier with the
    /// `alarm` tool; when its deadline passes the loop injects this so a
    /// turn runs even though no new signal arrived.
    Alarm(AlarmFired),
    /// A host pulse firing — the recurring moment of self-attention. Carries
    /// bare situational facts; what to do with such a moment is core.md's job.
    Pulse { note: String },
    /// Mail from another part of the agent, addressed to this scene. It drives a
    /// turn on its own — that is what makes a message *reach* the person rather
    /// than sit in a mailbox until they happen to say something next.
    Mail(Vec<crate::foundation::registry::Message>),
}

/// One fired self-alarm, handed to the mind under "New signals".
struct AlarmFired {
    /// The note the mind left its future self ("check if they're still asleep").
    note: String,
}

/// A scene loop's pending self-alarms. The scene wakes for one of two reasons —
/// a new signal, or the soonest of these firing. Only the mind schedules them,
/// by calling the `alarm` tool. A flat Vec is plenty: a scene has at most a
/// handful pending at once.
struct Alarms {
    pending: Vec<PendingAlarm>,
}

struct PendingAlarm {
    fire_at: Instant,
    note: String,
}

impl Alarms {
    fn new() -> Self {
        Self { pending: Vec::new() }
    }

    /// Register a wake `delay` from `now` carrying `note`.
    fn schedule(&mut self, delay: Duration, note: String, now: Instant) {
        self.pending.push(PendingAlarm { fire_at: now + delay, note });
    }

    /// The soonest pending deadline, or `None` if nothing is scheduled — the
    /// loop then blocks on the inbound queue with no timer arm at all.
    fn next_deadline(&self) -> Option<Instant> {
        self.pending.iter().map(|a| a.fire_at).min()
    }

    /// Remove and return every alarm whose deadline has passed by `now`.
    fn take_due(&mut self, now: Instant) -> Vec<AlarmFired> {
        let mut fired = Vec::new();
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].fire_at <= now {
                let a = self.pending.swap_remove(i);
                fired.push(AlarmFired { note: a.note });
            } else {
                i += 1;
            }
        }
        fired
    }
}

/// Parse an alarm delay token: a bare integer is seconds, or an integer
/// with an `s`/`m`/`h` suffix (`30s`, `20m`, `1h`). `None` for anything
/// unparseable, so a malformed alarm is dropped rather than firing at a wrong
/// time.
fn parse_delay(tok: &str) -> Option<Duration> {
    let tok = tok.trim();
    let (digits, mult) = if let Some(n) = tok.strip_suffix(|c| c == 's' || c == 'S') {
        (n, 1)
    } else if let Some(n) = tok.strip_suffix(|c| c == 'm' || c == 'M') {
        (n, 60)
    } else if let Some(n) = tok.strip_suffix(|c| c == 'h' || c == 'H') {
        (n, 3600)
    } else {
        (tok, 1)
    };
    let n: u64 = digits.trim().parse().ok()?;
    Some(Duration::from_secs(n.saturating_mul(mult)))
}

/// Register a self-alarm from the `alarm` tool's `delay`/`note` arguments. A
/// delay that won't parse is logged and dropped (fix-forward — the mind isn't
/// blocked on it).
async fn schedule_alarm(reactor: &Reactor, alarms: &mut Alarms, scene: &Scene, delay: &str, note: &str) {
    match parse_delay(delay) {
        Some(delay) => {
            alarms.schedule(delay, note.to_owned(), Instant::now());
            reactor
                .inner
                .observatory
                .record(
                    Some(scene),
                    EventKind::AlarmScheduled { note: note.to_owned(), delay_s: delay.as_secs() },
                )
                .await;
            tracing::info!(scene = %scene, delay_s = delay.as_secs(), note = %note, "alarm scheduled");
        }
        None => {
            tracing::warn!(scene = %scene, token = %delay, "ignoring alarm with unparseable delay");
        }
    }
}

#[derive(Clone)]
pub struct Reactor {
    inner: Arc<ReactorInner>,
}

struct ReactorInner {
    memory: Memory,
    agent: AgentLayer,
    /// The reactor's single outbound seam: every channel signal it produces —
    /// text, synthesized speech, views — goes out here in transport-free form
    /// (see [`outbound`]). A transport adapter binds these to a wire. The reactor
    /// has no knowledge of HTTP, `Content-Type`, or response framing.
    out: mpsc::Sender<OutboundSignal>,
    /// Structured visibility into the session lifecycle. Turn, session, swap,
    /// worker and alarm events feed it; the HTTP front serves it.
    observatory: Observatory,
    /// Compiles agent-authored view source into an ESM module the browser
    /// imports. Invoked just-in-time when a view segment is released, so the
    /// compiled module URL is what rides the /view channel.
    view_compiler: crate::mind::views::ViewCompiler,
    /// Scene→tool-sink table the `/mcp` server routes tool calls through. Each
    /// scene loop registers its sink here as it stands up; shared (cloneable)
    /// with the HTTP front. See [`tools`].
    tools: ToolRegistry,
    /// Scene→barge-in state. The STT relay reports recognized speech here; the
    /// sequencer stamps each turn's voice span; `run_turn` drains the inferred
    /// "what went unheard" note into the next prompt. See [`interrupts`].
    interrupts: InterruptRegistry,
    /// Shared, process-wide LLM-vendor reachability + recovery policy. Read by every
    /// scene loop (via [`Vendor::scene_gate`]) to decide whether and when to drive a
    /// turn; written by `run_turn`'s terminal-failure / success paths. See [`Vendor`].
    vendor: Arc<Vendor>,
    /// Scene→live-subscriber counts, shared with the HTTP front. Rendered into
    /// each turn as one human-model presence sentence, so the mind knows which
    /// channels actually reach the person right now.
    presence: crate::body::presence::Presence,
    /// The live per-scene appearance state, shared (a cloneable handle) with the
    /// HTTP front's view bus. Read into each turn as `## On screen now` so the agent
    /// can see what it has shown — the screen is its own presentation surface, and
    /// without this it dismisses/re-shows views by guessing ids from the transcript.
    /// Read-only here: views are still *emitted* via `show_view` → the binder →
    /// `ViewBus::apply`; this is purely the reactor observing that authoritative state.
    views: crate::foundation::server::ViewBus,
    /// Absolute path to the agent's view workshop (`<data_dir>/views`).
    /// Handed to every worker session as its `cwd`, so a build sub-agent works in a
    /// real project dir — `ls`-ing existing projects, writing source — like a human
    /// in their repo. Absolutized at startup (the child may run with a different cwd).
    views_dir: PathBuf,
    /// Monotonic turn counter. Each turn claims the next id;
    /// it tags audio spans and the channel logs so a reply is traceable end to
    /// end. (The client no longer needs it — turns are internal to the mind.)
    turn_seq: AtomicU64,
    scenes: Mutex<HashMap<Scene, SceneHandle>>,
    /// Wall-monotonic time of the most recent inbound human signal across **all**
    /// scenes — the global "fresh input" signal the single consolidated reflection
    /// clock reads to decide base-vs-backoff cadence (see [`consolidated_reflection_loop`]).
    /// Written in [`Reactor::deliver_to_scene`]; read each reflection tick.
    last_signal_at: std::sync::Mutex<Instant>,
    /// Wakes the consolidated reflection loop when fresh input lands, so a scene
    /// that goes active after a long quiet doesn't wait out the backed-off gap
    /// before its first pass — the loop re-derives its deadline on every notify.
    reflect_wake: tokio::sync::Notify,
    /// Process-wide shutdown signal, triggered by [`crate::run_with_shutdown`] the
    /// moment a SIGINT/SIGTERM or the tray's Quit is observed. Read by every scene
    /// loop, the reflection loop, and the drive retry path so that, once shutdown
    /// begins, an idle loop winds down promptly and a failed prompt does **not**
    /// restart an ACP session — the children just received the same signal, and a
    /// respawn here would race the subprocess reap and could orphan a child.
    shutdown: Shutdown,
}

struct SceneHandle {
    inbound: mpsc::Sender<LoopInput>,
}

pub fn start(
    memory: Memory,
    agent: AgentLayer,
    mut inbound_rx: mpsc::Receiver<Signal>,
    mut warm_rx: mpsc::Receiver<Scene>,
    out: mpsc::Sender<OutboundSignal>,
    observatory: Observatory,
    view_compiler: crate::mind::views::ViewCompiler,
    tools: ToolRegistry,
    interrupts: InterruptRegistry,
    presence: crate::body::presence::Presence,
    views: crate::foundation::server::ViewBus,
    views_dir: PathBuf,
    shutdown: Shutdown,
) -> Reactor {
    let reactor = Reactor {
        inner: Arc::new(ReactorInner {
            memory,
            agent,
            out,
            observatory,
            view_compiler,
            tools,
            interrupts,
            presence,
            views,
            views_dir,
            turn_seq: AtomicU64::new(0),
            scenes: Mutex::new(HashMap::new()),
            vendor: Arc::new(Vendor::new(vendor_down_after(), backoff_base())),
            last_signal_at: std::sync::Mutex::new(Instant::now()),
            reflect_wake: tokio::sync::Notify::new(),
            shutdown,
        }),
    };
    let dispatch_reactor = reactor.clone();

    tokio::spawn(async move {
        while let Some(signal) = inbound_rx.recv().await {
            let scene = signal.scene.clone();
            dispatch_reactor.deliver_to_scene(scene, signal).await;
        }
        tracing::warn!("reactor inbound channel closed; dispatch loop exiting");
    });

    // Warm-up requests: a scene-presence GET (a client opening a `/api/out/*`
    // long-poll) asks us to stand the scene up now, so its subprocess and ACP
    // session are open before the first utterance lands. `ensure_scene` is
    // idempotent — repeated GETs for an already-live scene are no-ops.
    let warm_reactor = reactor.clone();
    tokio::spawn(async move {
        while let Some(scene) = warm_rx.recv().await {
            warm_reactor.ensure_scene(scene).await;
        }
        tracing::warn!("reactor warm channel closed; warm-up loop exiting");
    });

    // Re-warm scenes with a genuinely fresh, still-live conversation, so their loop
    // (and pulse) is up without waiting for a client to reconnect. Deliberately
    // conservative — see [`scenes_to_rewarm`]: each warm spawns a subprocess and an
    // LLM call, so warming a crowd at boot hurts startup UX and competes for our own
    // LLM rate limit right when the user wants to interact. Boot is not a special
    // case: this merely stands the loops up, and each one's first pulse carries the
    // "host process started Xm ago" fact like any other. Standing/scheduled work
    // (cron, serving) does not depend on this — it lives on the heartbeat, so a scene
    // going cold never drops a duty.
    let rewarm_reactor = reactor.clone();
    tokio::spawn(async move {
        for scene in scenes_to_rewarm(rewarm_reactor.inner.memory.data_dir()) {
            tracing::info!(scene = %scene, "re-warming recently-active scene");
            rewarm_reactor.ensure_scene(scene).await;
        }
    });

    // Consolidated reflection ("sleep"): one pass over every recently-active scene
    // on a single global clock, replacing the old per-scene timers. A single mind
    // settles the whole day across contexts at once — so it can link across scenes
    // and one writer (not N racing) touches the shared facet/people stores.
    let reflect_reactor = reactor.clone();
    tokio::spawn(async move {
        consolidated_reflection_loop(reflect_reactor).await;
    });

    reactor
}

/// The single consolidated reflection loop: one "sleep" pass over all
/// recently-active scenes, on one adaptive clock, never overlapping itself.
///
/// Anchored on the **last completed pass** (or loop start before the first), it
/// fires `base` after the anchor while any scene saw fresh input since (the active
/// cadence), else on a `backoff_gap` doubling toward `reflect_max` while the whole
/// system is quiet — the same rule the old per-scene loops used, now global (see
/// [`next_reflection_at`]). A fresh signal arriving mid-gap pokes [`reflect_wake`]
/// so the loop re-derives its deadline immediately rather than waiting out a long
/// backoff. Each tick consolidates only scenes with enough on their frontier; a
/// tick with nothing ready is a cheap no-op. Returns (the task ends) only when
/// reflection is disabled outright (`reflect=off` or `reflect_every=0`).
async fn consolidated_reflection_loop(reactor: Reactor) {
    let reflect_base = reflect_interval();
    let reflect_max = reflect_max_interval();
    if reflect_base.is_none() {
        tracing::info!("consolidated reflection disabled");
        return;
    }
    let loop_started = Instant::now();
    let mut last_reflection: Option<Instant> = None;
    let mut backoff_gap = reflect_base.unwrap_or(DEFAULT_REFLECT_EVERY);

    loop {
        let last_activity = *reactor.inner.last_signal_at.lock().unwrap();
        let Some(at) =
            next_reflection_at(loop_started, last_activity, last_reflection, reflect_base, backoff_gap)
        else {
            return;
        };
        let now = Instant::now();
        if at > now {
            // Sleep until due, but wake early if fresh input lands — then re-loop to
            // recompute the deadline (which only actually fires once it's past).
            // Shutdown ends the loop rather than starting a doomed "sleep" pass.
            tokio::select! {
                _ = tokio::time::sleep(at.saturating_duration_since(now)) => {}
                _ = reactor.inner.reflect_wake.notified() => continue,
                _ = reactor.inner.shutdown.cancelled() => {
                    tracing::info!("shutdown requested; ending consolidated reflection loop");
                    return;
                }
            }
        }

        // Shutdown may have arrived without the sleep above (deadline already past):
        // don't open a reflection subprocess into a dying process group.
        if reactor.inner.shutdown.is_triggered() {
            tracing::info!("shutdown requested; ending consolidated reflection loop");
            return;
        }

        // Due. Adapt the backoff against the *old* anchor before re-anchoring: fresh
        // input since the last pass snaps the gap back to base; a quiet pass doubles
        // it toward the cap. Re-anchor on `now` whether or not anything consolidates,
        // so a no-op tick can't hot-spin the clock.
        let now = Instant::now();
        let last_activity = *reactor.inner.last_signal_at.lock().unwrap();
        let anchor = last_reflection.unwrap_or(loop_started);
        backoff_gap = if last_activity > anchor {
            reflect_base.unwrap_or(DEFAULT_REFLECT_EVERY)
        } else {
            backoff_gap.checked_mul(2).unwrap_or(reflect_max).min(reflect_max)
        };
        last_reflection = Some(now);

        // The scenes to consider — the same source that decides which loops exist, so
        // we consolidate exactly the scenes that were reflecting under the old design.
        let scenes = recent_scenes(reactor.inner.memory.data_dir(), REWARM_WINDOW);
        heartbeat::consolidate(&reactor, &scenes).await;
    }
}

/// Channels that do **not** count as a scene being alive. Exactly one: `clock`,
/// where the host's own wakes are recorded — a pulse firing, an alarm coming due.
///
/// This is load-bearing. Pulses are journaled (a restart otherwise sees a turn with
/// no cause), but a heartbeat is not a conversation, and anything that spends money
/// on the strength of "this scene looks busy" would otherwise feed itself: the
/// re-warm gate below re-warms an idle scene, whose first act is a pulse, whose row
/// makes the scene look freshly active, so it is re-warmed again next boot —
/// forever, each one costing a subprocess and an LLM call. Reflection has the same
/// shape (see [`heartbeat::reflectable`]): a scene left alone would tick its way over
/// the frontier threshold on heartbeats and reflect on nothing.
///
/// Excluding the channel is exact rather than a heuristic on entry bodies: nothing
/// but the clock is ever written there, which is the reason the clock got a channel
/// of its own. Note this excludes clock rows from being a *reason* to act — never
/// from being read; a reconstruction still sees every wake.
///
/// **Only the clock belongs here, and `worker` specifically does not** — this list is
/// read by two questions, and they want different answers. "Is the scene alive?" (the
/// re-warm gate below) and "is there enough here to consolidate?"
/// ([`heartbeat::reflectable`]) share it, and a worker report is not presence but *is*
/// content worth settling into an episode. Excluding it here would silently stop
/// finished work from ever reaching a scene's episodes.
///
/// The related bug — a report journaled into a *stranger's* scene, because a
/// sceneless-owned worker ran in a borrowed one — is fixed where it is caused, by
/// journaling under the work's own origin scene, not by making the channel invisible.
const NON_ACTIVITY_CHANNELS: [&str; 1] = ["clock"];

/// Scenes whose raw memory saw activity within `window`, each paired with the
/// newest modification time seen across its channel folders (its last signal). The
/// directories are under `<data_dir>/memory/raw/`; errors read as "no scenes" —
/// re-warm is best-effort.
///
/// "Activity" means a signal that someone or something outside the host's own clock
/// put there: an inbound utterance, an emitted reply, a view shown, a worker
/// reporting. The clock's channel is skipped (see [`NON_ACTIVITY_CHANNELS`]), so the
/// newest mtime still marks the last *real* signal and never a bare self-attention
/// tick. That is what lets the re-warm gate treat mtime as "last input" without a
/// separate journal scan.
fn scenes_with_activity(
    data_dir: &std::path::Path,
    window: Duration,
) -> Vec<(Scene, std::time::SystemTime)> {
    let raw = data_dir.join("memory").join("raw");
    let Some(cutoff) = std::time::SystemTime::now().checked_sub(window) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&raw) else {
        return Vec::new();
    };
    let mut scenes = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // Not every child of `raw/` is a scene — foundation's own frame log lives
        // there too, and its `<run>/` children look exactly like channel folders.
        // Without this, every boot after the first recorded frame stands a full
        // per-scene loop up for a directory: subprocess, pulse, consolidation.
        if !path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(layout::is_scene_dir)
        {
            continue;
        }
        // Newest mtime across this scene's signal-bearing channel folders — the
        // time of its last signal. Only directories: a channel folder's mtime moves
        // when a day-folder is created in it, whereas `scene.json` is a one-time
        // identity sidecar and says nothing about whether anyone is still here.
        let newest = std::fs::read_dir(&path).ok().and_then(|days| {
            days.flatten()
                .filter(|d| d.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .filter(|d| match d.file_name().into_string() {
                    Ok(name) => !NON_ACTIVITY_CHANNELS.contains(&name.as_str()),
                    Err(_) => true,
                })
                .filter_map(|d| d.metadata().and_then(|m| m.modified()).ok())
                .max()
        });
        if let Some(newest) = newest
            && newest >= cutoff
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
        {
            scenes.push((Scene(name.to_owned()), newest));
        }
    }
    scenes
}

/// Scenes whose raw memory saw activity within `window`. Thin projection of
/// [`scenes_with_activity`] for callers that only need the ids (the consolidated
/// reflection pass).
fn recent_scenes(data_dir: &std::path::Path, window: Duration) -> Vec<Scene> {
    scenes_with_activity(data_dir, window)
        .into_iter()
        .map(|(scene, _)| scene)
        .collect()
}

/// A scene whose last input is older than this is not re-warmed at startup: it has
/// gone quiet, and standing work no longer lives in a per-scene loop (cron/serving
/// run on the heartbeat), so there is nothing to keep alive by warming it.
const REWARM_MAX_IDLE: Duration = Duration::from_secs(24 * 3600);

/// Where the re-warm gate persists its per-scene bookkeeping (see
/// [`scenes_to_rewarm`]). Sits outside `raw/` so writing it never perturbs the
/// activity mtimes [`scenes_with_activity`] reads.
fn rewarm_state_path(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("memory").join("rewarm.json")
}

/// Scene id → the unix-seconds mtime of its newest raw signal at the moment we last
/// re-warmed it. A missing/corrupt file reads as empty: at worst one extra re-warm.
fn load_rewarm_state(data_dir: &std::path::Path) -> HashMap<String, u64> {
    std::fs::read(rewarm_state_path(data_dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_rewarm_state(data_dir: &std::path::Path, state: &HashMap<String, u64>) {
    if let Ok(bytes) = serde_json::to_vec_pretty(state) {
        // Best-effort: a lost write just costs at most one extra re-warm next boot.
        let _ = std::fs::write(rewarm_state_path(data_dir), bytes);
    }
}

/// Which recently-active scenes to re-warm at startup — deliberately conservative.
///
/// Warming a scene is expensive: it spawns an ACP subprocess and its first pulse is
/// an LLM call. Warming many scenes at once slows startup and floods our own LLM
/// rate limit *exactly* when the user is trying to interact — so we warm only the
/// scenes that plausibly still have a live conversation, and never re-warm the same
/// quiet scene twice. A scene is re-warmed only when BOTH hold:
///   1. its last input is newer than [`REWARM_MAX_IDLE`] (a day quiet → stay cold),
///      enforced by the `scenes_with_activity` window; and
///   2. we have not already re-warmed it for that same, unchanged input — so
///      restarting the host repeatedly within a day doesn't re-warm a quiet scene
///      each time.
///
/// "Input" here is raw-memory activity, which excludes the clock's own channel —
/// pulses and alarms are journaled, but they are the host talking to itself and
/// must never look like input, or condition 1 would be satisfied by the very
/// heartbeat re-warming produced (see [`NON_ACTIVITY_CHANNELS`]). Standing/scheduled
/// work (cron, serving) does NOT rely on re-warming: it belongs to the global
/// heartbeat session, not a per-scene loop, so letting an idle scene go cold never
/// drops a duty.
fn scenes_to_rewarm(data_dir: &std::path::Path) -> Vec<Scene> {
    let prior = load_rewarm_state(data_dir);
    let mut warm = Vec::new();
    // Only scenes carried forward here (all within REWARM_MAX_IDLE) stay in the
    // map; scenes that have since gone quiet fall out, keeping it bounded.
    let mut next: HashMap<String, u64> = HashMap::new();
    for (scene, mtime) in scenes_with_activity(data_dir, REWARM_MAX_IDLE) {
        let epoch = mtime
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        next.insert(scene.0.clone(), epoch);
        if prior.get(&scene.0) == Some(&epoch) {
            // Condition 2: already re-warmed for this exact input, nothing new
            // since — leave it cold and let a fresh signal wake it on demand.
            continue;
        }
        warm.push(scene);
    }
    save_rewarm_state(data_dir, &next);
    warm
}

#[cfg(test)]
mod rewarm_tests {
    use super::*;

    /// Lay down `<data_dir>/memory/raw/<scene>/<channel>/<day>/<channel>.jsonl`,
    /// the shape [`crate::mind::memory::layout`] writes.
    fn seed_channel(data_dir: &std::path::Path, scene: &str, channel: &str) {
        let day = data_dir
            .join("memory")
            .join("raw")
            .join(scene)
            .join(channel)
            .join("2026-07-27");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join(format!("{channel}.jsonl")), b"{}\n").unwrap();
    }

    fn names(scenes: Vec<(Scene, std::time::SystemTime)>) -> Vec<String> {
        scenes.into_iter().map(|(s, _)| s.0).collect()
    }

    /// The trap: a scene whose only record is its own heartbeat must not look
    /// alive, or re-warming it would keep it alive on nothing but its own pulses.
    #[test]
    fn a_scene_with_only_clock_signals_is_not_active() {
        let dir = tempfile::tempdir().unwrap();
        seed_channel(dir.path(), "quiet", "clock");
        assert!(
            names(scenes_with_activity(dir.path(), Duration::from_secs(3600))).is_empty(),
            "pulses alone must not mark a scene active"
        );
        assert!(scenes_to_rewarm(dir.path()).is_empty(), "and so must not re-warm it");
    }

    /// Foundation's own frame log lives under `raw/` beside the scenes, and its
    /// `<run>/` children are indistinguishable from channel folders by shape. Before
    /// the reserved-name skip this returned `Scene("sessions")`, so every boot after
    /// the first recorded frame warmed a full loop for a directory.
    #[test]
    fn the_frame_log_is_not_a_scene() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir
            .path()
            .join("memory")
            .join("raw")
            .join("sessions")
            .join("a1b2c3d4e5f6");
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(run.join("1.jsonl"), b"{}\n").unwrap();

        assert!(
            names(scenes_with_activity(dir.path(), Duration::from_secs(3600))).is_empty(),
            "the frame log is foundation's, not a conversation"
        );
        assert!(scenes_to_rewarm(dir.path()).is_empty(), "and must not be warmed");
    }

    /// Everything else a scene records still counts — including the newly
    /// journaled outbound channels, which are real evidence someone is here.
    #[test]
    fn real_signals_still_mark_a_scene_active() {
        let dir = tempfile::tempdir().unwrap();
        seed_channel(dir.path(), "talking", "text");
        seed_channel(dir.path(), "talking", "clock");
        seed_channel(dir.path(), "watching", "view");
        let mut got = names(scenes_with_activity(dir.path(), Duration::from_secs(3600)));
        got.sort();
        assert_eq!(got, ["talking", "watching"]);
    }

    /// Condition 2 still holds once pulses are journaled: a scene re-warmed for a
    /// given input is not re-warmed again while nothing but the clock has moved.
    #[test]
    fn a_scene_is_not_rewarmed_twice_for_the_same_input() {
        let dir = tempfile::tempdir().unwrap();
        seed_channel(dir.path(), "talking", "text");
        assert_eq!(scenes_to_rewarm(dir.path()), vec![Scene("talking".into())]);

        // A later boot, after the scene has done nothing but pulse: still cold.
        seed_channel(dir.path(), "talking", "clock");
        assert!(scenes_to_rewarm(dir.path()).is_empty());
    }
}

impl Reactor {
    async fn deliver_to_scene(&self, scene: Scene, signal: Signal) {
        // Mark global activity and poke the consolidated reflection clock, so a scene
        // going active after a long quiet gets its first pass without waiting out the
        // backed-off gap.
        *self.inner.last_signal_at.lock().unwrap() = Instant::now();
        self.inner.reflect_wake.notify_one();

        let sender = self.get_or_create_scene(scene.clone()).await;

        // A new signal never cancels the in-flight prompt: the serial per-scene
        // loop folds it into the next turn (fix-forward), and the lightweight
        // reactor decides per turn whether to act or wait for the rest.
        if let Err(err) = sender.send(LoopInput::Human(signal)).await {
            tracing::error!(scene = %scene, error = %err, "scene inbound channel closed; dropping signal");
        }
    }

    /// Stand a scene's loop up now (idempotent), so its warm-up prologue runs and
    /// the scene is hot before the first utterance. Driven by a scene-presence
    /// signal — a client opening one of the scene's `/api/out/*` long-polls; an
    /// already-live scene is a no-op.
    pub async fn ensure_scene(&self, scene: Scene) {
        let _ = self.get_or_create_scene(scene).await;
    }

    async fn get_or_create_scene(&self, scene: Scene) -> mpsc::Sender<LoopInput> {
        let mut scenes = self.inner.scenes.lock().await;
        if let Some(handle) = scenes.get(&scene) {
            return handle.inbound.clone();
        }

        let (tx, rx) = mpsc::channel::<LoopInput>(SCENE_QUEUE_CAPACITY);
        scenes.insert(scene.clone(), SceneHandle { inbound: tx.clone() });
        drop(scenes);

        // The scene's tool control channel: the `/mcp` server forwards delegate/
        // alarm/ask calls here, the loop applies them. Register the sink before the
        // loop's session opens so a tool call can never arrive with no route.
        let (control_tx, control_rx) = mpsc::channel::<SceneControl>(SCENE_QUEUE_CAPACITY);

        // The scene's output beats: say/show_view tool calls (and the loop's turn
        // brackets) flow to a dedicated sequencer task that paces speech and views.
        // Output bypasses the turn loop so it streams while the prompt still runs.
        let (beats_tx, beats_rx) = mpsc::channel::<sequencer::Beat>(SCENE_QUEUE_CAPACITY);
        {
            let seq_reactor = self.clone();
            let seq_scene = scene.clone();
            tokio::spawn(async move {
                sequencer::run_sequencer(seq_reactor, seq_scene, beats_rx).await;
            });
        }

        self.inner
            .tools
            .register(
                scene.clone(),
                ToolSink { control: control_tx.clone(), beats: beats_tx.clone() },
            )
            .await;

        let task_reactor = self.clone();
        let task_scene = scene.clone();
        // The worker registry posts its reports back into this same queue, so
        // hand the loop a sender clone to seed it.
        let task_worker_inbound = tx.clone();
        tokio::spawn(async move {
            per_scene_loop(
                task_reactor,
                task_scene,
                rx,
                task_worker_inbound,
                control_rx,
                control_tx,
                beats_tx,
            )
            .await;
        });

        tx
    }
}

/// Why the per-scene loop's wait resolved. Keeps the `select!` arms tiny so the
/// borrow checker doesn't trip on mutating `workers`/`alarms` inside them.
enum Woke {
    Inbound(Option<LoopInput>),
    Control(Option<SceneControl>),
    /// Mail landed in this scene's Reaction inbox.
    Mail,
    Timer,
    /// Process shutdown began while this loop was idle — stop waiting and exit.
    Shutdown,
}

/// Apply one tool control command. Both are side-effects that run without a turn.
/// The live-worker map and the alarm list are the loop's own state, so this is the
/// only place an off-loop tool call touches them — through the control channel, no
/// locking.
async fn apply_control(
    reactor: &Reactor,
    scene: &Scene,
    workers: &mut workers::WorkerRegistry,
    alarms: &mut Alarms,
    ctl: SceneControl,
) -> Option<LoopInput> {
    match ctl {
        SceneControl::CreateWorker { id, task, owner } => {
            if let Err(err) = workers.spawn_with_id(reactor, id, task, owner).await {
                tracing::warn!(scene = %scene, error = %err, "failed to create a working session");
            }
            None
        }
        SceneControl::Alarm { delay, note } => {
            schedule_alarm(reactor, alarms, scene, &delay, &note).await;
            None
        }
    }
}

async fn per_scene_loop(
    reactor: Reactor,
    scene: Scene,
    mut inbound: mpsc::Receiver<LoopInput>,
    worker_inbound: mpsc::Sender<LoopInput>,
    mut control: mpsc::Receiver<SceneControl>,
    // Held only to keep the control channel open: the registry holds the other
    // sender, but keeping a clone here means `control.recv()` never resolves to
    // `None` while this loop runs, so a quiet tool channel can't end the scene.
    _control_keepalive: mpsc::Sender<SceneControl>,
    // The scene's output sequencer inlet. The loop sends each turn's TurnStart/
    // TurnEnd brackets here; the `/mcp` handler sends the say/show_view beats
    // between them. The same sender is the keepalive for the sequencer task.
    beats: mpsc::Sender<sequencer::Beat>,
) {
    // The scene's persistent reactor session: opened lazily on the first turn,
    // then reused for every later turn as the scene's continuous mind. Only this
    // loop touches it, so a plain local `Option` suffices; the heartbeat swap
    // below replaces it in place, between turns.
    let mut reactor_session: Option<Arc<AcpSession>> = None;
    // Retained for the observatory's budget readout; the reactor turn no longer
    // feeds it, so the hot-swap it gated never fires (the reactor re-opens cold on
    // failure instead). Left in place until the hot-swap path is fully retired.
    let mut budget = heartbeat::ContextBudget::new();
    // The scene's live working sessions. Heavy/tool-using work the reactor
    // delegates runs here; workers post progress and results back through
    // `worker_inbound` into this same loop.
    let mut workers = workers::WorkerRegistry::new(scene.clone(), worker_inbound);
    // Self-alarms the mind has scheduled. They give the loop a second reason to
    // wake — time passing — on top of an incoming signal; see the `select!` below.
    let mut alarms = Alarms::new();

    // The scene's address in the switchboard. **Registered once, here, for as long as
    // this loop lives** — not per session open. A scene is one conversation and has one
    // voice; the underlying session rotates beneath it (cold reopen, hot swap) and that
    // is an implementation detail nothing outside should be able to observe. Registering
    // at session-open minted a second Reaction for the same scene on every reopen, and
    // `Address::Scene` then resolved to whichever the lookup happened to find first.
    // Scope-bound: released on every way out of this loop, including the ones added
    // after this line was written.
    let voice = registry::register_scoped(
        registry::mint(),
        registry::Role::Reaction,
        Some(scene.clone()),
        None,
        "the scene's voice".to_string(),
    );
    let voice_id = voice.id();
    let voice_mail = voice.mail.clone();

    tracing::info!(scene = %scene, voice = voice_id, "reactor per-scene loop up");

    // No warm-up: the reactor session opens lazily on its first turn (a subprocess
    // spawn + system-prompt prime would only stall that first turn behind it). The
    // journal snapshot is delivered by that first turn's fresh-session branch.

    // Pulse bookkeeping: the host's recurring self-attention timer. `last_activity`
    // resets on every turn, so pulses only fire into genuine quiet; the first pulse
    // after the loop stands up also carries how long ago the host process started,
    // which is all "wake on boot" amounts to.
    let pulse_every = pulse_interval();
    let loop_started = Instant::now();
    let mut last_activity = Instant::now();
    let mut pulsed_once = false;

    // Pending turn-driving items, hoisted out of the main loop so the batch
    // survives across iterations while the vendor is down — a failed retry must not
    // drop the mail it was attempting to deliver. Cleared on a successful turn (the
    // mail went out) and on a reachable-but-failed blip (the apology was emitted);
    // held while down.
    let mut batch: Vec<LoopInput> = Vec::new();

    loop {
        // Wait for a turn-driving reason: a new signal, a fired alarm, a due host
        // pulse, a worker question, or — while the vendor is down — a backoff retry
        // (429/generic). Tool control commands (delegate/alarm) are pure side-effects
        // applied without a turn; only a worker `ask` becomes a turn-driving item.
        'wait: loop {
            let gate = reactor.inner.vendor.scene_gate();
            // Mail already sitting in `batch` (e.g. held while the vendor was down)
            // needs no fresh signal to act on — drive it now while reachable. While
            // down, fall through to the timer logic.
            if !batch.is_empty() && matches!(gate, SceneGate::Go) {
                break 'wait;
            }
            let down = !matches!(gate, SceneGate::Go);
            // While down, suppress pulses — they call the model and would just fail.
            let pulse_at = if down { None } else { pulse_every.map(|d| last_activity + d) };
            // While down, the recovery timer: the backoff retry deadline (429/generic).
            // Up → no such timer.
            let recover_at = match gate {
                SceneGate::Go => None,
                SceneGate::Retry { at } => Some(at),
            };
            let deadline = [alarms.next_deadline(), pulse_at, recover_at]
                .into_iter()
                .flatten()
                .min();
            let woke = match deadline {
                Some(deadline) => tokio::select! {
                    recvd = inbound.recv() => Woke::Inbound(recvd),
                    ctl = control.recv() => Woke::Control(ctl),
                    _ = voice_mail.notified() => Woke::Mail,
                    _ = sleep_until(deadline) => Woke::Timer,
                    _ = reactor.inner.shutdown.cancelled() => Woke::Shutdown,
                },
                None => tokio::select! {
                    recvd = inbound.recv() => Woke::Inbound(recvd),
                    ctl = control.recv() => Woke::Control(ctl),
                    _ = voice_mail.notified() => Woke::Mail,
                    _ = reactor.inner.shutdown.cancelled() => Woke::Shutdown,
                },
            };
            match woke {
                Woke::Inbound(Some(s)) => {
                    enqueue(&reactor, &scene, &mut workers, &mut batch, s).await;
                    // While Down: collect mail without driving a turn. The
                    // probe cadence will attempt catch-up once the vendor
                    // recovers.
                    if !down {
                        break 'wait;
                    }
                }
                Woke::Inbound(None) => {
                    tracing::info!(scene = %scene, "per-scene inbound closed; exiting loop");
                    return;
                }
                Woke::Shutdown => {
                    tracing::info!(scene = %scene, "shutdown requested; exiting per-scene loop");
                    return;
                }
                // Mail for the scene's voice. It drives a turn like any other
                // reason to speak — that is what makes `send_message(to: scene)`
                // actually reach the person rather than wait for them to say
                // something next. A spurious wake (the notify raced a take) finds
                // an empty inbox and simply goes back to waiting.
                Woke::Mail => {
                    if let Some(mail) = registry::global().take_pending(voice_id) {
                        enqueue(
                            &reactor,
                            &scene,
                            &mut workers,
                            &mut batch,
                            LoopInput::Mail(mail),
                        )
                        .await;
                        registry::global().finish_turn(voice_id);
                        if !down {
                            break 'wait;
                        }
                    }
                }
                // The keepalive sender means this is effectively unreachable; treat
                // a closed control channel as "nothing to apply" and keep waiting.
                Woke::Control(None) => continue 'wait,
                Woke::Control(Some(ctl)) => {
                    if let Some(input) =
                        apply_control(&reactor, &scene, &mut workers, &mut alarms, ctl).await
                    {
                        enqueue(&reactor, &scene, &mut workers, &mut batch, input).await;
                        if !down {
                            break 'wait;
                        }
                    }
                    // A delegate/alarm side-effect was applied; keep waiting for a
                    // turn-driving reason rather than running an empty turn.
                }
                Woke::Timer => {
                    let now = Instant::now();
                    if down {
                        // Alarms still fire and queue while down — the mind asked to
                        // be woken, and the note isn't lost — but they don't alone
                        // drive a turn; a backoff retry does.
                        for fired in alarms.take_due(now) {
                            reactor
                                .inner
                                .observatory
                                .record(Some(&scene), EventKind::AlarmFired { note: fired.note.clone() })
                                .await;
                            enqueue(&reactor, &scene, &mut workers, &mut batch, LoopInput::Alarm(fired)).await;
                        }
                        // Only a transient backoff drives a model retry, and only with
                        // mail to deliver. Out of energy holds instead — the shared
                        // poller flips us back Up and the top of 'wait then drains the
                        // mail without a doomed model call.
                        if let SceneGate::Retry { at } = gate
                            && at <= now
                            && !batch.is_empty()
                        {
                            tracing::info!(scene = %scene, mail = batch.len(), "backoff retry firing");
                            break 'wait;
                        }
                        continue 'wait;
                    }
                    for fired in alarms.take_due(now) {
                        reactor
                            .inner
                            .observatory
                            .record(Some(&scene), EventKind::AlarmFired { note: fired.note.clone() })
                            .await;
                        enqueue(&reactor, &scene, &mut workers, &mut batch, LoopInput::Alarm(fired)).await;
                    }
                    if let Some(at) = pulse_at
                        && at <= now
                    {
                        let idle_m = (now - last_activity).as_secs() / 60;
                        let note = if pulsed_once {
                            format!("nothing new here for {idle_m}m")
                        } else {
                            let up_m = (now - loop_started).as_secs() / 60;
                            format!(
                                "nothing new here for {idle_m}m — you've just come back up (host process started {up_m}m ago)"
                            )
                        };
                        pulsed_once = true;
                        // Reset so a swallowed pulse doesn't re-fire in a tight loop.
                        last_activity = now;
                        tracing::info!(scene = %scene, "pulse fired");
                        enqueue(&reactor, &scene, &mut workers, &mut batch, LoopInput::Pulse { note }).await;
                    }
                    if !batch.is_empty() {
                        break 'wait;
                    }
                }
            }
        }

        // A timer can resolve with nothing actually due; don't run an empty turn.
        // (While Down, the probe only breaks 'wait with non-empty mail, so this
        // guard is for the Up path's pulse/alarm timers.)
        if batch.is_empty() {
            continue;
        }

        let was_down = reactor.inner.vendor.is_down();

        // Commit-after-quiet: wait for things to settle before replying. Skipped
        // while down — a backoff retry should attempt catch-up ASAP rather than wait
        // for more mail to settle (the retry cadence already coalesces arrivals).
        if !was_down {
            let closed = loop {
                while let Ok(extra) = inbound.try_recv() {
                    enqueue(&reactor, &scene, &mut workers, &mut batch, extra).await;
                }
                match timeout(RESPONSE_SETTLE, inbound.recv()).await {
                    // another utterance — keep collecting
                    Ok(Some(extra)) => enqueue(&reactor, &scene, &mut workers, &mut batch, extra).await,
                    Ok(None) => break true, // inbound closed mid-settle
                    Err(_) => break false,  // quiet elapsed → commit to a reply
                }
            };
            if closed {
                tracing::info!(scene = %scene, "per-scene inbound closed; exiting loop");
                return;
            }
        }

        // Forget any workers that have finished, so the registry doesn't grow.
        workers.reap();

        match run_reactor_turn(
            &reactor,
            &scene,
            &batch,
            &mut workers,
            &mut reactor_session,
            voice_id,
            &beats,
        )
        .await
        {
            Ok(()) => {
                // The turn delivered the mail; clear the backlog. (If this was a
                // retry, the turn already flipped the vendor Up via note_success.)
                batch.clear();
                // A reply landed — stop the presence owed-reply clock (no-op if
                // nothing was owed, e.g. a pulse turn).
                reactor.inner.presence.note_delivered(&scene);
                // Between turns: if the live session has grown past budget, hot-swap
                // it now. The human is consuming the reply just delivered, so the
                // summarize-and-reopen happens in that natural gap — invisible, never
                // a cold restart. A swap failure leaves the warm session in place.
                // (Reflection is no longer kicked off here — it runs on its own
                // periodic clock in the wait loop above, decoupled from compaction.)
                if budget.should_swap() {
                    if let Some(current) = reactor_session.clone() {
                        match timeout(
                            SWAP_TIMEOUT,
                            heartbeat::swap(&reactor, &scene, &current, voice_id),
                        )
                        .await
                        {
                            Ok(Ok(fresh)) => {
                                reactor_session = Some(fresh);
                                budget.reset();
                                tracing::info!(scene = %scene, "reactor session hot-swapped");
                            }
                            Ok(Err(err)) => {
                                tracing::warn!(scene = %scene, error = %err, "hot-swap failed; keeping warm session");
                            }
                            Err(_) => {
                                // The live session ignored the summarize prompt for the
                                // whole window — treat it as wedged and discard it, the
                                // same as a failed turn; the next turn cold-opens a fresh
                                // session from the journal snapshot.
                                tracing::warn!(scene = %scene, "hot-swap timed out; discarding unresponsive session");
                                if let Some(dead) = reactor_session.take() {
                                    reactor
                                        .inner
                                        .observatory
                                        .record(
                                            Some(&scene),
                                            EventKind::SessionClosed {
                                                kind: SessionKind::Reactor,
                                                id: dead.id().0.to_string(),
                                            },
                                        )
                                        .await;
                                }
                                budget.reset();
                                reactor.inner.observatory.set_budget(&scene, 0).await;
                            }
                        }
                    }
                }
            }
            Err(err) => {
                tracing::warn!(scene = %scene, error = %err, "turn failed");
                // Discard the possibly-wedged session; the next turn cold-opens a
                // fresh one and rebuilds context from the journal snapshot.
                if let Some(dead) = reactor_session.take() {
                    reactor
                        .inner
                        .observatory
                        .record(
                            Some(&scene),
                            EventKind::SessionClosed {
                                kind: SessionKind::Reactor,
                                id: dead.id().0.to_string(),
                            },
                        )
                        .await;
                }
                // Dropping the session means the next turn cold-opens and its
                // fresh-session branch re-ingests the journal snapshot.
                budget.reset();
                reactor.inner.observatory.set_budget(&scene, 0).await;
                // Key on the vendor state the turn just wrote, not the pre-turn one:
                // a turn that flipped the vendor down holds the mail — a backoff drives
                // it at the next retry deadline. Only a still-reachable blip (already
                // apologized inside run_turn) drops it.
                if reactor.inner.vendor.is_down() {
                    tracing::info!(scene = %scene, mail = batch.len(), "vendor down; holding mail for recovery");
                } else {
                    batch.clear();
                }
            }
        }

        // Any completed turn is activity: the pulse clock restarts, so pulses
        // only ever fire into genuine quiet.
        last_activity = Instant::now();

        // Coalesce mid-turn arrivals. Utterances that queued while this turn ran
        // (a generation is now seconds, not ~1s) are siblings of the thread we just
        // answered, not fresh threads — pull them all into one batch so they drive a
        // SINGLE next turn (the commit-after-quiet settle still applies on top),
        // instead of one redundant turn each. Without this, each nudge that landed
        // mid-turn ("好了吗?" → "准备好了吗?") pops alone on re-entry and re-answers.
        // Up only: while down, mail is held deliberately and the backoff path owns
        // catch-up, so leave the queue for it. `try_recv` never surfaces pulses or
        // alarms (those are generated inside `'wait`, not sent over `inbound`).
        if !reactor.inner.vendor.is_down() {
            while let Ok(extra) = inbound.try_recv() {
                enqueue(&reactor, &scene, &mut workers, &mut batch, extra).await;
            }
        }
    }
}

/// Render just the human requests in a batch (skipping worker reports, alarms, and
/// pulses) — the text handed to Deliberation as the turn's task. Skipping reports is
/// what keeps it from re-ingesting its own prior output (a feedback loop).
fn render_human_from_batch(batch: &[LoopInput]) -> String {
    use crate::mind::memory::snapshot::{Speaker, transcript_line};
    use std::fmt::Write as _;
    let mut s = String::new();
    for input in batch {
        if let LoopInput::Human(sig) = input {
            let chan = sig.channel.with_stream(sig.stream.as_deref());
            let _ = writeln!(s, "{}", transcript_line(Speaker::Them, &chan, &sig.body));
        }
    }
    s
}


/// A reactor turn: the single fast conversational voice. An ACP session
/// ([`SessionRole::Reactor`]) on the small model, carrying `speaking.md` as its system
/// prompt and a `say` + `show_view` `/mcp` surface, with the agent's own built-in tools
/// switched off at session open. A turn is a single quick generation: it speaks by
/// calling `say`, and may call `show_view` to put a view a worker already built on
/// screen; both feed the sequencer. Text it merely types is working-out and is never
/// voiced. The speed comes from the small model + a single generation, not from
/// bypassing the adapter.
///
/// Deliberation — the scene's reading and thinking — runs in parallel: the turn's human
/// request is handed to it ([`workers::WorkerRegistry::deliberate`]),
/// which works off the floor and reports back as an ordinary `LoopInput::Worker` the
/// reactor voices on a later turn. So the reactor stays the single fast voice.
///
/// v1 keeps it simple — no mid-turn reorganization. A turn is one fast generation, so a
/// human speaking during it just queues and the serial loop folds it into the next turn.
async fn run_reactor_turn(
    reactor: &Reactor,
    scene: &Scene,
    batch: &[LoopInput],
    workers: &mut workers::WorkerRegistry,
    reactor_session: &mut Option<Arc<AcpSession>>,
    voice_id: registry::SessionId,
    beats: &mpsc::Sender<sequencer::Beat>,
) -> anyhow::Result<()> {
    let turn_id = reactor.inner.turn_seq.fetch_add(1, Ordering::Relaxed);

    // This turn's delta: live worker status (so the voice can surface deliberation's
    // progress), presence, any barge-in note, and the new signals. The projected state
    // it all hangs off is assembled in [`turn_context`].
    let worker_status = workers.render_status().await;
    let presence_note = format!("## Presence\n{}", reactor.inner.presence.render(scene));
    let interrupted = reactor
        .inner
        .interrupts
        .take_pending(scene)
        .await
        .map(|i| interrupts::render_interruption(&i))
        .unwrap_or_default();
    let new_signals = format!("## New signals\n{}", render_batch(batch));
    // What the agent has on screen right now — its own presentation surface. Read
    // fresh every turn (it's a current fact, not durable memory), so a view dismissed
    // last turn is gone from this list now: the agent can see what's up and dismiss by
    // real id instead of guessing from the transcript.
    let on_screen = render_on_screen(&reactor.inner.views.on_screen(scene).await);

    // Open (or reuse) the persistent reactor session. `speaking.md` is prepended to its
    // first prompt; the session then remembers prior turns. Whether it is fresh no
    // longer changes what the turn carries — see [`turn_context`].
    let session = match reactor_session {
        Some(s) => s.clone(),
        None => {
            let opened = open_reactor_session(reactor, scene, voice_id).await?;
            *reactor_session = Some(opened.clone());
            opened
        }
    };

    let context = turn_context(
        &reactor.inner.memory,
        scene,
        &worker_status,
        &on_screen,
        &presence_note,
        &interrupted,
        &new_signals,
    )
    .await;

    tracing::info!(scene = %scene, ctx_chars = context.chars().count(), "reactor: prompting session");
    let _ = beats.send(sequencer::Beat::TurnStart { turn: turn_id }).await;

    let spoke = match drive_voice(&session, scene, context).await {
        Ok(text) => {
            // Speech arrives as `say` calls, which the MCP surface already put on the
            // sequencer while the turn was running. Anything the model *typed* is
            // working-out, not utterance — voicing it too would say every reply twice,
            // and the tool's own description promises plain text is not spoken.
            tracing::info!(
                scene = %scene,
                unspoken_chars = text.chars().count(),
                "reactor: turn done"
            );
            true
        }
        Err(err) => {
            tracing::warn!(scene = %scene, error = %err, "reactor turn failed");
            // Drop the possibly-wedged session so the next turn re-opens cold.
            *reactor_session = None;
            if reactor.inner.vendor.note_unreachable() {
                let _ = beats
                    .send(sequencer::Beat::Say(
                        "我暂时连不上模型，先攒着你的消息，等恢复了一起处理。".to_string(),
                    ))
                    .await;
            }
            false
        }
    };

    // Close the bracket and record what was spoken (for barge-in resolution).
    let (done_tx, done_rx) = oneshot::channel();
    let _ = beats.send(sequencer::Beat::TurnEnd { done: done_tx }).await;
    let reply = done_rx.await.unwrap_or_default();
    reactor.inner.interrupts.end_turn(scene, turn_id, &reply).await;

    if spoke {
        let _ = reactor.inner.vendor.note_success();
        // Hand the turn's human request to Deliberation — the scene's reader — so it works
        // off the floor while the voice moves on; its report rides back as a WorkerReport
        // the reactor voices on a later turn. Spawned once per scene, then followed up.
        // Nothing to hand off on a pure report/pulse turn.
        let task = render_human_from_batch(batch);
        if !task.trim().is_empty() {
            if let Err(e) = workers.deliberate(reactor, task).await {
                tracing::warn!(scene = %scene, error = %e, "deliberation spawn/follow-up failed");
            }
        }
    }
    Ok(())
}

/// One turn's whole prompt: the projected state, then this turn's delta.
///
/// **There is no fresh-session branch here, and that absence is the change.** The
/// projection used to be inlined only when a session was opened, on the reasoning that
/// the session remembers its own open and later turns need send only the delta. That
/// reasoning holds for a *transcript* and fails for *state*: a task opened, a duty
/// closed, or a scene memory written mid-conversation is exactly what the session
/// cannot have remembered, because it did not exist yet. So the window was correct at
/// session open and drifted for every turn after — and since a scene's session is
/// long-lived by design, that is most of the conversation. Code re-reads the current
/// state and injects it on every turn instead.
///
/// The costs are real and accepted. The block rides in every user message, so the
/// session's history accumulates one copy per turn until the next hot-swap rotates it
/// — which is why the bound in [`crate::mind::memory::snapshot::CARRIED_FORWARD_CHARS`]
/// belongs to code. And the reads (the generated prompt, the task dimension, the log
/// tail) now happen per turn rather than per session; each is small, none can fail the
/// turn, and the alternative is an agent that answers from a stale window.
async fn turn_context(
    memory: &Memory,
    scene: &Scene,
    worker_status: &str,
    on_screen: &str,
    presence: &str,
    interrupted: &str,
    new_signals: &str,
) -> String {
    let projected = snapshot::window(memory, scene).await;
    join_sections(&[projected.as_str(), worker_status, on_screen, presence, interrupted, new_signals])
}

#[cfg(test)]
mod turn_context_tests {
    use super::*;
    use crate::mind::memory::layout;
    use crate::mind::memory::tasks::{Task, TaskKind, write_task};

    /// The bug this change exists to fix. A scene's memory written — or a task opened
    /// — *after* the session was already up used to be invisible until the session
    /// rotated; the second turn of one live session must carry it.
    #[tokio::test]
    async fn the_projection_rides_a_reused_session_too() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        let scene = Scene("boss".into());

        // Turn one, on a session opened just now: nothing written yet.
        let first = turn_context(&memory, &scene, "", "", "", "", "## New signals\n>在吗").await;
        assert!(!first.contains("mid-migration"), "{first}");

        // Mid-conversation, the state moves under the live session.
        let path = layout::scene_prompt_path(dir.path(), &scene);
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        tokio::fs::write(&path, "He is mid-migration this week; keep answers terse.")
            .await
            .unwrap();
        let mut owed = Task::new("Ship the flash cards", TaskKind::Wip);
        owed.title = "Ship the flash cards".into();
        write_task(dir.path(), &owed).await.unwrap();

        // Turn two, same session — no re-open, no rotation.
        let second = turn_context(&memory, &scene, "", "", "", "", "## New signals\n>那卡片呢").await;
        assert!(second.contains("mid-migration"), "{second}");
        assert!(second.contains("- [wip] Ship the flash cards"), "{second}");
        assert!(second.contains("## New signals"), "{second}");
    }

    /// The projected state leads and the turn's delta follows, so the new signals sit
    /// last — closest to the reply the model is about to write.
    #[tokio::test]
    async fn projected_state_leads_and_the_new_signals_come_last() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        let scene = Scene("boss".into());
        let text = turn_context(
            &memory,
            &scene,
            "## Workers\nbuilding a view",
            "",
            "## Presence\nhere",
            "",
            "## New signals\n>好了没",
        )
        .await;
        let at = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("missing {needle}: {text}"));
        assert!(at("## Recent (last 30 minutes)") < at("## Workers"));
        assert!(at("## Workers") < at("## Presence"));
        assert!(at("## Presence") < at("## New signals"));
        assert!(text.trim_end().ends_with("好了没"), "{text}");
    }
}

/// Open a fresh **reactor** session for `scene`, carrying `speaking.md` as its system
/// prompt (prepended to the first prompt). It speaks via plain message text and gets a
/// minimal `show_view`-only `/mcp` surface, so a turn is a single quick generation that
/// may also put one already-built view on screen.
///
/// `voice_id` is the loop's own switchboard registration — the *same* id across every
/// reopen and every hot-swap, because the scene has one voice however many subprocesses
/// have hosted it. Passing it is what puts `X-HI-Session-Id` on the session's MCP
/// attach; without it the voice held a mailbox it had no identity to send from, and
/// `send_message` answered "this session has none" to the one rung that talks most.
async fn open_reactor_session(
    reactor: &Reactor,
    scene: &Scene,
    voice_id: registry::SessionId,
) -> anyhow::Result<Arc<AcpSession>> {
    let session = Arc::new(
        reactor
            .inner
            .agent
            .session(
                scene,
                SessionRole::Reactor,
                Some(voice_id),
                SessionOpts {
                    system_prompt: Some(
                        crate::identity::reactor_system_prompt(
                            reactor.inner.memory.data_dir(),
                        )
                        .await,
                    ),
                    cwd: None,
                    // `say` and `show`, and nothing else — enforced, not requested.
                    // The rung is fast because it *cannot* wait on anything, and that
                    // argument is worth nothing if it can quietly open a file.
                    builtin_tools: Some(Vec::new()),
                },
            )
            .await?,
    );
    reactor
        .inner
        .observatory
        .record(
            Some(scene),
            EventKind::SessionOpened {
                kind: SessionKind::Reactor,
                id: session.id().0.to_string(),
            },
        )
        .await;
    Ok(session)
}

/// Prompt the reactor session and return its spoken text (every `agent_message_chunk`
/// concatenated). Tool calls — the reactor's only tool is `show_view` — are dispatched
/// server-side through hi-agent's `/mcp` (which emits the `Beat::Show`), so the drive
/// loop just keeps streaming speech past them, exactly like a worker's loop; `wait()`
/// then parks the session and surfaces any real prompt error (a gateway 402/429, a
/// transport reset) to the caller's classifier.
async fn drive_voice(session: &AcpSession, scene: &Scene, context: String) -> anyhow::Result<String> {
    let mut run = session.prompt(context).await?;
    let mut text = String::new();
    while let Some(update) = run.next_update().await {
        match update {
            SessionUpdate::Text(t) => text.push_str(&t),
            SessionUpdate::Thought(t) => {
                tracing::debug!(scene = %scene, chars = t.chars().count(), "reactor: model is thinking");
            }
            // `show_view` dispatches server-side via `/mcp`; the reactor keeps speaking.
            // Its surface is `show_view`-only and the dispatch guard blocks any other
            // expression tool, so there is nothing to intercept here. The frame is
            // recorded at the wire by the tap, not read here — this rung interprets
            // nothing it is not about to say.
            SessionUpdate::Frame(_) => {}
        }
    }
    let result = run.wait().await?;
    tracing::info!(
        scene = %scene,
        stop = ?result.stop_reason,
        reply_chars = text.chars().count(),
        "reactor: turn complete"
    );
    Ok(text)
}


/// Append one signal the agent *emitted* — worded text, a voiced span, a view it
/// put up — to the durable log, then carry on. Every carrier that puts something
/// in front of the person routes through here rather than settling for a
/// `tracing::` line, because a log that only holds the inbound half cannot tell a
/// restarted mind what it already said and showed, and a mind that can't tell will
/// say it again.
///
/// Recorded at the moment the thing leaves — never buffered to make a tidier row.
/// Best-effort, like every other append site: a failed write is logged and the
/// reply still goes out; the log is not allowed to swallow a turn.
async fn record_out(reactor: &Reactor, scene: &Scene, channel: Channel, body: String) {
    let entry = JournalEntry::SignalOut {
        id: Uuid::now_v7().to_string(),
        ts: Utc::now(),
        channel,
        scene: scene.clone(),
        body,
        media: None,
        origin: Some(Origin::Reactor),
    };
    if let Err(err) = reactor.inner.memory.journal.append(entry).await {
        tracing::error!(scene = %scene, channel = %channel, error = %err, "journal append failed for outbound signal");
    }
}

/// Append one turn-driving signal that reached the mind without crossing a wire —
/// a pulse, a fired alarm, a worker's report. Without these the log shows a turn's
/// output with nothing that could have caused it, and a restart cannot tell that
/// the turn happened at all, let alone why.
async fn record_in(
    reactor: &Reactor,
    scene: &Scene,
    channel: Channel,
    origin: Origin,
    body: String,
) {
    let entry = JournalEntry::SignalIn {
        id: Uuid::now_v7().to_string(),
        ts: Utc::now(),
        channel,
        scene: scene.clone(),
        body,
        stream: None,
        media: None,
        origin: Some(origin),
    };
    if let Err(err) = reactor.inner.memory.journal.append(entry).await {
        tracing::error!(scene = %scene, channel = %channel, error = %err, "journal append failed for internal signal");
    }
}

async fn emit_thought_chunk(reactor: &Reactor, scene: &Scene, text: String) {
    // Per chunk, as it is written — not coalesced into one row per utterance. The
    // log's promise is durability before reaction, and buffering to make a neater
    // row would mean a crash mid-utterance loses words the agent already sent.
    // Readers re-join the chunks in `(ts, id)` order, which is exactly what the
    // merge already gives them.
    record_out(reactor, scene, Channel::Text, text.clone()).await;
    let _ = reactor
        .inner
        .out
        .send(OutboundSignal::Text {
            scene: scene.clone(),
            chunk: text,
        })
        .await;
}

/// Carry one release action to its wire carrier: speech to TTS, a view to
/// /view. Thought mirroring and the once-per-turn reply log are handled inline
/// by the caller, since they track the raw spoken chunk rather than the paced
/// emits.
async fn perform(
    emit: interleave::Emit,
    synth_tx: &Option<mpsc::Sender<String>>,
    reactor: &Reactor,
    scene: &Scene,
) {
    match emit {
        interleave::Emit::Speak(sentence) => {
            if let Some(tx) = synth_tx {
                let _ = tx.send(sentence).await;
            }
        }
        interleave::Emit::ShowView { id, op, source, geometry } => {
            emit_view(reactor, scene, id, op, source, geometry).await
        }
    }
}

async fn emit_end_of_utterance(reactor: &Reactor, scene: &Scene) {
    let _ = reactor
        .inner
        .out
        .send(OutboundSignal::TextEnd { scene: scene.clone() })
        .await;
}

/// Join non-empty prompt sections with a blank line between them, trimming each.
/// Lets a turn assemble whichever of {snapshot, worker status, new signals}
/// actually have content without leaving stray blank headers.
fn join_sections(sections: &[&str]) -> String {
    sections
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Render the agent's own screen as a prompt section: the ids currently displayed,
/// z-order top-most last. Always emitted (unlike the empty-dropping sections) — when
/// the screen is clear the agent needs to *know* it's clear so it stops firing blind
/// dismisses at ids that are already gone. Kept to bare ids: the reactor shows/dismisses
/// by id, and the id is all it needs to target one.
fn render_on_screen(ids: &[String]) -> String {
    use std::fmt::Write as _;
    let mut s = String::from("## On screen now\n");
    if ids.is_empty() {
        s.push_str("(nothing is on screen — the room is clear)");
    } else {
        for id in ids {
            let _ = writeln!(s, "- {id}");
        }
        s.push_str("(these are the views currently up, top-most last; dismiss one by its id)");
    }
    s
}

fn render_batch(batch: &[LoopInput]) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    for input in batch {
        match input {
            LoopInput::Human(sig) => {
                use crate::mind::memory::snapshot::{Speaker, transcript_line};
                let chan = sig.channel.with_stream(sig.stream.as_deref());
                let _ = writeln!(s, "{}", transcript_line(Speaker::Them, &chan, &sig.body));
            }
            LoopInput::Worker(report) => {
                let _ = writeln!(s, "{}", workers::render_report(report));
            }
            LoopInput::Alarm(a) => {
                let _ = writeln!(s, "{}", render_alarm(a));
            }
            LoopInput::Pulse { note } => {
                let _ = writeln!(s, "{}", render_pulse(note));
            }
            LoopInput::Mail(mail) => {
                let _ = writeln!(s, "{}", render_mail(mail));
            }
        }
    }
    s
}

/// Mail from elsewhere in the agent, as the voice sees it.
///
/// Each message names the session it came from, because that id **is** the reply
/// address: `send_message` back to it and the answer lands where it was asked for.
/// Framed as something the agent already knows rather than something someone told
/// it — there is no colleague here, and the voice must never speak of one.
fn render_mail(mail: &[crate::foundation::registry::Message]) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    for m in mail {
        match m.from {
            Some(from) => {
                let _ = writeln!(
                    s,
                    "(from your own background work, session {from}) {}",
                    m.text.trim()
                );
            }
            None => {
                let _ = writeln!(s, "{}", m.text.trim());
            }
        }
    }
    s.truncate(s.trim_end().len());
    s
}

fn render_alarm(a: &AlarmFired) -> String {
    format!("(alarm) \"{}\"", a.note)
}

fn render_pulse(note: &str) -> String {
    format!("(pulse) {note}")
}

/// How one turn-driving input is recorded in the durable log, or `None` when it
/// needs no row of its own.
///
/// A human signal returns `None`: the wire handler that accepted it already
/// journaled it, with its stream and its media, before it ever reached this queue.
/// Recording it again here would show every utterance twice.
///
/// The rest reach the mind without crossing a wire, so this is their only chance to
/// be written down. Each goes on the channel that names its origin — a heartbeat is
/// not something the person said, and neither is deliberation's answer coming back.
/// A worker's row keeps the substance and drops the framing [`workers::render_report`]
/// wraps it in for the prompt; that framing is an instruction to the voice, not part
/// of the signal.
fn journal_form(input: &LoopInput) -> Option<(Channel, Origin, String)> {
    match input {
        LoopInput::Human(_) => None,
        LoopInput::Worker(report) => Some((
            Channel::Worker,
            Origin::Worker,
            workers::render_report_plainly(report),
        )),
        // An alarm is a wake the mind asked for; a pulse is one the host simply
        // delivers. Same channel, different hand on it.
        LoopInput::Alarm(a) => Some((Channel::Clock, Origin::Reactor, render_alarm(a))),
        LoopInput::Pulse { note } => Some((Channel::Clock, Origin::Host, render_pulse(note))),
        // Mail crosses no wire, so this is its only chance to be written down.
        LoopInput::Mail(mail) => Some((Channel::Worker, Origin::Worker, render_mail(mail))),
    }
}

/// Record one turn-driving input, then queue it for the turn. Every path that puts
/// something in a scene's batch goes through here, so nothing can drive a turn
/// unlogged — which is the whole point: a turn whose cause was never written down
/// is a turn a restart cannot account for.
/// Put one input in front of the mind, journaling it on the way.
///
/// The one input that may not end up here is a worker's report: if the worker was
/// spawned by another session, the report belongs to *that* session and is delivered
/// into it instead, never reaching the scene. Work travels up the chain of owners to
/// whoever asked for it; it does not appear beside the person's own words in a
/// conversation nobody addressed it to.
///
/// It is still journaled either way — the report crossed an agent boundary, and the
/// log records what crossed regardless of where it went next.
async fn enqueue(
    reactor: &Reactor,
    scene: &Scene,
    workers: &mut workers::WorkerRegistry,
    batch: &mut Vec<LoopInput>,
    input: LoopInput,
) {
    if let Some((channel, origin, body)) = journal_form(&input) {
        record_in(reactor, scene, channel, origin, body).await;
    }
    if let LoopInput::Worker(report) = &input
        && let Some(owner) = report.owner
    {
        let text = workers::render_report(report);
        if workers.deliver_to(owner, text) {
            return;
        }
        // The owner is gone. Surfacing one rung too high beats losing finished work.
        tracing::info!(
            scene = %scene, session = report.id, owner,
            "report owner is gone; falling back to the scene"
        );
    }
    batch.push(input);
}

/// Background task: drain one turn's synthesized audio frames onto the /audio
/// channel, emitting an `AudioFrame` per chunk and a closing `AudioEnd`. The
/// span's `AudioBegin` (which carries the codec) is sent by the caller before
/// this task is spawned. Send errors are ignored — no subscriber connected is
/// fine.
///
/// One journal row per *span*, written as the span closes — a span is one voiced
/// utterance, and a frame is not a signal, so there is no smaller honest unit
/// here. The row records the act of voicing (codec and size), not the words: the
/// words are already on /text as they were written, and repeating them would show
/// every reply twice in a reconstruction. Reading the two together says what was
/// said *and* that it was actually spoken aloud — which the text rows alone can't,
/// since a turn with TTS unconfigured is silent and writes no span at all.
async fn forward_frames(
    reactor: Reactor,
    mut frames: mpsc::Receiver<Bytes>,
    out: mpsc::Sender<OutboundSignal>,
    scene: Scene,
    turn: u64,
    codec: String,
) {
    let mut total = 0usize;
    while let Some(bytes) = frames.recv().await {
        total += bytes.len();
        let _ = out
            .send(OutboundSignal::AudioFrame {
                scene: scene.clone(),
                turn,
                bytes,
            })
            .await;
    }
    let _ = out
        .send(OutboundSignal::AudioEnd {
            scene: scene.clone(),
            turn,
        })
        .await;
    tracing::info!(
        target: "channel",
        dir = "out",
        channel = "audio",
        scene = %scene,
        turn = turn,
        bytes = total,
        "channel out (tts stream)",
    );
    // A span that carried no frames was never heard — TTS opened and produced
    // nothing — so there is nothing to record.
    if total > 0 {
        record_out(
            &reactor,
            &scene,
            Channel::Audio,
            format!("spoke the reply aloud ({codec}, {total} bytes)"),
        )
        .await;
    }
}

/// Emit one agent-authored view on the /view channel for this scene. A `show`/
/// `replace` compiles the source to a module first (just-in-time, after the
/// preceding sentence has flushed, so it stays paced to narration); a `dismiss`
/// carries no module. A compile failure is logged and the view is dropped — the
/// turn's speech already went out, so a broken view never breaks the reply.
async fn emit_view(
    reactor: &Reactor,
    scene: &Scene,
    id: String,
    op: ViewOp,
    source: String,
    geometry: Option<Geometry>,
) {
    let module_url = if op == ViewOp::Dismiss {
        None
    } else {
        match reactor.inner.view_compiler.compile(&source).await {
            Ok(url) => Some(url),
            Err(err) => {
                tracing::error!(scene = %scene, id = %id, error = %err, "view compile failed; dropping view");
                return;
            }
        }
    };
    tracing::info!(
        target: "channel",
        dir = "out",
        channel = "view",
        scene = %scene,
        id = %id,
        op = ?op,
        module = module_url.as_deref().unwrap_or(""),
        "channel out (view)",
    );
    // Before it goes on the wire: showing something is as much an utterance as
    // saying it, and the screen persists across restarts, so a mind that can't read
    // back what it put up will put it up again.
    let line = render_view_line(&id, op, module_url.as_deref());
    record_out(reactor, scene, Channel::View, line).await;
    let _ = reactor
        .inner
        .out
        .send(OutboundSignal::View {
            scene: scene.clone(),
            envelope: ViewEnvelope { id, op, module_url, geometry },
        })
        .await;
}

/// One view operation as a transcript line. Deliberately not the view's source:
/// the compiled module is already on disk and its URL is a content hash, so the
/// hash identifies *what* was shown at a fixed cost, while the JSX itself would
/// put kilobytes of markup on the hot path and back into every later prompt. The
/// id carries the meaning — it is what the agent shows, replaces and dismisses by,
/// and what `## On screen now` lists.
fn render_view_line(id: &str, op: ViewOp, module_url: Option<&str>) -> String {
    let verb = match op {
        ViewOp::Show => "showed",
        ViewOp::Replace => "replaced",
        ViewOp::Dismiss => "dismissed",
    };
    match module_url {
        Some(url) => format!("{verb} \"{id}\" ({url})"),
        None => format!("{verb} \"{id}\""),
    }
}

#[cfg(test)]
mod alarm_tests {
    use super::{Alarms, parse_delay};
    use std::time::Duration;
    use tokio::time::Instant;

    #[test]
    fn parse_delay_reads_units() {
        assert_eq!(parse_delay("1200"), Some(Duration::from_secs(1200)));
        assert_eq!(parse_delay("30s"), Some(Duration::from_secs(30)));
        assert_eq!(parse_delay("20m"), Some(Duration::from_secs(1200)));
        assert_eq!(parse_delay("1h"), Some(Duration::from_secs(3600)));
        assert_eq!(parse_delay("  45  "), Some(Duration::from_secs(45)));
    }

    #[test]
    fn parse_delay_rejects_garbage() {
        assert_eq!(parse_delay("soon"), None);
        assert_eq!(parse_delay(""), None);
        assert_eq!(parse_delay("m"), None);
    }

    #[test]
    fn fires_in_deadline_order_and_keeps_the_rest() {
        let t0 = Instant::now();
        let mut alarms = Alarms::new();
        assert_eq!(alarms.next_deadline(), None);

        alarms.schedule(Duration::from_secs(60), "later".into(), t0);
        alarms.schedule(Duration::from_secs(10), "sooner".into(), t0);
        assert_eq!(alarms.next_deadline(), Some(t0 + Duration::from_secs(10)));

        // Nothing due before the soonest deadline.
        assert!(alarms.take_due(t0 + Duration::from_secs(5)).is_empty());

        // At 10s only "sooner" fires; the 60s one stays pending.
        let fired = alarms.take_due(t0 + Duration::from_secs(10));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].note, "sooner");
        assert_eq!(alarms.next_deadline(), Some(t0 + Duration::from_secs(60)));

        // Past the last deadline the remaining one fires and the queue empties.
        let fired = alarms.take_due(t0 + Duration::from_secs(120));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].note, "later");
        assert_eq!(alarms.next_deadline(), None);
    }
}
