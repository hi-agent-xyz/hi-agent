//! Reaction — the *mind*. Per-conversation queues + one persistent session per conversation.
//!
//! One mpsc per conversation, one task per conversation; turns run serially against a single
//! Reaction agent session that is opened and primed when the conversation stands up, then
//! reused as the conversation's continuous voice. Deliberation has its own prewarmed
//! session and runs off the floor; Reaction never blocks on it.
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
//!    in-flight prompt. The per-reaction loop is serial — it runs one turn to
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
//! long-running, the mind calls the `delegate` tool with the task; the reaction
//! spawns a channel-mute [`workers`] session for it and keeps talking. The worker
//! runs with the same substrate (memory, tools) but no voice of its own, and
//! posts its result — or a question, if it gets stuck — back into this conversation's
//! queue, where it lands as just another input the next turn folds into what the
//! mind says.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Context;
mod cognition;
mod heartbeat;
mod reflection;
mod interleave;
mod interrupts;
pub mod outbound;
mod sequencer;
mod tools;
mod workers;

pub use interrupts::InterruptRegistry;
pub use outbound::OutboundSignal;
pub use tools::{LoopControl, Spoken, ToolOwner, ToolRegistry, ToolSink};

use chrono::Utc;
use tokio::sync::{Mutex, mpsc, oneshot, watch};
use tokio::time::{Instant, sleep_until, timeout};

use crate::foundation::codex::{AgentSession, SessionOpts, SessionUpdate};
use crate::foundation::agent::AgentLayer;
use crate::identity::Role;
use crate::foundation::config;
use crate::foundation::registry;
use crate::foundation::shutdown::Shutdown;
use crate::mind::memory::{Memory, snapshot};
use crate::foundation::observatory::{EventKind, Observatory};
use crate::types::{Channel, JournalEntry, Origin, Signal, ViewEnvelope, ViewOp, ViewTraits};
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

/// Default idle interval between host pulses — the conversation's recurring moment of
/// self-attention. A pulse is not a schedule of work: it injects bare situational
/// facts ("nothing new for 30m") and core.md tells the mind what such a moment is
/// for (read down its active tasks, glance at setups it owns); most pulses should
/// conclude with nothing to do or say. Override via `pulse`; `0`/`off`
/// disables. Boot is not a special case — the first pulse after the host starts
/// simply carries that fact.
const DEFAULT_PULSE: Duration = Duration::from_secs(1800);

/// Resolve the pulse interval from the stored `pulse` tunable in duration grammar
/// if set (`None` for `0`/`off` — pulses disabled), else [`DEFAULT_PULSE`].
/// Shared with [`cognition`], which paces its own glance-up on the same knob: one
/// "how often does this agent look up from what it's doing" setting, not a conversation one
/// plus a brain one that can disagree. It also keeps journey testing honest — dropping
/// `pulse` for a session speeds up every wake there is, rather than all but one.
pub(super) fn pulse_interval() -> Option<Duration> {
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

/// Default base reflection cadence — how often a conversation with fresh input
/// consolidates ([`reflect_interval`]). The idle backoff grows from here.
const DEFAULT_REFLECT_EVERY: Duration = Duration::from_secs(60);
/// Default ceiling on the idle backoff ([`reflect_max_interval`]): a long-quiet
/// conversation re-checks at most this often.
const DEFAULT_REFLECT_MAX: Duration = Duration::from_secs(8 * 3600);

/// Resolve a stored duration tunable in duration grammar (`90s`/`30m`/`1h`; bare
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
/// (`reflect=off`) or `reflect_every` is `0`/`off`. A conversation with
/// fresh input consolidates this often; once it goes quiet the gap backs off from
/// here up to [`reflect_max_interval`].
fn reflect_interval() -> Option<Duration> {
    reflect_enabled()
        .then(|| duration_tunable(config::tunables::get(config::KEY_REFLECT_EVERY), DEFAULT_REFLECT_EVERY))
        .flatten()
}

/// The ceiling on the idle backoff: a caught-up, quiet conversation doubles its gap from
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
/// outage, not a one-off blip. A managed 402 pauses immediately; everything else
/// follows the classifier below.
const DEFAULT_VENDOR_DOWN_AFTER: u32 = 2;
/// A transient-outage retry never waits longer than this — the 1h ceiling.
const BACKOFF_CAP: Duration = Duration::from_secs(3600);
/// Once a managed 402 is observed, the process-wide vendor gate checks the
/// broker frequently enough that a subscription or refill clears the view and
/// wakes held work without waiting for the normal account cadence.
const ENERGY_RECOVERY_POLL: Duration = Duration::from_secs(5);

/// The base transient-outage retry gap. `vendor_probe` in duration grammar;
/// `off`/`0`/unset/unparseable → default. (Kept under the historical config key.)
fn backoff_base() -> Duration {
    duration_tunable(config::tunables::get(config::KEY_VENDOR_PROBE), DEFAULT_VENDOR_PROBE)
        .unwrap_or(DEFAULT_VENDOR_PROBE)
}

/// The consecutive generic-failure count that flips the reaction into an informed
/// backoff. `vendor_down_after`; `0`/unparseable → default.
fn vendor_down_after() -> u32 {
    config::tunables::get(config::KEY_VENDOR_DOWN_AFTER)
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_VENDOR_DOWN_AFTER)
}

/// What a failed turn means for what to do next — the whole of this layer's error
/// handling, deliberately coarse.
///
/// **Three outcomes, not a taxonomy.** The question here is only *"should I keep
/// hammering?"* — not why the upstream said no, nor when it will relent, nor what the
/// person should do about it. Those are upper-layer concerns: for managed accounts the
/// broker already polls the balance every 60s and clears the out-of-energy state on
/// refill ([`crate::foundation::broker::spawn_refresh_loop`]), which is the mechanism
/// that resumes us. Modelling reset times or user actions down here would duplicate it
/// and drift from it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Disposition {
    /// Transient: the upstream blipped. Back off and try again.
    Retry,
    /// **This session** is bad — the vendor is fine. Discard it and cold-open on the
    /// next turn, and **do not touch the process-wide vendor gate**: one crashed
    /// subprocess must not make every other conversation believe the model is unreachable.
    Restart,
    /// Stop trying. Out of quota, out of credit, refused credentials — retrying cannot
    /// help, and hammering costs a subprocess spawn per attempt (487 of them, once).
    /// Resuming is not this layer's call.
    Pause,
}

/// Classify a terminal turn failure. Defaults to [`Disposition::Retry`], so an error
/// nobody has seen before behaves exactly as everything did before this existed.
///
/// Matching on message text is crude and is the honest option: these arrive as
/// `-32603 Internal error` with the upstream's own words inside, so there is no
/// structured code to switch on at this boundary.
fn disposition(err: &str) -> Disposition {
    let e = err.to_ascii_lowercase();
    // Quota / credit / credentials. Observed live: `402 budget exceeded` (487 times in
    // one night, every one of them futile) and `403 预扣费额度失败 … authentication_failed`.
    if crate::foundation::energy_state::is_402_text(&e)
        || e.contains("budget exceeded")
        || e.contains("insufficient")
        || e.contains("quota")
        || e.contains("额度")
        || e.contains("authentication_failed")
        || e.contains("invalid api key")
        || e.contains("invalid_api_key")
    {
        return Disposition::Pause;
    }
    // The session, not the vendor: the subprocess died or the wire went away.
    if e.contains("session not found")
        || e.contains("connection closed")
        || e.contains("broken pipe")
        || e.contains("unexpected eof")
        || e.contains("channel closed")
    {
        return Disposition::Restart;
    }
    Disposition::Retry
}

/// How the reaction loop should treat the vendor right now — the read side of [`Vendor`].
#[derive(Clone, Copy, Debug)]
enum TurnGate {
    /// Reachable: drive turns normally.
    Go,
    /// Transient outage (429 / generic): hold mail, and drive a catch-up turn once
    /// `at` (the current backoff deadline) passes. A failed retry grows the gap.
    Retry { at: Instant },
    /// Paused: hold mail and drive **nothing**. Cleared by whoever owns the reason —
    /// for managed accounts, the broker's energy poll on refill.
    Hold,
}

/// The vendor's reachability and, when down, how to recover from it.
#[derive(Clone, Copy, Debug)]
enum VendorState {
    Up,
    /// Transient backoff (429 / generic). `try_at` is the next retry deadline;
    /// `attempt` grows the gap toward [`BACKOFF_CAP`]; `silent` suppresses the user
    /// notice for a pure rate-limit (429), which the user needn't hear about.
    Backoff { try_at: Instant, attempt: u32, silent: bool },
    /// Stopped, with no deadline of our own. Managed energy and unrelated permanent
    /// failures are tracked independently so a balance refill cannot clear an invalid
    /// key or another condition it does not own.
    Paused { energy: bool, permanent: bool },
}

