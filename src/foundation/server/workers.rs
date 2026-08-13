//! Live sessions — the switchboard read directly, so "what is running right now"
//! stops being a question you answer by folding an event log.
//!
//! Nothing else on the wire answers it. `GET /api/sessions` is a summary mirror and
//! carries no workers on purpose ([`AgentView`](crate::foundation::observatory::AgentView)
//! states why: a working session belongs to whoever created it). So worker lifecycle exists
//! only as `worker_spawned` / `worker_resumed` / `worker_finished` frames on the
//! `GET /api/sessions/events` SSE stream, and "is that watch still up?" means pairing
//! spawns against finishes by hand. Both halves of that pairing have already been
//! wrong in production:
//!
//! - `server.log`, 2026-08-03: `WARN worker report dropped; reaction loop gone worker=9`.
//!   A restart ate an in-flight worker's report — the spawn frame is in the log, the
//!   finish frame never came, and the work is gone (`docs/user-journeys/gaps.md` §3).
//! - The same run: the agent said a price watch was "挂着呢,一直在盯" while
//!   `GET /api/sessions` showed one reaction session and **zero workers** (§2). Silence
//!   was read as health because there was no cheap way to check liveness.
//!
//! These handlers read [`registry::global()`] — the process switchboard, which is the
//! thing that actually holds the sessions — rather than any derived mirror. Whatever
//! is here is live by construction: an entry exists between `register` and
//! `unregister`, and a session whose drive task has ended is simply absent.
//!
//! **Read-only, and that is not an omission.** The registry has no stop/abort verb:
//! `Registry::unregister` removes the address (a subsequent `send` reports `Unknown`)
//! but does not touch the codex subprocess or the drive task behind it, so exposing it
//! as `POST /api/workers/{id}/stop` would report a stop that did not happen. A real
//! stop verb has to be built in the registry first; there is no route for one here.
//!
//! - `GET /api/workers` — every live session, busy first, most recently started next.
//! - `GET /api/workers/{id}` — one session plus its full retained message tail.
//! - `GET /api/workers/ended` — sessions that are no longer live, most recent first.
//! - `GET /api/workers/{id}/frames` — one session's verbatim wire log, live or ended.
//! - `GET /api/workers/{id}/messages` — the same log folded into what happened.
//!
//! **The live roster and the ended list stay separate endpoints on purpose.** Everything on
//! `GET /api/workers` is live by construction, which is the property this whole module
//! exists for; folding ended rows into it would mean a caller could no longer tell
//! "running" from "ran" — the exact confusion behind §2 above. They are joined in the
//! reader, where the difference stays visible on the page.

use axum::Json;
use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::SecondsFormat;
use serde::{Deserialize, Serialize};

use crate::foundation::registry::{self, SessionId, Status};

