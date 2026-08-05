//! Task-review endpoints — a human's way into the task ledger.
//!
//! The agent opens tasks and does not reliably close them.
//! [`docs/user-journeys/gaps.md`](../../../docs/user-journeys/gaps.md) records the
//! shape of it: a passing question ("最近 GitHub 上在火什么") settled into
//! `tasks/github-trending-list/facet.md` as `kind: wip / state: open` and stayed
//! there, and two task facets written by the same rung in the same run disagreed on
//! whether frontmatter existed at all. Nothing in the host enforces that discipline,
//! so the ledger accumulates.
//!
//! That accumulation is not inert. Cognition's glance-up timer is gated on the open
//! count — `body::reactor::cognition::note_for` returns `None` at zero and wakes the
//! rung at anything above it — so one never-closed task keeps the timer firing
//! forever, and every one of them rides in every agent's window through
//! [`crate::mind::memory::tasks::projection`]. Closing a task is therefore the
//! cheapest repair there is, and until now there was no way to do it but hand-edit
//! `facet.md`.
//!
//! - `GET   /api/tasks` — every task, open **and** closed. Reviewing rot is the use
//!   case, so `done`/`dropped` are visible rather than filtered the way
//!   [`crate::mind::memory::tasks::open_tasks`] filters them.
//! - `PATCH /api/tasks/{subject}` — set `state`, `title`, `kind`. Any field absent
//!   means unchanged.
//!
//! Two things worth knowing before reading further:
//!
//! **A record too broken to parse is the point, not an error.** The task parser is
//! keep-biased — a missing `state` reads as open, an unparseable `due` reads as
//! untimed — so nothing ever *fails*, it just quietly loses fields. This endpoint
//! flags that with `malformed: true` by comparing what the record says against what
//! the parser took, and a subject whose file cannot be read at all is still listed.
//! Dropping it would hide exactly the rot a human is here to find.
//!
//! **A patch regenerates the whole record**, because
//! [`crate::mind::memory::tasks::write_task`] does — that is the facet convention,
//! not a choice made here. So an unparseable `due:` line, or a frontmatter key the
//! task shape does not know, is dropped by a patch rather than preserved. `malformed`
//! on the listing is the warning that a patch will cost something.
//!
//! Tasks are global, not scene-scoped, so these take no `X-HI-Scene`. Reads are one
//! directory listing plus a file per task; the write is the facet layer's atomic
//! temp-then-rename.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::foundation::server::AppState;
use crate::mind::memory::facets;
use crate::mind::memory::tasks::{self, Task, TaskKind, TaskState};

// ── the wire shape ────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct TaskDto {
    /// The facet subject — the directory name, and the task's identity. `PATCH`
    /// addresses a task by exactly this.
    subject: String,
    title: String,
    /// One of `wip`/`serving`/`watch`/`deadline`/`staged` — [`TaskKind::as_str`],
    /// which is the spelling the frontmatter itself uses.
    kind: &'static str,
    /// One of `open`/`done`/`dropped` — [`TaskState::as_str`], likewise.
    state: &'static str,
    #[serde(rename = "reportTo")]
    report_to: Option<String>,
    due: Option<String>,
    checked: Option<String>,
    /// `null` when the record carries no liveness contract at all, so the view can
    /// tell "nothing written" from "written and empty".
    liveness: Option<LivenessDto>,
    /// The agent's prose below the frontmatter, verbatim.
    body: String,
    /// The record on disk says something the parser could not honour, or could not
    /// be read. See [`is_malformed`].
    malformed: bool,
}

#[derive(Serialize)]
struct LivenessDto {
    verify: Option<String>,
    restart: Option<String>,
    owner: Option<String>,
}

/// RFC3339 with a `Z`, seconds precision — `2026-08-09T10:00:00Z`. Not
/// `to_rfc3339()`, which renders `+00:00` and would hand the view two spellings of
/// the same instant depending on which field it came from.
fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn dto(t: &Task, malformed: bool) -> TaskDto {
    TaskDto {
        subject: t.subject.clone(),
        title: t.title.clone(),
        kind: t.kind.as_str(),
        state: t.state.as_str(),
        report_to: t.report_to.as_ref().map(|s| s.0.clone()),
        due: t.due.map(rfc3339),
        checked: t.checked.map(rfc3339),
        liveness: if t.liveness.is_empty() {
            None
        } else {
            Some(LivenessDto {
                verify: t.liveness.verify.clone(),
                restart: t.liveness.restart.clone(),
                owner: t.liveness.owner.clone(),
            })
        },
        body: t.body.clone(),
        malformed,
    }
}

