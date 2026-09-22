//! Task review endpoints.
//!
//! **Two shapes, and the split is the whole point.** `GET /api/tasks` is a *list*: every task
//! across the five lifecycle statuses, carrying only what a row draws. `GET /api/tasks/{subject}`
//! is the *record*: one task with its prose, its whole timeline, its files and its frontmatter.
//!
//! They were one endpoint, and the list is what a board polls. On the live store that meant
//! 2.0 MB every few seconds — 166 records' bodies and timelines, 95% of them closed work
//! nothing on screen renders more than a title and a date for. A list that ships the record
//! is a list that grows without bound with the history behind it, which is the one thing a
//! polled read must not do: the panel reads one record when a row is opened, so the cost of
//! the account is paid by the person who asked to see it.
//!
//! `PATCH /api/tasks/{subject}` changes `status` or `title`. Status transitions stamp
//! `completed_at` and `cancelled_at` automatically and clear them when reopened.
//! `GET /api/tasks/{subject}/files/{*path}` serves one file out of a task's own folder.
//!
//! **A reply typed on a row is not one of these routes.** It is something a person said, so it
//! arrives where everything they say arrives — `POST /api/in/text?task=<subject>` — and this
//! module supplies only the two halves that are about the row: which row it names
//! ([`said_on`]), and the `replied` line the row keeps ([`record_said_on`]).

use std::path::Path as FsPath;
use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::foundation::attachments::{self, Probe};
use crate::foundation::server::{AppState, stores};
use crate::mind::memory::facets;
use crate::mind::memory::media::{content_type, ext_of, resolve_in_root, safe_rel_path};
use crate::mind::memory::tasks::{self, Task, TaskStatus, TimelineEntry, TimelineKind};
use crate::types::{Content, Message, TaskRef};

/// One task, whole — what `GET /api/tasks/{subject}` answers with.
///
/// Every field a row carries ([`RowDto`]) is here under the same name, so a panel handed the
/// record never has to fall back to the row it was opened from.
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
    /// The running record, oldest first — why the row exists, what happened, what was
    /// delivered, who is being waited on, and every status change. **It is the whole of
    /// what this row says**: the prose that used to sit above it went with the account.
    timeline: Vec<MomentDto>,
    /// The same moment [`RowDto::latest`] carries, computed once here rather than derived a
    /// second time from `timeline` by whoever is drawing — two derivations of "the newest
    /// thing a mind said" is two things to keep agreeing forever.
    latest: Option<MomentDto>,
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
}

/// One row of the board — what `GET /api/tasks` answers with, once per task.
///
/// **What is missing from it is the design.** No `body`, no `timeline`, no `files`: those are
/// the record, and a row is not the record. What a card, a ledger line and the home chart
/// actually read is the identity, the clocks, and one sentence — so the sentence ([`latest`])
/// is computed here and the account stays behind the click that asks for it.
///
/// `extra` survives because the chart reads `project:`/`systems:` off it to decide what a row
/// is about, and it is already capped at [`EXTRA_FIELDS`] × [`EXTRA_VALUE_CHARS`] — 32 KB
/// across one live store's whole ledger, against 1.8 MB for the bodies and timelines beside it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RowDto {
    subject: String,
    title: String,
    status: &'static str,
    created_at: Option<String>,
    status_since: Option<String>,
    due_at: Option<String>,
    checked_at: Option<String>,
    completed_at: Option<String>,
    cancelled_at: Option<String>,
    liveness: Option<LivenessDto>,
    /// The newest line anybody *said* on the row — see [`latest_moment`]. This is the one line
    /// a card prints and the only thing that answers whether a person is being waited on.
    latest: Option<MomentDto>,
    /// What the row's lines placed, newest first and at most [`ROW_SHOWN`]: the views it made
    /// and the attachments its lines carried — see [`shown`]. Home draws its tiles from this
    /// and reads nothing else for them.
    attached: Vec<ShownDto>,
    malformed: bool,
    extra: Vec<FieldDto>,
    extra_dropped: usize,
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
    /// What the line says — without the store's `⟨attached …⟩` marker, which is drawn as
    /// what it names rather than read as words.
    text: String,
    /// What the line carries: the attachments it was written with, or the view a `made`
    /// line names. The panel draws them under the sentence they are evidence for.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attached: Vec<ShownDto>,
}

