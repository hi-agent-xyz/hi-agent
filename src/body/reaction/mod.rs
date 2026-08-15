//! Reaction — the *mind*. Per-conversation queues + one persistent session per conversation.
//!
//! One mpsc per conversation, one task per conversation; turns run serially against a single
//! Reaction agent session that is opened and primed when the conversation stands up, then
//! reused as the conversation's continuous voice. The reading and thinking is Cognition's,
//! which stands in its own loop and answers by mail; Reaction never blocks on it.
//!
//! ## Turn-taking lives here, not in the client
//!
//! The client is a dumb face: it streams the mic and renders what arrives. It
//! does not decide *when* the agent speaks — the mind does, and these are the
//! three rules:
//!
//! 1. **The settle batches; it does not decide when to speak.** A finalized
//!    utterance does not immediately drive a turn: the human speaks in bursts,
//!    each burst arrives as its own inbound signal (one segmented utterance over
//!    `/api/in/audio`), and the loop waits [a short quiet](RESPONSE_SETTLE) so a
//!    turn is not spent per word. **That is all it does.** It was once the
//!    turn-taking rule as well — wait for quiet, then answer — and it could never
//!    have been one: it counts *finalized utterances*, which a person mid-thought
//!    produces constantly. Measured live, every gap between one speaker's bursts
//!    was longer than the settle, so it coalesced nothing and the voice answered
//!    a half-finished sentence four times in twenty-five seconds.
//! 2. **The floor decides, and it decides at the mouth.** Whether the room is the
//!    voice's to speak into is asked when the words are ready, not when the turn
//!    that wrote them began — seconds earlier, which is long enough for the person
//!    to have started a new sentence or said the thing that mattered. `say` is
//!    refused if their voice is sounding, or if a line landed that this turn never
//!    saw; a refusal is a refusal, not a queue, and the reply is written afresh by
//!    the next turn. See [`floor`].
//! 3. **Fix-forward, no reflexive cancel.** A new signal never cancels the
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
//!    [`floor`].
//!
//! Rule 2 makes generation speculative, which reverses a claim this header used
//! to make — *superseded drafts are never generated rather than
//! generated-then-discarded*. They are now generated and discarded, because the
//! alternative is deciding whether someone has finished talking **before**
//! spending the turn, and nothing on the input side can know that. The discarded
//! turn is not wasted: its thinking stays in the warm session, so the reply that
//! replaces it is better for having been written twice.
//!
//! ## Heavy work goes to a working session, not onto the floor
//!
//! The mind keeps a single voice, so it must never block the floor on slow
//! work. When a turn needs research, multi-step tool use, or anything
//! long-running, the mind calls the `delegate` tool with the task; the reaction
//! spawns a [`workers`] session for it and keeps talking. The worker
//! runs with the same substrate (memory, tools) but holds no `hi_say`, and
//! posts its result — or a question, if it gets stuck — back into this conversation's
//! queue, where it lands as just another input the next turn folds into what the
//! mind says.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Context;
mod cognition;
mod duties;
mod heartbeat;

// The standing instruction sentences these two assemble, re-exported so the tool layer's
// prefix sweep can read them without the modules themselves going public. Nothing else
// reads them, so the re-export is test-only. See [`heartbeat::PROACTIVITY_HEADING`].
#[cfg(test)]
pub(crate) use duties::DUTY_BRIEF_TAIL;
#[cfg(test)]
pub(crate) use heartbeat::{CONSOLIDATION_TOOLS, PROACTIVITY_HEADING};
mod reflection;
mod interleave;
mod floor;
pub mod outbound;
mod sequencer;
mod tools;
mod workers;

pub use duties::DutyDelivery;
pub use floor::{Busy, Floor};
pub use outbound::OutboundSignal;
pub use tools::{LoopControl, Said, Spoken, ToolOwner, ToolRegistry, ToolSink};

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

/// How long the queue is left to settle before a turn is spent on it — a
/// **batching** window, and nothing more.
///
/// It reads as a patience dial and is not one. Whether the person has finished
/// their thought is decided at the mouth ([`floor`]), where the answer is still
/// current; this only stops one thought arriving as six utterances from costing
/// six generations. So it wants to be *small*: every millisecond here is latency
/// on a reply into a silence that is already real.
///
/// **It was once the turn-taking rule**, and the reason it could not be is that a
/// person mid-thought produces finalized utterances constantly — this timer's only
/// input. Live, one speaker's gaps between bursts ran 1.4–4.9s, every one of them
/// past this window, so it coalesced nothing at all while claiming to be what kept
/// the voice from answering half a thought.
const RESPONSE_SETTLE: Duration = Duration::from_millis(700);

/// Default idle interval between glance-ups — the agent's recurring moment of
/// self-attention, and **Cognition's alone**. It is not a schedule of work: it injects
/// bare situational facts ("nothing new for 30m") and `cognition.md` tells the brain
/// what such a moment is for (read down the ledger, glance at the setups it owns); most
/// of them should conclude with nothing to do. Override via `pulse`; `0`/`off` silences
/// the recurring arm and never the boot one, which is restart recovery rather than a
/// cadence (see [`cognition`]).
///
/// **The voice has no pulse, and that absence is deliberate.** The conversation loop used
/// to wake itself on this same cadence and run a turn into an empty room. Three things
/// killed it. Reaction is tools-off, so a wake handed it nothing it could not already see
/// in the window it gets on *every* turn — the least-informed rung was the one deciding
/// whether to speak. The measured outcome was silence at the exact moment there was
/// something to do: two post-restart pulses, both concluding without a `say`, while a
/// standing duty sat unread in the ledger (`docs/user-journeys/gaps.md#1`). And it was the
/// most expensive wake in the system, because the projected window rides every turn and
/// accumulates in the session. Unprompted speech now comes from the rung that can
/// actually check: Cognition glances up, reads the ledger, and mails the voice — which
/// drives a turn like any other reason to speak.
const DEFAULT_PULSE: Duration = Duration::from_secs(1800);

/// Resolve the glance-up interval from the stored `pulse` tunable in duration grammar
/// if set (`None` for `0`/`off` — the recurring arm disabled), else [`DEFAULT_PULSE`].
/// Read by [`cognition`], which paces itself on it, and by [`check_in_cap`], which takes
/// it as the ceiling the check-in floor widens to: one "how often does this agent look up
/// from what it's doing" setting, not a brain one plus a conversation one that can
/// disagree. It also keeps journey testing honest — dropping `pulse` for a session
/// speeds up every cadence there is, rather than all but one.
pub(super) fn pulse_interval() -> Option<Duration> {
    duration_tunable(config::tunables::get(config::KEY_PULSE), DEFAULT_PULSE)
}

/// The floor under an open-ended silence: how long the voice may stay quiet while its
/// own thinking is still running before the host wakes it to say where things stand.
///
/// Five minutes because the failure it answers was measured — an errand ran, the voice
/// said "I'll report once confirmed" with no number, and the person filled the next
/// thirteen to eighteen minutes by asking "progress?" three times in one morning. It
/// only ever fires while work is genuinely in flight, and a wake is permission to
/// speak, not an obligation: the voice reads the room and may stay quiet.
const DEFAULT_CHECK_IN: Duration = Duration::from_secs(300);

/// The floor gap for an open-ended silence ([`DEFAULT_CHECK_IN`]), or `None` when
/// `check_in` is `0`/`off` — which leaves only the check-ins the voice arms itself
/// through `say`'s `back_in`, never no check-ins at all.
fn check_in_interval() -> Option<Duration> {
    duration_tunable(config::tunables::get(config::KEY_CHECK_IN), DEFAULT_CHECK_IN)
}