/// (still-open, undated, due, subject) — open first, then soonest due, nulls last,
/// then subject. Computed from the parsed [`Task`] rather than from [`TaskDto`]'s
/// strings so the date ordering is a comparison of instants, not of text.
type SortKey = (u8, u8, i64, String);

fn sort_key(t: &Task) -> SortKey {
    (
        if t.state.is_open() { 0 } else { 1 },
        if t.due.is_some() { 0 } else { 1 },
        t.due.map_or(0, |d| d.timestamp()),
        t.subject.clone(),
    )
}

/// The stand-in for a subject the index lists but whose record will not read.
/// Takes the same keep-biased defaults the task parser takes — open, `wip`, title
/// from the subject — so an unreadable task sorts and renders beside the rest
/// instead of vanishing from the list that exists to show it.
fn unreadable(subject: &str) -> Task {
    let mut t = Task::new(subject, TaskKind::Wip);
    t.title = subject.replace('-', " ");
    t
}

// ── malformed detection ───────────────────────────────────────────────────────

/// One frontmatter scalar, straight off the record.
///
/// A deliberate narrow copy of `mind::memory::episodes::frontmatter_field`, which is
/// `pub(super)` to the memory module: same convention (split on the first `:`,
/// JSON-decode a quoted value, skip a line carrying no `:` rather than ending the
/// scan), because the whole job here is to compare what the file *says* against what
/// the parser *took*, and reading it by a different convention would invent
/// disagreements that aren't there.
fn raw_field(content: &str, key: &str) -> Option<String> {
    let fm = content.strip_prefix("---\n")?;
    let block = &fm[..fm.find("\n---\n")?];
    for line in block.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        if k.trim() != key {
            continue;
        }
        let v = v.trim();
        if v.starts_with('"')
            && let Ok(s) = serde_json::from_str::<String>(v)
        {
            return Some(s);
        }
        return Some(v.to_owned());
    }
    None
}

/// Whether the record on disk asserts something the parsed [`Task`] does not carry.
///
/// The parser never errors — it is keep-biased by design, so a misspelt `state`
/// silently reads as open and a `due: next tuesday` silently reads as untimed. That
/// is the right reading for the projection (a stale line beats a dropped promise)
/// and the wrong one for a review surface, where the difference between "this task
/// has no deadline" and "this task's deadline is unreadable" is the whole reason
/// someone opened the page.
///
/// So: a present, non-empty frontmatter value that the parse did not take is
/// malformed. Unknown keys are not — the frontmatter has always tolerated them.
fn is_malformed(raw: &str, t: &Task) -> bool {
    let field = |k: &str| raw_field(raw, k).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    field("state").is_some_and(|v| v != t.state.as_str())
        || field("kind").is_some_and(|v| v != t.kind.as_str())
        || (field("due").is_some() && t.due.is_none())
        || (field("checked").is_some() && t.checked.is_none())
}

// ── list ──────────────────────────────────────────────────────────────────────