/// One thing a line carries, as a surface draws it (`docs/arch/showing.md`).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShownDto {
    /// `att:<id>` or `view:<ref>` — the one way either is named everywhere.
    #[serde(rename = "ref")]
    reff: String,
    /// `picture`, `clip` or `view`.
    kind: &'static str,
    /// The tile's picture. Absent for a view nobody has taken a picture of yet, which a tile
    /// draws as its name until one lands ([`super::view::warm_for_rows`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    preview: Option<String>,
    /// An attachment's own bytes, for the viewer. A view has none: it is opened by its ref.
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    /// A view's name as the band labels it.
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
    /// On a row: when the line that placed it was written, and what kind of line it was —
    /// which line a tile opens the panel at, and whether the picture was a delivery, the work
    /// going on, or something the person is asked to judge.
    #[serde(skip_serializing_if = "Option::is_none")]
    at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<&'static str>,
}

/// Most things a row carries: what Home hangs under a card. A task that attaches forty
/// figures hangs its newest six; the panel has the rest, under the lines that carried them.
const ROW_SHOWN: usize = 6;

/// What drawing a row or a record needs besides the record: which views are on disk to be
/// opened, and what each attachment its lines carry is. Loaded once per task, so building a
/// row reads each attachment's probe at most once — and the store keeps them in memory, so
/// a ledger polled every few seconds reads each off disk once for the life of the process.
struct Showing<'a> {
    data_dir: &'a FsPath,
    views: &'a std::collections::HashSet<String>,
    probes: std::collections::HashMap<String, Probe>,
}

impl<'a> Showing<'a> {
    async fn load(
        data_dir: &'a FsPath,
        views: &'a std::collections::HashSet<String>,
        task: &Task,
    ) -> Showing<'a> {
        let mut probes = std::collections::HashMap::new();
        for entry in &task.timeline {
            for id in entry.attached() {
                if probes.contains_key(id) {
                    continue;
                }
                if let Some(probe) = attachments::probe(data_dir, id).await {
                    probes.insert(id.to_owned(), probe);
                }
            }
        }
        Showing { data_dir, views, probes }
    }
}

/// What `entry` carries. A `made` line carries the view it names while that view is still on
/// disk and is something a task can make; any other line carries the attachments the store
/// wrote on it that this store holds — an id typed by hand, or one whose object is gone, is
/// ignored the way a `made` ref to a deleted view is.
fn carried(entry: &TimelineEntry, showing: &Showing<'_>) -> Vec<ShownDto> {
    if let Some(view) = entry.made_ref() {
        if !crate::mind::views::can_be_a_result(view) || !showing.views.contains(view) {
            return Vec::new();
        }
        return vec![ShownDto {
            reff: format!("view:{view}"),
            kind: "view",
            preview: super::view_shots::url_for_ref(showing.data_dir, view),
            url: None,
            label: Some(super::view_bus::humanize_ref(view)),
            width: None,
            height: None,
            duration_ms: None,
            at: None,
            line: None,
        }];
    }
    entry
        .attached()
        .into_iter()
        .filter_map(|id| {
            let probe = showing.probes.get(id)?;
            Some(ShownDto {
                reff: format!("{}{id}", attachments::PREFIX),
                kind: probe.kind.as_str(),
                preview: Some(attachments::preview_url(id)),
                url: Some(attachments::url(id)),
                label: None,
                width: Some(probe.width),
                height: Some(probe.height),
                duration_ms: probe.duration_ms,
                at: None,
                line: None,
            })
        })
        .collect()
}

fn moment(entry: &TimelineEntry, showing: &Showing<'_>) -> MomentDto {
    MomentDto {
        at: entry.at.map(rfc3339),
        kind: entry.kind.as_str(),
        text: entry.said().to_owned(),
        attached: carried(entry, showing),
    }
}

/// The newest thing anybody said on the row — a mind's line, or the person's own through
/// the row's reply box — and the one line a card prints under the title.
///
/// `moved` and `made` are excluded because the store writes them about things it merely
/// witnessed: a status change is the consequence of a decision, not a statement about one,
/// and a builder rendering its page is not anybody saying anything. Neither can raise a wait
/// nor answer it. A row whose only entries are those has said nothing and gets no line.
///
/// **`replied` is the store's too and is not excluded**, because what it witnessed is somebody
/// speaking: the words are the person's, typed into this row. It is what answers a wait —
/// after it the next step is ours, and a row that still needs them says so again under it.
fn latest_moment(task: &Task, showing: &Showing<'_>) -> Option<MomentDto> {
    task.timeline
        .iter()
        .rev()
        .find(|entry| !matches!(entry.kind, TimelineKind::Moved | TimelineKind::Made))
        .map(|entry| moment(entry, showing))
}