/// One live session, as the review view reads it.
///
/// **`state` is one word where the registry has two booleans, and the fold happens here
/// rather than in the reader.** `busy` and `queued` are not two axes — they are three
/// readings of one thing (running · mail waiting · idle) — and shipping them separately
/// meant every reader had to recombine them, which the review view did not: it drew `idle`
/// from one and a `mail waiting` chip from the other, so a session with work sitting in
/// its inbox read as quiet. Folding server-side also means `curl /api/workers` during a
/// journey test reads the same state word the page shows, which is the same reason the
/// message fold lives in [`crate::foundation::codex::messages`] rather than in the view.
///
/// The registry keeps both booleans — they are what the loops branch on — and nothing else
/// on the wire needs them.
///
/// One field still does not map one-to-one onto [`Status`], and the difference is the
/// registry's, not this module's:
///
/// - `queued`, folded into `state` above, is a **bool** in `Status` and not a count: the
///   registry answers "is anything waiting for its next turn", never how many. It merges a
///   burst into one prompt, so a count would not survive being taken anyway.
///
/// **`owner` used to be a label and is now the id, with the label beside it.** It rendered
/// as the owner's *role word* when the owner was still registered and as the bare id only
/// once the owner had died — so the id, the one thing a reader can build a tree out of, was
/// available exactly when it was useless, and a row said `owner: "cognition"` the rest of
/// the time. That happened to be unambiguous only because one session per rung is live at a
/// time; it is a string coincidence, not an address. `owner` is the id; `owner_role` is the
/// word; a dead owner keeps its id and reports no word.
#[derive(Serialize)]
struct WorkerDto {
    id: String,
    /// The role, as the `X-HI-Role` header and `tools_for_role` spell it — `reaction`,
    /// `cognition`, `reflection`, `worker` — so a row here and a tool
    /// surface line up by eye.
    role: &'static str,
    /// Which kind of working session, for the five that share the `worker` surface;
    /// `null` for a rung.
    ///
    /// **This used to be unanswerable.** The switchboard held a role type of its own with
    /// no room for the specialism, so `create_worker` consumed the type to pick a system
    /// prompt and nothing kept it — every `view-builder` and `file-filer` in flight
    /// reported here as a bare `worker`, and "has the reviewer ever actually run?" had no
    /// answer outside a transcript. One `Role` carries both now.
    #[serde(rename = "type")]
    worker_type: Option<&'static str>,
    /// The id of the session that created this one, when it has an owner. A tree is keyed
    /// on this.
    owner: Option<String>,
    /// The owner's role word while the owner is still live, else `null` — an owner that
    /// shut down while its worker runs is the condition behind the dropped report, so the
    /// `owner` id above is kept either way.
    owner_role: Option<&'static str>,
    /// The one line this session shows up as — see
    /// [`registry::Status::title`](crate::foundation::registry::Status::title). It used to be
    /// the whole brief a worker was sent, and a card could only ever show its first clause.
    title: String,
    /// The ledger task this session serves, when it was created against one.
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<String>,
    /// `running` · `waiting` · `idle` — see the type doc. One word, derived here.
    state: &'static str,
    turns: u64,
    /// RFC3339, whole seconds, UTC.
    started: String,
    /// When it entered `state`, RFC3339. The elapsed figure a reader wants: `started`
    /// measures uptime, which is the same for a session quiet for five minutes and one that
    /// finished a turn two seconds ago.
    state_since: String,
    /// The most recent non-blank line of the session's retained output, or `null` if
    /// it has not said anything yet. The full tail is on `GET /api/workers/{id}`.
    ///
    /// What it has **said**. A session can work for minutes without saying anything, so
    /// this being `null` is not evidence of trouble — see `doing`.
    tail: Option<String>,
    /// The last thing it was seen **doing** — a shell command, a tool call — or `null`
    /// before it has done anything.
    ///
    /// The field that answers "is this alive". `tail` could not: the switchboard's output
    /// tail is fed only from the session's own text, so a worker grinding through commands
    /// reported nothing for as long as it worked.
    ///
    /// **It is only meaningful while `state` is `running`.** Nothing clears it when a turn
    /// ends, and there is nothing it could honestly be cleared to — it is the last thing
    /// seen, which is a fact whenever it is read. A reader that draws it beside a
    /// non-running state is what made a finished session say `idle` and `thinking` at once.
    doing: Option<String>,
    /// When `doing` was last replaced, RFC3339. What separates a busy session that is
    /// working from one that is hung: the line alone cannot.
    doing_at: Option<String>,
}

/// `GET /api/workers` — every live session in the switchboard.
///
/// Every session, not only a worker. A worker's owner is a session too, and the
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

/// How many rows an `ended` or `frames` request returns without asking.
const DEFAULT_LIMIT: usize = 50;
const MAX_ENDED: usize = 500;
/// A frame log holds one JSON-RPC line per streamed chunk, so a busy session's file runs to
/// tens of thousands of lines. The tail is what a reader wants and the cap is what keeps one
/// click off a multi-megabyte response.
const MAX_FRAMES: usize = 2_000;
/// Messages are the fold of those frames — twenty-odd frames to a message on the logs this
/// was measured against — so a default that covers a whole working session is a few hundred
/// rows, not a few thousand.
const DEFAULT_MESSAGES: usize = 200;
const MAX_MESSAGES: usize = 2_000;

#[derive(Deserialize)]
pub struct Paging {
    limit: Option<usize>,
}

/// `GET /api/workers/ended?limit=N` — sessions that are no longer live, most recent first.
///
/// The half of the roster the switchboard structurally cannot answer: an entry exists only
/// between `register` and `unregister`, so a worker that finished thirty seconds ago is
/// simply absent, and `gaps.md` §2 is what that absence looked like from the outside — an
/// agent insisting a watch was running against a page that showed nothing.
///
/// Includes sessions a **restart** ended (`how: "restart"`), which have no close record
/// because nothing got to write one. Those are the rows worth the whole endpoint.
pub async fn get_ended(Query(paging): Query<Paging>) -> Response {
    let limit = paging.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_ENDED);
    let ended = registry::global().recent_ended(limit);
    Json(serde_json::json!({ "ended": ended })).into_response()
}

