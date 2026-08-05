//! Live sessions — the switchboard read directly, so "what is running right now"
//! stops being a question you answer by folding an event log.
//!
//! Nothing else on the wire answers it. `GET /api/sessions` is keyed by scene and
//! carries no workers on purpose ([`SceneView`](crate::foundation::observatory::SceneView)
//! states why: a working session belongs to whoever created it, and the rungs that
//! create them — Cognition, Reflection — have no scene). So worker lifecycle exists
//! only as `worker_spawned` / `worker_resumed` / `worker_finished` frames on the
//! `GET /api/sessions/events` SSE stream, and "is that watch still up?" means pairing
//! spawns against finishes by hand. Both halves of that pairing have already been
//! wrong in production:
//!
//! - `server.log`, 2026-08-03: `WARN worker report dropped; scene loop gone worker=9`.
//!   A restart ate an in-flight worker's report — the spawn frame is in the log, the
//!   finish frame never came, and the work is gone (`docs/user-journeys/gaps.md` §3).
//! - The same run: the agent said a price watch was "挂着呢,一直在盯" while
//!   `GET /api/sessions` showed one reactor session and **zero workers** (§2). Silence
//!   was read as health because there was no cheap way to check liveness.
//!
//! These handlers read [`registry::global()`] — the process switchboard, which is the
//! thing that actually holds the sessions — rather than any derived mirror. Whatever
//! is here is live by construction: an entry exists between `register` and
//! `unregister`, and a session whose drive task has ended is simply absent.
//!
//! **Read-only, and that is not an omission.** The registry has no stop/abort verb:
//! `Registry::unregister` removes the address (a subsequent `send` reports `Unknown`)
//! but does not touch the ACP subprocess or the drive task behind it, so exposing it
//! as `POST /api/workers/{id}/stop` would report a stop that did not happen. A real
//! stop verb has to be built in the registry first; there is no route for one here.
//!
//! - `GET /api/workers` — every live session, busy first, most recently started next.
//! - `GET /api/workers/{id}` — one session plus its full retained message tail.

use axum::Json;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::SecondsFormat;
use serde::Serialize;

use crate::foundation::registry::{self, Role, SessionId, Status};

/// One live session, as the review view reads it.
///
/// Three fields do not map one-to-one onto [`Status`], and the difference is the
/// registry's, not this module's:
///
/// - `role` is the **rung** (`worker`, `deliberation`, `cognition`, `reflection`,
///   `reaction`), not the [`WorkerType`](crate::identity::WorkerType) a worker was
///   created with. The switchboard never learns the worker type — `create_worker`
///   consumes it to pick a system prompt and nothing stores it — so `general` /
///   `view-builder` / `file-filer` cannot be reported from here.
/// - `owner` is a session id in `Status`. It is rendered as the owner's rung when
///   that session is still registered (so the common case reads `cognition`), and as
///   the bare id when the owner has already gone.
/// - `queued` is a **bool** in `Status`, not a count: the registry answers "is
///   anything waiting for its next turn", never how many. It merges a burst into one
///   prompt, so a count would not survive being taken anyway.
#[derive(Serialize)]
struct WorkerDto {
    id: String,
    role: &'static str,
    /// The scene this is hosted under, `null` for the sceneless rungs. May be a
    /// pseudo-scene (`*cognition*`, `*consolidation*`) — a routing tag that names no
    /// conversation; it is passed through as-is rather than blanked, because that is
    /// what the session's `X-HI-Scene` actually says.
    scene: Option<String>,
    owner: Option<String>,
    task: String,
    busy: bool,
    queued: bool,
    turns: u64,
    /// RFC3339, whole seconds, UTC.
    started: String,
    /// The most recent non-blank line of the session's retained output, or `null` if
    /// it has not said anything yet. The full tail is on `GET /api/workers/{id}`.
    tail: Option<String>,
}

/// `GET /api/workers` — every live session in the switchboard.
///
/// Every session, not only `Role::Worker`. A worker's owner is a session too, and the
/// bug this endpoint exists for was an agent asserting that work was in flight when the
/// switchboard was empty — a filter here would rebuild exactly that blind spot. The
/// `role` field says what each row is, so the view can narrow; the endpoint cannot
/// widen if it has already thrown rows away.
pub async fn get_workers() -> Response {
    let mut live = live_sessions();
    sort_live(&mut live);
    let workers: Vec<WorkerDto> = live.iter().map(|st| dto(st, tail(st.id))).collect();
    Json(serde_json::json!({ "workers": workers })).into_response()
}

/// `GET /api/workers/{id}` — one session with its full retained message tail.
///
/// The tail is what the switchboard kept (`OUTPUT_TAIL_CHARS`, ~4k chars of the
/// session's own text output), split into lines with the blanks dropped. It is a live
/// tail and not an archive: the durable copy is the log.
pub async fn get_worker(Path(id): Path<String>) -> Response {
    let Ok(id) = id.trim().parse::<SessionId>() else {
        return err("a session id is a number");
    };
    let Some(st) = registry::global().status(id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("no live session {id}") })),
        )
            .into_response();
    };
    let messages = message_lines(id);
    let worker = dto(&st, messages.last().cloned());
    Json(serde_json::json!({ "worker": worker, "messages": messages })).into_response()
}

// ── reading the switchboard ───────────────────────────────────────────────────

/// Every session the registry still holds.
///
/// The registry exposes no enumeration verb — `Registry::status` is keyed by id, and
/// `children`/`reachable` both need an id to start from — so the id space is walked
/// instead. Ids are minted from one process-wide counter starting at 1, so minting a
/// fresh one gives an exact upper bound on everything handed out so far; ids are u64
/// and cost nothing, and the burned one is never registered.
///
/// Cost is one hashmap lookup per id minted this run, each taking the registry lock
/// briefly. That is fine for a debug surface at human polling rates and is the price of
/// not keeping a second, drift-prone index of live sessions on this side.
fn live_sessions() -> Vec<Status> {
    let upper = registry::mint();
    (1..upper).filter_map(|id| registry::global().status(id)).collect()
}

