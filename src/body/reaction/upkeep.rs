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
//! **It calls the session directly**, which took two things that are not about compaction:
//! a directory of live sessions (below), because `docs/arch/host.md` said sessions were
//! host-owned while a rung's handle was a local in its own loop; and a turn permit rather
//! than a race, because losing the single in-flight-turn slot used to be an *error*, and a
//! rung whose prompt errors drops its long-lived session and cold-opens — so maintenance
//! touching a session from outside could destroy the thread it was tidying.
//!
//! With both, this is an ordinary call: [`AgentSession::compact`] steps aside if a turn
//! holds the session, and a turn arriving mid-compaction waits for it instead of failing.
//! Maintenance is never urgent, so it is never the one that waits.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use crate::foundation::codex::AgentSession;

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

/// The live sessions, by slug — **the thing that was missing.**
///
/// `docs/arch/host.md` says sessions are host-owned, and they were not: a rung's handle was
/// a local in its own loop, so anything wanting to touch a session had to be routed back
/// through that loop as a message. Workers already had a directory ([`super::workers`]);
/// the three standing rungs did not, and the asymmetry was invisible until something needed
/// to reach all of them.
///
/// Held as `Weak`, so this never keeps a session alive: dropping the handle is still what
/// closes a session, and a slug whose session has gone simply stops resolving. Registering
/// is the only thing an owner has to do, and forgetting to costs it maintenance rather than
/// correctness.
fn live() -> &'static Mutex<HashMap<SessionSlug, Weak<AgentSession>>> {
    static LIVE: OnceLock<Mutex<HashMap<SessionSlug, Weak<AgentSession>>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Put a session in the directory, replacing whatever was there under that slug — a rung
/// that cold-opens after a failure registers again, and the dead handle it replaces would
/// otherwise sit there resolving to nothing.
pub(super) fn attend(id: &SessionSlug, session: &Arc<AgentSession>) {
    let mut live = live().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    live.retain(|_, weak| weak.strong_count() > 0);
    live.insert(id.clone(), Arc::downgrade(session));
}

/// Compact every session that has gone quiet with a full enough window, forever.
///
/// **It calls the session directly**, which it can because the two halves that made that
/// unsafe are gone: the handle is in the directory above, and
/// [`AgentSession::compact`](crate::foundation::codex::AgentSession::compact) steps aside
/// when a turn holds the session rather than colliding with it. What this replaces is a
/// message routed back through the owning loop — real plumbing standing in for ownership
/// the design already claimed to have.
pub(super) async fn sweep_forever() {
    loop {
        tokio::time::sleep(SWEEP_EVERY).await;
        for id in due() {
            let Some(session) = live()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&id)
                .and_then(Weak::upgrade)
            else {
                continue;
            };
            match session.compact().await {
                // Also the answer when a turn had the session — see `compact`. Both mean
                // "still full, come back later", which is the only thing to do with either.
                Ok(false) => tracing::debug!(session = %id, "upkeep: nothing compacted"),
                Ok(true) => tracing::info!(session = %id, "upkeep: compacted"),
                Err(err) => {
                    tracing::warn!(session = %id, error = %format!("{err:#}"), "upkeep: compaction refused")
                }
            }
        }
    }
}

/// The sessions worth compacting: quiet, quiet for a while, and full enough to be worth a
/// model call. Every session in the directory is a candidate — a worker that has genuinely
/// been idle an hour with a full window is as worth tidying as a rung, and the reason
/// workers were excluded before was that they had no channel to be asked on, which was a
/// fact about the plumbing rather than about workers.
///
/// The reading comes off the switchboard rather than the session handle, because that is
/// what makes this a scan: no locks on live sessions, no await, just the numbers every turn
/// boundary already writes there ([`super::note_window`]).
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

    /// The threshold this sweep filters on is the one the loop decides with — two numbers
    /// here would let the sweep pick sessions the compaction call always declines, which is
    /// work for nothing and the exact cost this design removed three cadences over.
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