/// The ceiling the floor backs off to. A job that runs for hours should not be
/// interrupted every five minutes, so each consecutive host-armed check-in doubles the
/// gap — the reflection backoff's shape — and stops at the glance-up cadence, which is
/// already the answer to "how often does this agent look up from what it's doing". A word
/// from the voice or the person resets it: the conversation is live again.
fn check_in_cap() -> Duration {
    pulse_interval().unwrap_or(DEFAULT_PULSE)
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
    /// The voice's own check-in coming due — it said they'd hear back by now, or it
    /// left a silence open-ended while its thinking ran and the host put a floor under
    /// it ([`tools::NextWord`]).
    ///
    /// **The only clock this loop still answers to**, and it never wears the `(pulse)`
    /// marker Cognition's glance-up does. That marker means a quiet moment almost nothing
    /// is worth breaking; this is the moment a word was *owed*, and rendering the two the
    /// same would tell the voice to stay quiet at precisely the instant it should speak.
    CheckIn { owed: tools::Owed },
    /// Mail from another part of the agent, addressed to this conversation. It drives a
    /// turn on its own — that is what makes a message *reach* the person rather
    /// than sit in a mailbox until they happen to say something next.
    Mail {
        mail: Vec<crate::foundation::registry::Message>,
        /// Whether this mail answers a question the voice handed down and the person is
        /// still waiting on — in which case it is **a reply owed**, not one signal among
        /// many.
        ///
        /// **This flag replaces `WorkerReport::is_deliberation`, and it exists for the
        /// same failure.** Deliberation's answer arrived on the report path, where the
        /// host could frame it as must-relay; Cognition's arrives as ordinary mail, and
        /// Cognition's own prompt tells it that everything it sends is *a proposal, never
        /// a delivery*. That is right for a finding it raised on its own and wrong for an
        /// answer to a question a person asked thirty seconds ago: a voice that speaks
        /// only what it chooses to is entitled to drop a proposal, and dropping it means
        /// the person who asked never hears back. So the host, which is the only thing that
        /// knows a hand-down is outstanding, says which kind of message this is.
        owed: bool,
    },
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
    /// Process-wide floor state. The STT relay reports recognized speech here; the
    /// sequencer stamps each turn's voice span; `run_turn` drains the inferred
    /// "what went unheard" note into the next prompt, and gates `say` on whether the
    /// floor is theirs at all. See [`floor`].
    floor: Floor,
    /// Shared, process-wide LLM-vendor reachability + recovery policy. Read by every
    /// reaction loop (via [`Vendor::turn_gate`]) to decide whether and when to drive a
    /// turn; managed energy is written by the global vendor gate, while turn failures
    /// write only their own retry/permanent reasons. See [`Vendor`].
    vendor: Arc<Vendor>,
    /// Wakes every parked reaction loop after the process-wide gate changes level. The
    /// level itself lives in [`Vendor`], so missed notifications are harmless.
    vendor_wake: tokio::sync::Notify,
    /// Live-subscriber counts, shared with the HTTP front. Read at one place only —
    /// [`sequencer::open_tts`], to decide whether a speech span is worth
    /// synthesizing. Nothing about it is projected: see
    /// [`crate::body::attachments`].
    attachments: crate::body::attachments::Attachments,
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
    duty_rx: mpsc::Receiver<DutyDelivery>,
    out: mpsc::Sender<OutboundSignal>,
    observatory: Observatory,
    view_compiler: crate::mind::views::ViewCompiler,
    tools: ToolRegistry,
    floor: Floor,
    attachments: crate::body::attachments::Attachments,
    views: crate::foundation::server::ViewBus,
    views_dir: PathBuf,
    shutdown: Shutdown,
    server_ready: watch::Receiver<bool>,
) -> anyhow::Result<Reaction> {
    let source = crate::mind::views::factory::out_of_energy_view();
    let energy_view = ViewEnvelope {
        id: crate::mind::views::factory::OUT_OF_ENERGY_VIEW_ID.to_string(),
        op: ViewOp::Show,
        module_url: Some(
            view_compiler
                .compile(source)
                .await
                .context("compiling the built-in out-of-energy view")?,
        ),
        // Declares nothing. The notice takes the agent's half of the screen; the
        // conversation rails beside it, so the person keeps the record of what was
        // said and the line to answer on. See `docs/arch/stage.md`.
        traits: None,
        // No ref on purpose. A ref exists so a *restored* view can be recompiled from
        // its source, and the condition slot is never restored from one: it is
        // host-owned and re-derived here, from the embedded source, on every boot —
        // `reconcile_energy_view` re-applies the live level at startup. Handing it a
        // ref would put a disk read into the out-of-energy path, which
        // `out_of_energy_view` deliberately keeps out of it.
        view_ref: None,
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
            floor,
            attachments,
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
        registry::mint(Role::Reflection, None),
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
        registry::mint(Role::Cognition, None),
        Role::Cognition,
        None,
        "the shared brain".to_string(),
    );
    cognition::spawn(reaction.clone(), cognition_reg);

    // The duty inbox: where a listener's traffic lands. Spawned after Cognition because a
    // handler it opens is owned by Cognition, and an owner that resolves to nothing is an
    // escalation addressed to nobody. Registration above is synchronous, so this line is
    // ordered rather than merely likely.
    duties::spawn(reaction.clone(), duty_rx);

    Ok(reaction)
}

/// Channels that do **not** count as a conversation being alive. Exactly one: `clock`,
/// where the host's own wakes are recorded — today a check-in coming due, and nothing
/// else since the voice's pulse was cut.
///
/// This is load-bearing. Those wakes are journaled (a restart otherwise sees a turn with
/// no cause), but the host noticing the time is not a conversation, and anything that
/// spends money on the strength of "this conversation looks busy" would feed itself: a
/// conversation left alone would tick its way over the frontier threshold on its own clock
/// rows and reflect on nothing. [`heartbeat::reflectable`] is the reader that makes that
/// concrete — and now the only one, the re-warm gate that used to be the other having gone
/// when the boot warm-up became unconditional.
///
/// Excluding the channel is exact rather than a heuristic on entry bodies: nothing
/// but the clock is ever written there, which is the reason the clock got a channel
/// of its own. Note this excludes clock rows from being a *reason* to act — never
/// from being read; a reconstruction still sees every wake.
///
/// **Only the clock belongs here, and `worker` specifically does not**: a worker report is
/// not presence but *is* content worth settling into an episode. Excluding it here would
/// silently stop finished work from ever reaching the episodes.
const NON_ACTIVITY_CHANNELS: [&str; 1] = ["clock"];