/// Busy first, then most recently started. What is running now is what the reader came
/// for; among the idle, the freshest is the one they are most likely asking about.
fn sort_live(rows: &mut [Status]) {
    rows.sort_by(|a, b| b.busy.cmp(&a.busy).then(b.started.cmp(&a.started)));
}

/// The session's retained output as non-blank lines, oldest first.
fn message_lines(id: SessionId) -> Vec<String> {
    registry::global()
        .messages(id)
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The last non-blank line of a session's output — cheap, because the registry already
/// keeps only a bounded tail.
fn tail(id: SessionId) -> Option<String> {
    message_lines(id).pop()
}

fn dto(st: &Status, tail: Option<String>) -> WorkerDto {
    WorkerDto {
        id: st.id.to_string(),
        role: role_name(st.role),
        scene: st.scene.as_ref().map(|s| s.0.clone()),
        owner: owner_label(st.owner),
        task: st.task.clone(),
        busy: st.busy,
        queued: st.queued,
        turns: st.turns,
        started: st.started.to_rfc3339_opts(SecondsFormat::Secs, true),
        tail,
    }
}

/// The rung, lowercased — the same spellings the `X-HI-Role` header and
/// `tools_for_role` use, so a row here and a tool surface can be lined up by eye.
fn role_name(role: Role) -> &'static str {
    match role {
        Role::Reaction => "reaction",
        Role::Deliberation => "deliberation",
        Role::Cognition => "cognition",
        Role::Reflection => "reflection",
        Role::Worker => "worker",
    }
}

/// The owner's rung when it is still registered, else its bare id. An owner that has
/// shut down while its worker runs is the exact condition behind the dropped report, so
/// it is shown as an id rather than dropped to `null`.
fn owner_label(owner: Option<SessionId>) -> Option<String> {
    let owner = owner?;
    Some(match registry::global().status(owner) {
        Some(st) => role_name(st.role).to_string(),
        None => owner.to_string(),
    })
}

/// A uniform JSON error body with a 400 (same shape as `people.rs`).
fn err(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": msg }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Scene;
    use chrono::{TimeZone, Utc};

    fn status(id: SessionId, busy: bool, minute: u32) -> Status {
        Status {
            id,
            role: Role::Worker,
            scene: Some(Scene("boss".into())),
            owner: None,
            task: "check oil prices".into(),
            busy,
            queued: false,
            turns: 3,
            started: Utc.with_ymd_and_hms(2026, 8, 5, 3, minute, 0).unwrap(),
        }
    }

    /// What is running now comes first; among the idle, the freshest. Both keys matter:
    /// sorting by start time alone buries a busy session under newer idle ones.
    #[test]
    fn busy_first_then_most_recently_started() {
        let mut rows = vec![
            status(1, false, 10),
            status(2, true, 5),
            status(3, false, 40),
            status(4, true, 30),
        ];
        sort_live(&mut rows);
        assert_eq!(
            rows.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![4u64, 2, 3, 1],
            "busy (newest first), then idle (newest first)"
        );
    }

    /// The one shape the view is written against. `started` is whole-second RFC3339 —
    /// chrono's default carries sub-second digits, which is not what is advertised.
    #[test]
    fn a_row_carries_the_advertised_fields() {
        let dto = dto(&status(9, true, 12), Some("last line".into()));
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["id"], "9");
        assert_eq!(v["role"], "worker");
        assert_eq!(v["scene"], "boss");
        assert_eq!(v["owner"], serde_json::Value::Null);
        assert_eq!(v["task"], "check oil prices");
        assert_eq!(v["busy"], true);
        assert_eq!(v["queued"], false);
        assert_eq!(v["turns"], 3);
        assert_eq!(v["started"], "2026-08-05T03:12:00Z");
        assert_eq!(v["tail"], "last line");
    }

    #[test]
    fn every_rung_has_a_name() {
        for (role, name) in [
            (Role::Reaction, "reaction"),
            (Role::Deliberation, "deliberation"),
            (Role::Cognition, "cognition"),
            (Role::Reflection, "reflection"),
            (Role::Worker, "worker"),
        ] {
            assert_eq!(role_name(role), name);
        }
    }

    /// A live owner reads as its rung; one that has already shut down reads as its id
    /// rather than vanishing — an orphaned worker is the condition worth seeing.
    #[test]
    fn an_owner_reads_as_its_rung_and_a_dead_one_as_its_id() {
        let owner = registry::mint();
        registry::global().register(owner, Role::Cognition, None, None, String::new());
        assert_eq!(owner_label(Some(owner)).as_deref(), Some("cognition"));

        registry::global().unregister(owner);
        assert_eq!(owner_label(Some(owner)), Some(owner.to_string()));
        assert_eq!(owner_label(None), None);
    }

    /// The tail is the last thing said, not the first, and blank lines are not "said".
    #[test]
    fn the_tail_is_the_last_nonblank_line() {
        let id = registry::mint();
        registry::global().register(id, Role::Worker, None, None, "an errand".into());
        assert_eq!(tail(id), None, "nothing said yet");

        registry::global().record_output(id, "first\n\nsecond\n\n");
        assert_eq!(message_lines(id), vec!["first", "second"]);
        assert_eq!(tail(id).as_deref(), Some("second"));

        registry::global().unregister(id);
        assert!(message_lines(id).is_empty(), "a closed session retains nothing");
    }
}