#[derive(Deserialize)]
pub struct FramesQuery {
    /// Which run's log to read. Defaults to the current run, which is what a live session
    /// is always in; an ended row carries its own run and must pass it.
    run: Option<String>,
    limit: Option<usize>,
}

/// `GET /api/workers/{id}/frames?run=<run>&limit=N` — one session's verbatim wire log.
///
/// The tap has written this file since it was given a data dir
/// ([`WireTap::with_durable_log`](crate::foundation::codex::WireTap::with_durable_log)) and
/// **nothing has ever read it back.** The only other wire route,
/// `GET /api/wire/frames/events`, streams the in-memory ring — 4000 frames, global across
/// every session — so a specific session's history was unreachable even while it ran, and
/// unreachable *by construction* once it ended, because the path is keyed by
/// `(run, session)` and ids restart at 1 each boot.
///
/// Returns the **tail**, newest last, parsed if each line parses and passed through as a
/// string if not — a line the current build cannot parse is still the record of what
/// happened, and dropping it would be the one thing this endpoint must not do.
pub async fn get_frames(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<super::AppState>>,
    Path(id): Path<String>,
    Query(q): Query<FramesQuery>,
) -> Response {
    let (run, text) = match read_log(&state, &id, q.run).await {
        Ok(found) => found,
        Err(response) => return response,
    };
    let Some(text) = text else {
        return Json(serde_json::json!({
            "run": run, "session": id, "frames": [], "truncated": false, "total": 0
        }))
        .into_response();
    };
    let limit = q.limit.unwrap_or(MAX_FRAMES).clamp(1, MAX_FRAMES);

    let (frames, truncated, total) = tail_frames(&text, limit);
    Json(serde_json::json!({
        "run": run,
        "session": id,
        "frames": frames,
        "truncated": truncated,
        "total": total,
    }))
    .into_response()
}

/// `GET /api/workers/{id}/messages?run=<run>&limit=N` — the same log, read as what happened.
///
/// Same file, different question. `frames` answers *what crossed the wire*, which is the
/// record and has to stay verbatim; this answers *what the session did*, which is what a
/// person opening a session is asking. One agent message is an `item/started`, hundreds of
/// deltas and an `item/completed` — six hundred rows saying one sentence — so the frame log
/// rendered row-per-line is unreadable by construction, not by styling. The fold is in
/// [`crate::foundation::codex::messages`] and lives server-side so that every reader gets
/// the same reading: the view, `curl` during a journey test, and whatever comes next.
///
/// Nothing here is stored. The fold is derived on every read, which is what makes it safe
/// for it to be opinionated — a mistake in it is a display bug, and the record it was folded
/// from is still on disk, still whole.
pub async fn get_messages(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<super::AppState>>,
    Path(id): Path<String>,
    Query(q): Query<FramesQuery>,
) -> Response {
    let (run, text) = match read_log(&state, &id, q.run).await {
        Ok(found) => found,
        Err(response) => return response,
    };
    let Some(text) = text else {
        return Json(serde_json::json!({
            "run": run, "session": id, "messages": [], "turns": [],
            "truncated": false, "total": 0, "frames": 0, "unreadable": 0
        }))
        .into_response();
    };
    let limit = q.limit.unwrap_or(DEFAULT_MESSAGES).clamp(1, MAX_MESSAGES);

    let folded = crate::foundation::codex::messages::fold(&text);
    let total = folded.messages.len();
    let truncated = total > limit;
    // The tail, like `frames` — the end of a session is what a reader came for, and the
    // start of a long one is the standing prompt they have read a hundred times.
    let messages = &folded.messages[total.saturating_sub(limit)..];

    Json(serde_json::json!({
        "run": run,
        "session": id,
        "messages": messages,
        "turns": folded.turns,
        "truncated": truncated,
        "total": total,
        // Both counts travel, because "4,000 frames and no messages" has to be readable as
        // a fold that failed rather than as a session that did nothing.
        "frames": folded.frames,
        "unreadable": folded.unreadable,
    }))
    .into_response()
}

