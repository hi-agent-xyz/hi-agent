//! Task review endpoints.
//!
//! `GET /api/tasks` returns every task across the five lifecycle statuses.
//! `PATCH /api/tasks/{subject}` changes `status` or `title`. Status transitions stamp
//! `completed_at` and `cancelled_at` automatically and clear them when reopened.
//! `GET /api/tasks/{subject}/files/{*path}` serves one file out of a task's own folder.

use std::path::Path as FsPath;
use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::foundation::server::AppState;
use crate::mind::memory::facets;
use crate::mind::memory::media::{content_type, ext_of, resolve_in_root, safe_rel_path};
use crate::mind::memory::snapshot;
use crate::mind::memory::tasks::{self, OnIt, Task, TaskStatus, TimelineEntry};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskDto {
    subject: String,
    title: String,
    status: &'static str,
    created_at: Option<String>,
    /// When the status last changed. **Not "when was this touched"** — a ticket that sat in
    /// `doing` for four days was rewritten six times in five hours while it did so, so the
    /// file's own mtime called it freshly tended right up to the day it was closed by hand.
    /// This is the clock the idle boundary reads, and the panel's only measure of how long a
    /// row has stood where it stands: the timeline dates the move, this says how long ago.
    status_since: Option<String>,
    due_at: Option<String>,
    checked_at: Option<String>,
    completed_at: Option<String>,
    cancelled_at: Option<String>,
    liveness: Option<LivenessDto>,
    /// The running record, oldest first — what was asked, what landed, what is in the
    /// way, what was checked, and every status change. This is what the panel renders;
    /// `body` is the long prose behind it.
    timeline: Vec<MomentDto>,
    body: String,
    /// The artifacts the record itself points at, that are actually on disk beside the
    /// `facet.md` — see [`referenced_files`].
    files: Vec<FileDto>,
    malformed: bool,
    /// Frontmatter this schema does not know, in the file's order — `systems:`, `report_to:`,
    /// and the dated note keys the agent keeps its own ledger in. The store preserves them
    /// because a writer that does not understand a line is not entitled to drop it, and the
    /// same argument reaches here: a person looking at a task cannot read what the record
    /// says if the projection keeps only the twelve keys the code happens to parse.
    ///
    /// **Capped, and the cap is reported.** The board polls this list every few seconds and
    /// one live record carries 144 of these, 85 KB of them. Values are clipped and the tail
    /// is dropped, with [`TaskDto::extra_dropped`] saying how many — a projection that
    /// silently truncated would read as a record with nothing more in it.
    extra: Vec<FieldDto>,
    /// Fields past [`EXTRA_FIELDS`], which are in the record and not in this response.
    extra_dropped: usize,
    /// Who is on this task right now, or `null` where the switchboard says nobody is.
    /// **`null` is not "fine"** — on a `doing` row it is the alarm, and the panel says so
    /// there; on a `todo` or a duty it is the ordinary state and the panel says nothing.
    on_it: Option<OnItDto>,
}

/// One file in the task's own folder, addressed the way the record spells it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileDto {
    /// Verbatim as the record writes it, so the panel can match the token it is about to
    /// render against this list without normalising anything.
    path: String,
    bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MomentDto {
    /// Absent on a hand-written line carrying no instant the store could read. The panel
    /// shows the line without a time rather than guessing one.
    at: Option<String>,
    kind: &'static str,
    text: String,
}

fn moment(entry: &TimelineEntry) -> MomentDto {
    MomentDto {
        at: entry.at.map(rfc3339),
        kind: entry.kind.as_str(),
        text: entry.text.clone(),
    }
}

/// What the switchboard says about this task's worker.
///
/// The **same join the agent's own window reads** — `snapshot::working_on_tasks`, projected
/// through `tasks::worker_note` there and through the panel here. One derivation, because
/// two would be free to disagree and the board's is the one that would go wrong: a restart's
/// casualties are not in the roster, so a board deriving this from `GET /api/workers` would
/// report cut-off work as merely unattended.
///
/// Structured rather than a rendered sentence: the panel writes the words, in the reader's
/// language, and the two locales it already carries are the reason this cannot be prose here.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OnItDto {
    /// `live` — a session is registered under this subject · `reopening` — the last stop cut
    /// its worker off and the host is putting it back, which calls for no move at all ·
    /// `lost` — it would not reopen, so the errand needs starting again or writing off.
    state: &'static str,
    /// The session's slug, so a reader can find it on the roster. `null` off `live`, where
    /// there is no session to name.
    session: Option<String>,
    /// Mid-turn, or waiting for its next instruction.
    busy: Option<bool>,
    /// When it entered that state, RFC3339 — the elapsed figure a reader wants, since uptime
    /// is the same for a session quiet for five minutes and one that just finished a turn.
    since: Option<String>,
    /// The last thing it was seen doing — a shell command, a tool call. **Only while busy**:
    /// nothing clears the field when a turn ends, so drawing it beside an idle session is
    /// what made a finished worker read as `idle` and `thinking` at once.
    doing: Option<String>,
    /// Why its last finished turn died, when the session is quiet enough for that to still be
    /// the news. `idle` after a turn that failed reads as patience, and the move it invites is
    /// "get on with it" rather than "it fell over".
    failed: Option<String>,
    /// The same ending without a fault: the last turn was stopped, by its owner or by a
    /// shutdown. Not an alarm — a decision somebody made.
    stopped: bool,
}