/// `GET /api/tasks` — every task under the `tasks` dimension, closed ones included.
///
/// Walks [`facets::facet_subject_index`] rather than the tasks directory: it is the
/// same rule (a subject exists once it has a `facet.md`) reached through the public
/// facet API, and it is the same list the mind is seeded with, so this view and the
/// agent are looking at one set of subjects.
pub async fn get_tasks(State(state): State<Arc<AppState>>) -> Response {
    let dir = &state.data_dir;
    let index = match facets::facet_subject_index(dir).await {
        Ok(index) => index,
        Err(e) => return err(&e.to_string()),
    };
    let prefix = format!("{}/", tasks::DIMENSION);

    let mut rows: Vec<(SortKey, TaskDto)> = Vec::new();
    for facet_ref in index {
        let Some(subject) = facet_ref.strip_prefix(prefix.as_str()) else {
            continue;
        };
        // Two reads per task: the parsed record, and the raw text to hold it against.
        // The alternative is re-implementing the parse out here, which is how the two
        // would drift.
        let parsed = tasks::read_task(dir, subject).await;
        let raw = facets::read_facet(dir, tasks::DIMENSION, subject).await;
        let (task, malformed) = match (parsed, raw) {
            (Ok(Some(t)), Ok(Some(raw))) => {
                let malformed = is_malformed(&raw, &t);
                (t, malformed)
            }
            // Listed by the index and unreadable now — a race with a rename, a torn
            // file, a permission problem. Listed anyway, flagged: an unreadable task
            // is the most broken thing this page can show, and it is the one thing a
            // silent skip would hide completely.
            _ => (unreadable(subject), true),
        };
        rows.push((sort_key(&task), dto(&task, malformed)));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let out: Vec<TaskDto> = rows.into_iter().map(|(_, d)| d).collect();
    Json(serde_json::json!({ "tasks": out })).into_response()
}

// ── patch ─────────────────────────────────────────────────────────────────────

/// A partial update. Every field is optional; absent means unchanged. Nothing here
/// can *create* a task — a subject that does not exist is a 404, not an insert.
#[derive(Deserialize)]
pub struct TaskPatch {
    state: Option<String>,
    title: Option<String>,
    kind: Option<String>,
}

/// `open`/`done`/`dropped`, strictly.
///
/// The store's own parse is keep-biased and reads anything unrecognised as open —
/// correct when reading a file an agent hand-wrote, wrong when a client is
/// explicitly asking for a change, because a typo would silently *re-open* the task
/// the human was trying to close. An explicit write gets an explicit rejection.
fn parse_state(s: &str) -> Option<TaskState> {
    match s.trim() {
        "open" => Some(TaskState::Open),
        "done" => Some(TaskState::Done),
        "dropped" => Some(TaskState::Dropped),
        _ => None,
    }
}

/// The five kinds, strictly — same reasoning as [`parse_state`], where the
/// keep-biased fallback is `wip`.
fn parse_kind(s: &str) -> Option<TaskKind> {
    match s.trim() {
        "wip" => Some(TaskKind::Wip),
        "serving" => Some(TaskKind::Serving),
        "watch" => Some(TaskKind::Watch),
        "deadline" => Some(TaskKind::Deadline),
        "staged" => Some(TaskKind::Staged),
        _ => None,
    }
}

/// `PATCH /api/tasks/{subject}` — close, drop, re-open, retitle, or re-kind one task.
///
/// Read, apply, write the whole record back through
/// [`crate::mind::memory::tasks::write_task`] — the facet convention is regenerate,
/// don't patch, so this is a read-modify-write and the last writer wins. That is the
/// same contract every other facet writer lives under; a task is small and a human
/// clicking a button is not racing anyone.
///
/// The subject is slugged before use, so no path can escape the tasks dimension.
pub async fn patch_task(
    State(state): State<Arc<AppState>>,
    Path(subject): Path<String>,
    Json(patch): Json<TaskPatch>,
) -> Response {
    let subject = facets::slug(&subject);
    if subject.is_empty() {
        return not_found("no such task");
    }
    let mut task = match tasks::read_task(&state.data_dir, &subject).await {
        Ok(Some(task)) => task,
        Ok(None) => return not_found("no such task"),
        Err(e) => return err(&e.to_string()),
    };

    if let Some(v) = &patch.state {
        match parse_state(v) {
            Some(s) => task.state = s,
            None => return err("state must be open, done or dropped"),
        }
    }
    if let Some(v) = &patch.kind {
        match parse_kind(v) {
            Some(k) => task.kind = k,
            None => return err("kind must be wip, serving, watch, deadline or staged"),
        }
    }
    if let Some(v) = &patch.title {
        let title = v.trim();
        if title.is_empty() {
            return err("title must contain a usable character");
        }
        task.title = title.to_owned();
    }

    if let Err(e) = tasks::write_task(&state.data_dir, &task).await {
        // The write refuses a value carrying this machine's absolute data-dir path;
        // that refusal is a message worth passing through verbatim.
        return err(&e.to_string());
    }
    // Rendered from what was just written, and by construction that record is one
    // this module's own parse round-trips — hence `malformed: false`.
    Json(serde_json::json!({ "ok": true, "task": dto(&task, false) })).into_response()
}

/// A uniform JSON error body with a 400.
fn err(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": msg }))).into_response()
}

/// The same body with a 404, for a subject that isn't there.
fn not_found(msg: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": msg }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Scene;
    use chrono::TimeZone;

    #[tokio::test]
    async fn dto_carries_every_field_the_view_contracts_on() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = Task::new("Watch oil prices", TaskKind::Watch);
        t.report_to = Some(Scene("boss".into()));
        t.due = Utc.with_ymd_and_hms(2026, 8, 9, 10, 0, 0).single();
        t.checked = Utc.with_ymd_and_hms(2026, 8, 4, 22, 10, 0).single();
        t.liveness.verify = Some("last row of drive/ledgers/oil.jsonl is under 30m old".into());
        t.body = "Brent, every three hours.".into();
        tasks::write_task(dir.path(), &t).await.unwrap();

        let got = tasks::read_task(dir.path(), "watch-oil-prices").await.unwrap().unwrap();
        let v = serde_json::to_value(dto(&got, false)).unwrap();
        assert_eq!(v["subject"], "watch-oil-prices");
        assert_eq!(v["title"], "Watch oil prices");
        assert_eq!(v["kind"], "watch");
        assert_eq!(v["state"], "open");
        assert_eq!(v["reportTo"], "boss");
        // `Z`, not `+00:00` — one spelling of an instant on the wire.
        assert_eq!(v["due"], "2026-08-09T10:00:00Z");
        assert_eq!(v["checked"], "2026-08-04T22:10:00Z");
        let verify = "last row of drive/ledgers/oil.jsonl is under 30m old";
        assert_eq!(v["liveness"]["verify"], verify);
        assert!(v["liveness"]["restart"].is_null());
        assert_eq!(v["body"], "Brent, every three hours.");
        assert_eq!(v["malformed"], false);
    }

    /// Absent is `null`, never an empty object or an empty string — the view branches
    /// on it.
    #[test]
    fn a_bare_task_renders_nulls_not_empties() {
        let bare = Task::new("Ship the deck", TaskKind::Wip);
        let v = serde_json::to_value(dto(&bare, false)).unwrap();
        assert!(v["liveness"].is_null());
        assert!(v["due"].is_null());
        assert!(v["checked"].is_null());
        assert!(v["reportTo"].is_null());
        assert_eq!(v["body"], "");
    }

    #[test]
    fn open_leads_then_soonest_due_then_subject() {
        let mut closed = Task::new("archive", TaskKind::Wip);
        closed.state = TaskState::Done;
        let mut soon = Task::new("soon", TaskKind::Deadline);
        soon.due = Utc.with_ymd_and_hms(2026, 8, 6, 0, 0, 0).single();
        let mut later = Task::new("later", TaskKind::Deadline);
        later.due = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).single();
        let undated_a = Task::new("aardvark", TaskKind::Wip);
        let undated_z = Task::new("zebra", TaskKind::Wip);

        let mut rows: Vec<(SortKey, String)> = [&closed, &undated_z, &later, &undated_a, &soon]
            .into_iter()
            .map(|t| (sort_key(t), t.subject.clone()))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        let order: Vec<String> = rows.into_iter().map(|(_, s)| s).collect();
        assert_eq!(order, vec!["soon", "later", "aardvark", "zebra", "archive"]);
    }

    /// The reason this endpoint exists rather than a filtered `open_tasks` view: a
    /// record the parser silently rounded off must say so, with whatever did read.
    #[tokio::test]
    async fn a_record_the_parser_rounded_off_is_flagged_not_dropped() {
        let dir = tempfile::tempdir().unwrap();
        facets::update_facet(
            dir.path(),
            tasks::DIMENSION,
            "watch the queue",
            "---\nkind: watch\nstate: Done\ndue: next tuesday\nnote: whatever\n---\n\nWatching.\n",
        )
        .await
        .unwrap();

        let raw = facets::read_facet(dir.path(), tasks::DIMENSION, "watch-the-queue")
            .await
            .unwrap()
            .unwrap();
        let t = tasks::read_task(dir.path(), "watch-the-queue").await.unwrap().unwrap();
        assert!(is_malformed(&raw, &t));

        let v = serde_json::to_value(dto(&t, true)).unwrap();
        assert_eq!(v["kind"], "watch", "what did parse is still reported");
        assert_eq!(v["state"], "open", "keep-biased: `Done` is not a state, so it reads as open");
        assert!(v["due"].is_null());
        assert_eq!(v["body"], "Watching.");
        assert_eq!(v["malformed"], true);

        // A record this module wrote is not flagged — an unknown key alone is fine,
        // and every value round-trips.
        let healthy = Task::new("Ship the deck", TaskKind::Wip);
        tasks::write_task(dir.path(), &healthy).await.unwrap();
        let raw = facets::read_facet(dir.path(), tasks::DIMENSION, "ship-the-deck")
            .await
            .unwrap()
            .unwrap();
        let t = tasks::read_task(dir.path(), "ship-the-deck").await.unwrap().unwrap();
        assert!(!is_malformed(&raw, &t));
    }

    #[test]
    fn an_unreadable_subject_still_lists() {
        let t = unreadable("half-a-slide-deck");
        let v = serde_json::to_value(dto(&t, true)).unwrap();
        assert_eq!(v["subject"], "half-a-slide-deck");
        assert_eq!(v["title"], "half a slide deck");
        assert_eq!(v["state"], "open");
        assert_eq!(v["kind"], "wip");
        assert_eq!(v["malformed"], true);
    }

    /// A typo must not silently re-open the task someone was closing.
    #[test]
    fn patch_values_are_strict_where_the_store_is_forgiving() {
        assert_eq!(parse_state("done"), Some(TaskState::Done));
        assert_eq!(parse_state(" dropped "), Some(TaskState::Dropped));
        assert_eq!(parse_state("Done"), None);
        assert_eq!(parse_state("finished"), None);
        assert_eq!(parse_state(""), None);
        assert_eq!(parse_kind("serving"), Some(TaskKind::Serving));
        assert_eq!(parse_kind("watching"), None);
    }
}