/// The front half both readers share: check the id, check the run, read the file.
///
/// `Ok((run, None))` is a session with no log — not an error. A session that never spoke to
/// a subprocess has none, and neither does one whose run's files have been cleared.
async fn read_log(
    state: &super::AppState,
    id: &str,
    run: Option<String>,
) -> Result<(String, Option<String>), Response> {
    let Ok(session) = id.trim().parse::<SessionId>() else {
        return Err(err("a session id is a number"));
    };
    let run = run.unwrap_or_else(|| crate::foundation::run::id().to_string());
    // `run` lands in a path segment, so it is checked rather than trusted: a run id is
    // twelve hex characters ([`crate::foundation::run::id`]) and anything else — a `..`,
    // a separator, an absolute path — is refused before it reaches the filesystem.
    if !is_run_id(&run) {
        return Err(err("a run id is twelve hex characters"));
    }

    let path = crate::mind::memory::layout::session_frames_path(&state.data_dir, &run, session);
    match tokio::fs::read_to_string(&path).await {
        Ok(text) => Ok((run, Some(text))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((run, None)),
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "cannot read a session frame log");
            Err(err("that session's frame log could not be read"))
        }
    }
}

/// The last `limit` frames of a log, oldest first, plus whether anything was cut and how
/// many there were in all.
///
/// A line that will not parse is passed through **as a string** rather than dropped. This
/// is the verbatim record of what crossed the wire; a build that has met a shape it cannot
/// deserialize still has to show it, because "the frame we could not read" is exactly the
/// frame someone is looking for.
fn tail_frames(text: &str, limit: usize) -> (Vec<serde_json::Value>, bool, usize) {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let total = lines.len();
    let truncated = total > limit;
    let frames = lines[total.saturating_sub(limit)..]
        .iter()
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|_| serde_json::Value::String((*line).to_string()))
        })
        .collect();
    (frames, truncated, total)
}

/// Whether `s` is a run id as [`crate::foundation::run::id`] mints them: exactly twelve
/// lowercase hex characters. The gate on a path segment that arrives from a query string.
fn is_run_id(s: &str) -> bool {
    s.len() == 12 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
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
    registry::global().statuses()
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
        role: st.role.as_str(),
        worker_type: st.role.worker_type().map(|t| t.as_str()),
        owner: st.owner.map(|o| o.to_string()),
        owner_role: owner_role(st.owner),
        title: st.title.clone(),
        subject: st.subject.clone(),
        state: state_of(st),
        turns: st.turns,
        started: stamp(st.started),
        state_since: stamp(st.state_since),
        tail,
        doing: st.doing.clone(),
        doing_at: st.doing_at.map(stamp),
    }
}

/// The one word for what a session is doing with itself — see [`WorkerDto::state`].
///
/// `busy` wins over `queued`, because mail landing mid-turn changes nothing a reader can
/// act on: it will be picked up by the turn after this one, and the session is running
/// either way. `waiting` is the case worth naming — nothing in flight and work already
/// sitting there — because a wait that keeps growing is a drain that has stopped.
fn state_of(st: &Status) -> &'static str {
    if st.busy {
        "running"
    } else if st.queued {
        "waiting"
    } else {
        "idle"
    }
}

fn stamp(at: chrono::DateTime<chrono::Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The owner's role word while the owner is still registered, else `None`.
///
/// The id travels separately and unconditionally ([`WorkerDto::owner`]), so an owner that
/// has shut down under a running worker keeps its address here and only loses its label.
/// That is the orphan condition worth seeing, and it used to be the *only* case in which
/// the id was reported at all.
///
/// A local `role_name` match used to stand where `Role::as_str` is called — a fourth
/// spelling of the same five strings, kept in step with the header by comment alone.
fn owner_role(owner: Option<SessionId>) -> Option<&'static str> {
    let owner = owner?;
    registry::global().status(owner).map(|st| st.role.as_str())
}