fn on_it(entry: Option<&OnIt>) -> Option<OnItDto> {
    let bare = |state| OnItDto {
        state,
        session: None,
        busy: None,
        since: None,
        doing: None,
        failed: None,
        stopped: false,
    };
    match entry? {
        OnIt::Live(worker) => {
            // Only on a quiet worker: while it is busy, what it is doing now is the answer
            // and last turn's ending is stale.
            let trouble = worker
                .last_turn
                .as_ref()
                .filter(|_| !worker.busy)
                .filter(|outcome| outcome.is_trouble());
            Some(OnItDto {
                state: "live",
                session: Some(worker.session.to_string()),
                busy: Some(worker.busy),
                since: Some(rfc3339(worker.since)),
                doing: worker.busy.then(|| worker.doing.clone()).flatten(),
                failed: trouble.and_then(|outcome| outcome.error().map(str::to_owned)),
                stopped: trouble.is_some_and(|outcome| outcome.error().is_none()),
            })
        }
        OnIt::Reopening => Some(bare("reopening")),
        OnIt::Lost => Some(bare("lost")),
    }
}

/// Most foreign frontmatter fields one task ships, and the most characters of any one value.
/// Both are board-poll budgets, not statements about the record.
const EXTRA_FIELDS: usize = 24;
const EXTRA_VALUE_CHARS: usize = 240;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldDto {
    /// The key as written. Empty for a line the frontmatter carries that is not a `key: value`
    /// at all — kept rather than dropped, for the same reason the store keeps it.
    key: String,
    value: String,
    /// Whether `value` is the whole of what the record says.
    clipped: bool,
}

/// The store's verbatim lines, read back as fields.
///
/// Indented lines continue the field above them, which is how the agent writes a multi-line
/// value; a line that opens no key and continues nothing is its own keyless field. Nothing is
/// re-ordered and nothing is dropped for being unrecognised — the only losses are the two caps,
/// and both are counted.
fn extra_fields(lines: &[String]) -> (Vec<FieldDto>, usize) {
    let mut out: Vec<FieldDto> = Vec::new();
    let mut dropped = 0usize;
    for line in lines {
        let continuation = line.starts_with([' ', '\t']);
        let piece = line.trim();
        if continuation {
            // A blank indented line carries nothing in any reading, and appending it would
            // put a trailing space on the value above.
            if piece.is_empty() {
                continue;
            }
            if let Some(last) = out.last_mut() {
                if push_clipped(&mut last.value, piece) {
                    last.clipped = true;
                }
                continue;
            }
            // An indented line continuing nothing falls through as its own keyless field.
        }
        // A blank line is not a field. Nothing is lost saying so: there is nothing in it.
        if piece.is_empty() {
            continue;
        }
        if out.len() >= EXTRA_FIELDS {
            dropped += 1;
            continue;
        }
        let (key, raw) = match line.split_once(':') {
            Some((key, value)) if !continuation => (key.trim().to_owned(), value.trim()),
            _ => (String::new(), piece),
        };
        let value = unquote(raw);
        let clipped = value.chars().count() > EXTRA_VALUE_CHARS;
        out.push(FieldDto {
            key,
            value: value.chars().take(EXTRA_VALUE_CHARS).collect(),
            clipped,
        });
    }
    (out, dropped)
}

/// Append `piece` to `value`, stopping at the cap. `true` if anything was left behind — the
/// caller's `clipped` flag is the only thing that keeps a cut value from reading as a whole one.
fn push_clipped(value: &mut String, piece: &str) -> bool {
    let room = EXTRA_VALUE_CHARS.saturating_sub(value.chars().count());
    let mut add = String::new();
    if !value.is_empty() {
        add.push(' ');
    }
    add.push_str(piece);
    let taken: String = add.chars().take(room).collect();
    let truncated = taken.chars().count() < add.chars().count();
    value.push_str(&taken);
    truncated
}

