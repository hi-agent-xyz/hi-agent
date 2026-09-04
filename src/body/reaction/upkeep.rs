//! The upkeep sweep — the one thing in this host that runs on a clock, and it wakes no
//! agent to think.
//!
//! **Why this is not the cadence this design removed three of.** Those woke a rung to
//! *judge*: read the ledger and decide, look at the room and decide whether to speak, keep a
//! promise. The wake itself was the cost — a full window read to reach a conclusion, and
//! 46% of the glance-up's reached none. This walks a list of numbers the switchboard
//! already holds. A sweep that finds nothing costs one lock and a comparison per session,
//! and it produces a model call only when one is genuinely owed, for work with no judgment
//! in it.
//!
//! **It decides *whether*; the loop that owns the session decides *when*.** That split is
//! not tidiness, it is the only race-free shape available: a compaction takes the session's
//! single in-flight-turn slot, so a sweep holding the handle itself would collide with the
//! loop's own `prompt` — and a rung whose prompt fails *drops its long-lived session and
//! cold-opens* ([`super::cognition`]), losing the thread that whole design exists to keep.
//! So the sweep only rings a bell, and the loop compacts where nothing else can be running.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::Notify;

use crate::foundation::registry::{self, SessionSlug};

/// How often the sweep looks. Nothing here needs to be prompt: it is asking whether a
/// session that has already been quiet for an hour is still quiet, so ten minutes of slack
/// on a sixty-minute threshold is slack nobody can observe.
const SWEEP_EVERY: Duration = Duration::from_secs(10 * 60);

/// How long a session must have been idle before its window is worth compacting.
///
/// **The point is to be well past the end of a burst.** Compacting the moment a busy
/// stretch stops throws away context that the next turn was about to use, and pays for a
/// model call that the next burst may make again. An hour is not a measured number and does
/// not need to be — it is "long enough that this is over", and the sweep's own ten-minute
/// grain already makes it approximate.
const IDLE_FOR: Duration = Duration::from_secs(60 * 60);

/// The bells, one per session that has asked for one. Held here rather than on the
/// switchboard because the switchboard knows nothing about compaction, and this is the only
/// caller.
fn bells() -> &'static Mutex<HashMap<SessionSlug, Arc<Notify>>> {
    static BELLS: OnceLock<Mutex<HashMap<SessionSlug, Arc<Notify>>>> = OnceLock::new();
    BELLS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The bell this session listens on, created on first ask. A loop takes one at startup and
/// selects on it; a session that never takes one is simply never swept.
pub(super) fn bell(id: &SessionSlug) -> Arc<Notify> {
    bells()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entry(id.clone())
        .or_insert_with(|| Arc::new(Notify::new()))
        .clone()
}

/// Ring every session that has gone quiet with a full enough window, forever.
pub(super) async fn sweep_forever() {
    loop {
        tokio::time::sleep(SWEEP_EVERY).await;
        for id in due() {
            if let Some(bell) = bells()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&id)
                .cloned()
            {
                tracing::info!(session = %id, "upkeep: ringing for a compaction");
                bell.notify_one();
            }
        }
    }
}

/// The sessions worth ringing: quiet, quiet for a while, and full enough to be worth a
/// model call.
///
/// The fill test is deliberately here as well as in [`super::compact_if_full`], which runs
/// on the far side of the bell. This one keeps the sweep from waking loops for nothing; that
/// one is the decision, made against a reading taken after the wake rather than before it.
fn due() -> Vec<SessionSlug> {
    let now = chrono::Utc::now();
    registry::global()
        .statuses()
        .into_iter()
        .filter(|st| !st.busy && !st.queued)
        .filter(|st| {
            (now - st.state_since).to_std().is_ok_and(|idle| idle >= IDLE_FOR)
        })
        .filter(|st| st.window_percent.is_some_and(|pct| pct >= super::COMPACT_ABOVE_PERCENT))
        .map(|st| st.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bell is per session and stable: the loop takes one at startup and keeps it for
    /// the life of the process, so a sweep that rings later must reach the same one.
    #[test]
    fn a_session_gets_one_bell_and_keeps_it() {
        let id = registry::mint(crate::identity::Role::Cognition, None);
        assert!(Arc::ptr_eq(&bell(&id), &bell(&id)));
        let other = registry::mint(crate::identity::Role::Reflection, None);
        assert!(!Arc::ptr_eq(&bell(&id), &bell(&other)));
    }

    /// The threshold this sweep filters on is the one the loop decides with — two numbers
    /// here would let the sweep ring for sessions the far side always declines, which is a
    /// wake for nothing and the exact cost this whole design removed three cadences over.
    #[test]
    fn the_sweep_and_the_decision_share_one_threshold() {
        assert_eq!(super::super::COMPACT_ABOVE_PERCENT, 50);
    }

    /// Slack is the point. The sweep's grain is deliberately coarse against the idle
    /// window, because "has this been quiet for about an hour" is the question — nothing
    /// downstream can observe the difference between 60 and 70 minutes of silence.
    #[test]
    fn the_sweep_is_coarse_against_the_idle_window() {
        assert!(SWEEP_EVERY < IDLE_FOR, "a sweep rarer than the window would miss sessions");
        assert!(SWEEP_EVERY * 4 <= IDLE_FOR, "and it should be slack, not precision");
    }
}