impl Reaction {
    async fn reconcile_energy_view(&self, out: bool) {
        let envelope = if out {
            self.inner.energy_view.clone()
        } else {
            ViewEnvelope {
                id: crate::mind::views::factory::OUT_OF_ENERGY_VIEW_ID.to_string(),
                op: ViewOp::Dismiss,
                module_url: None,
                traits: None,
                view_ref: None,
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
        registry::global().start_turn(&voice_id);

        // A new signal never cancels the in-flight prompt: the serial loop folds it
        // into the next turn (fix-forward), and the lightweight reaction decides per
        // turn whether to act or wait for the rest.
        //
        // **Counted here, before the send, because this is the moment it becomes
        // something a running generation cannot have seen.** The loop dequeues
        // nothing while a turn runs, so anything keyed off the batch would not move
        // until the turn that needs the fact is already over. A reply produced after
        // this point is out of date, and [`floor::Floor::may_speak`] refuses it.
        self.inner.floor.note_heard();
        if let Err(err) = sender.send(LoopInput::Human(signal)).await {
            registry::global().finish_turn(&voice_id);
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
            return (handle.id.clone(), handle.inbound.clone());
        }

        // Register the stable voice address before any asynchronous startup work.
        // Cognition's boot recovery may need to deliver here while the codex subprocess
        // is still opening/warming; the mailbox can safely queue that message until
        // the loop reaches its wait.
        let voice = registry::register_scoped(
            registry::mint(Role::Reaction, None),
            Role::Reaction,
            None,
            "the voice".to_string(),
        );

        let voice_id = voice.id();
        let (tx, rx) = mpsc::channel::<LoopInput>(LOOP_QUEUE_CAPACITY);
        *slot = Some(VoiceHandle {
            id: voice_id.clone(),
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

        // How many utterances the mouth has accepted. The loop holds the other end so a
        // turn can tell whether it spoke while its bracket is still open.
        let said = Arc::new(AtomicU64::new(0));
        // When the voice next owes them a word. `say` writes it from the `/mcp` task;
        // the loop below reads it as a deadline. See [`tools::NextWord`].
        let next_word = tools::NextWord::default();

        self.inner
            .tools
            .register(
                ToolOwner::Reaction,
                ToolSink {
                    control: control_tx.clone(),
                    mouth: Some(tools::Mouth {
                        beats: beats_tx.clone(),
                        said: said.clone(),
                        next_word: next_word.clone(),
                        floor: self.inner.floor.clone(),
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
                Speaking { beats: beats_tx, said, next_word },
                voice,
            )
            .await;
        });

        (voice_id, tx)
    }
}

/// The loop's end of the mouth, whose other end is [`tools::Mouth`] on the `/mcp`
/// side. The loop sends each turn's `TurnStart`/`TurnEnd` brackets down `beats`; the
/// tool handler sends the `Say`/`Show` beats between them, so the two halves are the
/// same mouth seen from either side of a turn.
///
/// They travel together because a turn asks one question of both — *what did this
/// actually put in front of the person* — and the count is the only half that can
/// answer it while the bracket is still open.
struct Speaking {
    beats: mpsc::Sender<sequencer::Beat>,
    /// Utterances the mouth has accepted, ever. See [`tools::Mouth::said`].
    said: Arc<AtomicU64>,
    /// When the voice next owes them a word — armed by `say`, read here as a deadline.
    /// It travels with the mouth for the same reason `said` does: it is set at the
    /// instant of speech and read by the loop that has to act on it.
    next_word: tools::NextWord,
}

/// Why the reaction loop's wait resolved. Keeps the `select!` arms tiny so the
/// borrow checker doesn't trip on mutating `workers` inside them.
enum Woke {
    Inbound(Option<LoopInput>),
    Control(Option<LoopControl>),
    /// Mail landed in the Reaction inbox.
    Mail,
    /// The process-wide vendor gate changed level. Re-read [`Vendor::turn_gate`];
    /// the notification carries no state and therefore cannot go stale.
    Vendor,
    /// A deadline came up: a vendor-recovery probe, or the voice's own check-in
    /// ([`tools::NextWord`]). Which one is worked out on the far side, from the
    /// deadlines themselves, so both keep one arm.
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
        LoopControl::CreateWorker { id, title, task, kind, owner, resume, subject } => {
            if let Err(err) = workers
                .spawn_with_id(reaction, id, title, task, kind, owner, resume, subject)
                .await
            {
                tracing::warn!(error = %err, "failed to create a working session");
            }
            None
        }
        LoopControl::CancelWorker { id, reply } => {
            let _ = reply.send(workers.interrupt(id).await);
            None
        }
        LoopControl::CloseWorker { id, reply } => {
            let _ = reply.send(workers.close(id));
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
    // The loop's end of the mouth: the sequencer inlet and the utterance count.
    speaking: Speaking,
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
    // What that session has already been told, so a turn carries only what it hasn't. It
    // lives beside the session because it is a claim about that session's history, and it
    // is voided the moment the history is — see [`WindowMemo`].
    let mut window_memo = WindowMemo::default();
    // What the live session has accumulated, for the observatory readout only. Reset
    // when the session is replaced, so the number always describes the session on air.
    let mut session_chars: usize = 0;
    // The live working sessions. Heavy/tool-using work the reaction
    // delegates runs here; workers post progress and results back through
    // `worker_inbound` into this same loop.
    let mut workers = workers::WorkerRegistry::new(worker_inbound);
    let voice_id = voice.id();
    let voice_mail = voice.mail.clone();
    tracing::info!(voice = %voice_id, "reaction per-reaction loop up");

    // Pull the voice's cold start ahead of the person's first message: it opens a
    // subprocess, initializes the wire + MCP, and pre-sends its system prompt. Input and
    // recovery mail queue while it runs. Cognition warms itself in its own loop.
    let mut startup_warm_pending = false;
    if reaction.wait_for_server_ready().await {
        startup_warm_pending =
            warm_sessions(&reaction, &voice_id, &mut reaction_session, &mut window_memo).await;
    }

    // The check-in floor's current gap, doubling while the voice keeps leaving an
    // open-ended silence over running work and resetting the moment either side speaks
    // to it. `None` = `check_in: off`, i.e. only the check-ins the voice arms itself.
    let check_in_base = check_in_interval();
    let mut check_in_gap = check_in_base;

    // Whether the voice has handed something down that the person is still waiting on.
    // Set when a turn hands the human request to Cognition, cleared when Cognition's
    // answer comes back — which is the moment that answer must be relayed rather than
    // weighed. See [`LoopInput::Mail::owed`] for why the host has to be the one holding
    // this rather than either rung.
    let mut reply_owed = false;

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
                startup_warm_pending =
                    warm_sessions(&reaction, &voice_id, &mut reaction_session, &mut window_memo).await;
                continue 'wait;
            }
            // Mail already sitting in `batch` (e.g. held while the vendor was down)
            // needs no fresh signal to act on — drive it now while reachable. While
            // down, fall through to the timer logic.
            if !batch.is_empty() && matches!(gate, TurnGate::Go) {
                break 'wait;
            }
            let down = !matches!(gate, TurnGate::Go);
            // The voice's own check-in. Suppressed while down — it calls the model and
            // would just fail — but *not* dropped: an owed word is still owed after an
            // outage, and later rather than never is the whole point of it.
            let check_in_at = if down { None } else { speaking.next_word.due_at() };
            // While down, the recovery timer: the backoff retry deadline (429/generic).
            // Up → no such timer.
            let recover_at = match gate {
                TurnGate::Go => None,
                TurnGate::Retry { at } => Some(at),
                // No conversation-local deadline: the process-wide gate owns recovery.
                TurnGate::Hold => None,
            };
            let deadline = [recover_at, check_in_at].into_iter().flatten().min();
            let woke = match deadline {
                Some(deadline) => tokio::select! {
                    biased;
                    _ = reaction.inner.vendor_wake.notified() => Woke::Vendor,
                    recvd = inbound.recv() => Woke::Inbound(recvd),
                    ctl = control.recv() => Woke::Control(ctl),
                    _ = voice_mail.notified() => Woke::Mail,
                    _ = sleep_until(deadline) => Woke::Timer,
                    _ = reaction.inner.shutdown.cancelled() => Woke::Shutdown,
                },
                None => tokio::select! {
                    biased;
                    _ = reaction.inner.vendor_wake.notified() => Woke::Vendor,
                    recvd = inbound.recv() => Woke::Inbound(recvd),
                    ctl = control.recv() => Woke::Control(ctl),
                    _ = voice_mail.notified() => Woke::Mail,
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
                // Mail for the conversation's voice. It drives a turn like any other
                // reason to speak — that is what makes `send_message(to: conversation)`
                // actually reach the person rather than wait for them to say
                // something next. A spurious wake (the notify raced a take) finds
                // an empty inbox and simply goes back to waiting.
                Woke::Mail => {
                    if let Some(mail) = registry::global().drain_pending(&voice_id) {
                        // A reply is owed only if one was outstanding *and* this mail is
                        // from the rung it was handed to. Mail from Reflection, or an
                        // unsolicited finding Cognition raised on its own, stays a
                        // proposal the voice weighs — that judgment is the voice's and
                        // this must not overrule it.
                        let from_brain = registry::global()
                            .session_of_role(Role::Cognition)
                            .is_some_and(|brain| {
                                mail.iter().any(|m| m.from.as_ref() == Some(&brain.id))
                            });
                        let owed = reply_owed && from_brain;
                        if owed {
                            reply_owed = false;
                        }
                        enqueue(
                            &reaction,
                            &mut workers,
                            &mut batch,
                            LoopInput::Mail { mail, owed },
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
                    // The word the voice owes them. Taking it disarms it, so a voice that
                    // reads the room and stays quiet is not re-woken for the same overdue
                    // promise on the next iteration.
                    if let Some(owed) = speaking.next_word.take_due(now) {
                        // Fired whether or not anyone is looking. It used to be dropped
                        // into an empty room, because the words would have been withheld
                        // anyway and a return would wake the voice with a fresher read —
                        // and both of those went with the presence gate. What the voice
                        // says now lands in the conversation and waits there.
                        tracing::info!(promised = owed.promised, "check-in fired");
                        enqueue(&reaction, &mut workers, &mut batch, LoopInput::CheckIn { owed })
                            .await;
                    }
                    if !batch.is_empty() {
                        break 'wait;
                    }
                }
            }
        }

        // A wake can resolve with nothing to run — [`enqueue`] hands a worker's report to
        // another owner without ever pushing it, and a check-in can be cleared between the
        // deadline and the take. Don't run an empty turn. (While Down, the probe only
        // breaks 'wait with non-empty mail, so this guard is the Up path's.)
        if batch.is_empty() {
            continue;
        }

        // A turn-driving reason has been accepted. Include the settle window in the
        // turn: from the person's point of view the voice is already preparing its
        // response, even though it is briefly collecting adjacent input.
        registry::global().start_turn(&voice_id);

        let was_down = reaction.inner.vendor.is_down();

        // Let adjacent arrivals join this batch, so six utterances of one thought
        // cost one generation rather than six. **Not a decision about whether they
        // have finished** — that is asked at the mouth, when the answer is still
        // current ([`floor`]). Skipped while down: a backoff retry should attempt
        // catch-up ASAP rather than wait for more mail to settle (the retry cadence
        // already coalesces arrivals).
        if !was_down {
            let closed = loop {
                while let Ok(extra) = inbound.try_recv() {
                    enqueue(&reaction, &mut workers, &mut batch, extra).await;
                }
                match timeout(RESPONSE_SETTLE, inbound.recv()).await {
                    // another utterance — keep collecting
                    Ok(Some(extra)) => enqueue(&reaction, &mut workers, &mut batch, extra).await,
                    Ok(None) => break true, // inbound closed mid-settle
                    Err(_) => break false,  // quiet elapsed → the batch is closed
                }
            };
            if closed {
                registry::global().finish_turn(&voice_id);
                tracing::info!("reaction inbound closed; exiting loop");
                return;
            }
        }

        // Forget any workers that have finished, so the registry doesn't grow.
        workers.reap();

        // Why this turn is running, read before the batch is cleared — it decides how
        // the check-in floor paces itself below.
        let by_human = batch.iter().any(|i| matches!(i, LoopInput::Human(_)));
        let by_floor = batch
            .iter()
            .any(|i| matches!(i, LoopInput::CheckIn { owed } if !owed.promised));
        // The agent's own thinking coming back — the thing a promise made *about* it was
        // for. Captured with what was armed going in, so the discharge below can tell an
        // untouched promise from a fresh one this turn just made.
        let by_thinking_back = batch.iter().any(|i| matches!(i, LoopInput::Mail { owed: true, .. }));
        let armed_before = speaking.next_word.peek();
        let said_before = speaking.said.load(Ordering::Relaxed);

        let turn_result = run_reaction_turn(
            &reaction,
            &batch,
            &mut reaction_session,
            &mut window_memo,
            &voice_id,
            &speaking,
            &mut reply_owed,
        )
        .await;
        registry::global().finish_turn(&voice_id);

        match turn_result {
            Ok(added) => {
                // The turn delivered the mail; clear the backlog. (If this was a
                // retry, the turn already flipped the vendor Up via note_success.)
                batch.clear();
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

        // A promise is discharged by the thing it was about coming back. This turn was
        // handed its own thinking with an instruction to relay it, so waking the voice
        // later to say "you told them they'd hear by now" would be the host arguing with
        // a word already spoken. Only when the slot is untouched: a turn that named a new
        // number ("still on it — another five minutes") armed a *new* promise, and that
        // one is exactly the promise worth keeping.
        if by_thinking_back && speaking.next_word.peek() == armed_before {
            speaking.next_word.clear();
        }

        // How the check-in floor paces itself, and the dial is **whether the last one was
        // worth it**. A word from the person, a number the voice just named, or a check-in
        // that actually produced speech all mean the cadence is earning its keep, so it
        // stays at the base gap. One that came and went in silence doubles it, up to the
        // glance-up cadence — a job running for hours must not be interrupted every five
        // minutes, and the voice is the only thing that knows whether there was anything to say.
        // (The reflection backoff's shape, for the same reason.)
        let spoke = speaking.said.load(Ordering::Relaxed) > said_before;
        if by_human || spoke || speaking.next_word.peek().is_some_and(|o| o.promised) {
            check_in_gap = check_in_base;
        } else if by_floor {
            check_in_gap = check_in_gap.map(|gap| (gap * 2).min(check_in_cap()));
        }

        // The floor itself. `reaction.md` asks the voice to put a size on every silence
        // it opens; when it doesn't, this is what keeps "never go dark on a long job"
        // from resting entirely on the model remembering to. A promise the voice made
        // itself outranks it — `floor` leaves an armed slot alone.
        //
        // Only while the conversation's own thinking is still running. A quiet agent with
        // nothing in flight owes nobody a word — and nothing else wakes this loop into an
        // empty room. **Work handed further up is out of scope on purpose**: Cognition's
        // workers are not this loop's to describe, and their substance comes back down
        // the report path, which drives a turn of its own.
        if let Some(gap) = check_in_gap
            && workers::thinking()
        {
            speaking.next_word.floor(gap);
        }

        // Coalesce mid-turn arrivals. Utterances that queued while this turn ran
        // (a generation is now seconds, not ~1s) are siblings of the thread we just
        // answered, not fresh threads — pull them all into one batch so they drive a
        // SINGLE next turn (the commit-after-quiet settle still applies on top),
        // instead of one redundant turn each. Without this, each nudge that landed
        // mid-turn ("好了吗?" → "准备好了吗?") pops alone on re-entry and re-answers.
        // Up only: while down, mail is held deliberately and the backoff path owns
        // catch-up, so leave the queue for it.
        if !reaction.inner.vendor.is_down() {
            while let Ok(extra) = inbound.try_recv() {
                enqueue(&reaction, &mut workers, &mut batch, extra).await;
            }
        }
    }
}

/// Render just the human requests in a batch (skipping worker reports and the host's own
/// wakes) — the text handed down to Cognition as the turn's task. Skipping reports is what
/// keeps it from re-ingesting its own prior output (a feedback loop).
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
/// Cognition — the reading and thinking — runs in parallel: the turn's human request is
/// handed down to it ([`hand_down_to_cognition`]), which works off the floor and answers
/// by mail, driving a turn of its own. So the reaction stays the single fast voice.
///
/// `handed_down` is set when this turn actually handed something down, so the loop knows
/// a reply is outstanding and can frame the answer as one when it arrives.
///
/// v1 keeps it simple — no mid-turn reorganization. A turn is one fast generation, so a
/// human speaking during it just queues and the serial loop folds it into the next turn.
async fn run_reaction_turn(
    reaction: &Reaction,
    batch: &[LoopInput],
    reaction_session: &mut Option<Arc<AgentSession>>,
    memo: &mut WindowMemo,
    voice_id: &registry::SessionId,
    speaking: &Speaking,
    handed_down: &mut bool,
) -> anyhow::Result<usize> {
    let turn_id = reaction.inner.turn_seq.fetch_add(1, Ordering::Relaxed);
    reaction.inner.floor.note_turn_started(turn_id);

    // This turn's delta: whether the conversation's own thinking is still running (so the
    // voice can say "still on it" rather than guess), any barge-in note, and the new
    // signals. The projected state it all hangs off is assembled in [`turn_context`].
    //
    // **There is no `## Presence` section any more.** It carried a decaying belief about
    // how present the person was, derived from open channels and window activations, and
    // the belief could not be derived — a window behind an editor and a person leaning in
    // are the same subscription. What replaced it is not a better estimate: messages keep,
    // so nothing has to be decided about whether anyone is there.
    let worker_status = workers::render_status();
    let interrupted = reaction
        .inner
        .floor
        .take_pending()
        .await
        .map(|i| floor::render_interruption(&i))
        .unwrap_or_default();
    let new_signals = format!("## New signals\n{}", render_batch(batch));
    // What the agent has on screen right now — its own presentation surface. Read
    // fresh every turn (it's a current fact, not durable memory), so a view dismissed
    // last turn is gone from this list now: the agent can see what's up and dismiss by
    // real id instead of guessing from the transcript.
    let on_screen = render_on_screen(&reaction.inner.views.on_screen().await);

    // Open (or reuse) the persistent reaction session. `reaction.md` rides its
    // `baseInstructions`; the session then remembers prior turns — which is what the window
    // memo is a claim about, so a thread we just opened has been told nothing.
    //
    // **This path does not seed, and that is deliberate.** A cold open here means the warm-up
    // never ran or its session died, so there is already a batch waiting: spending a whole
    // generation on a seed first would make someone wait twice. Forgetting instead puts the
    // same window on the turn they are waiting for.
    let session = match reaction_session {
        Some(s) => s.clone(),
        None => {
            let opened = open_reaction_session(reaction, voice_id).await?;
            *reaction_session = Some(opened.clone());
            memo.forget();
            opened
        }
    };

    let context = turn_context(
        &reaction.inner.memory,
        voice_id,
        memo,
        &worker_status,
        &on_screen,
        &interrupted,
        &new_signals,
    )
    .await;

    // Captured before the prompt is handed over — it is moved into `drive_voice`.
    let context_chars = context.chars().count();
    tracing::info!(ctx_chars = context_chars, "reaction: prompting session");
    let _ = speaking
        .beats
        .send(sequencer::Beat::TurnStart { turn: turn_id })
        .await;

    let mut turn_error = None;
    let completed = match drive_voice(&session, voice_id, context).await {
        Ok(Drove { text, compacted }) => {
            // Speech arrives as `say` calls, which the MCP surface already put on the
            // sequencer while the turn was running. Anything the model *typed* is
            // working-out, not utterance — voicing it too would say every reply twice,
            // and the tool's own description promises plain text is not spoken. So this
            // count is a size, not a shortfall: a turn that typed and called no `say`
            // chose silence, which is an ordinary move and not the host's to correct.
            tracing::info!(typed_chars = text.chars().count(), "reaction: turn done");
            if compacted {
                // Codex replaced this thread's history with a summary of it. Whatever
                // the model could see a moment ago, it cannot be assumed to see now —
                // so the memo's claim is void and the next turn re-sends the window.
                tracing::info!("reaction: thread compacted; window re-sent next turn");
                memo.forget();
            }
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
    let _ = speaking
        .beats
        .send(sequencer::Beat::TurnEnd { done: done_tx })
        .await;
    let reply = done_rx.await.unwrap_or_default();
    reaction.inner.floor.end_turn(turn_id, &reply).await;

    // `completed` is about the generation, not about speech: a turn that finished
    // without erroring counts, whether or not it chose to call `hi_say`.
    if completed {
        // Success clears only transient generic backoff. Managed energy and its
        // retained view are owned by the broker-backed vendor gate.
        let _ = reaction.inner.vendor.note_success();
        // Hand the turn's human request down to Cognition — the reading and thinking the
        // voice cannot do — so it works off the floor while the voice moves on. Its answer
        // comes back as mail, which drives a turn of its own.
        //
        // **This is a post, not a spawn.** It used to open (or resume) a whole session
        // per conversation; Cognition is already standing, already has an inbox, and
        // already wakes on it, so the hand-down is one line into machinery that existed
        // anyway. Nothing to hand off on a turn nobody spoke into — a report, a check-in.
        let task = render_human_from_batch(batch);
        if !task.trim().is_empty() {
            *handed_down = hand_down_to_cognition(reaction, task).await;
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

/// One turn's whole prompt: whatever the thread does not already know, then this turn's
/// delta.
///
/// **There is no fresh-session branch here, and that absence is the change.** The
/// projection used to be inlined only when a session was opened, on the reasoning that
/// the session remembers its own open and later turns need send only the delta. That
/// reasoning holds for a *transcript* and fails for *state*: a task opened, a duty
/// closed, or a conversation memory written mid-conversation is exactly what the session
/// cannot have remembered, because it did not exist yet. So the window was correct at
/// session open and drifted for every turn after — and since the conversation's session is
/// long-lived by design, that is most of the conversation. Code re-reads the current
/// state on every turn instead.
///
/// **Re-reading it every turn was right; re-sending it every turn was not**, and for a
/// long time this function did both. A block identical to the one three turns up is
/// already in front of the model — sending it again buys nothing and costs a permanent
/// copy in a finite window. Measured on one live thread: 108 turns, 10,125 chars each,
/// of which the standing sections were 5,848 and moved 14 times between them. The thread
/// became 80% its own preamble, and the compaction that followed kept ten copies of that
/// preamble and dropped every tool call — the voice's own examples of speaking with it.
///
/// So each block declares a [`Cadence`] and `memo` holds what this thread was last told.
/// The reads still happen per turn (they have to, to know whether anything moved) and
/// they are small; what stops happening is the repetition.
async fn turn_context(
    memory: &Memory,
    voice_id: &registry::SessionId,
    memo: &mut WindowMemo,
    worker_status: &str,
    on_screen: &str,
    interrupted: &str,
    new_signals: &str,
) -> String {
    let mut blocks = snapshot::window(memory, voice_id).await;
    // The turn's own view of live work and screen: state like the rest, and just as
    // repetitive — `On screen now` moved 12 times in 108 turns, `Still looking into` 32.
    blocks.push(snapshot::Block {
        key: "workers",
        cadence: snapshot::Cadence::OnChange,
        text: worker_status.to_string(),
        compare_as: None,
    });
    blocks.push(snapshot::Block {
        key: "screen",
        cadence: snapshot::Cadence::OnChange,
        text: on_screen.to_string(),
        compare_as: None,
    });
    let carried = memo.take(blocks);
    // Neither of these is state. A barge-in note is consumed when it is read, and the
    // signals *are* the turn — they go every time, unconditionally.
    join_sections(&[carried.as_str(), interrupted, new_signals])
}

/// What this thread has already been told, so a turn carries only what it hasn't.
///
/// Hashes rather than copies: the question is only "is this the same text as last time",
/// and keeping the answer costs eight bytes per block instead of ten kilobytes.
///
/// **Forgetting is the load-bearing half.** The memo is a claim about what the model can
/// still see, so it is only true while the thread's history is intact. A fresh thread has
/// never been told anything, and a compacted one has had its history rewritten by a
/// summarizer that keeps no promises about what it preserved — so both
/// [`forget`](Self::forget) it, and the next turn re-sends the window whole.
struct WindowMemo {
    sent: std::collections::HashMap<&'static str, u64>,
    /// Set when the thread can no longer see its own history. Cleared by the turn that
    /// pays for it.
    ///
    /// A memo that has told nothing is looking at a thread that knows nothing, so a
    /// [`Default`] memo is cold and the first turn through carries the window whole.
    cold: bool,
}

impl Default for WindowMemo {
    fn default() -> Self {
        Self { sent: std::collections::HashMap::new(), cold: true }
    }
}

impl WindowMemo {
    /// A fresh thread, or one whose history a compaction just replaced.
    fn forget(&mut self) {
        self.sent.clear();
        self.cold = true;
    }

    /// The blocks this turn must carry, joined — and remember them as sent.
    fn take(&mut self, blocks: Vec<snapshot::Block>) -> String {
        let cold = std::mem::take(&mut self.cold);
        let mut out: Vec<String> = Vec::new();
        for block in blocks {
            if block.text.trim().is_empty() {
                // An absent block is not a changed one: a task ledger that reads empty
                // this turn must not spend the next turn's diff announcing itself.
                continue;
            }
            let digest = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                block.compare_as.as_deref().unwrap_or(&block.text).hash(&mut h);
                h.finish()
            };
            let send = match block.cadence {
                snapshot::Cadence::ColdOnly => cold,
                snapshot::Cadence::OnChange => {
                    cold || self.sent.get(block.key) != Some(&digest)
                }
            };
            if send {
                self.sent.insert(block.key, digest);
                out.push(block.text);
            }
        }
        join_sections(&out.iter().map(String::as_str).collect::<Vec<_>>())
    }
}

#[cfg(test)]
mod turn_context_tests {
    use super::*;
    use crate::mind::memory::layout;
    use crate::mind::memory::tasks::{Task, TaskStatus, write_task};
    use crate::types::{Channel, JournalEntry};

    /// Put one line in the log, so the recent tail has something to carry.
    async fn heard(memory: &Memory, body: &str) {
        memory
            .journal
            .append(JournalEntry::SignalIn {
                id: uuid::Uuid::now_v7().to_string(),
                ts: chrono::Utc::now(),
                channel: Channel::Text,
                body: body.to_string(),
                stream: None,
                media: None,
                origin: None,
                sender: None,
            })
            .await
            .unwrap();
    }

    /// The bug this change exists to fix. The conversation's memory written — or a task opened
    /// — *after* the session was already up used to be invisible until the session
    /// rotated; the second turn of one live session must carry it.
    #[tokio::test]
    async fn the_projection_rides_a_reused_session_too() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();

        // One memo across both turns, because it is one session: the point of the test is
        // that turn two carries what changed under a thread that never rotated.
        let mut memo = WindowMemo::default();

        // Turn one, on a session opened just now: nothing written yet.
        let first =
            turn_context(&memory, &0.into(), &mut memo, "", "", "", "## New signals\n>在吗").await;
        assert!(!first.contains("mid-migration"), "{first}");

        // Mid-conversation, the state moves under the live session.
        let path = layout::reaction_seed_path(dir.path());
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        tokio::fs::write(&path, "He is mid-migration this week; keep answers terse.")
            .await
            .unwrap();
        let mut owed = Task::new("Ship the flash cards", TaskStatus::Doing);
        owed.title = "Ship the flash cards".into();
        write_task(dir.path(), &owed).await.unwrap();

        // Turn two, same session — no re-open, no rotation.
        let second =
            turn_context(&memory, &0.into(), &mut memo, "", "", "", "## New signals\n>那卡片呢").await;
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
            &0.into(),
            &mut WindowMemo::default(),
            "## Workers\nbuilding a view",
            "## On screen now\ntasks",
            "",
            "## New signals\n>好了没",
        )
        .await;
        let at = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("missing {needle}: {text}"));
        assert!(at("## Recent (last 30 minutes)") < at("## Workers"));
        assert!(at("## Workers") < at("## On screen now"));
        assert!(at("## On screen now") < at("## New signals"));
        assert!(text.trim_end().ends_with("好了没"), "{text}");
        assert!(!text.contains("## Presence"), "the presence projection is gone: {text}");
    }

    /// The repetition this exists to stop. Two turns with nothing moved between them: the
    /// second carries the signals and **not** a second copy of the state, because the
    /// thread can still see the first.
    #[tokio::test]
    async fn an_unchanged_block_is_not_sent_twice() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        let path = layout::reaction_seed_path(dir.path());
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        tokio::fs::write(&path, "He is mid-migration this week.").await.unwrap();
        let mut memo = WindowMemo::default();

        let first =
            turn_context(&memory, &0.into(), &mut memo, "", "", "", "## New signals\n>在吗").await;
        assert!(first.contains("mid-migration"), "{first}");

        let second =
            turn_context(&memory, &0.into(), &mut memo, "", "", "", "## New signals\n>还在吗").await;
        assert!(!second.contains("mid-migration"), "said once is said: {second}");
        assert!(second.contains("还在吗"), "the turn itself always rides: {second}");
        assert!(second.len() < first.len() / 2, "{} vs {}", second.len(), first.len());
    }

    /// The tail is a retelling of signals already in the thread, so it rides a cold
    /// context and nothing else.
    #[tokio::test]
    async fn the_recent_tail_rides_a_cold_turn_only() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        heard(&memory, "把周报发我").await;
        let mut memo = WindowMemo::default();

        let cold = turn_context(&memory, &0.into(), &mut memo, "", "", "", "## New signals\n>在吗").await;
        assert!(cold.contains("## Recent (last 30 minutes)"), "{cold}");

        let warm = turn_context(&memory, &0.into(), &mut memo, "", "", "", "## New signals\n>在吗").await;
        assert!(!warm.contains("## Recent (last 30 minutes)"), "{warm}");
    }

    /// What the 2026-08-13 thread needed and did not have. A compaction replaces the
    /// history with a summary that keeps no promises about what it preserved — on that
    /// thread it dropped every tool call in sixty turns — so the memo's claim is void and
    /// the next turn re-sends the window whole, tail included.
    #[tokio::test]
    async fn a_compaction_makes_the_next_turn_carry_everything_again() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        heard(&memory, "把周报发我").await;
        let path = layout::reaction_seed_path(dir.path());
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        tokio::fs::write(&path, "He is mid-migration this week.").await.unwrap();
        let mut memo = WindowMemo::default();

        let first = turn_context(&memory, &0.into(), &mut memo, "", "", "", "## New signals\n>在吗").await;
        let quiet = turn_context(&memory, &0.into(), &mut memo, "", "", "", "## New signals\n>在吗").await;
        assert!(!quiet.contains("mid-migration"), "{quiet}");

        memo.forget();
        let after = turn_context(&memory, &0.into(), &mut memo, "", "", "", "## New signals\n>在吗").await;
        assert!(after.contains("mid-migration"), "{after}");
        assert!(after.contains("## Recent (last 30 minutes)"), "{after}");
        assert_eq!(after.len(), first.len(), "a cold turn carries what the first one did");
    }

    /// What [`seed_session`] rests on: one cold pass returns the window whole and leaves the
    /// memo warm, so the first real turn after a seed carries the signals and not the window
    /// again. The seed is that pass with no signals attached.
    #[tokio::test]
    async fn seeding_hands_over_the_window_and_leaves_the_memo_warm() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        heard(&memory, "把周报发我").await;
        let path = layout::reaction_seed_path(dir.path());
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        tokio::fs::write(&path, "He is mid-migration this week.").await.unwrap();
        let mut memo = WindowMemo::default();

        let seed = turn_context(&memory, &0.into(), &mut memo, "", "", "", "").await;
        assert!(seed.contains("mid-migration"), "{seed}");
        assert!(seed.contains("## Recent (last 30 minutes)"), "{seed}");
        assert!(!seed.contains("## New signals"), "a seed is not a turn: {seed}");

        let first_turn =
            turn_context(&memory, &0.into(), &mut memo, "", "", "", "## New signals\n>在吗").await;
        assert!(!first_turn.contains("mid-migration"), "the seed already said it: {first_turn}");
        assert!(!first_turn.contains("## Recent (last 30 minutes)"), "{first_turn}");
        assert!(first_turn.contains("在吗"), "{first_turn}");
    }

    /// A ledger whose only movement is its own clock is not news. This is the turn-level
    /// half of [`crate::mind::memory::tasks::without_elapsed`]: 65 of 92 re-sends on one
    /// live thread were exactly this, at 431 chars each.
    #[tokio::test]
    async fn a_ticking_clock_in_the_ledger_is_not_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        let mut owed = Task::new("Watch the ops group", TaskStatus::Doing);
        owed.title = "Watch the ops group".into();
        owed.created_at = Some(chrono::Utc::now() - chrono::Duration::days(3));
        write_task(dir.path(), &owed).await.unwrap();
        let mut memo = WindowMemo::default();

        let first =
            turn_context(&memory, &0.into(), &mut memo, "", "", "", "## New signals\n>在吗").await;
        assert!(first.contains("Watch the ops group"), "{first}");

        // Age it a day. The line changes — `open 3d` becomes `open 4d` — and nothing about
        // the duty has.
        owed.created_at = Some(chrono::Utc::now() - chrono::Duration::days(4));
        write_task(dir.path(), &owed).await.unwrap();
        let aged =
            turn_context(&memory, &0.into(), &mut memo, "", "", "", "## New signals\n>还在吗").await;
        assert!(!aged.contains("Watch the ops group"), "only the clock moved: {aged}");

        // Close it, and it must be told at once.
        owed.status = TaskStatus::Done;
        write_task(dir.path(), &owed).await.unwrap();
        let closed =
            turn_context(&memory, &0.into(), &mut memo, "", "", "", "## New signals\n>好了吗").await;
        assert!(closed.contains("# Active tasks"), "a duty leaving the ledger is news: {closed}");
    }

    /// Codex announces it on the item, and both `item/started` and `item/completed` carry
    /// the same one — either is the news, and nothing else in the stream is.
    #[test]
    fn a_compaction_frame_is_recognised_either_side() {
        let started = serde_json::json!({
            "method": "item/started",
            "params": {"item": {"type": "contextCompaction", "id": "c1"}}
        });
        let completed = serde_json::json!({
            "method": "item/completed",
            "params": {"item": {"type": "contextCompaction", "id": "c1"}}
        });
        let a_tool_call = serde_json::json!({
            "method": "item/completed",
            "params": {"item": {"type": "mcpToolCall", "tool": "hi_say"}}
        });
        assert!(is_compaction(&started));
        assert!(is_compaction(&completed));
        assert!(!is_compaction(&a_tool_call));
        assert!(!is_compaction(&serde_json::json!({"method": "turn/completed"})));
    }
}

/// Hand this turn's human request down to Cognition, and report whether it landed —
/// which is whether the person is now waiting on an answer.
///
/// **One post into a standing inbox.** Cognition is already up, already has a mailbox,
/// and already wakes on it, so this is the whole of the hand-down: no session to open, no
/// warm one to resume, no fallback spawn. That is what retiring Deliberation bought.
///
/// Posted on the **host's** behalf (`from: None`) rather than sent from the voice. The
/// distinction is not cosmetic: `send` applies the addressing rules that govern one agent
/// reaching another, and this is the host driving its own loop. It also renders bare, so
/// the request reaches Cognition as the request rather than as a colleague quoting it.
///
/// `false` when there is nobody to hand to — Cognition has not warmed yet, or died — and
/// the caller must not then wait for an answer that cannot come. The request is not lost:
/// it is in the transcript the next hand-down carries.
async fn hand_down_to_cognition(reaction: &Reaction, task: String) -> bool {
    let Some(brain) = registry::global().session_of_role(Role::Cognition) else {
        tracing::warn!("no cognition session to hand down to; the voice answers alone this turn");
        return false;
    };
    let delivery = registry::global().post(&brain.id, task.clone());
    reaction
        .inner
        .observatory
        .record(EventKind::MessageSent {
            from: None,
            to: brain.id.clone(),
            delivery,
            message: task,
        })
        .await;
    match delivery {
        registry::Delivery::Delivered => true,
        other => {
            tracing::warn!(cognition = %brain.id, delivery = ?other, "hand-down did not land");
            false
        }
    }
}

/// Open and prime the conversation's Reaction session.
///
/// Idempotent, so the managed-energy Resume edge can call this again after a startup
/// pause without replacing a session that is already live.
async fn warm_sessions(
    reaction: &Reaction,
    voice_id: &registry::SessionId,
    reaction_session: &mut Option<Arc<AgentSession>>,
    memo: &mut WindowMemo,
) -> bool {
    let blocked_before = crate::foundation::energy_state::is_out();
    // One warm-up, not two. The second was Deliberation's, and Cognition — which now
    // holds that job — warms itself inside its own loop rather than being stood up from
    // the conversation's.
    warm_reaction_session(reaction, voice_id, reaction_session, memo).await;
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
    voice_id: &registry::SessionId,
    held: &mut Option<Arc<AgentSession>>,
    memo: &mut WindowMemo,
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

    // Opening the thread carries layer 1: `reaction.md` rides `baseInstructions` on
    // `thread/start`, so the character is in place the moment the session exists. Layer 2
    // is this next line — what it *knows* coming in, handed over before anyone has said
    // anything to it.
    seed_session(reaction, &session, voice_id, memo).await;
    tracing::info!("reaction session warmed");
    *held = Some(session);
}

/// Hand a freshly opened thread its **seed**: the first message it ever reads.
///
/// `docs/arch/data.md` calls this layer 2, and this is the whole of it — the generated
/// `prompts/seed/reaction.md`, plus what is computed from the record at seed time: how to
/// be with the people in front of it, the open ledger, who it can reach, and the tail of
/// what happened before it existed.
///
/// **Built by the same code a turn uses**, with a cold memo and no signals, so there is one
/// definition of "the window" rather than two that drift. The memo comes back warm, which
/// is what stops the first real turn from saying all of it again.
///
/// The sequencer is unarmed until `TurnStart`, so a `say` from this prompt would be
/// dropped without a trace — which is why the text says not to speak rather than relying on
/// it. Best-effort throughout: a seed that fails to send leaves the memo cold, and the
/// first turn carries the window itself, exactly as it did before this existed.
async fn seed_session(
    reaction: &Reaction,
    session: &AgentSession,
    voice_id: &registry::SessionId,
    memo: &mut WindowMemo,
) {
    memo.forget();
    let window =
        turn_context(&reaction.inner.memory, voice_id, memo, "", "", "", "").await;
    if window.trim().is_empty() {
        tracing::info!("reaction seed: nothing to carry in yet");
        return;
    }
    let seed = format!("{SEED_PREAMBLE}\n\n{window}");
    tracing::info!(seed_chars = seed.chars().count(), "reaction: seeding the thread");
    let mut run = match session.prompt(seed).await {
        Ok(run) => run,
        Err(err) => {
            tracing::warn!(error = %err, "reaction seed failed; the first turn will carry it");
            memo.forget();
            return;
        }
    };
    while let Some(update) = run.next_update().await {
        if let Some(what) = update.activity() {
            registry::global().record_activity(voice_id, &what);
        }
    }
    if let Err(err) = run.wait().await {
        tracing::warn!(error = %err, "reaction seed did not complete; the first turn will carry it");
        memo.forget();
    }
}

/// What the seed says about itself, so the rung does not answer it.
///
/// A seed arrives in the same slot a person's words arrive in, and without this it reads
/// as someone opening with a status dump — which is a thing you reply to.
const SEED_PREAMBLE: &str = "This is what you know coming in, not something anyone said to \
you. Nobody has spoken yet. Read it, say nothing, and wait for the first signal.";

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
    voice_id: &registry::SessionId,
) -> anyhow::Result<Arc<AgentSession>> {
    let session = Arc::new(
        reaction
            .inner
            .agent
            .session(
                Role::Reaction,
                Some(voice_id.clone()),
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

/// What one drive of the voice produced.
struct Drove {
    /// Everything it **typed** — working-out, not speech.
    text: String,
    /// Codex replaced this thread's history with a summary mid-turn.
    compacted: bool,
}

/// Prompt the reaction session and return the text it **typed** (every
/// `agent_message_chunk` concatenated) — which is its working-out, not its speech.
/// Speech is only ever what went through the `say` tool. Tool calls — `say`, `show`,
/// `send_message` — are dispatched server-side through hi-agent's `/mcp` (which emits
/// the beats), so the drive loop just keeps streaming text past them, exactly like a
/// worker's loop; `wait()` then parks the session and surfaces any real prompt error
/// (a gateway 402/429, a transport reset) to the caller's classifier.
async fn drive_voice(
    session: &AgentSession,
    voice_id: &registry::SessionId,
    context: String,
) -> anyhow::Result<Drove> {
    let mut run = session.prompt(context).await?;
    let mut text = String::new();
    let mut compacted = false;
    while let Some(update) = run.next_update().await {
        // The voice was the one rung reporting nothing at all to the switchboard: its
        // words go to the transcript rather than the output tail, so its roster row read
        // "nothing said yet" for the whole life of the process. What it is *doing* is the
        // honest answer to "is the mouth alive", and it is the only one available here.
        if let Some(what) = update.activity() {
            registry::global().record_activity(voice_id, &what);
        }
        match update {
            SessionUpdate::Text(t) => text.push_str(&t),
            SessionUpdate::Thought(t) => {
                tracing::debug!(chars = t.chars().count(), "reaction: model is thinking");
            }
            // `show` dispatches server-side via `/mcp`; the reaction keeps speaking.
            // Its surface is `show`-only and the dispatch guard blocks any other
            // expression tool, so there is nothing to intercept here.
            //
            // **One frame is read, and only for what it invalidates.** A compaction is
            // not the agent doing something; it is codex rewriting the thread's history
            // out from under both of us. The host cannot see what survived — on the
            // 2026-08-13 thread, what did not was every tool call in 60 turns — so all it
            // takes from this is that anything it believed the model could still see is
            // no longer a safe assumption.
            SessionUpdate::Frame(frame) => {
                if is_compaction(&frame) {
                    compacted = true;
                }
            }
        }
    }
    let result = run.wait().await?;
    tracing::info!(
        stop = ?result.stop_reason,
        typed_chars = text.chars().count(),
        "reaction: turn complete"
    );
    Ok(Drove { text, compacted })
}

/// Is this frame codex telling us it just replaced the thread's history?
///
/// Matched on the item type rather than a method name, because the same `contextCompaction`
/// item arrives on both `item/started` and `item/completed` and either one is the news.
fn is_compaction(frame: &serde_json::Value) -> bool {
    frame
        .get("params")
        .and_then(|p| p.get("item"))
        .and_then(|i| i.get("type"))
        .and_then(serde_json::Value::as_str)
        == Some("contextCompaction")
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
    record_out_as(reaction, channel, body, Uuid::now_v7().to_string(), Utc::now()).await;
}

/// [`record_out`] under a key the caller already holds — used when the same
/// emission also becomes a message in the conversation and the two must agree.
async fn record_out_as(
    reaction: &Reaction,
    channel: Channel,
    body: String,
    id: String,
    ts: chrono::DateTime<Utc>,
) {
    let entry = JournalEntry::SignalOut {
        id,
        ts,
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
/// a check-in coming due, a worker's report, mail. Without these the log shows a turn's
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
        // **Machine channels take no sender at all** — `clock` and `worker` are the
        // agent's own machinery moving, not a person doing something. `None` here is
        // not "we don't know who": it is "there was nobody", which is why a stretch of
        // pure clock and worker traffic must teach the record nothing about anyone.
        sender: None,
    };
    if let Err(err) = reaction.inner.memory.journal.append(entry).await {
        tracing::error!(channel = %channel, error = %err, "journal append failed for internal signal");
    }
}

/// One `say` becomes one message: journaled, then appended to the conversation
/// under the same id.
///
/// **The key is minted here and used twice**, rather than each side generating its
/// own, because the conversation is rebuilt from the journal at boot — and two keys
/// for one message would mean the list changed shape whenever it reloaded.
///
/// Journaled first, and before anything is split for speech: durability precedes
/// reaction, and `say` already holds the whole text, so there is no window in which
/// a crash could lose words the agent had already sent.
async fn emit_message(reaction: &Reaction, text: String) {
    let id = Uuid::now_v7().to_string();
    let ts = Utc::now();
    record_out_as(reaction, Channel::Text, text.clone(), id.clone(), ts).await;
    let _ = reaction
        .inner
        .out
        .send(OutboundSignal::Text { id, ts, text })
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
        interleave::Emit::Show { id, op, source, traits, view_ref } => {
            emit_view(reaction, id, op, source, traits, view_ref).await
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
            LoopInput::CheckIn { owed } => {
                let _ = writeln!(s, "{}", render_check_in(owed, Instant::now()));
            }
            LoopInput::Mail { mail, owed: false } => {
                let _ = writeln!(s, "{}", registry::render(mail));
            }
            // The must-relay framing Deliberation's report used to carry, on the path the
            // answer actually travels now. "Relay it" is not "dump it verbatim": the
            // voice still says it in its own plain words and reconciles with whatever it
            // already said — what it may not do is read this and stay quiet.
            LoopInput::Mail { mail, owed: true } => {
                let _ = writeln!(
                    s,
                    "Your thinking is back — this is the answer you owe the person, so relay \
what matters here in your own plain words now (don't leave them waiting, and don't just \
acknowledge it — tell them what you found):\n{}",
                    registry::render(mail)
                );
            }
        }
    }
    s
}

/// What a check-in looks like in the turn's "New signals".
///
/// A **fact plus the floor is yours**, never a script. The host knows exactly two
/// things — that a word is owed and how long it has been owed for — and states them;
/// what the work has actually reached is in `## Still looking into` and the projected
/// ledger, and what is worth saying about it is Reaction's alone. It may also be
/// nothing: the wake is permission to speak, not an instruction to.
///
/// The two sources read differently on purpose. A promise the voice made is a fact the
/// *person* holds too — they were told a number and it has passed, so silence now is a
/// visibly broken promise. A host floor is only the agent's own rule about going dark;
/// nobody is waiting on a specific minute, and saying so keeps the voice from inventing
/// a promise it never made.
fn render_check_in(owed: &tools::Owed, now: Instant) -> String {
    // Since the promise was made, not since the deadline: the deadline is normally
    // *now*, and "that was 0s ago" is not the sentence anyone means.
    let waited = tools::render_gap(now.saturating_duration_since(owed.at) + owed.after);
    if owed.promised {
        format!(
            "(check-in) You told them they'd hear from you within {} — you said that {waited} \
             ago and haven't spoken since. If it's done, say what came of it; if it's still \
             running, say where it's got to and give them a new number.",
            tools::render_gap(owed.after),
        )
    } else {
        format!(
            "(check-in) You've been quiet {waited} while your own thinking runs, and you \
             left them no number. Nobody is waiting on a particular minute — but if \
             there's something real to say about where it's got to, this is the moment \
             they'd rather hear it than have to ask."
        )
    }
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
/// not something the person said, and neither is the brain's answer coming back.
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
        // On `Channel::Clock`, and the only thing left there: a check-in is the host
        // noticing the time, not something the person said. It must not hold a
        // conversation warm by itself ([`NON_ACTIVITY_CHANNELS`]).
        LoopInput::CheckIn { owed } => Some((
            Channel::Clock,
            Origin::Host,
            render_check_in(owed, Instant::now()),
        )),
        // Mail crosses no wire, so this is its only chance to be written down.
        // The `owed` framing is deliberately dropped: it is an instruction to the voice
        // about this turn, not part of the signal, and a later reader of the journal is
        // not the voice.
        LoopInput::Mail { mail, .. } => {
            Some((Channel::Worker, Origin::Worker, registry::render(mail)))
        }
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
        && let Some(owner) = report.owner.clone()
    {
        let text = workers::render_report(report);
        let delivery = workers.deliver_to(owner.clone(), text.clone());
        // Work travelling *up* is the edge most worth seeing, and it is host-posted
        // (`from: None`) — so nothing observed it while only `send_message` was
        // instrumented. Recorded whether or not it landed: a report that missed its
        // owner is precisely what you would open the inspector to find.
        reaction
            .inner
            .observatory
            .record(
                EventKind::MessageSent { from: None, to: owner.clone(), delivery, message: text },
            )
            .await;
        if matches!(delivery, registry::Delivery::Delivered) {
            return;
        }
        // The owner is gone. Surfacing one rung too high beats losing finished work.
        tracing::info!(
            session = %report.id, owner = %owner,
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
    view_ref: Option<String>,
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
            envelope: ViewEnvelope { id, op, module_url, traits, view_ref },
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

/// The hand-down's return path, pinned at the point where it can silently go wrong.
///
/// Killing Deliberation moved the conversation's answer from the **report** path, where
/// the host framed it as must-relay, onto the **mail** path, where Cognition's own prompt
/// says everything it sends is *a proposal, never a delivery*. Both halves of that need
/// to keep being true at once: an answer the person is waiting for must be framed as owed,
/// and an unsolicited finding must not be — otherwise the voice either drops replies or
/// loses the judgment about when to speak that makes it worth having.
#[cfg(test)]
mod hand_down_tests {
    use super::*;
    use crate::foundation::registry::Message;

    fn mail(text: &str, from: Option<registry::SessionId>) -> Message {
        Message { from, text: text.to_string() }
    }

    #[test]
    fn an_answer_the_person_is_waiting_for_is_framed_as_owed() {
        let rendered =
            render_batch(&[LoopInput::Mail { mail: vec![mail("the label says 500mg", Some(7.into()))], owed: true }]);
        assert!(rendered.contains("the label says 500mg"), "{rendered}");
        assert!(rendered.contains("the answer you owe the person"), "{rendered}");
        assert!(rendered.contains("don't leave them waiting"), "{rendered}");
    }

    /// The other half, and the one that would rot quietly: if every message from the
    /// brain were framed as owed, "everything you send is a proposal" would be a lie the
    /// prompt tells and the host contradicts, and the voice would narrate every
    /// background finding at the person.
    #[test]
    fn an_unsolicited_finding_stays_a_proposal() {
        let rendered =
            render_batch(&[LoopInput::Mail { mail: vec![mail("the backup job died", Some(7.into()))], owed: false }]);
        assert!(rendered.contains("the backup job died"), "{rendered}");
        assert!(!rendered.contains("owe the person"), "{rendered}");
    }

    /// A promise made *about* the thinking is discharged by the thinking coming back —
    /// which is now an owed mail rather than a report, and the check-in pacing keys on it.
    #[test]
    fn owed_mail_is_what_counts_as_the_thinking_coming_back() {
        let back = LoopInput::Mail { mail: vec![mail("done", Some(7.into()))], owed: true };
        let unsolicited = LoopInput::Mail { mail: vec![mail("fyi", Some(7.into()))], owed: false };
        assert!(matches!(back, LoopInput::Mail { owed: true, .. }));
        assert!(!matches!(unsolicited, LoopInput::Mail { owed: true, .. }));
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

#[cfg(test)]
mod check_in_tests {
    use super::*;
    use crate::body::reaction::tools::Owed;

    fn owed(after_secs: u64, promised: bool) -> Owed {
        Owed {
            at: Instant::now(),
            after: Duration::from_secs(after_secs),
            promised,
        }
    }

    /// A promise the person heard and a floor the host set are different situations,
    /// and the note has to read as the one it is. Telling the voice it "said ten
    /// minutes" when it named no number invents a promise, which is exactly the kind of
    /// claim `reaction.md` spends a section forbidding.
    #[test]
    fn the_two_sources_do_not_read_alike() {
        let kept = render_check_in(&owed(600, true), Instant::now());
        let floor = render_check_in(&owed(300, false), Instant::now());

        assert!(kept.starts_with("(check-in)"), "{kept}");
        assert!(floor.starts_with("(check-in)"), "{floor}");
        assert_ne!(kept, floor);
        assert!(kept.contains("10m"), "a promise names the number it made: {kept}");
        assert!(
            !floor.contains("told them"),
            "a floor must not claim a promise nobody heard: {floor}"
        );
    }

    /// The elapsed span is measured from when the promise was *made*, not from the
    /// deadline — which is normally now, and "that was 0s ago" is not a sentence.
    #[test]
    fn the_span_counts_from_the_promise() {
        let o = owed(600, true);
        let note = render_check_in(&o, o.at + Duration::from_secs(60));
        assert!(note.contains("11m"), "10m promised + 1m late: {note}");
    }

    /// The voice's only clock wake must read as the moment a word was *owed*. Cognition's
    /// `(pulse)` marker means the opposite — a quiet moment almost nothing is worth
    /// breaking — and wearing it here would tell the voice to stay quiet at precisely the
    /// instant it should speak.
    #[test]
    fn a_check_in_never_wears_the_glance_up_marker() {
        let note = render_check_in(&owed(300, false), Instant::now());
        assert!(!note.contains("(pulse)"), "{note}");
        assert!(note.contains("(check-in)"), "{note}");
    }
}