/// A frontmatter value as the writer meant it: [`facets`] JSON-quotes anything with a colon
/// or a newline in it, and a panel showing the escapes would be showing the encoding.
fn unquote(value: &str) -> String {
    if value.starts_with('"')
        && let Ok(decoded) = serde_json::from_str::<String>(value)
    {
        return decoded;
    }
    value.to_owned()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LivenessDto {
    verify: Option<String>,
    restart: Option<String>,
    owner: Option<String>,
    start_key: Option<String>,
}

fn rfc3339(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn dto(task: &Task, malformed: bool, on_it: Option<&OnIt>, files: Vec<FileDto>) -> TaskDto {
    let (extra, extra_dropped) = extra_fields(&task.extra);
    TaskDto {
        subject: task.subject.clone(),
        title: task.title.clone(),
        status: task.status.as_str(),
        created_at: task.created_at.map(rfc3339),
        status_since: task.status_since.map(rfc3339),
        due_at: task.due_at.map(rfc3339),
        checked_at: task.checked_at.map(rfc3339),
        completed_at: task.completed_at.map(rfc3339),
        cancelled_at: task.cancelled_at.map(rfc3339),
        liveness: if task.liveness.is_empty() {
            None
        } else {
            Some(LivenessDto {
                verify: task.liveness.verify.clone(),
                restart: task.liveness.restart.clone(),
                owner: task.liveness.owner.clone(),
                start_key: task.liveness.start_key.clone(),
            })
        },
        timeline: task.timeline.iter().map(moment).collect(),
        body: task.body.clone(),
        files,
        malformed,
        extra,
        extra_dropped,
        on_it: self::on_it(on_it),
    }
}

/// The files a task's own record points at, that are on disk in its folder.
///
/// **Not a listing of the folder, deliberately.** A task folder is where the work
/// happened, not a shelf of deliverables: one live store holds 39,946 files under
/// `tasks/` — cloned repos, `__pycache__`, scraped HTML — and a single task's *top level*
/// holds 114. Listing that is showing somebody the workshop floor when they asked what
/// was made. What they came back for is the file the account names — *"the completed
/// report is `inspection-report.md` in this task directory"* — and until that sentence is
/// reachable, the panel is pointing at something the reader cannot open.
///
/// So the record stays the authority and this only makes its own references resolvable:
/// every inline-code token the prose or the timeline spells, kept when a regular file of
/// that name is really there. A record naming a file it never wrote lists nothing; a file
/// nobody wrote down stays where it is, which is the same rule the ledger runs on
/// everywhere else — two listings would mean one of them is wrong and no way to tell
/// which.
async fn referenced_files(data_dir: &FsPath, task: &Task) -> Vec<FileDto> {
    let mut candidates: Vec<String> = Vec::new();
    code_spans(&task.body, &mut candidates);
    for entry in &task.timeline {
        code_spans(&entry.text, &mut candidates);
    }
    candidates.retain(|token| names_a_file(token));
    candidates.sort();
    candidates.dedup();
    // A body is a few KB of prose and carries a handful of these; the cap is a backstop
    // against a record that pasted a directory listing into itself, not a policy. It
    // bounds the stats per task, and what it drops is reported in the log rather than
    // silently vanishing from the panel.
    const MAX_CANDIDATES: usize = 32;
    if candidates.len() > MAX_CANDIDATES {
        tracing::debug!(
            subject = %task.subject,
            named = candidates.len(),
            "task names more files than the panel resolves; keeping the first {MAX_CANDIDATES}"
        );
        candidates.truncate(MAX_CANDIDATES);
    }

    let dir = facets::subject_dir(data_dir, tasks::DIMENSION, &task.subject);
    let mut files = Vec::new();
    for path in candidates {
        let Some(full) = resolve_in_root(&dir, &path).await else {
            continue;
        };
        let bytes = tokio::fs::metadata(&full).await.map_or(0, |meta| meta.len());
        files.push(FileDto { path, bytes });
    }
    files
}

/// Every ``` `…` ``` span in `text`, appended to `out`. Markdown's inline code is what
/// both prompts tell every writer to spell a filename in, and it is the only marker in
/// these bodies that means "this is a name, not a word".
fn code_spans(text: &str, out: &mut Vec<String>) {
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else {
            return;
        };
        out.push(after[..close].to_owned());
        rest = &after[close + 1..];
    }
}

/// Whether a code span could be a file in the task's folder — before asking the disk.
///
/// These bodies are mostly made of things spelled the same way that are not files:
/// `status_since`, `hi_say`, a SHA-256, a shell line. Requiring an extension and rejecting
/// whitespace keeps the stat count down; `facet.md` is excluded because it is the panel
/// the reader is already looking at. Everything past this is decided by whether the file
/// is actually there.
fn names_a_file(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 200
        && !token.contains(char::is_whitespace)
        && safe_rel_path(token)
        && !ext_of(token).is_empty()
        && token != facets::FACET_FILE
        && !token.split('/').any(|seg| seg.starts_with('.'))
}

/// Todo, doing, serving, done, cancelled. Work uses due order; duties put the ones least
/// recently confirmed alive on top, never-confirmed first; closed tasks use newest closing
/// timestamp first.
type SortKey = (u8, u8, i64, String);

fn sort_key(task: &Task) -> SortKey {
    match task.status {
        TaskStatus::Todo | TaskStatus::Doing => (
            if task.status == TaskStatus::Todo { 0 } else { 1 },
            if task.due_at.is_some() { 0 } else { 1 },
            task.due_at.map_or(0, |due| due.timestamp()),
            task.subject.clone(),
        ),
        TaskStatus::Serving => (
            2,
            if task.checked_at.is_some() { 1 } else { 0 },
            task.checked_at.map_or(0, |at| at.timestamp()),
            task.subject.clone(),
        ),
        TaskStatus::Done => (
            3,
            if task.completed_at.is_some() { 0 } else { 1 },
            task.completed_at.map_or(0, |at| -at.timestamp()),
            task.subject.clone(),
        ),
        TaskStatus::Cancelled => (
            4,
            if task.cancelled_at.is_some() { 0 } else { 1 },
            task.cancelled_at.map_or(0, |at| -at.timestamp()),
            task.subject.clone(),
        ),
    }
}

fn unreadable(subject: &str) -> Task {
    let mut task = Task::new(subject, TaskStatus::Todo);
    task.title = subject.replace('-', " ");
    task.created_at = None;
    task
}

fn raw_field(content: &str, key: &str) -> Option<String> {
    let frontmatter = content.strip_prefix("---\n")?;
    let block = &frontmatter[..frontmatter.find("\n---\n")?];
    for line in block.lines() {
        let Some((candidate, value)) = line.split_once(':') else {
            continue;
        };
        if candidate.trim() != key {
            continue;
        }
        return Some(unquote(value.trim()));
    }
    None
}

fn is_malformed(raw: &str, task: &Task) -> bool {
    let field = |key: &str| {
        raw_field(raw, key)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };

    let status_bad = match field("status") {
        // A record written before `serving` existed says `doing` and reads back as
        // `serving`. The reader correcting it is not a defect in the record, and flagging
        // it would put an "invalid fields" warning on every duty the agent ever opened.
        Some(value) if value == "doing" && task.status == TaskStatus::Serving => false,
        Some(value) => value != task.status.as_str(),
        None => {
            field("state").is_some_and(|value| !matches!(value.as_str(), "open" | "done" | "dropped"))
                || field("kind").is_some_and(|value| {
                    !matches!(
                        value.as_str(),
                        "wip" | "serving" | "watch" | "deadline" | "staged"
                    )
                })
        }
    };

    let invalid_timestamp = |new_key: &str, legacy_key: Option<&str>, parsed: bool| {
        let present = field(new_key).is_some()
            || legacy_key.is_some_and(|key| field(key).is_some());
        present && !parsed
    };

    status_bad
        || invalid_timestamp("created_at", None, task.created_at.is_some())
        || invalid_timestamp("due_at", Some("due"), task.due_at.is_some())
        || invalid_timestamp("checked_at", Some("checked"), task.checked_at.is_some())
        || invalid_timestamp("completed_at", None, task.completed_at.is_some())
        || invalid_timestamp("cancelled_at", None, task.cancelled_at.is_some())
        || (task.completed_at.is_some() && task.status != TaskStatus::Done)
        || (task.cancelled_at.is_some() && task.status != TaskStatus::Cancelled)
}

pub async fn get_tasks(State(state): State<Arc<AppState>>) -> Response {
    let dir = &state.data_dir;
    let index = match facets::facet_subject_index(dir).await {
        Ok(index) => index,
        Err(error) => return err(&error.to_string()),
    };
    let prefix = format!("{}/", tasks::DIMENSION);
    // Once for the whole board, not once per row: it walks the switchboard, and the answer
    // has to be the same for every card in one response.
    let working = snapshot::working_on_tasks();

    let mut rows: Vec<(SortKey, TaskDto)> = Vec::new();
    for facet_ref in index {
        let Some(subject) = facet_ref.strip_prefix(prefix.as_str()) else {
            continue;
        };
        let parsed = tasks::read_task(dir, subject).await;
        let raw = facets::read_facet(dir, tasks::DIMENSION, subject).await;
        let (task, malformed) = match (parsed, raw) {
            (Ok(Some(task)), Ok(Some(raw))) => {
                let malformed = is_malformed(&raw, &task);
                (task, malformed)
            }
            _ => (unreadable(subject), true),
        };
        let on_it = working.get(&task.subject);
        let files = referenced_files(dir, &task).await;
        rows.push((sort_key(&task), dto(&task, malformed, on_it, files)));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let tasks: Vec<TaskDto> = rows.into_iter().map(|(_, task)| task).collect();
    Json(serde_json::json!({ "tasks": tasks })).into_response()
}

#[derive(Deserialize)]
pub struct TaskPatch {
    status: Option<String>,
    title: Option<String>,
}

fn parse_status(value: &str) -> Option<TaskStatus> {
    match value.trim() {
        "todo" => Some(TaskStatus::Todo),
        "doing" => Some(TaskStatus::Doing),
        "serving" => Some(TaskStatus::Serving),
        "done" => Some(TaskStatus::Done),
        "cancelled" => Some(TaskStatus::Cancelled),
        _ => None,
    }
}

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
        Err(error) => return err(&error.to_string()),
    };

    if let Some(value) = &patch.status {
        match parse_status(value) {
            Some(status) => task.set_status(status, Utc::now()),
            None => {
                return err("status must be todo, doing, serving, done or cancelled");
            }
        }
    }
    if let Some(value) = &patch.title {
        let title = value.trim();
        if title.is_empty() {
            return err("title must contain a usable character");
        }
        task.title = title.to_owned();
    }

    if let Err(error) = tasks::write_task(&state.data_dir, &task).await {
        return err(&error.to_string());
    }
    let working = snapshot::working_on_tasks();
    let on_it = working.get(&task.subject);
    let files = referenced_files(&state.data_dir, &task).await;
    Json(serde_json::json!({ "ok": true, "task": dto(&task, false, on_it, files) })).into_response()
}