/// What this task's lines placed, newest first, once each, at most [`ROW_SHOWN`]: the views
/// it made and the attachments its lines carried (`docs/arch/home.md` § *Internal mapping*).
///
/// **Only what a line placed, never the names its prose spells**, which is what this used to
/// read. Four spellings were matched anywhere in the record, so a note that another task's
/// page was on screen made that page this task's result and a shoe report hung under a KTV
/// task. A mention has no verb. A `made` line is the store's witness of a session serving the
/// task rendering the view ([`tasks::record_made`]); an attachment is on the line whose
/// writer handed it over through `hi_task_note`. Both are facts, not readings.
///
/// Ordered by the line that placed each, which is the one clock a view has.
fn shown(task: &Task, showing: &Showing<'_>) -> Vec<ShownDto> {
    let mut out: Vec<ShownDto> = Vec::new();
    for entry in task.timeline.iter().rev() {
        for mut item in carried(entry, showing) {
            if out.iter().any(|seen| seen.reff == item.reff) {
                continue;
            }
            item.at = entry.at.map(rfc3339);
            item.line = Some(entry.kind.as_str());
            out.push(item);
            if out.len() == ROW_SHOWN {
                return out;
            }
        }
    }
    out
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

fn liveness_dto(task: &Task) -> Option<LivenessDto> {
    if task.liveness.is_empty() {
        return None;
    }
    Some(LivenessDto {
        verify: task.liveness.verify.clone(),
        restart: task.liveness.restart.clone(),
        owner: task.liveness.owner.clone(),
        start_key: task.liveness.start_key.clone(),
    })
}

fn dto(task: &Task, malformed: bool, files: Vec<FileDto>, showing: &Showing<'_>) -> TaskDto {
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
        liveness: liveness_dto(task),
        timeline: task.timeline.iter().map(|entry| moment(entry, showing)).collect(),
        latest: latest_moment(task, showing),
        files,
        malformed,
        extra,
        extra_dropped,
    }
}

/// One task as a row.
///
/// Ships neither prose nor timeline, which is the property that has to hold: the list stays
/// proportional to how many tasks there are rather than to how much has been written on them.
/// What it does carry out of them is what a chart draws — one line and the views it made.
///
/// **No files.** They came back onto the row once so Home could count them under each task,
/// at a `stat` per named file per task per ledger read; Home no longer counts anything it does
/// not draw, and the one other reader, the board's file links, reads them off the record.
fn row(task: &Task, malformed: bool, showing: &Showing<'_>) -> RowDto {
    let (extra, extra_dropped) = extra_fields(&task.extra);
    RowDto {
        subject: task.subject.clone(),
        title: task.title.clone(),
        status: task.status.as_str(),
        created_at: task.created_at.map(rfc3339),
        status_since: task.status_since.map(rfc3339),
        due_at: task.due_at.map(rfc3339),
        checked_at: task.checked_at.map(rfc3339),
        completed_at: task.completed_at.map(rfc3339),
        cancelled_at: task.cancelled_at.map(rfc3339),
        liveness: liveness_dto(task),
        latest: latest_moment(task, showing),
        attached: shown(task, showing),
        malformed,
        extra,
        extra_dropped,
    }
}