/// A uniform JSON error body with a 400 (same shape as `people.rs`).
fn err(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": msg }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{Role, WorkerType};
    use chrono::{TimeZone, Utc};

    fn status(id: SessionId, busy: bool, minute: u32) -> Status {
        Status {
            id,
            role: Role::Worker(WorkerType::General),
            owner: None,
            title: "check oil prices".into(),
            subject: None,
            busy,
            queued: false,
            turns: 3,
            started: Utc.with_ymd_and_hms(2026, 8, 5, 3, minute, 0).unwrap(),
            // A minute after it started, so a test can tell the two clocks apart — which is
            // the whole reason there are two.
            state_since: Utc.with_ymd_and_hms(2026, 8, 5, 3, minute + 1, 0).unwrap(),
            doing: None,
            doing_at: None,
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
        assert_eq!(v["type"], "general");
        assert_eq!(v["owner"], serde_json::Value::Null);
        assert_eq!(v["owner_role"], serde_json::Value::Null);
        assert_eq!(v["doing"], serde_json::Value::Null);
        assert_eq!(v["title"], "check oil prices");
        assert_eq!(v["state"], "running");
        assert_eq!(v["turns"], 3);
        assert_eq!(v["started"], "2026-08-05T03:12:00Z");
        assert_eq!(v["state_since"], "2026-08-05T03:13:00Z");
        assert_eq!(v["doing_at"], serde_json::Value::Null);
        assert_eq!(v["tail"], "last line");
        assert!(v.get("busy").is_none(), "folded into `state`");
        assert!(v.get("queued").is_none(), "folded into `state`");
    }

    /// The three states, and the precedence between them. `waiting` is the one the old
    /// two-boolean shape could not say: a session with work already in its inbox reported
    /// `busy: false`, which every reader spelled `idle`.
    #[test]
    fn two_booleans_fold_to_one_state_word() {
        let cases = [
            (false, false, "idle"),
            (false, true, "waiting"),
            (true, false, "running"),
            // Mail landing mid-turn changes nothing a reader can act on: the next turn
            // picks it up and this one is still running.
            (true, true, "running"),
        ];
        for (busy, queued, want) in cases {
            let mut st = status(1, busy, 0);
            st.queued = queued;
            assert_eq!(state_of(&st), want, "busy={busy} queued={queued}");
            assert_eq!(serde_json::to_value(dto(&st, None)).unwrap()["state"], want);
        }
    }

    /// Every role reaches a row, and a worker's row carries its specialism while a
    /// rung's `type` is null. The five specialisms all report `role: worker` — the
    /// surface they share — so the view narrows on `role` and labels on `type`.
    #[test]
    fn a_row_names_its_role_and_a_worker_row_its_type() {
        for role in Role::ALL {
            let mut st = status(1, false, 0);
            st.role = *role;
            let v = serde_json::to_value(dto(&st, None)).unwrap();
            assert_eq!(v["role"], role.as_str());
            match role.worker_type() {
                Some(t) => assert_eq!(v["type"], t.as_str()),
                None => assert_eq!(v["type"], serde_json::Value::Null),
            }
        }
    }

    /// The regression this whole change exists for: a specialist in flight is reportable
    /// as one. Before, `create_worker` consumed the type to choose a prompt and the
    /// switchboard recorded a bare `worker`, so nothing downstream could tell a view
    /// reviewer from a file filer.
    #[test]
    fn a_specialist_is_reportable_as_one() {
        let id = registry::mint();
        registry::global().register(id, Role::Worker(WorkerType::ViewReviewer), None, "judge it".into(), None);
        let st = registry::global().status(id).unwrap();
        let v = serde_json::to_value(dto(&st, None)).unwrap();
        assert_eq!(v["role"], "worker");
        assert_eq!(v["type"], "view-reviewer");
        registry::global().unregister(id);
    }

    /// The id is always the id, and the role word is the part that can go missing. A
    /// worker whose owner shut down under it keeps a usable address — which is what the
    /// old label-only field lost in exactly the case that matters.
    #[test]
    fn an_owner_keeps_its_id_and_only_loses_its_role_word() {
        let owner = registry::mint();
        registry::global().register(owner, Role::Cognition, None, String::new(), None);
        assert_eq!(owner_role(Some(owner)), Some("cognition"));

        let worker = status_owned_by(owner);
        let v = serde_json::to_value(dto(&worker, None)).unwrap();
        assert_eq!(v["owner"], owner.to_string(), "the tree is keyed on this");
        assert_eq!(v["owner_role"], "cognition");

        registry::global().unregister(owner);
        assert_eq!(owner_role(Some(owner)), None, "no live owner, no word");
        let v = serde_json::to_value(dto(&worker, None)).unwrap();
        assert_eq!(v["owner"], owner.to_string(), "but still an address");
        assert_eq!(v["owner_role"], serde_json::Value::Null);
        assert_eq!(owner_role(None), None);
    }

    fn status_owned_by(owner: SessionId) -> Status {
        let mut st = status(99, true, 0);
        st.owner = Some(owner);
        st
    }

    /// `doing` travels, and it is what a session working in silence has to show. A row
    /// with an empty `tail` and a live `doing` is the shape this field was added for.
    #[test]
    fn a_row_carries_what_it_is_doing_even_with_nothing_said() {
        let mut st = status(3, true, 0);
        st.doing = Some("$ cargo test".into());
        st.doing_at = Some(Utc.with_ymd_and_hms(2026, 8, 5, 3, 4, 0).unwrap());
        let v = serde_json::to_value(dto(&st, None)).unwrap();
        assert_eq!(v["doing"], "$ cargo test");
        assert_eq!(v["tail"], serde_json::Value::Null, "said nothing, still visibly working");
        // The age is what separates working from hung; the line alone says only "alive".
        assert_eq!(v["doing_at"], "2026-08-05T03:04:00Z");
    }

    /// `run` reaches the filesystem as a path segment, so only a real run id passes. The
    /// traversal cases are the point: `session_frames_path` joins this straight in.
    #[test]
    fn only_a_real_run_id_reaches_the_filesystem() {
        assert!(is_run_id("0123456789ab"));
        assert!(is_run_id(&"f".repeat(12)));

        for bad in [
            "",
            "0123456789a",       // eleven
            "0123456789abc",     // thirteen
            "0123456789AB",      // uppercase is not what `run::id` mints
            "0123456789ag",      // g is not hex
            "../../../../etc/p", // the one that matters
            "..",
            "/absolute/pa",
            "abcdef/012345",
        ] {
            assert!(!is_run_id(bad), "{bad:?} must not reach a path");
        }
    }

    /// A real run id round-trips: whatever `run::id()` mints must pass its own gate, or
    /// the live-session default is rejected by the check meant to protect it.
    #[test]
    fn the_current_run_id_passes_its_own_gate() {
        assert!(is_run_id(crate::foundation::run::id()));
    }

    /// The tail is the newest frames in wire order, and the count reported is the whole
    /// file — so a reader can tell "this is all of it" from "this is the end of it".
    #[test]
    fn the_frame_tail_is_the_newest_in_order_and_says_what_it_cut() {
        let text = (1..=5)
            .map(|i| format!(r#"{{"seq":{i}}}"#))
            .collect::<Vec<_>>()
            .join("\n");

        let (frames, truncated, total) = tail_frames(&text, 2);
        assert_eq!(total, 5);
        assert!(truncated);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["seq"], 4, "oldest of the tail first");
        assert_eq!(frames[1]["seq"], 5);

        let (all, truncated, total) = tail_frames(&text, 50);
        assert_eq!((all.len(), truncated, total), (5, false, 5));
    }

    /// A line this build cannot parse is still the record of what crossed the wire, so it
    /// comes through as a string. Dropping it is the one thing a verbatim log may not do.
    #[test]
    fn an_unparseable_frame_survives_as_text() {
        let (frames, _, total) = tail_frames("{\"seq\":1}\nnot json at all\n\n{\"seq\":3}\n", 10);
        assert_eq!(total, 3, "blank lines are not frames");
        assert_eq!(frames[0]["seq"], 1);
        assert_eq!(frames[1], "not json at all");
        assert_eq!(frames[2]["seq"], 3);
    }

    /// An empty log is an empty list, not an error and not a panic on the slice.
    #[test]
    fn an_empty_log_is_an_empty_list() {
        let (frames, truncated, total) = tail_frames("", 10);
        assert!(frames.is_empty());
        assert_eq!((truncated, total), (false, 0));
        let (frames, _, total) = tail_frames("\n\n  \n", 10);
        assert!(frames.is_empty());
        assert_eq!(total, 0);
    }

    /// The tail is the last thing said, not the first, and blank lines are not "said".
    #[test]
    fn the_tail_is_the_last_nonblank_line() {
        let id = registry::mint();
        registry::global().register(id, Role::Worker(WorkerType::General), None, "an errand".into(), None);
        assert_eq!(tail(id), None, "nothing said yet");

        registry::global().record_output(id, "first\n\nsecond\n\n");
        assert_eq!(message_lines(id), vec!["first", "second"]);
        assert_eq!(tail(id).as_deref(), Some("second"));

        registry::global().unregister(id);
        assert!(message_lines(id).is_empty(), "a closed session retains nothing");
    }
}