/// One file out of a task's own folder, by the path the record spells.
///
/// Guarded exactly as `drive/` is, and for the same reason: every segment of both the
/// subject and the path came from an agent. [`facets::slug`] settles the subject,
/// [`resolve_in_root`] settles the rest — a syntactic pass that stops `..`, then a
/// canonicalised containment check that also defeats a symlink inside the folder pointing
/// at somebody's `~/.ssh`.
///
/// Read-only. A task's folder is written by the sessions doing the work, and a write verb
/// here would be a second writer on files two of them are already sharing.
pub async fn get_task_file(
    State(state): State<Arc<AppState>>,
    Path((subject, path)): Path<(String, String)>,
) -> Response {
    let subject = facets::slug(&subject);
    if subject.is_empty() {
        return not_found("no such task");
    }
    let dir = facets::subject_dir(&state.data_dir, tasks::DIMENSION, &subject);
    let Some(full) = resolve_in_root(&dir, &path).await else {
        return not_found("no such file");
    };
    let Ok(bytes) = tokio::fs::read(&full).await else {
        return not_found("no such file");
    };
    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type(&path)));
    // The deliverable in a task folder is advanced in place — `general.md` asks a worker
    // to keep one file that is always the current best version — so a cached copy is a
    // reader looking at an older draft with nothing to tell them so.
    resp.headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

