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
            let outcome = session.compact().await;
            // **Publish the reading before doing anything else with the result**, because
            // the number this sweep decides on is the number a turn boundary last wrote —
            // and a compaction is the one turn that does not go through one. Without this
            // the sweep re-selected the session it had just compacted, every cycle, for as
            // long as it stayed idle: 27 firings in five hours on 2026-09-06, on a thread
            // that was 8% full after the first. Unconditional because the reading is worth
            // republishing whatever happened — a compaction that failed leaves the thread
            // exactly as full as it was, which is the answer that correctly selects it again.
            super::note_window(&id, Some(&session));
            match outcome {
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
/// boundary already writes there ([`super::note_window`]) — and, since a compaction reaches
/// no such boundary, the one [`sweep_forever`] writes there itself.
fn due() -> Vec<SessionSlug> {
    due_among(registry::global().statuses(), chrono::Utc::now())
}

/// The selection itself, over a roster handed in — so the one property that matters can be
/// asserted without a live registry: **a session this returns must stop being returned once
/// it has been compacted.** It did not, for as long as the reading stayed stale.
fn due_among(
    statuses: Vec<registry::Status>,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<SessionSlug> {
    statuses
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

    /// A quiet worker with a full window, as the switchboard held it at 05:35 on 2026-09-06.
    fn quiet_and_full(window_percent: Option<u8>) -> registry::Status {
        let now = chrono::Utc::now();
        registry::Status {
            id: registry::mint(
                crate::identity::Role::Worker(crate::identity::WorkerType::General),
                Some("wecom-xiaoli-standing-listener"),
            ),
            role: crate::identity::Role::Worker(crate::identity::WorkerType::General),
            owner: None,
            title: "run persistent WeCom Xiaoli listener".into(),
            subject: Some("wecom-xiaoli-standing-listener".into()),
            busy: false,
            queued: false,
            turns: 12,
            started: now - chrono::Duration::hours(6),
            state_since: now - chrono::Duration::hours(2),
            doing: None,
            doing_at: None,
            last_turn: None,
            window_percent,
        }
    }

    /// **The loop that ran 27 times.** Selecting an idle, full session is right; selecting
    /// it *again* after compacting it is the bug, and the only thing standing between the
    /// two is that the compaction wrote a fresh reading back. codex ends a compaction turn
    /// reporting `last.inputTokens` of 0, so that is the reading the sweep leaves behind —
    /// imprecise (the thread was really 8% full) and sufficient, because the next real turn
    /// overwrites it. What must never happen again is the reading not moving at all.
    #[test]
    fn a_compacted_session_stops_being_due() {
        let now = chrono::Utc::now();
        assert_eq!(
            due_among(vec![quiet_and_full(Some(82))], now).len(),
            1,
            "an idle session at 82% is exactly what this sweep is for"
        );
        assert!(
            due_among(vec![quiet_and_full(Some(0))], now).is_empty(),
            "compacted, and still selected: this is the five-hour loop of 2026-09-06"
        );
    }

    /// The reading is the *only* thing that changes after a compaction: it takes the session
    /// through no turn the registry sees, so `state_since` does not move and the session
    /// stays as idle as it was. A fix that leaned on the clock instead would not have held.
    #[test]
    fn nothing_else_about_a_compacted_session_changes() {
        let now = chrono::Utc::now();
        let before = quiet_and_full(Some(82));
        let after = quiet_and_full(Some(0));
        assert_eq!(before.busy, after.busy);
        assert_eq!(before.queued, after.queued);
        assert!((now - after.state_since).to_std().expect("idle") >= IDLE_FOR);
    }
}
