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
//!
//! **It crosses on the control channel each rung already has**, as one more
//! [`LoopControl`](super::tools::LoopControl) variant, for the reason that enum exists at
//! all: the loop owns the state each of its messages touches. There is no second map of
//! handles and no extra `select!` arm — the arm that already carries `CreateWorker` carries
//! this, and a rung with no live sink is skipped, which is what a rung that is not up
//! should be.

use std::time::Duration;

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

/// Tell every session that has gone quiet with a full enough window to compact, forever.
///
/// **The message crosses on the rung's existing control channel** ([`super::tools::LoopControl`]),
/// which is there for exactly this: work whose state the loop owns. Nothing new is wired —
/// no second map of handles, no extra arm — and a rung with no live control sink is simply
/// skipped, which is what a rung that is not up should be.
pub(super) async fn sweep_forever(tools: super::tools::ToolRegistry) {
    loop {
        tokio::time::sleep(SWEEP_EVERY).await;
        for (id, owner) in due() {
            let Some(sink) = tools.get(owner).await else { continue };
            tracing::info!(session = %id, "upkeep: asking for a compaction");
            if sink.control.send(super::tools::LoopControl::Compact).await.is_err() {
                tracing::warn!(session = %id, "upkeep: control channel gone");
            }
        }
    }
}

/// The sessions worth asking: quiet, quiet for a while, and full enough to be worth a
/// model call. Paired with the rung to send to, because only the standing rungs compact —
/// a worker has no control channel of its own and is left to codex.
///
/// The fill test is deliberately here as well as in [`super::compact_if_full`], which runs
/// on the far side of the bell. This one keeps the sweep from waking loops for nothing; that
/// one is the decision, made against a reading taken after the wake rather than before it.
fn due() -> Vec<(SessionSlug, super::tools::ToolOwner)> {
    let now = chrono::Utc::now();
    registry::global()
        .statuses()
        .into_iter()
        .filter(|st| !st.busy && !st.queued)
        .filter(|st| {
            (now - st.state_since).to_std().is_ok_and(|idle| idle >= IDLE_FOR)
        })
        .filter(|st| st.window_percent.is_some_and(|pct| pct >= super::COMPACT_ABOVE_PERCENT))
        .filter_map(|st| {
            let owner = super::tools::ToolOwner::from_role(Some(st.role.as_str()))?;
            Some((st.id, owner))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only the standing rungs are asked. A worker has no control channel of its own, and
    /// is deliberately left to codex — its window can go from a tenth full to full inside a
    /// single turn, which no boundary policy can catch.
    #[test]
    fn only_a_rung_can_be_asked_to_compact() {
        use crate::identity::{Role, WorkerType};
        assert!(super::super::tools::ToolOwner::from_role(Some(Role::Cognition.as_str())).is_some());
        assert!(super::super::tools::ToolOwner::from_role(Some(Role::Reflection.as_str())).is_some());
        assert!(
            super::super::tools::ToolOwner::from_role(Some(
                Role::Worker(WorkerType::General).as_str()
            ))
            .is_none(),
            "a worker has no control channel to be asked on"
        );
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