fn err(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": message })),
    )
        .into_response()
}

fn not_found(message: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": message })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
        use chrono::TimeZone;

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, day, hour, 0, 0)
            .single()
            .unwrap()
    }

    #[tokio::test]
    async fn dto_carries_status_and_lifecycle_timestamps() {
        let dir = tempfile::tempdir().unwrap();
        let mut task = Task::new("Watch oil prices", TaskStatus::Serving);
        task.created_at = Some(at(1, 9));
        task.status_since = Some(at(2, 9));
        task.due_at = Some(at(9, 10));
        task.checked_at = Some(at(4, 22));
        task.liveness.verify =
            Some("last row of drive/ledgers/oil.jsonl is under 30m old".into());
        task.body = "Brent, every three hours.".into();
        tasks::write_task(dir.path(), &task).await.unwrap();

        let got = tasks::read_task(dir.path(), "watch-oil-prices")
            .await
            .unwrap()
            .unwrap();
        let value = serde_json::to_value(dto(&got, false, None, Vec::new())).unwrap();
        assert_eq!(value["status"], "serving");
        assert_eq!(value["createdAt"], "2026-08-01T09:00:00Z");
        assert_eq!(value["statusSince"], "2026-08-02T09:00:00Z");
        assert_eq!(value["dueAt"], "2026-08-09T10:00:00Z");
        assert_eq!(value["checkedAt"], "2026-08-04T22:00:00Z");
        assert!(value["completedAt"].is_null());
        assert!(value["cancelledAt"].is_null());
        assert!(value.get("kind").is_none());
        assert!(value.get("state").is_none());
    }

    /// The seam the panel is built on: prose and running record reach it as two things,
    /// so the record renders as dated lines and the account as the prose above them.
    #[tokio::test]
    async fn dto_carries_the_running_record_apart_from_the_prose() {
        let dir = tempfile::tempdir().unwrap();
        let mut task = Task::new("Daily ops digest", TaskStatus::Doing);
        task.body = "The long account, written whole.".into();
        task.timeline = vec![
            TimelineEntry::new(
                tasks::TimelineKind::Asked,
                at(1, 9),
                "it goes to the Feishu group, not to me",
            ),
            TimelineEntry::new(tasks::TimelineKind::Blocked, at(2, 11), "no im:chat scope"),
        ];
        tasks::write_task(dir.path(), &task).await.unwrap();

        let got = tasks::read_task(dir.path(), "daily-ops-digest")
            .await
            .unwrap()
            .unwrap();
        let value = serde_json::to_value(dto(&got, false, None, Vec::new())).unwrap();
        assert_eq!(value["body"], "The long account, written whole.");
        assert_eq!(value["timeline"][0]["kind"], "asked");
        assert_eq!(value["timeline"][0]["at"], "2026-08-01T09:00:00Z");
        assert_eq!(
            value["timeline"][0]["text"],
            "it goes to the Feishu group, not to me"
        );
        assert_eq!(value["timeline"][1]["kind"], "blocked");
    }

    /// A task nobody has recorded anything about serves an empty list, never a null — the
    /// panel maps over it, and an empty board is the ordinary state of a fresh ledger.
    #[test]
    fn a_task_with_no_running_record_serves_an_empty_list() {
        let task = Task::new("Ship the deck", TaskStatus::Todo);
        let value = serde_json::to_value(dto(&task, false, None, Vec::new())).unwrap();
        assert_eq!(value["timeline"], serde_json::json!([]));
    }

    /// A live worker reaches the card as structure, not as a sentence — the panel has two
    /// locales and the words are its own.
    #[test]
    fn a_live_worker_reaches_the_card() {
        let task = Task::new("Ship the deck", TaskStatus::Doing);
        let entry = OnIt::Live(tasks::WorkingOnIt {
            session: "worker-3".parse().unwrap(),
            busy: true,
            doing: Some("cargo test -p hi-agent".into()),
            since: at(3, 9),
            last_turn: None,
        });
        let value = serde_json::to_value(dto(&task, false, Some(&entry), Vec::new())).unwrap();
        assert_eq!(value["onIt"]["state"], "live");
        assert_eq!(value["onIt"]["session"], "worker-3");
        assert_eq!(value["onIt"]["busy"], true);
        assert_eq!(value["onIt"]["since"], "2026-08-03T09:00:00Z");
        assert_eq!(value["onIt"]["doing"], "cargo test -p hi-agent");
    }

    /// `doing` is only meaningful while the session is running — nothing clears it when a
    /// turn ends, so a quiet worker carrying the last command it ran would read as busy.
    /// A turn that died is the opposite: on a quiet worker that ending *is* the news.
    #[test]
    fn a_quiet_worker_shows_how_its_turn_ended_and_not_what_it_last_ran() {
        let task = Task::new("Ship the deck", TaskStatus::Doing);
        let entry = OnIt::Live(tasks::WorkingOnIt {
            session: "worker-3".parse().unwrap(),
            busy: false,
            doing: Some("cargo test -p hi-agent".into()),
            since: at(3, 9),
            last_turn: Some(crate::foundation::registry::TurnOutcome::Failed(
                "stream closed".into(),
            )),
        });
        let value = serde_json::to_value(dto(&task, false, Some(&entry), Vec::new())).unwrap();
        assert!(value["onIt"]["doing"].is_null());
        assert_eq!(value["onIt"]["failed"], "stream closed");
        assert_eq!(value["onIt"]["stopped"], false);
    }

    /// While it is busy, what it is doing now is the answer and last turn's ending is stale.
    #[test]
    fn a_busy_worker_does_not_report_last_turns_ending() {
        let task = Task::new("Ship the deck", TaskStatus::Doing);
        let entry = OnIt::Live(tasks::WorkingOnIt {
            session: "worker-3".parse().unwrap(),
            busy: true,
            doing: None,
            since: at(3, 9),
            last_turn: Some(crate::foundation::registry::TurnOutcome::Interrupted),
        });
        let value = serde_json::to_value(dto(&task, false, Some(&entry), Vec::new())).unwrap();
        assert!(value["onIt"]["failed"].is_null());
        assert_eq!(value["onIt"]["stopped"], false);
    }

    /// The two the restart leaves behind, which the roster cannot answer at all: one is
    /// coming back on its own and needs no move, the other needs somebody put on it.
    #[test]
    fn the_restarts_casualties_are_distinguishable_from_each_other() {
        let task = Task::new("Ship the deck", TaskStatus::Doing);
        let reopening = serde_json::to_value(dto(&task, false, Some(&OnIt::Reopening), Vec::new())).unwrap();
        let lost = serde_json::to_value(dto(&task, false, Some(&OnIt::Lost), Vec::new())).unwrap();
        assert_eq!(reopening["onIt"]["state"], "reopening");
        assert!(reopening["onIt"]["session"].is_null());
        assert_eq!(lost["onIt"]["state"], "lost");
    }

    /// Nobody on it is `null`, not an object saying so. What that absence *means* depends on
    /// the status — an alarm on `doing`, the ordinary state on a `todo` — and that judgment
    /// is the panel's, which is the only place that knows what it is drawing.
    #[test]
    fn nobody_on_it_is_an_absence() {
        let task = Task::new("Ship the deck", TaskStatus::Doing);
        let value = serde_json::to_value(dto(&task, false, None, Vec::new())).unwrap();
        assert!(value["onIt"].is_null());
    }

    /// A record with no `status_since` of its own reads back as its creation instant — the
    /// store's fallback, which is older and therefore errs toward the idle boundary rather
    /// than hiding behind it. The panel inherits that rather than reinventing it.
    #[tokio::test]
    async fn a_record_with_no_status_stamp_falls_back_to_when_it_was_opened() {
        let dir = tempfile::tempdir().unwrap();
        facets::update_facet(
            dir.path(),
            tasks::DIMENSION,
            "old-row",
            "---\nstatus: doing\ntitle: \"Old row\"\ncreated_at: \"2026-08-01T09:00:00Z\"\n---\n",
        )
        .await
        .unwrap();
        let task = tasks::read_task(dir.path(), "old-row").await.unwrap().unwrap();
        let value = serde_json::to_value(dto(&task, false, None, Vec::new())).unwrap();
        assert_eq!(value["statusSince"], "2026-08-01T09:00:00Z");
    }

    #[test]
    fn bare_task_has_no_due_or_liveness_metadata() {
        let task = Task::new("Ship the deck", TaskStatus::Todo);
        let value = serde_json::to_value(dto(&task, false, None, Vec::new())).unwrap();
        assert!(value["dueAt"].is_null());
        assert!(value["checkedAt"].is_null());
        assert!(value["liveness"].is_null());
    }

    /// The ledger a real task carries is mostly not schema — `systems:` on 78 of one live
    /// store's 108 records, `report_to:` on 10 — and a panel that showed only the keys the
    /// parser knows was showing a fraction of the record it claimed to be.
    #[test]
    fn foreign_frontmatter_reaches_the_panel() {
        let mut task = Task::new("Deploy KUT", TaskStatus::Doing);
        task.extra = vec![
            "systems: KUT, gz, hi-agent".into(),
            "report_to: prdo8qht".into(),
        ];
        let value = serde_json::to_value(dto(&task, false, None, Vec::new())).unwrap();
        assert_eq!(value["extra"][0]["key"], "systems");
        assert_eq!(value["extra"][0]["value"], "KUT, gz, hi-agent");
        assert_eq!(value["extra"][0]["clipped"], false);
        assert_eq!(value["extra"][1]["key"], "report_to");
        assert_eq!(value["extraDropped"], 0);
    }

    /// A quoted value is shown as the writer meant it. The store quotes anything carrying a
    /// colon, and a panel rendering the escapes would be rendering the encoding.
    #[test]
    fn a_quoted_value_is_shown_unquoted() {
        let mut task = Task::new("Deploy KUT", TaskStatus::Doing);
        task.extra = vec![r#"note: "16:20 — the callback is still not registered""#.into()];
        let value = serde_json::to_value(dto(&task, false, None, Vec::new())).unwrap();
        assert_eq!(
            value["extra"][0]["value"],
            "16:20 — the callback is still not registered"
        );
    }

    /// Indented lines continue the field above, which is how a multi-line value is written.
    /// A line continuing nothing keeps its place as a keyless field rather than vanishing.
    #[test]
    fn an_indented_line_continues_the_field_above_it() {
        let mut task = Task::new("Deploy KUT", TaskStatus::Doing);
        task.extra = vec![
            "note: first".into(),
            "  second".into(),
            "  third".into(),
            "   ".into(),
        ];
        let value = serde_json::to_value(dto(&task, false, None, Vec::new())).unwrap();
        assert_eq!(value["extra"][0]["value"], "first second third");
        assert_eq!(value["extra"].as_array().unwrap().len(), 1);
    }

    /// The board polls this list; one live record carries 147 foreign keys running to 85 KB.
    /// Both caps report what they cut, because a truncation that says nothing reads as a
    /// record with nothing more in it.
    #[test]
    fn the_caps_say_what_they_left_out() {
        let mut task = Task::new("Watch the group", TaskStatus::Serving);
        task.extra = (0..EXTRA_FIELDS + 6)
            .map(|i| format!("CHECK_{i}: still up"))
            .chain(std::iter::once(format!("long: {}", "x".repeat(400))))
            .collect();
        let value = serde_json::to_value(dto(&task, false, None, Vec::new())).unwrap();
        assert_eq!(value["extra"].as_array().unwrap().len(), EXTRA_FIELDS);
        assert_eq!(value["extraDropped"], 7);

        let mut one = Task::new("Watch the group", TaskStatus::Serving);
        one.extra = vec![format!("long: {}", "x".repeat(400))];
        let value = serde_json::to_value(dto(&one, false, None, Vec::new())).unwrap();
        assert_eq!(
            value["extra"][0]["value"].as_str().unwrap().chars().count(),
            EXTRA_VALUE_CHARS
        );
        assert_eq!(value["extra"][0]["clipped"], true);
    }

    /// What the parser understands is not foreign frontmatter, and must not be listed twice.
    #[tokio::test]
    async fn schema_keys_are_not_repeated_as_foreign_fields() {
        let dir = tempfile::tempdir().unwrap();
        facets::update_facet(
            dir.path(),
            tasks::DIMENSION,
            "kut",
            "---\nstatus: doing\ntitle: \"Deploy KUT\"\nsystems: KUT, gz\n---\n",
        )
        .await
        .unwrap();
        let task = tasks::read_task(dir.path(), "kut").await.unwrap().unwrap();
        let value = serde_json::to_value(dto(&task, false, None, Vec::new())).unwrap();
        let keys: Vec<&str> = value["extra"]
            .as_array()
            .unwrap()
            .iter()
            .map(|field| field["key"].as_str().unwrap())
            .collect();
        assert_eq!(keys, vec!["systems"]);
    }

    /// The whole point of resolving against disk: a record names things that are spelled
    /// like files and are not, and the reader must not be handed a link to a 404.
    #[tokio::test]
    async fn only_the_named_files_that_exist_come_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut task = Task::new("Inspect gz-02 /data disk usage", TaskStatus::Done);
        task.body = "The completed report is `inspection-report.md` in this task \
             directory. `hi_say` carried the headline; `status_since` moved with it, and \
             the draft `never-written.md` was abandoned."
            .into();
        task.timeline = vec![TimelineEntry::new(
            tasks::TimelineKind::Landed,
            at(25, 6),
            "`notes/working.md` has the sampling method",
        )];
        tasks::write_task(dir.path(), &task).await.unwrap();

        let folder = facets::subject_dir(dir.path(), tasks::DIMENSION, &task.subject);
        tokio::fs::write(folder.join("inspection-report.md"), "# report")
            .await
            .unwrap();
        tokio::fs::create_dir_all(folder.join("notes")).await.unwrap();
        tokio::fs::write(folder.join("notes/working.md"), "how")
            .await
            .unwrap();

        let files = referenced_files(dir.path(), &task).await;
        let named: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(named, vec!["inspection-report.md", "notes/working.md"]);
        assert_eq!(files[0].bytes, 8);
    }

    /// `facet.md` is the panel the reader is already looking at, and the folder's own
    /// history is not an artifact of the work.
    #[test]
    fn the_record_itself_is_never_offered_as_one_of_its_artifacts() {
        assert!(!names_a_file("facet.md"));
        assert!(!names_a_file(".history/facet.md"));
        assert!(!names_a_file("../../config.db"), "no climbing out of the folder");
        assert!(!names_a_file("hi_say"), "no extension, so not a filename");
        assert!(!names_a_file("ls -la /data"), "a shell line is not a path");
        assert!(names_a_file("inspection-report.md"));
        assert!(names_a_file("evidence/p95.json"));
    }

    #[test]
    fn closed_tasks_sort_by_recent_closing_time() {
        let mut older = Task::new("older", TaskStatus::Done);
        older.completed_at = Some(at(5, 9));
        let mut recent = Task::new("recent", TaskStatus::Done);
        recent.completed_at = Some(at(7, 9));
        let todo = Task::new("todo", TaskStatus::Todo);

        let mut rows = [&older, &recent, &todo]
            .into_iter()
            .map(|task| (sort_key(task), task.subject.clone()))
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            rows.into_iter().map(|(_, subject)| subject).collect::<Vec<_>>(),
            vec!["todo", "recent", "older"]
        );
    }

    /// Within the serving column the top card is the one whose health is least established,
    /// so a duty that has gone quiet cannot hide under one confirmed a minute ago.
    #[test]
    fn duties_sort_by_how_long_since_confirmed_alive() {
        let silent = Task::new("silent", TaskStatus::Serving);
        let mut stale = Task::new("stale", TaskStatus::Serving);
        stale.checked_at = Some(at(2, 9));
        let mut fresh = Task::new("fresh", TaskStatus::Serving);
        fresh.checked_at = Some(at(8, 9));
        let doing = Task::new("doing", TaskStatus::Doing);
        let done = Task::new("done", TaskStatus::Done);

        let mut rows = [&fresh, &done, &silent, &doing, &stale]
            .into_iter()
            .map(|task| (sort_key(task), task.subject.clone()))
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            rows.into_iter().map(|(_, subject)| subject).collect::<Vec<_>>(),
            vec!["doing", "silent", "stale", "fresh", "done"]
        );
    }

    /// The board must not paint "invalid fields" across every duty the agent opened before
    /// `serving` existed — the reader corrected those records, it did not find them broken.
    #[tokio::test]
    async fn a_duty_predating_serving_is_corrected_not_flagged() {
        let dir = tempfile::tempdir().unwrap();
        facets::update_facet(
            dir.path(),
            tasks::DIMENSION,
            "watch-the-ops-group",
            "---\nstatus: doing\ntitle: \"Watch the ops group\"\nverify: \"a row landed today\"\n\
             restart: \"launchctl kickstart the label\"\n---\n",
        )
        .await
        .unwrap();
        let raw = facets::read_facet(dir.path(), tasks::DIMENSION, "watch-the-ops-group")
            .await
            .unwrap()
            .unwrap();
        let task = tasks::read_task(dir.path(), "watch-the-ops-group")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(task.status, TaskStatus::Serving);
        assert!(!is_malformed(&raw, &task));
    }

    #[tokio::test]
    async fn malformed_current_and_legacy_records_are_flagged() {
        let dir = tempfile::tempdir().unwrap();
        facets::update_facet(
            dir.path(),
            tasks::DIMENSION,
            "bad-current",
            "---\nstatus: Doing\ncreated_at: yesterday\n---\n",
        )
        .await
        .unwrap();
        let raw = facets::read_facet(dir.path(), tasks::DIMENSION, "bad-current")
            .await
            .unwrap()
            .unwrap();
        let task = tasks::read_task(dir.path(), "bad-current")
            .await
            .unwrap()
            .unwrap();
        assert!(is_malformed(&raw, &task));

        facets::update_facet(
            dir.path(),
            tasks::DIMENSION,
            "bad-legacy",
            "---\nkind: watching\nstate: Open\n---\n",
        )
        .await
        .unwrap();
        let raw = facets::read_facet(dir.path(), tasks::DIMENSION, "bad-legacy")
            .await
            .unwrap()
            .unwrap();
        let task = tasks::read_task(dir.path(), "bad-legacy")
            .await
            .unwrap()
            .unwrap();
        assert!(is_malformed(&raw, &task));
    }

    #[test]
    fn patch_status_values_are_strict() {
        assert_eq!(parse_status("todo"), Some(TaskStatus::Todo));
        assert_eq!(parse_status(" doing "), Some(TaskStatus::Doing));
        assert_eq!(parse_status("serving"), Some(TaskStatus::Serving));
        assert_eq!(parse_status("cancelled"), Some(TaskStatus::Cancelled));
        assert_eq!(parse_status("open"), None);
        assert_eq!(parse_status("watch"), None);
        assert_eq!(parse_status("Done"), None);
    }
}