/// The files a task's own record points at, that are on disk in its folder.
///
/// **Not a listing of the folder, deliberately.** A task folder is where the work
/// happened, not a shelf of deliverables: one live store holds 39,946 files under
/// `tasks/` — cloned repos, `__pycache__`, scraped HTML — and a single task's *top level*
/// holds 114. Listing that is showing somebody every scratch file when they asked what
/// was made. What they came back for is the file the record names — *"the completed
/// report is `inspection-report.md` in this task directory"* — and until that sentence is
/// reachable, the panel is pointing at something the reader cannot open.
///
/// So the record stays the authority and this only makes its own references resolvable:
/// every inline-code token a line spells, kept when a regular file of that name is really
/// there. A record naming a file it never wrote lists nothing; a file
/// nobody wrote down stays where it is, which is the same rule the ledger runs on
/// everywhere else — two listings would mean one of them is wrong and no way to tell
/// which.
async fn referenced_files(data_dir: &FsPath, task: &Task) -> Vec<FileDto> {
    let mut candidates: Vec<String> = Vec::new();
    for entry in &task.timeline {
        code_spans(&entry.text, &mut candidates);
    }
    candidates.retain(|token| names_a_file(token));
    candidates.sort();
    candidates.dedup();
    // A record carries a handful of these; the cap is a backstop
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

/// Read one task off disk, saying whether the record disagrees with itself.
///
/// A record that will not parse is not an error to the reader: it is a row that has to appear
/// on the board *marked*, because a task that vanished from the list because its file is
/// malformed is a task nobody will ever go and fix.
async fn read_row(dir: &FsPath, subject: &str) -> (Task, bool) {
    let parsed = tasks::read_task(dir, subject).await;
    let raw = tasks::read_record(dir, subject).await;
    match (parsed, raw) {
        (Ok(Some(task)), Ok(Some(raw))) => {
            let malformed = is_malformed(&raw, &task);
            (task, malformed)
        }
        _ => (unreadable(subject), true),
    }
}

/// What a reader already has, if anything.
#[derive(Deserialize)]
pub struct TasksQuery {
    /// The version the caller last saw. Absent asks for the ledger as it stands; present
    /// parks until it stops agreeing — see [`stores::StoreVersions::wait`].
    since: Option<u64>,
}

/// `GET /api/tasks` — the list. Every task, as a row.
///
/// **It answers when the ledger moves, not when asked again.** `?since=<version>` parks until
/// the store disagrees with that number, so a board holding this open costs nothing at all
/// while nothing happens and hears about a change when it happens rather than by the next
/// tick. Without `since` it answers straight away, which is what a first read is.
///
/// What it costs when it does answer: a read of every record and a `stat` per file each one
/// names — so proportional to how many tasks there are, and *not* to how much has been
/// written on them, because neither the prose nor the timeline is on the wire. That second
/// property is the one that has to hold. One live store's ledger is 166 records and 2.1 MB of
/// account; the rows are the 166.
///
/// The stats are affordable here **only because this is not on a clock.** They were taken off
/// the list when it was, and came back with the park.
///
/// The version is captured **before** the records are read, so a write landing mid-read is
/// reported as still pending rather than swallowed: the caller reads once more and finds the
/// same thing. The other order would let a change be lost behind a version that claims it.
pub async fn get_tasks(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TasksQuery>,
) -> Response {
    let version = match query.since {
        None => state.stores.version(tasks::DIMENSION).await,
        Some(since) => {
            match state.stores.wait(tasks::DIMENSION, since, stores::WAIT).await {
                Some(version) => version,
                // Still current. Said out loud rather than by holding the connection open
                // until something upstream loses patience with it.
                None => {
                    return Json(serde_json::json!({ "version": since, "unchanged": true }))
                        .into_response();
                }
            }
        }
    };
    let dir = &state.data_dir;
    let index = tasks::subjects(dir).await;
    // Read once for the whole list, not once per task: which views exist is a fact about the
    // tree, and the rows only ask it to say which of the names they spell is really one.
    let views = super::view::existing_refs(dir).await;

    let mut rows: Vec<(SortKey, RowDto)> = Vec::new();
    // Views a row carries that nobody has taken a picture of — one that was only ever
    // reviewed, never shown. Home used to drop those, hiding the task's result until someone
    // opened it; now it asks for the picture, a few per read.
    let mut unshot: Vec<String> = Vec::new();
    for subject in &index {
        let (task, malformed) = read_row(dir, subject).await;
        let showing = Showing::load(dir, &views, &task).await;
        let drawn = row(&task, malformed, &showing);
        for item in &drawn.attached {
            if item.preview.is_none()
                && let Some(view) = item.reff.strip_prefix("view:")
            {
                unshot.push(view.to_owned());
            }
        }
        rows.push((sort_key(&task), drawn));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    super::view::warm_for_rows(&state, unshot);

    let tasks: Vec<RowDto> = rows.into_iter().map(|(_, task)| task).collect();
    Json(serde_json::json!({ "version": version, "tasks": tasks })).into_response()
}

/// `GET /api/tasks/{subject}` — the record. One task, whole.
///
/// Read when a person opens a row, which is the only moment the account behind it is worth
/// what it costs: this is where [`referenced_files`] does its stats, and where the prose and
/// the whole timeline are on the wire.
pub async fn get_task(State(state): State<Arc<AppState>>, Path(subject): Path<String>) -> Response {
    let subject = facets::slug(&subject);
    if subject.is_empty() {
        return not_found("no such task");
    }
    let dir = &state.data_dir;
    // A row the list will not show is not a record to serve. `read_row` invents an
    // `unreadable` stand-in for a file it cannot parse, which is right for a list — the row
    // has to appear, marked — and wrong here, where it would answer a direct read with a
    // fabricated task the store has no such file for.
    match tasks::read_record(dir, &subject).await {
        Ok(Some(_)) => {}
        Ok(None) => return not_found("no such task"),
        Err(error) => return err(&error.to_string()),
    }
    let (task, malformed) = read_row(dir, &subject).await;
    let files = referenced_files(dir, &task).await;
    let views = super::view::existing_refs(dir).await;
    let showing = Showing::load(dir, &views, &task).await;
    Json(serde_json::json!({ "task": dto(&task, malformed, files, &showing) })).into_response()
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
    // The same lock the verbs take, so the person's click and a worker's line cannot each
    // write back a copy missing the other.
    let _held = tasks::write_lock().lock().await;
    let mut task = match tasks::read_task(&state.data_dir, &subject).await {
        Ok(Some(task)) => task,
        Ok(None) => return not_found("no such task"),
        Err(error) => return err(&error.to_string()),
    };

    if let Some(value) = &patch.status {
        match parse_status(value) {
            Some(status) => task.set_status(status, Utc::now(), tasks::Hand::Board),
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
    // Say so directly rather than waiting to be told by the watcher that is about to see this
    // same write. Not a second mechanism to keep in step — it is the same counter, and a
    // doubled bump costs one redundant read — but the one that does not depend on the
    // platform having given us a watcher, and the one that lands inside the click rather than
    // a settle later. The watcher's job is the writes that do *not* come through here: the
    // agent keeps these records with a shell.
    state.stores.bump(tasks::DIMENSION).await;
    let files = referenced_files(&state.data_dir, &task).await;
    let views = super::view::existing_refs(&state.data_dir).await;
    let showing = Showing::load(&state.data_dir, &views, &task).await;
    Json(serde_json::json!({ "ok": true, "task": dto(&task, false, files, &showing) })).into_response()
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

/// The row a reply box names, as the message said on it will carry it — or `None` when
/// nothing is filed under that subject.
///
/// The title is read here, from the ledger, and never taken from the window: it is what the
/// face draws over the bubble and what the prompt writes beside the subject, and both should
/// say what the row was called, not what a client said it was called.
pub(crate) async fn said_on(data_dir: &FsPath, subject: &str) -> anyhow::Result<Option<TaskRef>> {
    let subject = facets::slug(subject);
    if subject.is_empty() {
        return Ok(None);
    }
    let Some(task) = tasks::read_task(data_dir, &subject).await? else {
        return Ok(None);
    };
    let title = if task.title.trim().is_empty() { task.subject.clone() } else { task.title };
    Ok(Some(TaskRef { subject: task.subject, title }))
}

/// Write a message said on a row into that row's record as a `replied` line. Nothing for a
/// message typed anywhere else.
///
/// **Masked, because the record can be read with a shell.** A wait is often for a key, so the
/// answer to one is often a key — and the one seam that keeps a typed credential out of a
/// model is the prompt, which a worker reading the record file never crosses. The ingress has
/// already filed any credential it found; this writes the path in its place. The conversation
/// and the journal keep what was typed.
///
/// A failure is logged and goes no further. The message is already journalled and on its way
/// to Reaction, and refusing it now would cost them the words to save a line about them.
pub(crate) async fn record_said_on(state: &AppState, message: &Message) {
    let Some(task) = &message.task else {
        return;
    };
    let said = match &message.content {
        Content::Text(text) | Content::Speech { text, .. } => {
            state.privacy.store().mask_known(text).into_owned()
        }
        Content::File(file) => format!("handed over {}", file.name),
    };
    match tasks::record_reply(&state.data_dir, &task.subject, message.ts, &said).await {
        // Said directly for the reason `patch_task` says it: the watcher would see this write
        // a settle later, and the reply box is waiting on it now.
        Ok(true) => state.stores.bump(tasks::DIMENSION).await,
        Ok(false) => {}
        Err(error) => tracing::error!(
            subject = %task.subject,
            error = %format!("{error:#}"),
            "a reply said on a task did not reach its record; the message went on"
        ),
    }
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

    /// The property the whole split exists for: **a row's size is not a function of how much
    /// has been written on the task.** A record with a hundred long timeline entries has to
    /// serialize to about what an empty one does, or the board is back where it started the
    /// next time somebody keeps a good ledger.
    #[test]
    fn a_row_does_not_carry_the_record() {
        let mut task = Task::new("Watch the group", TaskStatus::Serving);
        task.timeline = (0..100)
            .map(|i| TimelineEntry {
                at: Some(at(1, 9)),
                kind: TimelineKind::Update,
                text: format!("{i}: {}", "y".repeat(2_000)),
            })
            .collect();

        let value = serde_json::to_value(row(&task, false, &showing(&views(&[])))).unwrap();
        assert!(value.get("body").is_none(), "a row carries no prose: {value}");
        assert!(value.get("timeline").is_none(), "a row carries no timeline: {value}");
        assert!(value.get("files").is_none(), "files are the record's, not the row's: {value}");

        let bytes = serde_json::to_string(&value).unwrap().len();
        assert!(bytes < 4_000, "a row of a 400 KB record came to {bytes} bytes");
    }

    /// `moved` and `made` are the store's own lines, so neither can raise a wait nor answer one.
    /// A row whose only entries are those has said nothing, and gets no line rather than the
    /// transition spelled for a machine.
    #[test]
    fn latest_is_the_newest_line_anybody_said() {
        let mut task = Task::new("Ship the deck", TaskStatus::Doing);
        let entry = |kind, text: &str| TimelineEntry {
            at: Some(at(1, 9)),
            kind,
            text: text.to_owned(),
        };
        task.timeline = vec![
            entry(TimelineKind::Created, "make the deck"),
            entry(TimelineKind::Waiting, "which quarter?"),
            entry(TimelineKind::Moved, "todo \u{2192} doing"),
        ];
        let value = serde_json::to_value(row(&task, false, &showing(&views(&[])))).unwrap();
        assert_eq!(value["latest"]["kind"], "waiting");
        assert_eq!(value["latest"]["text"], "which quarter?");

        // And the record says the same thing under the same name, so a panel and a card are
        // never reading two answers to one question.
        let full = serde_json::to_value(dto(&task, false, Vec::new(), &showing(&views(&[])))).unwrap();
        assert_eq!(full["latest"], value["latest"]);

        task.timeline = vec![
            entry(TimelineKind::Moved, "todo \u{2192} doing"),
            entry(TimelineKind::Made, "`deck/leader`"),
        ];
        let value = serde_json::to_value(row(&task, false, &showing(&views(&[])))).unwrap();
        assert!(value["latest"].is_null(), "a row with only the store's lines has said nothing: {value}");
    }

    /// **Their reply through the row answers the wait above it**, though the store wrote the
    /// line: what it witnessed was them speaking. A transition after it changes nothing, and a
    /// row that still needs them says so again underneath.
    #[test]
    fn a_reply_on_the_row_answers_the_wait_above_it() {
        let mut task = Task::new("Try the new voice", TaskStatus::Doing);
        let entry = |kind, text: &str| TimelineEntry { at: Some(at(1, 9)), kind, text: text.to_owned() };
        task.timeline = vec![
            entry(TimelineKind::Waiting, "press Run at http://127.0.0.1:7788, listen, reply ACCEPT or REJECT"),
            entry(TimelineKind::Replied, "ACCEPT"),
            entry(TimelineKind::Moved, "doing \u{2192} done"),
        ];
        let value = serde_json::to_value(row(&task, false, &showing(&views(&[])))).unwrap();
        assert_eq!(value["latest"]["kind"], "replied");
        assert_eq!(value["latest"]["text"], "ACCEPT");

        task.timeline.push(entry(TimelineKind::Waiting, "which of the two takes?"));
        let value = serde_json::to_value(row(&task, false, &showing(&views(&[])))).unwrap();
        assert_eq!(value["latest"]["kind"], "waiting");
    }

    fn views(refs: &[&str]) -> std::collections::HashSet<String> {
        refs.iter().map(|r| (*r).to_owned()).collect()
    }

    /// A row drawn against `views` and no attachments — the store holds none in these tests
    /// unless one places them.
    fn showing(views: &std::collections::HashSet<String>) -> Showing<'_> {
        Showing {
            data_dir: FsPath::new("/nonexistent-hi-agent-data"),
            views,
            probes: std::collections::HashMap::new(),
        }
    }

    /// **A mention is not a result, in any spelling.** These are the four the old matcher read
    /// and the two sentences that put pictures under the wrong task on a live instance — a
    /// note about what the screen was showing, in both directions. None of them reaches the
    /// row, because nothing in prose says the task made anything.
    #[test]
    fn a_view_the_record_only_mentions_is_not_a_result() {
        let mut task = Task::new("Put the KTV method page up", TaskStatus::Doing);
        task.timeline = vec![
            TimelineEntry::new(TimelineKind::Update, at(1, 9), "inline code says `deck/leader`"),
            TimelineEntry::new(TimelineKind::Update, at(1, 9), "quoted says \"health/checkin\""),
            TimelineEntry::new(
                TimelineKind::Update,
                at(1, 9),
                "a builder just wrote data/views/knq/commentary.jsx",
            ),
            TimelineEntry::new(
                TimelineKind::Update,
                at(1, 9),
                "and the field reads view_ref: \"xiaoyuanzhu/vocab\"",
            ),
            TimelineEntry::new(
                TimelineKind::Update,
                at(1, 9),
                "\u{5c4f}\u{4e0a}\u{73b0}\u{5728}\u{6302}\u{7684}\u{662f} `research-two-pairs`",
            ),
        ];
        let known = views(&[
            "deck/leader",
            "health/checkin",
            "knq/commentary",
            "xiaoyuanzhu/vocab",
            "research-two-pairs",
        ]);
        assert!(shown(&task, &showing(&known)).is_empty());
    }

    /// What a row carries is its `made` lines: newest first, once each, and only while the
    /// view is still on disk to open. A hand-typed `made` line cannot bring back the two
    /// classes that are never a task's product.
    #[test]
    fn a_row_carries_what_the_task_made_newest_first() {
        let mut task = Task::new("Research two pairs", TaskStatus::Doing);
        let made = |day, text: &str| TimelineEntry::new(TimelineKind::Made, at(day, 9), text);
        task.timeline = vec![
            made(1, "`shoes/log`"),
            made(2, "`shoes/report`"),
            TimelineEntry::new(TimelineKind::Update, at(3, 9), "the screen shows `polaroid/poster`"),
            made(4, "`shoes/deleted`"),
            made(5, "factory/home"),
            made(6, "`_qa-shoes-wide`"),
            made(7, "`shoes/log`"),
        ];
        let known = views(&[
            "shoes/log",
            "shoes/report",
            "polaroid/poster",
            "factory/home",
            "_qa-shoes-wide",
        ]);
        let carried: Vec<String> = shown(&task, &showing(&known)).into_iter().map(|s| s.reff).collect();
        assert_eq!(carried, vec!["view:shoes/log", "view:shoes/report"]);
    }

    /// **An attachment is on the row by the line that carried it, beside the views the task
    /// made, in the order the lines were written** — and an id nothing placed is nothing,
    /// however it got into the file. The panel's line reads as its sentence alone.
    #[tokio::test]
    async fn a_row_carries_the_attachments_its_lines_carried_beside_its_views() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let figure = work.path().join("pose_899.png");
        image::RgbImage::from_pixel(64, 36, image::Rgb([20, 120, 40])).save(&figure).unwrap();
        let placed = attachments::place(data.path(), &figure).await.unwrap();

        let mut task = Task::new("Calibrate the court", TaskStatus::Doing);
        task.timeline = vec![
            TimelineEntry::new(TimelineKind::Made, at(1, 9), "`court/lines`"),
            TimelineEntry::new(
                TimelineKind::Update,
                at(2, 9),
                format!("场地线压在线上 ⟨attached att:{} att:0123456789abcdef⟩", placed.id),
            ),
        ];
        let known = views(&["court/lines"]);
        let showing = Showing::load(data.path(), &known, &task).await;
        let value = serde_json::to_value(row(&task, false, &showing)).unwrap();
        let attached = value["attached"].as_array().unwrap();
        assert_eq!(attached.len(), 2, "the typed id is not an attachment: {value}");
        assert_eq!(attached[0]["ref"], format!("att:{}", placed.id));
        assert_eq!(attached[0]["kind"], "picture");
        assert_eq!(attached[0]["line"], "update");
        assert_eq!(attached[0]["preview"], format!("/api/attachments/{}/preview.v1", placed.id));
        assert_eq!(attached[0]["url"], format!("/api/attachments/{}", placed.id));
        assert_eq!(attached[1]["ref"], "view:court/lines");
        assert_eq!(value["latest"]["text"], "场地线压在线上", "the marker is drawn, never read");
        assert_eq!(value["latest"]["attached"][0]["ref"], format!("att:{}", placed.id));
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
        tasks::write_task(dir.path(), &task).await.unwrap();

        let got = tasks::read_task(dir.path(), "watch-oil-prices")
            .await
            .unwrap()
            .unwrap();
        let value = serde_json::to_value(dto(&got, false, Vec::new(), &showing(&views(&[])))).unwrap();
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

    /// The seam the panel is built on: the running record reaches it as dated lines, and it
    /// is the whole of what the row says — a record read off disk carries no prose beside it.
    #[tokio::test]
    async fn dto_carries_the_running_record_as_dated_lines() {
        let dir = tempfile::tempdir().unwrap();
        let mut task = Task::new("Daily ops digest", TaskStatus::Doing);
        task.timeline = vec![
            TimelineEntry::new(
                tasks::TimelineKind::Created,
                at(1, 9),
                "it goes to the Feishu group, not to me",
            ),
            TimelineEntry::new(tasks::TimelineKind::Waiting, at(2, 11), "no im:chat scope"),
        ];
        tasks::write_task(dir.path(), &task).await.unwrap();

        let got = tasks::read_task(dir.path(), "daily-ops-digest")
            .await
            .unwrap()
            .unwrap();
        let value = serde_json::to_value(dto(&got, false, Vec::new(), &showing(&views(&[])))).unwrap();
        assert!(value.get("body").is_none(), "no prose beside the record: {value}");
        assert_eq!(value["timeline"][0]["kind"], "created");
        assert_eq!(value["timeline"][0]["at"], "2026-08-01T09:00:00Z");
        assert_eq!(
            value["timeline"][0]["text"],
            "it goes to the Feishu group, not to me"
        );
        assert_eq!(value["timeline"][1]["kind"], "waiting");
    }

    /// A task nobody has recorded anything about serves an empty list, never a null — the
    /// panel maps over it, and an empty board is the ordinary state of a fresh ledger.
    #[test]
    fn a_task_with_no_running_record_serves_an_empty_list() {
        let task = Task::new("Ship the deck", TaskStatus::Todo);
        let value = serde_json::to_value(dto(&task, false, Vec::new(), &showing(&views(&[])))).unwrap();
        assert_eq!(value["timeline"], serde_json::json!([]));
    }

    #[test]
    fn bare_task_has_no_due_or_liveness_metadata() {
        let task = Task::new("Ship the deck", TaskStatus::Todo);
        let value = serde_json::to_value(dto(&task, false, Vec::new(), &showing(&views(&[])))).unwrap();
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
        let value = serde_json::to_value(dto(&task, false, Vec::new(), &showing(&views(&[])))).unwrap();
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
        let value = serde_json::to_value(dto(&task, false, Vec::new(), &showing(&views(&[])))).unwrap();
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
        let value = serde_json::to_value(dto(&task, false, Vec::new(), &showing(&views(&[])))).unwrap();
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
        let value = serde_json::to_value(dto(&task, false, Vec::new(), &showing(&views(&[])))).unwrap();
        assert_eq!(value["extra"].as_array().unwrap().len(), EXTRA_FIELDS);
        assert_eq!(value["extraDropped"], 7);

        let mut one = Task::new("Watch the group", TaskStatus::Serving);
        one.extra = vec![format!("long: {}", "x".repeat(400))];
        let value = serde_json::to_value(dto(&one, false, Vec::new(), &showing(&views(&[])))).unwrap();
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
        tasks::write_raw(dir.path(), "kut",
            "---\nstatus: doing\ntitle: \"Deploy KUT\"\nsystems: KUT, gz\n---\n",
        )
        .await
        .unwrap();
        let task = tasks::read_task(dir.path(), "kut").await.unwrap().unwrap();
        let value = serde_json::to_value(dto(&task, false, Vec::new(), &showing(&views(&[])))).unwrap();
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
        task.timeline = vec![
            TimelineEntry::new(
                tasks::TimelineKind::Delivered,
                at(25, 6),
                "The completed report is `inspection-report.md` in this task directory. \
                 `hi_say` carried the headline; `status_since` moved with it, and the draft \
                 `never-written.md` was abandoned.",
            ),
            TimelineEntry::new(
                tasks::TimelineKind::Delivered,
                at(25, 6),
                "`notes/working.md` has the sampling method",
            ),
        ];
        tasks::write_task(dir.path(), &task).await.unwrap();

        let folder = facets::subject_dir(dir.path(), tasks::DIMENSION, &task.subject);
        tokio::fs::create_dir_all(&folder).await.unwrap();
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
        tasks::write_raw(dir.path(), "watch-the-ops-group",
            "---\nstatus: doing\ntitle: \"Watch the ops group\"\nverify: \"a row landed today\"\n\
             restart: \"launchctl kickstart the label\"\n---\n",
        )
        .await
        .unwrap();
        let raw = tasks::read_record(dir.path(), "watch-the-ops-group")
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
        tasks::write_raw(dir.path(), "bad-current",
            "---\nstatus: Doing\ncreated_at: yesterday\n---\n",
        )
        .await
        .unwrap();
        let raw = tasks::read_record(dir.path(), "bad-current")
            .await
            .unwrap()
            .unwrap();
        let task = tasks::read_task(dir.path(), "bad-current")
            .await
            .unwrap()
            .unwrap();
        assert!(is_malformed(&raw, &task));

        tasks::write_raw(dir.path(), "bad-legacy",
            "---\nkind: watching\nstate: Open\n---\n",
        )
        .await
        .unwrap();
        let raw = tasks::read_record(dir.path(), "bad-legacy")
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