/// Shared, process-wide view of the upstream LLM vendor and how to recover from an
/// outage. The reaction loop reads it (via [`Vendor::turn_gate`]) to decide whether
/// and when to drive a turn; `run_turn`'s terminal path writes it. The vendor is a
/// shared resource, so one conversation detecting an outage steers all of them.
///
/// The `note_*` writers return whether the transition warrants a *one-time* user
/// notice (so the reaction announces "can't reach the model" exactly once),
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

    /// The reaction loop's scheduling read: drive now (Go) or retry at a deadline (Retry).
    fn turn_gate(&self) -> TurnGate {
        match *self.state.lock().unwrap() {
            VendorState::Up => TurnGate::Go,
            VendorState::Backoff { try_at, .. } => TurnGate::Retry { at: try_at },
            VendorState::Paused { .. } => TurnGate::Hold,
        }
    }

    /// Apply the managed-energy pause. It overrides a backoff because an observed 402
    /// is better information than a generic retry deadline, while preserving any
    /// unrelated permanent pause already present.
    fn note_energy_paused(&self) -> bool {
        let mut st = self.state.lock().unwrap();
        let (was, permanent) = match *st {
            VendorState::Paused { energy, permanent } => (energy, permanent),
            _ => (false, false),
        };
        *st = VendorState::Paused {
            energy: true,
            permanent,
        };
        !was
    }

    /// Clear only the managed-energy reason. An unrelated permanent pause survives.
    fn resume_energy(&self) -> bool {
        let mut st = self.state.lock().unwrap();
        match *st {
            VendorState::Paused {
                energy: true,
                permanent,
            } => {
                *st = if permanent {
                    VendorState::Paused {
                        energy: false,
                        permanent: true,
                    }
                } else {
                    self.generic_failures.store(0, Ordering::Relaxed);
                    VendorState::Up
                };
                true
            }
            _ => false,
        }
    }

    /// Stop for a permanent non-energy reason. No account refresh is allowed to
    /// clear this condition.
    fn note_permanent_paused(&self) -> bool {
        let mut st = self.state.lock().unwrap();
        let (energy, was) = match *st {
            VendorState::Paused { energy, permanent } => (energy, permanent),
            _ => (false, false),
        };
        *st = VendorState::Paused {
            energy,
            permanent: true,
        };
        !was
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
            // Already stopped for a reason a retry cannot fix. A generic failure on top
            // of that tells us nothing new and must not downgrade it into a backoff.
            VendorState::Paused { .. } => false,
        }
    }

    /// A turn (or retry) succeeded. Flip Up and reset the blip counter. Returns
    /// `true` if this ended an outage (so the caller logs the recovery).
    fn note_success(&self) -> bool {
        let mut st = self.state.lock().unwrap();
        self.generic_failures.store(0, Ordering::Relaxed);
        match *st {
            VendorState::Backoff { .. } => {
                *st = VendorState::Up;
                true
            }
            // A successful in-flight turn must not clear a process-wide 402 pause.
            // The balance transition owns that recovery edge.
            VendorState::Paused { .. } | VendorState::Up => false,
        }
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
        assert!(matches!(v.turn_gate(), TurnGate::Retry { .. }));
        assert!(!v.note_unreachable(), "a failed retry grows the backoff without re-announcing");
    }

    /// The three real errors this layer has actually seen, and the default.
    ///
    /// The two `Pause` cases are the ones that cost something: `402 budget exceeded`
    /// was retried 487 times in one night, and the `403` below is a dead balance that
    /// no amount of retrying can fix.
    #[test]
    fn dispositions_cover_what_was_observed_live() {
        assert_eq!(
            disposition("Internal error: API Error: 402 budget exceeded"),
            Disposition::Pause
        );
        assert_eq!(
            disposition("Failed to authenticate. API Error: 403 预扣费额度失败, 用户剩余额度: $0.12"),
            Disposition::Pause,
            "a dead balance arrives as an auth failure, and must not read as transient"
        );
        assert_eq!(
            disposition("API Error: 524 origin_response_timeout"),
            Disposition::Retry,
            "cloudflare timing out is exactly what backoff is for"
        );
        assert_eq!(
            disposition("session/prompt failed: connection closed"),
            Disposition::Restart,
            "the subprocess died; the vendor is fine and must not be marked down"
        );
        assert_eq!(
            disposition("something nobody has seen before"),
            Disposition::Retry,
            "an unknown error behaves exactly as everything did before this existed"
        );
    }

    /// A pause is not a backoff: nothing here schedules its end.
    #[test]
    fn pause_holds_until_something_else_clears_it() {
        let v = fresh();
        assert!(v.note_energy_paused(), "the transition is new");
        assert!(!v.note_energy_paused(), "and idempotent");
        assert!(v.is_down());
        assert!(matches!(v.turn_gate(), TurnGate::Hold), "a hold never offers a retry deadline");
        assert!(!v.note_success(), "an unrelated in-flight success cannot clear a 402 pause");
        assert!(matches!(v.turn_gate(), TurnGate::Hold));
        assert!(v.resume_energy());
        assert!(!v.is_down());
        assert!(!v.resume_energy(), "already up");
    }

    /// Learning the balance is gone is better information than a retry deadline, so it
    /// overrides one — but recovery must not stomp an unrelated backoff.
    #[test]
    fn pause_overrides_a_backoff_but_resume_does_not_stomp_one() {
        let v = fresh();
        v.note_unreachable();
        v.note_unreachable();
        assert!(matches!(v.turn_gate(), TurnGate::Retry { .. }));
        v.note_energy_paused();
        assert!(matches!(v.turn_gate(), TurnGate::Hold));

        let w = fresh();
        w.note_unreachable();
        w.note_unreachable();
        assert!(!w.resume_energy(), "not energy-paused; a balance refill is not its business");
        assert!(matches!(w.turn_gate(), TurnGate::Retry { .. }), "backoff survives");
    }

    #[test]
    fn energy_recovery_preserves_an_unrelated_permanent_pause() {
        let v = fresh();
        v.note_permanent_paused();
        v.note_energy_paused();
        assert!(v.resume_energy());
        assert!(matches!(v.turn_gate(), TurnGate::Hold));
        assert!(v.is_down());
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

/// The soonest a reflection may fire for a conversation, or `None` when reflection is
/// disabled (`base` is `None`). One adaptive clock, anchored on the **last
/// reflection** (or `loop_started` before the first) so a never-idle conversation still
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
    fn busy_conversation_fires_base_after_the_last_reflection() {
        let t0 = Instant::now();
        // Reflected at t0+60s; a later turn keeps activity ahead of the anchor, so
        // the next pass is base after the *reflection*, not pushed out by activity.
        let last_reflection = t0 + secs(60);
        let at = next_reflection_at(t0, t0 + secs(90), Some(last_reflection), Some(secs(60)), secs(60));
        assert_eq!(at, Some(last_reflection + secs(60)));
    }

    #[test]
    fn a_quiet_store_uses_the_backed_off_gap() {
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

const LOOP_QUEUE_CAPACITY: usize = 64;

/// One item in the conversation's turn queue. Both a human utterance and a worker's
/// report drive a reaction turn; they differ only in source. A human signal comes
/// through [`Reaction::deliver`]; a worker report is posted straight into
/// the queue by the worker's drive task. Neither interrupts live speech — both
/// wait their turn and are settled into one batch.
enum LoopInput {
    Human(Signal),
    Worker(workers::WorkerReport),
    /// A host pulse firing — the recurring moment of self-attention. Carries
    /// bare situational facts; what to do with such a moment is core.md's job.
    Pulse { note: String },
    /// The person came back — they brought the window forward after an absence.
    ///
    /// Kept distinct from [`LoopInput::Pulse`] rather than reusing it with a
    /// different note, because the two mean opposite things to the voice. A pulse
    /// is a quiet moment the prompt is explicit that almost nothing is worth
    /// breaking; a return is the moment a held word was being held *for*. Rendering
    /// this as `(pulse)` would tell Reaction to stay quiet at precisely the instant
    /// it should speak.
    Returned,
    /// Mail from another part of the agent, addressed to this conversation. It drives a
    /// turn on its own — that is what makes a message *reach* the person rather
    /// than sit in a mailbox until they happen to say something next.
    Mail(Vec<crate::foundation::registry::Message>),
}

/// Parse a duration token: a bare integer is seconds, or an integer with an
/// `s`/`m`/`h` suffix (`30s`, `20m`, `1h`). `None` for anything unparseable, so a
/// malformed setting falls back to its default rather than taking a wrong value.
///
/// Used only by [`duration_tunable`] — the config knobs (`pulse`, `reflect_every`,
/// `reflect_max`, `vendor_probe`) are written by hand and want this shorthand.
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

#[derive(Clone)]
pub struct Reaction {
    inner: Arc<ReactionInner>,
}

struct ReactionInner {
    memory: Memory,
    agent: AgentLayer,
    /// The reaction's single outbound seam: every channel signal it produces —
    /// text, synthesized speech, views — goes out here in transport-free form
    /// (see [`outbound`]). A transport adapter binds these to a wire. The reaction
    /// has no knowledge of HTTP, `Content-Type`, or response framing.
    out: mpsc::Sender<OutboundSignal>,
    /// Structured visibility into the session lifecycle. Turn, session,
    /// worker events feed it; the HTTP front serves it.
    observatory: Observatory,
    /// Compiles agent-authored view source into an ESM module the browser
    /// imports. Invoked just-in-time when a view segment is released, so the
    /// compiled module URL is what rides the /view channel.
    view_compiler: crate::mind::views::ViewCompiler,
    /// The tool-sink slot the `/mcp` server routes tool calls through. Each
    /// reaction loop registers its sink here as it stands up; shared (cloneable)
    /// with the HTTP front. See [`tools`].
    tools: ToolRegistry,
    /// Process-wide barge-in state. The STT relay reports recognized speech here; the
    /// sequencer stamps each turn's voice span; `run_turn` drains the inferred
    /// "what went unheard" note into the next prompt. See [`interrupts`].
    interrupts: InterruptRegistry,
    /// Shared, process-wide LLM-vendor reachability + recovery policy. Read by every
    /// reaction loop (via [`Vendor::turn_gate`]) to decide whether and when to drive a
    /// turn; managed energy is written by the global vendor gate, while turn failures
    /// write only their own retry/permanent reasons. See [`Vendor`].
    vendor: Arc<Vendor>,
    /// Wakes every parked reaction loop after the process-wide gate changes level. The
    /// level itself lives in [`Vendor`], so missed notifications are harmless.
    vendor_wake: tokio::sync::Notify,
    /// Live-subscriber counts, shared with the HTTP front. Rendered into
    /// each turn as one human-model presence sentence, so the mind knows which
    /// channels actually reach the person right now.
    presence: crate::body::presence::Presence,
    /// The live appearance state, shared (a cloneable handle) with the
    /// HTTP front's view bus. Read into each turn as `## On screen now` so the agent
    /// can see what it has shown — the screen is its own presentation surface, and
    /// without this it dismisses/re-shows views by guessing ids from the transcript.
    /// Agent-authored views are emitted via `show` → binder → `ViewBus::apply`.
    /// The vendor gate writes its one host-owned condition view directly through the
    /// idempotent `ViewBus::reconcile` path.
    views: crate::foundation::server::ViewBus,
    /// Precompiled full-screen managed-energy view. The stable id deliberately
    /// remains `vendor-outage` so old retained snapshots can be reconciled away.
    energy_view: ViewEnvelope,
    /// Absolute path to the agent's view workshop (`<data_dir>/views`).
    /// Handed to every worker session as its `cwd`, so a build sub-agent works in a
    /// real project dir — `ls`-ing existing projects, writing source — like a human
    /// in their repo. Absolutized at startup (the child may run with a different cwd).
    views_dir: PathBuf,
    /// Monotonic turn counter. Each turn claims the next id;
    /// it tags audio spans and the channel logs so a reply is traceable end to
    /// end. (The client no longer needs it — turns are internal to the mind.)
    turn_seq: AtomicU64,
    voice: Mutex<Option<VoiceHandle>>,
    /// Wall-monotonic time of the most recent inbound human signal — the global
    /// "fresh input" signal the single consolidated reflection
    /// clock reads to decide base-vs-backoff cadence (see [`reflection`]).
    /// Written in [`Reaction::deliver`]; read each reflection tick.
    last_signal_at: std::sync::Mutex<Instant>,
    /// Wakes the consolidated reflection loop when fresh input lands, so activity
    /// that goes active after a long quiet doesn't wait out the backed-off gap
    /// before its first pass — the loop re-derives its deadline on every notify.
    reflect_wake: tokio::sync::Notify,
    /// Process-wide shutdown signal, triggered by [`crate::run_with_shutdown`] the
    /// moment a SIGINT/SIGTERM or the tray's Quit is observed. Read by the conversation
    /// loop, the reflection loop, and the drive retry path so that, once shutdown
    /// begins, an idle loop winds down promptly and a failed prompt does **not**
    /// restart an agent session — the children just received the same signal, and a
    /// respawn here would race the subprocess reap and could orphan a child.
    shutdown: Shutdown,
    /// Becomes true after the HTTP server has been spawned on its bound listener.
    /// Eager agent sessions attach to our `/mcp` endpoint at `thread/start`, so
    /// startup warming waits on this structural edge instead of racing the server.
    server_ready: watch::Receiver<bool>,
}

struct VoiceHandle {
    id: registry::SessionId,
    inbound: mpsc::Sender<LoopInput>,
}

pub async fn start(
    memory: Memory,
    agent: AgentLayer,
    mut inbound_rx: mpsc::Receiver<Signal>,
    mut warm_rx: mpsc::Receiver<()>,
    out: mpsc::Sender<OutboundSignal>,
    observatory: Observatory,
    view_compiler: crate::mind::views::ViewCompiler,
    tools: ToolRegistry,
    interrupts: InterruptRegistry,
    presence: crate::body::presence::Presence,
    views: crate::foundation::server::ViewBus,
    views_dir: PathBuf,
    shutdown: Shutdown,
    server_ready: watch::Receiver<bool>,
) -> anyhow::Result<Reaction> {
    let (source, geom) = crate::mind::views::builtin::out_of_energy_view();
    let energy_view = ViewEnvelope {
        id: crate::mind::views::builtin::OUT_OF_ENERGY_VIEW_ID.to_string(),
        op: ViewOp::Show,
        module_url: Some(
            view_compiler
                .compile(source)
                .await
                .context("compiling the built-in out-of-energy view")?,
        ),
        traits: Some(
            serde_json::from_str(geom).context("parsing the built-in out-of-energy traits")?,
        ),
    };
    let vendor = Arc::new(Vendor::new(vendor_down_after(), backoff_base()));
    let reaction = Reaction {
        inner: Arc::new(ReactionInner {
            memory,
            agent,
            out,
            observatory,
            view_compiler,
            tools,
            interrupts,
            presence,
            views,
            energy_view,
            views_dir,
            turn_seq: AtomicU64::new(0),
            voice: Mutex::new(None),
            vendor,
            vendor_wake: tokio::sync::Notify::new(),
            last_signal_at: std::sync::Mutex::new(Instant::now()),
            reflect_wake: tokio::sync::Notify::new(),
            shutdown,
            server_ready,
        }),
    };

    // Reconcile both sides of the restored level before the HTTP listener starts:
    // observed pause => the view is present; available => a retained stale copy is
    // removed while every unrelated view remains.
    let mut gate_energy = crate::foundation::energy_state::subscribe();
    reaction.reconcile_energy_level().await;

    // One process-wide gate owns the managed-energy lifecycle end to end. It applies
    // Pause/Resume to the vendor scheduler, owns the retained view, wakes held conversation
    // loops, and polls the broker only while an observed 402 remains active.
    let gate_reaction = reaction.clone();
    tokio::spawn(async move {
        loop {
            gate_reaction.reconcile_energy_level().await;
            let wait = if crate::foundation::energy_state::is_out() {
                tokio::select! {
                    event = gate_energy.recv() => Some(event),
                    _ = tokio::time::sleep(ENERGY_RECOVERY_POLL) => {
                        let data_dir = gate_reaction.inner.memory.data_dir().to_path_buf();
                        let _ = crate::foundation::broker::poll_energy_now(&data_dir).await;
                        None
                    }
                    _ = gate_reaction.inner.shutdown.cancelled() => break,
                }
            } else {
                tokio::select! {
                    event = gate_energy.recv() => Some(event),
                    _ = gate_reaction.inner.shutdown.cancelled() => break,
                }
            };
            if matches!(
                wait,
                Some(Err(tokio::sync::broadcast::error::RecvError::Closed))
            ) {
                break;
            }
        }
    });
    let dispatch_reaction = reaction.clone();

    tokio::spawn(async move {
        while let Some(signal) = inbound_rx.recv().await {
            dispatch_reaction.deliver(signal).await;
        }
        tracing::warn!("reaction inbound channel closed; dispatch loop exiting");
    });

    // Warm-up requests: a presence GET (a client opening a `/api/out/*` long-poll)
    // asks us to stand the voice up now, so its subprocess and agent session are open
    // before the first utterance lands. `ensure_voice` is idempotent — repeated GETs
    // against an already-live loop are no-ops.
    let warm_reaction = reaction.clone();
    tokio::spawn(async move {
        while warm_rx.recv().await.is_some() {
            warm_reaction.ensure_voice().await;
        }
        tracing::warn!("reaction warm channel closed; warm-up loop exiting");
    });

    // Stand the voice up at boot so its subprocess and agent session are open before
    // anything arrives — the same warm-up a presence GET asks for, just not waiting
    // for one. There is nothing to *select* here any more: the machinery this
    // replaced picked which of N conversations were worth re-warming, at the cost of a
    // state file and an activity scan, and with one conversation the answer is
    // always "the one".
    let boot_reaction = reaction.clone();
    tokio::spawn(async move {
        boot_reaction.ensure_voice().await;
    });

    // Consolidated reflection ("sleep"): one pass over the shared frontier on
    // one global clock. One writer touches the shared facet/people stores.
    // **Registered synchronously here**, like Cognition below and for the same reason:
    // the address must exist before anything can be told to use it. Reflection used to
    // register *inside* each pass, so between passes it resolved to nothing and during
    // one its id was nobody's to know.
    let reflection_reg = registry::register_scoped(
        registry::mint(),
        Role::Reflection,
        None,
        "tending the agent's own house".to_string(),
    );
    reflection::spawn(reaction.clone(), reflection_reg);

    // Cognition: the  brain the conversation hands work up to.
    //
    // **Registered here, synchronously, before the task is spawned.** `tokio::spawn`
    // makes no ordering promise, and registering inside the loop would leave a window at
    // boot in which `send_message(to: "cognition")` — a thing the prompts now tell agents
    // to do — resolves to nothing. `start` runs before any reaction loop exists and conversation
    // loops are the only senders, so doing it on this line closes the window structurally
    // rather than making it merely unlikely.
    //
    // The registration is the address and lives as long as the process; Cognition opens
    // and primes its long-lived agent session as soon as the HTTP/MCP server is ready.
    let cognition_reg = registry::register_scoped(
        registry::mint(),
        Role::Cognition,
        None,
        "the shared brain".to_string(),
    );
    cognition::spawn(reaction.clone(), cognition_reg);

    Ok(reaction)
}

/// Channels that do **not** count as a conversation being alive. Exactly one: `clock`,
/// where the host's own wakes are recorded — a pulse firing, a return observed.
///
/// This is load-bearing. Pulses are journaled (a restart otherwise sees a turn with
/// no cause), but a heartbeat is not a conversation, and anything that spends money
/// on the strength of "this conversation looks busy" would otherwise feed itself: the
/// re-warm gate below re-warms an idle conversation, whose first act is a pulse, whose row
/// makes the conversation look freshly active, so it is re-warmed again next boot —
/// forever, each one costing a subprocess and an LLM call. Reflection has the same
/// shape (see [`heartbeat::reflectable`]): a conversation left alone would tick its way over
/// the frontier threshold on heartbeats and reflect on nothing.
///
/// Excluding the channel is exact rather than a heuristic on entry bodies: nothing
/// but the clock is ever written there, which is the reason the clock got a channel
/// of its own. Note this excludes clock rows from being a *reason* to act — never
/// from being read; a reconstruction still sees every wake.
///
/// **Only the clock belongs here, and `worker` specifically does not** — this list is
/// read by two questions, and they want different answers. "Is the conversation alive?" (the
/// re-warm gate below) and "is there enough here to consolidate?"
/// ([`heartbeat::reflectable`]) share it, and a worker report is not presence but *is*
/// content worth settling into an episode. Excluding it here would silently stop
/// finished work from ever reaching the episodes.
const NON_ACTIVITY_CHANNELS: [&str; 1] = ["clock"];


impl Reaction {
    async fn reconcile_energy_view(&self, out: bool) {
        let envelope = if out {
            self.inner.energy_view.clone()
        } else {
            ViewEnvelope {
                id: crate::mind::views::builtin::OUT_OF_ENERGY_VIEW_ID.to_string(),
                op: ViewOp::Dismiss,
                module_url: None,
                traits: None,
            }
        };
        self.inner.views.reconcile(envelope).await;
    }

    /// Re-apply the current managed-energy level to every owner: scheduler, retained
    /// view, and parked reaction loops. This is used for startup, live edges, lag
    /// recovery, and every fast poll while paused.
    async fn reconcile_energy_level(&self) {
        let out = crate::foundation::energy_state::is_out();
        let changed = if out {
            self.inner.vendor.note_energy_paused()
        } else {
            self.inner.vendor.resume_energy()
        };
        self.reconcile_energy_view(out).await;
        if changed {
            self.inner.vendor_wake.notify_waiters();
        }
    }

    /// Wait until eager agent sessions can attach to the live `/mcp` endpoint.
    ///
    /// A watch channel is used because every startup conversation waits independently and all
    /// of them must observe the same retained edge. Shutdown wins so a failed startup
    /// cannot leave warm-up tasks parked forever.
    pub(super) async fn wait_for_server_ready(&self) -> bool {
        let mut ready = self.inner.server_ready.clone();
        loop {
            if *ready.borrow() {
                return true;
            }
            tokio::select! {
                changed = ready.changed() => {
                    if changed.is_err() {
                        return false;
                    }
                }
                _ = self.inner.shutdown.cancelled() => return false,
            }
        }
    }

    async fn deliver(&self, signal: Signal) {
        // Mark activity and poke the consolidated reflection clock, so a conversation
        // going active after a long quiet gets its first pass without waiting out the
        // backed-off gap.
        *self.inner.last_signal_at.lock().unwrap() = Instant::now();
        self.inner.reflect_wake.notify_one();

        let (voice_id, sender) = self.get_or_create_voice().await;
        // The signal is now accepted by the voice. Marking the Reaction
        // session here closes the small gap between the final transcript and the
        // loop's next receive; the loop owns the matching finish edge.
        registry::global().start_turn(voice_id);

        // A new signal never cancels the in-flight prompt: the serial loop folds it
        // into the next turn (fix-forward), and the lightweight reaction decides per
        // turn whether to act or wait for the rest.
        if let Err(err) = sender.send(LoopInput::Human(signal)).await {
            registry::global().finish_turn(voice_id);
            tracing::error!(error = %err, "reaction inbound channel closed; dropping signal");
        }
    }

    /// Stand the voice's loop up now (idempotent), so its warm-up prologue runs and
    /// it is hot before the first utterance. Driven by a presence signal — a client
    /// opening one of the `/api/out/*` long-polls; an already-live loop is a no-op.
    pub async fn ensure_voice(&self) {
        let _ = self.get_or_create_voice().await;
    }

    async fn get_or_create_voice(&self) -> (registry::SessionId, mpsc::Sender<LoopInput>) {
        let mut slot = self.inner.voice.lock().await;
        if let Some(handle) = slot.as_ref() {
            return (handle.id, handle.inbound.clone());
        }

        // Register the stable voice address before any asynchronous startup work.
        // Cognition's boot recovery may need to deliver here while the codex subprocess
        // is still opening/warming; the mailbox can safely queue that message until
        // the loop reaches its wait.
        let voice = registry::register_scoped(
            registry::mint(),
            Role::Reaction,
            None,
            "the voice".to_string(),
        );

        let voice_id = voice.id();
        let (tx, rx) = mpsc::channel::<LoopInput>(LOOP_QUEUE_CAPACITY);
        *slot = Some(VoiceHandle {
            id: voice_id,
            inbound: tx.clone(),
        });
        drop(slot);

        // The loop did not exist when the last process-wide edge was applied.
        // Reconcile from the current level before it or the first client state can
        // treat the retained condition as optional history.
        self.reconcile_energy_view(crate::foundation::energy_state::is_out()).await;

        // The tool control channel: the `/mcp` server forwards delegate/
        // create_worker calls here, the loop applies them. Register the sink before the
        // loop's session opens so a tool call can never arrive with no route.
        let (control_tx, control_rx) = mpsc::channel::<LoopControl>(LOOP_QUEUE_CAPACITY);

        // The output beats: say/show tool calls (and the loop's turn
        // brackets) flow to a dedicated sequencer task that paces speech and views.
        // Output bypasses the turn loop so it streams while the prompt still runs.
        let (beats_tx, beats_rx) = mpsc::channel::<sequencer::Beat>(LOOP_QUEUE_CAPACITY);
        {
            let seq_reaction = self.clone();
            tokio::spawn(async move {
                sequencer::run_sequencer(seq_reaction, beats_rx).await;
            });
        }

        self.inner
            .tools
            .register(
                ToolOwner::Reaction,
                ToolSink {
                    control: control_tx.clone(),
                    mouth: Some(tools::Mouth {
                        beats: beats_tx.clone(),
                        presence: self.inner.presence.clone(),
                    }),
                },
            )
            .await;

        let task_reaction = self.clone();
        // The worker registry posts its reports back into this same queue, so
        // hand the loop a sender clone to seed it.
        let task_worker_inbound = tx.clone();
        tokio::spawn(async move {
            reaction_loop(
                task_reaction,
                rx,
                task_worker_inbound,
                control_rx,
                control_tx,
                beats_tx,
                voice,
            )
            .await;
        });

        (voice_id, tx)
    }
}

/// Why the reaction loop's wait resolved. Keeps the `select!` arms tiny so the
/// borrow checker doesn't trip on mutating `workers` inside them.
enum Woke {
    Inbound(Option<LoopInput>),
    Control(Option<LoopControl>),
    /// Mail landed in the Reaction inbox.
    Mail,
    /// The person came back after an absence — see [`crate::body::presence::Presence::returns`].
    Returned,
    /// The process-wide vendor gate changed level. Re-read [`Vendor::turn_gate`];
    /// the notification carries no state and therefore cannot go stale.
    Vendor,
    Timer,
    /// Process shutdown began while this loop was idle — stop waiting and exit.
    Shutdown,
}

/// Apply one tool control command. Both are side-effects that run without a turn.
/// The live-worker map is the loop's own state, so this is the
/// only place an off-loop tool call touches them — through the control channel, no
/// locking.
async fn apply_control(
    reaction: &Reaction,
    workers: &mut workers::WorkerRegistry,
    ctl: LoopControl,
) -> Option<LoopInput> {
    match ctl {
        LoopControl::CreateWorker { id, task, kind, owner } => {
            if let Err(err) = workers.spawn_with_id(reaction, id, task, kind, owner).await {
                tracing::warn!(error = %err, "failed to create a working session");
            }
            None
        }
    }
}

async fn reaction_loop(
    reaction: Reaction,
    mut inbound: mpsc::Receiver<LoopInput>,
    worker_inbound: mpsc::Sender<LoopInput>,
    mut control: mpsc::Receiver<LoopControl>,
    // Held only to keep the control channel open: the registry holds the other
    // sender, but keeping a clone here means `control.recv()` never resolves to
    // `None` while this loop runs, so a quiet tool channel can't end the loop.
    _control_keepalive: mpsc::Sender<LoopControl>,
    // The output sequencer inlet. The loop sends each turn's TurnStart/
    // TurnEnd brackets here; the `/mcp` handler sends the say/show beats
    // between them. The same sender is the keepalive for the sequencer task.
    beats: mpsc::Sender<sequencer::Beat>,
    // Registered synchronously by `ensure_voice`, before this task is spawned,
    // so recovery can already address the conversation while its warm-up runs. Held here so
    // every loop exit unregisters it by scope.
    voice: registry::Registration,
) {
    // The conversation's persistent reaction session: opened and primed during startup when
    // possible, then reused for every turn as the conversation's continuous mind. Only this
    // loop touches it, so a plain local `Option` suffices. It is replaced only when a
    // turn fails: the `Err` arm below discards the possibly-wedged session and the next
    // turn cold-opens. Size is not a reason to replace it — the underlying agent
    // compacts its own context (see [`heartbeat`]).
    let mut reaction_session: Option<Arc<AgentSession>> = None;
    // What the live session has accumulated, for the observatory readout only. Reset
    // when the session is replaced, so the number always describes the session on air.
    let mut session_chars: usize = 0;
    // The live working sessions. Heavy/tool-using work the reaction
    // delegates runs here; workers post progress and results back through
    // `worker_inbound` into this same loop.
    let mut workers = workers::WorkerRegistry::new(worker_inbound);
    let voice_id = voice.id();
    let voice_mail = voice.mail.clone();
    // Taken once, for the life of the loop: the handle must be the same one the
    // attention lane fires, and `Presence::returns` keeps one per conversation for exactly
    // that reason.
    let came_back = reaction.inner.presence.returns();

    tracing::info!(voice = voice_id, "reaction per-reaction loop up");

    // Pull both the voice's rungs' cold starts ahead of the person's first message. Reaction
    // and Deliberation each open a subprocess, initialize the wire + MCP, and pre-send their
    // system prompt. Input and recovery mail queue while the two independent warm-ups
    // run in parallel.
    let mut startup_warm_pending = false;
    if reaction.wait_for_server_ready().await {
        startup_warm_pending = warm_sessions(
            &reaction,
            voice_id,
            &mut reaction_session,
            &mut workers,
        )
        .await;
    }

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
        // Wait for a turn-driving reason. The process-wide gate wakes this loop when
        // managed energy changes; the loop always re-reads the current vendor level.
        'wait: loop {
            let gate = reaction.inner.vendor.turn_gate();
            if startup_warm_pending && matches!(gate, TurnGate::Go) {
                startup_warm_pending = warm_sessions(
                    &reaction,
                    voice_id,
                    &mut reaction_session,
                    &mut workers,
                )
                .await;
                continue 'wait;
            }
            // Mail already sitting in `batch` (e.g. held while the vendor was down)
            // needs no fresh signal to act on — drive it now while reachable. While
            // down, fall through to the timer logic.
            if !batch.is_empty() && matches!(gate, TurnGate::Go) {
                break 'wait;
            }
            let down = !matches!(gate, TurnGate::Go);
            // While down, suppress pulses — they call the model and would just fail.
            let pulse_at = if down { None } else { pulse_every.map(|d| last_activity + d) };
            // While down, the recovery timer: the backoff retry deadline (429/generic).
            // Up → no such timer.
            let recover_at = match gate {
                TurnGate::Go => None,
                TurnGate::Retry { at } => Some(at),
                // No conversation-local deadline: the process-wide gate owns recovery.
                TurnGate::Hold => None,
            };
            let deadline = [pulse_at, recover_at]
                .into_iter()
                .flatten()
                .min();
            let woke = match deadline {
                Some(deadline) => tokio::select! {
                    biased;
                    _ = reaction.inner.vendor_wake.notified() => Woke::Vendor,
                    recvd = inbound.recv() => Woke::Inbound(recvd),
                    ctl = control.recv() => Woke::Control(ctl),
                    _ = voice_mail.notified() => Woke::Mail,
                    _ = came_back.notified() => Woke::Returned,
                    _ = sleep_until(deadline) => Woke::Timer,
                    _ = reaction.inner.shutdown.cancelled() => Woke::Shutdown,
                },
                None => tokio::select! {
                    biased;
                    _ = reaction.inner.vendor_wake.notified() => Woke::Vendor,
                    recvd = inbound.recv() => Woke::Inbound(recvd),
                    ctl = control.recv() => Woke::Control(ctl),
                    _ = voice_mail.notified() => Woke::Mail,
                    _ = came_back.notified() => Woke::Returned,
                    _ = reaction.inner.shutdown.cancelled() => Woke::Shutdown,
                },
            };
            match woke {
                Woke::Inbound(Some(s)) => {
                    enqueue(&reaction, &mut workers, &mut batch, s).await;
                    // While Down: collect mail without driving a turn. The
                    // probe cadence will attempt catch-up once the vendor
                    // recovers.
                    if !down {
                        break 'wait;
                    }
                }
                Woke::Inbound(None) => {
                    tracing::info!("reaction inbound closed; exiting loop");
                    return;
                }
                Woke::Shutdown => {
                    tracing::info!("shutdown requested; exiting per-reaction loop");
                    return;
                }
                // They came back. This is the one wake the person causes without
                // saying anything, and it exists so that "hold it for their return"
                // is a promise the host can actually keep — otherwise a return is
                // invisible until they type, or until the pulse comes round, which
                // is half an hour by default.
                //
                // While the vendor is down it is dropped rather than held: a return
                // is a *moment*, and delivering it after the outage clears would
                // announce an arrival that already went stale. Mail is held because
                // its content keeps; this does not.
                Woke::Returned => {
                    if down {
                        continue 'wait;
                    }
                    tracing::info!("presence returned; waking the voice");
                    // Their coming back is itself the activity — otherwise the pulse
                    // that was already overdue fires straight after this turn.
                    last_activity = Instant::now();
                    enqueue(&reaction, &mut workers, &mut batch, LoopInput::Returned).await;
                    break 'wait;
                }
                // Mail for the conversation's voice. It drives a turn like any other
                // reason to speak — that is what makes `send_message(to: conversation)`
                // actually reach the person rather than wait for them to say
                // something next. A spurious wake (the notify raced a take) finds
                // an empty inbox and simply goes back to waiting.
                Woke::Mail => {
                    if let Some(mail) = registry::global().drain_pending(voice_id) {
                        enqueue(
                            &reaction,
                            &mut workers,
                            &mut batch,
                            LoopInput::Mail(mail),
                        )
                        .await;
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
                        apply_control(&reaction, &mut workers, ctl).await
                    {
                        enqueue(&reaction, &mut workers, &mut batch, input).await;
                        if !down {
                            break 'wait;
                        }
                    }
                    // A control side-effect was applied; keep waiting for a
                    // turn-driving reason rather than running an empty turn.
                }
                Woke::Vendor => continue 'wait,
                Woke::Timer => {
                    let now = Instant::now();
                    if down {
                        // Only a transient backoff drives a model retry, and only with
                        // mail to deliver.
                        if let TurnGate::Retry { at } = gate
                            && at <= now
                            && !batch.is_empty()
                        {
                            tracing::info!(mail = batch.len(), "backoff retry firing");
                            break 'wait;
                        }
                        continue 'wait;
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
                        tracing::info!("pulse fired");
                        enqueue(&reaction, &mut workers, &mut batch, LoopInput::Pulse { note }).await;
                    }
                    if !batch.is_empty() {
                        break 'wait;
                    }
                }
            }
        }

        // A timer can resolve with nothing actually due; don't run an empty turn.
        // (While Down, the probe only breaks 'wait with non-empty mail, so this
        // guard is for the Up path's pulse timer.)
        if batch.is_empty() {
            continue;
        }

        // A turn-driving reason has been accepted. Include the settle window in the
        // turn: from the person's point of view the voice is already preparing its
        // response, even though it is briefly collecting adjacent input.
        registry::global().start_turn(voice_id);

        let was_down = reaction.inner.vendor.is_down();

        // Commit-after-quiet: wait for things to settle before replying. Skipped
        // while down — a backoff retry should attempt catch-up ASAP rather than wait
        // for more mail to settle (the retry cadence already coalesces arrivals).
        if !was_down {
            let closed = loop {
                while let Ok(extra) = inbound.try_recv() {
                    enqueue(&reaction, &mut workers, &mut batch, extra).await;
                }
                match timeout(RESPONSE_SETTLE, inbound.recv()).await {
                    // another utterance — keep collecting
                    Ok(Some(extra)) => enqueue(&reaction, &mut workers, &mut batch, extra).await,
                    Ok(None) => break true, // inbound closed mid-settle
                    Err(_) => break false,  // quiet elapsed → commit to a reply
                }
            };
            if closed {
                registry::global().finish_turn(voice_id);
                tracing::info!("reaction inbound closed; exiting loop");
                return;
            }
        }

        // Forget any workers that have finished, so the registry doesn't grow.
        workers.reap();

        let turn_result = run_reaction_turn(
            &reaction,
            &batch,
            &mut workers,
            &mut reaction_session,
            voice_id,
            &beats,
        )
        .await;
        registry::global().finish_turn(voice_id);

        match turn_result {
            Ok(added) => {
                // The turn delivered the mail; clear the backlog. (If this was a
                // retry, the turn already flipped the vendor Up via note_success.)
                batch.clear();
                // A reply landed — stop the presence owed-reply clock (no-op if
                // nothing was owed, e.g. a pulse turn).
                reaction.inner.presence.note_delivered();
                // Report what the session has accumulated, for the dashboard only.
                // **Nothing thresholds on this.** Bounding a session's context is the
                // underlying agent's job — it compacts in place, near its real window,
                // with numbers we cannot see from out here. See `heartbeat`'s module doc
                // for why the character-counting hot-swap that used to live here was
                // deleted rather than retuned.
                session_chars = session_chars.saturating_add(added);
                reaction.inner.observatory.set_budget(session_chars).await;
            }
            Err(err) => {
                tracing::warn!(error = %err, "turn failed");
                let managed_402 = crate::foundation::energy_state::is_402_error(&err)
                    && crate::foundation::energy_state::is_out();
                if !managed_402 {
                    session_chars = 0;
                    reaction.inner.observatory.set_budget(0).await;
                    // Non-402 failures cold-open on the next attempt. A managed 402
                    // keeps the live session and its observatory identity in place.
                    if let Some(dead) = reaction_session.take() {
                        record_reaction_session_closed(&reaction, &dead).await;
                    }
                }
                // Key on the vendor state the turn just wrote, not the pre-turn one:
                // a turn that flipped the vendor down holds the mail — a backoff drives
                // it at the next retry deadline. Only a still-reachable blip (already
                // apologized inside run_turn) drops it.
                if reaction.inner.vendor.is_down() {
                    tracing::info!(mail = batch.len(), "vendor down; holding mail for recovery");
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
        // catch-up, so leave the queue for it. `try_recv` never surfaces a pulse
        // (those are generated inside `'wait`, not sent over `inbound`).
        if !reaction.inner.vendor.is_down() {
            while let Ok(extra) = inbound.try_recv() {
                enqueue(&reaction, &mut workers, &mut batch, extra).await;
            }
        }
    }
}

/// Render just the human requests in a batch (skipping worker reports and
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

/// A reaction turn: the single fast conversational voice. An agent session
/// ([`Role::Reaction`]) on the small model, carrying `reaction.md` as its system
/// prompt and a `say` + `show` `/mcp` surface, with the agent's own built-in tools
/// switched off at session open. A turn is a single quick generation: it speaks by
/// calling `say`, and may call `show` to put a view a worker already built on
/// screen; both feed the sequencer. Text it merely types is working-out and is never
/// voiced. The speed comes from the small model + a single generation, not from
/// bypassing the adapter.
///
/// Deliberation — the conversation's reading and thinking — runs in parallel: the turn's human
/// request is handed to it ([`workers::WorkerRegistry::deliberate`]),
/// which works off the floor and reports back as an ordinary `LoopInput::Worker` the
/// reaction voices on a later turn. So the reaction stays the single fast voice.
///
/// v1 keeps it simple — no mid-turn reorganization. A turn is one fast generation, so a
/// human speaking during it just queues and the serial loop folds it into the next turn.
async fn run_reaction_turn(
    reaction: &Reaction,
    batch: &[LoopInput],
    workers: &mut workers::WorkerRegistry,
    reaction_session: &mut Option<Arc<AgentSession>>,
    voice_id: registry::SessionId,
    beats: &mpsc::Sender<sequencer::Beat>,
) -> anyhow::Result<usize> {
    let turn_id = reaction.inner.turn_seq.fetch_add(1, Ordering::Relaxed);
    reaction.inner.interrupts.note_turn_started(turn_id);

    // This turn's delta: whether the conversation's own thinking is still running (so the
    // voice can say "still on it" rather than guess), presence, any barge-in note, and
    // the new signals. The projected state it all hangs off is assembled in
    // [`turn_context`].
    let worker_status = workers.render_status().await;
    let presence_note = format!("## Presence\n{}", reaction.inner.presence.render());
    let interrupted = reaction
        .inner
        .interrupts
        .take_pending()
        .await
        .map(|i| interrupts::render_interruption(&i))
        .unwrap_or_default();
    let new_signals = format!("## New signals\n{}", render_batch(batch));
    // What the agent has on screen right now — its own presentation surface. Read
    // fresh every turn (it's a current fact, not durable memory), so a view dismissed
    // last turn is gone from this list now: the agent can see what's up and dismiss by
    // real id instead of guessing from the transcript.
    let on_screen = render_on_screen(&reaction.inner.views.on_screen().await);

    // Open (or reuse) the persistent reaction session. `reaction.md` is prepended to its
    // first prompt; the session then remembers prior turns. Whether it is fresh no
    // longer changes what the turn carries — see [`turn_context`].
    let session = match reaction_session {
        Some(s) => s.clone(),
        None => {
            let opened = open_reaction_session(reaction, voice_id).await?;
            *reaction_session = Some(opened.clone());
            opened
        }
    };

    let context = turn_context(
        &reaction.inner.memory,
        voice_id,
        &worker_status,
        &on_screen,
        &presence_note,
        &interrupted,
        &new_signals,
    )
    .await;

    // Captured before the prompt is handed over — it is moved into `drive_voice`.
    let context_chars = context.chars().count();
    tracing::info!(ctx_chars = context_chars, "reaction: prompting session");
    let _ = beats.send(sequencer::Beat::TurnStart { turn: turn_id }).await;

    let mut turn_error = None;
    let spoke = match drive_voice(&session, context).await {
        Ok(text) => {
            // Speech arrives as `say` calls, which the MCP surface already put on the
            // sequencer while the turn was running. Anything the model *typed* is
            // working-out, not utterance — voicing it too would say every reply twice,
            // and the tool's own description promises plain text is not spoken.
            tracing::info!(
                unspoken_chars = text.chars().count(),
                "reaction: turn done"
            );
            true
        }
        Err(err) => {
            tracing::warn!(error = %err, "reaction turn failed");
            let err_text = err.to_string();
            let managed_402 = crate::foundation::energy_state::is_402_error(&err)
                && crate::foundation::energy_state::is_out();
            let disposition = disposition(&err_text);

            // What the failure means for whether to keep trying. Presentation is not
            // part of this classifier: only the process-wide managed-energy gate owns
            // the out-of-energy view, and generic model/network/key failures must never
            // borrow that explanation.
            match disposition {
                // The session was the problem, not the vendor. Say nothing to the
                // process-wide gate: one crashed subprocess must not make every other
                // conversation believe the model is unreachable.
                Disposition::Restart => {
                    tracing::warn!("session fault; reopening cold (vendor untouched)");
                }
                // Out of quota, credit, or credentials. A managed 402 has already
                // raised the durable energy level from the common wire boundary. Apply
                // its scheduling hold synchronously so this failed turn cannot drop its
                // mail before the global gate task receives the edge. Other permanent
                // failures retain their own pause reason and have no energy UI.
                Disposition::Pause => {
                    let first = if managed_402 {
                        reaction.inner.vendor.note_energy_paused()
                    } else {
                        reaction.inner.vendor.note_permanent_paused()
                    };
                    if first {
                        reaction.inner.vendor_wake.notify_waiters();
                        tracing::warn!(
                            error = %err_text,
                            managed_energy = managed_402,
                            "paused: retrying cannot help"
                        );
                    }
                }
                // A blip. Absorb one, then back off — unchanged behaviour, and the
                // default for any error nobody has classified.
                Disposition::Retry => {
                    let _ = reaction.inner.vendor.note_unreachable();
                }
            }
            turn_error = Some(err);
            false
        }
    };

    // Close the bracket and record what was spoken (for barge-in resolution).
    let (done_tx, done_rx) = oneshot::channel();
    let _ = beats.send(sequencer::Beat::TurnEnd { done: done_tx }).await;
    let reply = done_rx.await.unwrap_or_default();
    reaction.inner.interrupts.end_turn(turn_id, &reply).await;

    if spoke {
        // Success clears only transient generic backoff. Managed energy and its
        // retained view are owned by the broker-backed vendor gate.
        let _ = reaction.inner.vendor.note_success();
        // Hand the turn's human request to Deliberation — the conversation's reader — so it works
        // off the floor while the voice moves on; its report rides back as a WorkerReport
        // the reaction voices on a later turn. Spawned once per conversation, then followed up.
        // Nothing to hand off on a pure report/pulse turn.
        let task = render_human_from_batch(batch);
        if !task.trim().is_empty() {
            if let Err(e) = workers.deliberate(reaction, task).await {
                tracing::warn!(error = %e, "deliberation spawn/follow-up failed");
            }
        }
    }
    if let Some(err) = turn_error {
        return Err(err);
    }
    // What this turn added to the session's context: everything we sent plus everything
    // it said back. Nothing thresholds on it any more — the underlying agent bounds its
    // own context — but the turn still reports it, because it is the one honest measure
    // of what a turn costs and the observatory renders it.
    Ok(context_chars + reply.chars().count())
}

/// One turn's whole prompt: the projected state, then this turn's delta.
///
/// **There is no fresh-session branch here, and that absence is the change.** The
/// projection used to be inlined only when a session was opened, on the reasoning that
/// the session remembers its own open and later turns need send only the delta. That
/// reasoning holds for a *transcript* and fails for *state*: a task opened, a duty
/// closed, or a conversation memory written mid-conversation is exactly what the session
/// cannot have remembered, because it did not exist yet. So the window was correct at
/// session open and drifted for every turn after — and since the conversation's session is
/// long-lived by design, that is most of the conversation. Code re-reads the current
/// state and injects it on every turn instead.
///
/// The costs are real and accepted. The block rides in every user message, so the
/// session's history accumulates one copy per turn — which is why the bound in
/// [`crate::mind::memory::snapshot::CARRIED_FORWARD_CHARS`] belongs to code. Keeping
/// that block small is now the only lever we hold on context growth, since bounding the
/// session itself is the underlying agent's job (see [`heartbeat`]). And the reads (the generated prompt, the task dimension, the log
/// tail) now happen per turn rather than per session; each is small, none can fail the
/// turn, and the alternative is an agent that answers from a stale window.
async fn turn_context(
    memory: &Memory,
    voice_id: registry::SessionId,
    worker_status: &str,
    on_screen: &str,
    presence: &str,
    interrupted: &str,
    new_signals: &str,
) -> String {
    let projected = snapshot::window(memory, voice_id).await;
    join_sections(&[projected.as_str(), worker_status, on_screen, presence, interrupted, new_signals])
}

#[cfg(test)]
mod turn_context_tests {
    use super::*;
    use crate::mind::memory::layout;
    use crate::mind::memory::tasks::{Task, TaskStatus, write_task};

    /// The bug this change exists to fix. The conversation's memory written — or a task opened
    /// — *after* the session was already up used to be invisible until the session
    /// rotated; the second turn of one live session must carry it.
    #[tokio::test]
    async fn the_projection_rides_a_reused_session_too() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();

        // Turn one, on a session opened just now: nothing written yet.
        let first = turn_context(&memory, 0, "", "", "", "", "## New signals\n>在吗").await;
        assert!(!first.contains("mid-migration"), "{first}");

        // Mid-conversation, the state moves under the live session.
        let path = layout::conversation_prompt_path(dir.path());
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        tokio::fs::write(&path, "He is mid-migration this week; keep answers terse.")
            .await
            .unwrap();
        let mut owed = Task::new("Ship the flash cards", TaskStatus::Doing);
        owed.title = "Ship the flash cards".into();
        write_task(dir.path(), &owed).await.unwrap();

        // Turn two, same session — no re-open, no rotation.
        let second = turn_context(&memory, 0, "", "", "", "", "## New signals\n>那卡片呢").await;
        assert!(second.contains("mid-migration"), "{second}");
        assert!(second.contains("- [doing] Ship the flash cards"), "{second}");
        assert!(second.contains("## New signals"), "{second}");
    }

    /// The projected state leads and the turn's delta follows, so the new signals sit
    /// last — closest to the reply the model is about to write.
    #[tokio::test]
    async fn projected_state_leads_and_the_new_signals_come_last() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        let text = turn_context(
            &memory,
            0,
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

/// Open and prime the conversation's Reaction and Deliberation sessions together.
///
/// Both operations are idempotent, so the managed-energy Resume edge can call this
/// again after a startup pause without replacing sessions that are already live.
async fn warm_sessions(
    reaction: &Reaction,
    voice_id: registry::SessionId,
    reaction_session: &mut Option<Arc<AgentSession>>,
    workers: &mut workers::WorkerRegistry,
) -> bool {
    let blocked_before = crate::foundation::energy_state::is_out();
    let (_, deliberation) = tokio::join!(
        warm_reaction_session(reaction, voice_id, reaction_session),
        workers.warm_deliberation(reaction),
    );
    if let Err(err) = deliberation {
        tracing::warn!(
            error = %err,
            "deliberation warm-up failed; first task will cold-start"
        );
    }
    let blocked_after = crate::foundation::energy_state::is_out();
    if blocked_after {
        // `SessionRun::wait` records the durable 402 edge. Apply its scheduler level
        // synchronously so queued input cannot race the global gate task.
        reaction.reconcile_energy_level().await;
    }
    blocked_before || blocked_after
}

/// Open and prime the conversation's Reaction session before its first real turn.
///
/// The prompt contains only the system layer. The sequencer is deliberately unarmed
/// until `TurnStart`, so any accidental `say`/`show` output from this prompt is
/// dropped. The first real turn still receives the fresh every-turn window and signals.
///
/// Best-effort: a failed warm closes this session and leaves the slot empty, so the
/// first real turn cold-opens normally.
async fn warm_reaction_session(
    reaction: &Reaction,
    voice_id: registry::SessionId,
    held: &mut Option<Arc<AgentSession>>,
) {
    if held.is_some() {
        return;
    }
    if crate::foundation::energy_state::is_out() {
        tracing::info!("reaction warm-up held while out of energy");
        return;
    }

    let session = match open_reaction_session(reaction, voice_id).await {
        Ok(session) => session,
        Err(err) => {
            tracing::warn!(
                error = %err,
                "reaction warm-up could not open a session; first turn will cold-start"
            );
            return;
        }
    };

    // Opening the thread *is* the warm-up: `speaking.md` rides `baseInstructions` on
    // `thread/start`, so the voice is ready as soon as the session exists. Under ACP
    // this cost a turn, because a system prompt had nowhere to go but the first message.
    tracing::info!("reaction session warmed");
    *held = Some(session);
}

async fn record_reaction_session_closed(
    reaction: &Reaction,
    session: &AgentSession,
) {
    reaction
        .inner
        .observatory
        .record(
            EventKind::SessionClosed {
                kind: Role::Reaction,
                id: session.id().to_string(),
            },
        )
        .await;
}

/// Open a fresh **reaction** session for `conversation`, carrying `reaction.md` as its system
/// prompt (prepended to the first prompt). It speaks via plain message text and gets a
/// minimal `show`-only `/mcp` surface, so a turn is a single quick generation that
/// may also put one already-built view on screen.
///
/// `voice_id` is the loop's own switchboard registration — the *same* id across every
/// reopen, because the conversation has one voice however many subprocesses
/// have hosted it. Passing it is what puts `X-HI-Session-Id` on the session's MCP
/// attach; without it the voice held a mailbox it had no identity to send from, and
/// `send_message` answered "this session has none" to the one rung that talks most.
async fn open_reaction_session(
    reaction: &Reaction,
    voice_id: registry::SessionId,
) -> anyhow::Result<Arc<AgentSession>> {
    let session = Arc::new(
        reaction
            .inner
            .agent
            .session(
                Role::Reaction,
                Some(voice_id),
                SessionOpts {
                    system_prompt: Some(
                        crate::identity::reaction_system_prompt(
                            reaction.inner.memory.data_dir(),
                        )
                        .await,
                    ),
                    cwd: None,
                    // `say` and `show`, and nothing else — enforced, not requested.
                    // The rung is fast because it *cannot* wait on anything, and that
                    // argument is worth nothing if it can quietly open a file.
                    ..Default::default()
                },
            )
            .await?,
    );
    reaction
        .inner
        .observatory
        .record(
            EventKind::SessionOpened {
                kind: Role::Reaction,
                id: session.id().to_string(),
            },
        )
        .await;
    Ok(session)
}

/// Prompt the reaction session and return its spoken text (every `agent_message_chunk`
/// concatenated). Tool calls — the reaction's only tool is `show` — are dispatched
/// server-side through hi-agent's `/mcp` (which emits the `Beat::Show`), so the drive
/// loop just keeps streaming speech past them, exactly like a worker's loop; `wait()`
/// then parks the session and surfaces any real prompt error (a gateway 402/429, a
/// transport reset) to the caller's classifier.
async fn drive_voice(session: &AgentSession, context: String) -> anyhow::Result<String> {
    let mut run = session.prompt(context).await?;
    let mut text = String::new();
    while let Some(update) = run.next_update().await {
        match update {
            SessionUpdate::Text(t) => text.push_str(&t),
            SessionUpdate::Thought(t) => {
                tracing::debug!(chars = t.chars().count(), "reaction: model is thinking");
            }
            // `show` dispatches server-side via `/mcp`; the reaction keeps speaking.
            // Its surface is `show`-only and the dispatch guard blocks any other
            // expression tool, so there is nothing to intercept here. The frame is
            // recorded at the wire by the tap, not read here — this rung interprets
            // nothing it is not about to say.
            SessionUpdate::Frame(_) => {}
        }
    }
    let result = run.wait().await?;
    tracing::info!(
        stop = ?result.stop_reason,
        reply_chars = text.chars().count(),
        "reaction: turn complete"
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
async fn record_out(reaction: &Reaction, channel: Channel, body: String) {
    let entry = JournalEntry::SignalOut {
        id: Uuid::now_v7().to_string(),
        ts: Utc::now(),
        channel,
        body,
        media: None,
        origin: Some(Origin::Reaction),
    };
    if let Err(err) = reaction.inner.memory.journal.append(entry).await {
        tracing::error!(channel = %channel, error = %err, "journal append failed for outbound signal");
    }
}

/// Append one turn-driving signal that reached the mind without crossing a wire —
/// a pulse, a return, a worker's report. Without these the log shows a turn's
/// output with nothing that could have caused it, and a restart cannot tell that
/// the turn happened at all, let alone why.
async fn record_in(
    reaction: &Reaction,
    channel: Channel,
    origin: Origin,
    body: String,
) {
    let entry = JournalEntry::SignalIn {
        id: Uuid::now_v7().to_string(),
        ts: Utc::now(),
        channel,
        body,
        stream: None,
        media: None,
        origin: Some(origin),
    };
    if let Err(err) = reaction.inner.memory.journal.append(entry).await {
        tracing::error!(channel = %channel, error = %err, "journal append failed for internal signal");
    }
}

async fn emit_thought_chunk(reaction: &Reaction, text: String) {
    // Per chunk, as it is written — not coalesced into one row per utterance. The
    // log's promise is durability before reaction, and buffering to make a neater
    // row would mean a crash mid-utterance loses words the agent already sent.
    // Readers re-join the chunks in `(ts, id)` order, which is exactly what the
    // merge already gives them.
    record_out(reaction, Channel::Text, text.clone()).await;
    let _ = reaction
        .inner
        .out
        .send(OutboundSignal::Text {
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
    reaction: &Reaction,
) {
    match emit {
        interleave::Emit::Speak(sentence) => {
            if let Some(tx) = synth_tx {
                let _ = tx.send(sentence).await;
            }
        }
        interleave::Emit::Show { id, op, source, traits } => {
            emit_view(reaction, id, op, source, traits).await
        }
    }
}

async fn emit_end_of_utterance(reaction: &Reaction) {
    let _ = reaction
        .inner
        .out
        .send(OutboundSignal::TextEnd)
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
/// dismisses at ids that are already gone. Kept to bare ids: the reaction shows/dismisses
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
            LoopInput::Pulse { note } => {
                let _ = writeln!(s, "{}", render_pulse(note));
            }
            LoopInput::Returned => {
                let _ = writeln!(s, "{RETURNED_NOTE}");
            }
            LoopInput::Mail(mail) => {
                let _ = writeln!(s, "{}", registry::render(mail));
            }
        }
    }
    s
}

/// Shared with [`cognition`] so both rungs' quiet moments arrive under the same
/// `(pulse)` marker — the prompts already key on that word, and a brain-only variant
/// would be a second vocabulary for one thing.
pub(super) fn render_pulse(note: &str) -> String {
    format!("(pulse) {note}")
}

/// What a return looks like in the turn's "New signals".
///
/// Deliberately a *fact*, not an instruction: the host reports that a window came
/// forward after a quiet stretch and says nothing about whether that deserves a
/// word. Whether to speak, and what a held thing was worth, is Reaction's — the
/// host's job here was only to make the moment observable at all.
const RETURNED_NOTE: &str =
    "(they're back) They just brought the window forward after a stretch away.";

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
        LoopInput::Pulse { note } => Some((Channel::Clock, Origin::Host, render_pulse(note))),
        // A return is the person acting, but on no channel they typed into — so
        // nothing upstream journaled it, and this is its only chance to be written
        // down. `Channel::Clock` puts it in [`NON_ACTIVITY_CHANNELS`] on purpose:
        // like a pulse and unlike a worker report, a return is *presence, not
        // content*. It should not hold a conversation warm by itself, and it should not
        // push a conversation over the frontier threshold into consolidating on nothing.
        LoopInput::Returned => Some((Channel::Clock, Origin::Host, RETURNED_NOTE.to_owned())),
        // Mail crosses no wire, so this is its only chance to be written down.
        LoopInput::Mail(mail) => Some((Channel::Worker, Origin::Worker, registry::render(mail))),
    }
}

/// Record one turn-driving input, then queue it for the turn. Every path that puts
/// something in the conversation's batch goes through here, so nothing can drive a turn
/// unlogged — which is the whole point: a turn whose cause was never written down
/// is a turn a restart cannot account for.
/// Put one input in front of the mind, journaling it on the way.
///
/// The one input that may not end up here is a worker's report: if the worker was
/// spawned by another session, the report belongs to *that* session and is delivered
/// into it instead, never reaching the conversation. Work travels up the chain of owners to
/// whoever asked for it; it does not appear beside the person's own words in a
/// conversation nobody addressed it to.
///
/// It is still journaled either way — the report crossed an agent boundary, and the
/// log records what crossed regardless of where it went next.
async fn enqueue(
    reaction: &Reaction,
    workers: &mut workers::WorkerRegistry,
    batch: &mut Vec<LoopInput>,
    input: LoopInput,
) {
    if let Some((channel, origin, body)) = journal_form(&input) {
        record_in(reaction, channel, origin, body).await;
    }
    if let LoopInput::Worker(report) = &input
        && let Some(owner) = report.owner
    {
        let text = workers::render_report(report);
        let delivery = workers.deliver_to(owner, text.clone());
        // Work travelling *up* is the edge most worth seeing, and it is host-posted
        // (`from: None`) — so nothing observed it while only `send_message` was
        // instrumented. Recorded whether or not it landed: a report that missed its
        // owner is precisely what you would open the inspector to find.
        reaction
            .inner
            .observatory
            .record(
                EventKind::MessageSent { from: None, to: owner, delivery, message: text },
            )
            .await;
        if matches!(delivery, registry::Delivery::Delivered) {
            return;
        }
        // The owner is gone. Surfacing one rung too high beats losing finished work.
        tracing::info!(
            session = report.id, owner,
            "report owner is gone; falling back to the conversation"
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
    reaction: Reaction,
    mut frames: mpsc::Receiver<Bytes>,
    out: mpsc::Sender<OutboundSignal>,
    turn: u64,
    codec: String,
) {
    let mut total = 0usize;
    while let Some(bytes) = frames.recv().await {
        total += bytes.len();
        let _ = out
            .send(OutboundSignal::AudioFrame {
                turn,
                bytes,
            })
            .await;
    }
    let _ = out
        .send(OutboundSignal::AudioEnd {
            turn,
        })
        .await;
    tracing::info!(
        target: "channel",
        dir = "out",
        channel = "audio",
        turn = turn,
        bytes = total,
        "channel out (tts stream)",
    );
    // A span that carried no frames was never heard — TTS opened and produced
    // nothing — so there is nothing to record.
    if total > 0 {
        record_out(
            &reaction,
            Channel::Audio,
            format!("spoke the reply aloud ({codec}, {total} bytes)"),
        )
        .await;
    }
}

/// Emit one agent-authored view on the /view channel for this conversation. A `show`/
/// `replace` compiles the source to a module first (just-in-time, after the
/// preceding sentence has flushed, so it stays paced to narration); a `dismiss`
/// carries no module. A compile failure is logged and the view is dropped — the
/// turn's speech already went out, so a broken view never breaks the reply.
async fn emit_view(
    reaction: &Reaction,
    id: String,
    op: ViewOp,
    source: String,
    traits: Option<ViewTraits>,
) {
    let module_url = if op == ViewOp::Dismiss {
        None
    } else {
        match reaction.inner.view_compiler.compile(&source).await {
            Ok(url) => Some(url),
            Err(err) => {
                tracing::error!(id = %id, error = %err, "view compile failed; dropping view");
                return;
            }
        }
    };
    tracing::info!(
        target: "channel",
        dir = "out",
        channel = "view",
        id = %id,
        op = ?op,
        module = module_url.as_deref().unwrap_or(""),
        "channel out (view)",
    );
    // Before it goes on the wire: showing something is as much an utterance as
    // saying it, and the screen persists across restarts, so a mind that can't read
    // back what it put up will put it up again.
    let line = render_view_line(&id, op, module_url.as_deref());
    record_out(reaction, Channel::View, line).await;
    let _ = reaction
        .inner
        .out
        .send(OutboundSignal::View {
            envelope: ViewEnvelope { id, op, module_url, traits },
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
mod duration_tests {
    use super::parse_delay;
    use std::time::Duration;

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
}
