//! User-facing agent activity.
//!
//! This is a projection of live session facts, not a second state machine. The face
//! decides between Starting / Listening / Speaking / Typing / Working / Idle; this
//! endpoint supplies only the backend facts it cannot observe locally.

use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::{self, Stream};
use serde::Serialize;

use crate::foundation::registry::{self, SessionId, Status};
use crate::identity::Role;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct AgentActivity {
    /// The conversation's Reaction loop has registered and can accept work.
    reaction_ready: bool,
    /// Reaction is processing a turn. The face renders this as Typing.
    reaction_busy: bool,
    /// Relevant non-Reaction sessions with accepted work in flight.
    delegated_busy_count: usize,
}

/// `GET /api/activity` — an immediate snapshot followed by every live change.
pub async fn get_activity() -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = registry::global().subscribe_activity();
    let events = stream::unfold((rx, true), |(mut rx, first)| async move {
        if first {
            // Mark the version represented by the initial snapshot as seen. A change
            // racing after this call remains visible to `changed()` on the next poll.
            let _ = rx.borrow_and_update();
        } else if rx.changed().await.is_err() {
            return None;
        }

        let activity = project(&registry::global().statuses());
        let event = Event::default()
            .event("activity")
            .json_data(activity)
            .unwrap_or_else(|_| Event::default().comment("serialize error"));
        Some((Ok(event), (rx, false)))
    });

    Sse::new(events).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("activity"),
    )
}

fn project(statuses: &[Status]) -> AgentActivity {
    let by_id: HashMap<SessionId, &Status> =
        statuses.iter().map(|status| (status.id, status)).collect();
    let delegated_roots: HashSet<SessionId> = statuses
        .iter()
        .filter(|status| status.role == Role::Cognition)
        .map(|status| status.id)
        .collect();

    let active = |status: &&Status| status.busy || status.queued;
    let reaction_ready = statuses.iter().any(|status| status.role == Role::Reaction);
    let reaction_busy = statuses
        .iter()
        .filter(|status| status.role == Role::Reaction)
        .any(|status| status.busy || status.queued);

    // There is one user-facing voice. Cognition and its descendants are delegated work.
    // Reflection and its descendants are maintenance, not a user-facing obligation
    // represented by this status.
    let delegated_busy_count = statuses
        .iter()
        .filter(active)
        .filter(|status| match status.role {
            Role::Cognition => true,
            Role::Worker(_) => owner_chain_reaches(status.owner, &delegated_roots, &by_id),
            Role::Reaction | Role::Reflection => false,
        })
        .count();

    AgentActivity {
        reaction_ready,
        reaction_busy,
        delegated_busy_count,
    }
}

fn owner_chain_reaches(
    mut owner: Option<SessionId>,
    roots: &HashSet<SessionId>,
    statuses: &HashMap<SessionId, &Status>,
) -> bool {
    let mut seen = HashSet::new();
    while let Some(id) = owner {
        if roots.contains(&id) {
            return true;
        }
        if !seen.insert(id) {
            return false;
        }
        owner = statuses.get(&id).and_then(|status| status.owner);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::WorkerType;
    use chrono::{TimeZone, Utc};

    fn status(
        id: SessionId,
        role: Role,
        owner: Option<SessionId>,
        busy: bool,
        queued: bool,
    ) -> Status {
        Status {
            id,
            role,
            owner,
            task: String::new(),
            busy,
            queued,
            turns: 0,
            started: Utc.with_ymd_and_hms(2026, 8, 8, 0, 0, 0).unwrap(),
            state_since: Utc.with_ymd_and_hms(2026, 8, 8, 0, 0, 0).unwrap(),
            doing: None,
            doing_at: None,
        }
    }

    #[test]
    fn separates_reaction_from_relevant_delegated_work() {
        let statuses = vec![
            status(1, Role::Reaction, None, true, false),
            status(2, Role::Cognition, None, true, false),
            status(3, Role::Worker(WorkerType::General), Some(2), true, false),
            status(4, Role::Reflection, None, true, false),
            status(5, Role::Worker(WorkerType::General), Some(4), true, false),
        ];

        assert_eq!(
            project(&statuses),
            AgentActivity {
                reaction_ready: true,
                reaction_busy: true,
                delegated_busy_count: 2,
            }
        );
    }

    #[test]
    fn queued_conversation_work_counts_before_its_turn_starts() {
        let statuses = vec![
            status(1, Role::Reaction, None, false, false),
            status(2, Role::Cognition, None, false, true),
        ];

        assert_eq!(project(&statuses).delegated_busy_count, 1);
    }

    #[test]
    fn nested_brain_work_remains_conversation_work() {
        let statuses = vec![
            status(1, Role::Reaction, None, false, false),
            status(2, Role::Cognition, None, false, false),
            status(3, Role::Worker(WorkerType::General), Some(2), false, false),
            status(4, Role::Worker(WorkerType::General), Some(3), true, false),
        ];

        assert_eq!(project(&statuses).delegated_busy_count, 1);
    }

    #[test]
    fn cognition_workers_remain_working_after_cognition_finishes_its_turn() {
        let statuses = vec![
            status(1, Role::Reaction, None, false, false),
            status(2, Role::Cognition, None, false, false),
            status(3, Role::Worker(WorkerType::General), Some(2), true, false),
        ];

        assert_eq!(project(&statuses).delegated_busy_count, 1);
    }
}
