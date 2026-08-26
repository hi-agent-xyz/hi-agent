//! Task review endpoints.
//!
//! `GET /api/tasks` returns every task across the five lifecycle statuses.
//! `PATCH /api/tasks/{subject}` changes `status` or `title`. Status transitions stamp
//! `completed_at` and `cancelled_at` automatically and clear them when reopened.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::foundation::server::AppState;
use crate::mind::memory::facets;
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
    malformed: bool,
    /// Who is on this task right now, or `null` where the switchboard says nobody is.
    /// **`null` is not "fine"** — on a `doing` row it is the alarm, and the panel says so
    /// there; on a `todo` or a duty it is the ordinary state and the panel says nothing.
    on_it: Option<OnItDto>,
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

fn dto(task: &Task, malformed: bool, on_it: Option<&OnIt>) -> TaskDto {
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
        malformed,
        on_it: self::on_it(on_it),
    }
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
        let value = value.trim();
        if value.starts_with('"')
            && let Ok(decoded) = serde_json::from_str::<String>(value)
        {
            return Some(decoded);
        }
        return Some(value.to_owned());
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
        rows.push((sort_key(&task), dto(&task, malformed, on_it)));
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
    Json(serde_json::json!({ "ok": true, "task": dto(&task, false, on_it) })).into_response()
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
        let value = serde_json::to_value(dto(&got, false, None)).unwrap();
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
    /// so the record can be rendered as lines and the prose folded away behind them.
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
        let value = serde_json::to_value(dto(&got, false, None)).unwrap();
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
        let value = serde_json::to_value(dto(&task, false, None)).unwrap();
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
        let value = serde_json::to_value(dto(&task, false, Some(&entry))).unwrap();
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
        let value = serde_json::to_value(dto(&task, false, Some(&entry))).unwrap();
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
        let value = serde_json::to_value(dto(&task, false, Some(&entry))).unwrap();
        assert!(value["onIt"]["failed"].is_null());
        assert_eq!(value["onIt"]["stopped"], false);
    }

    /// The two the restart leaves behind, which the roster cannot answer at all: one is
    /// coming back on its own and needs no move, the other needs somebody put on it.
    #[test]
    fn the_restarts_casualties_are_distinguishable_from_each_other() {
        let task = Task::new("Ship the deck", TaskStatus::Doing);
        let reopening = serde_json::to_value(dto(&task, false, Some(&OnIt::Reopening))).unwrap();
        let lost = serde_json::to_value(dto(&task, false, Some(&OnIt::Lost))).unwrap();
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
        let value = serde_json::to_value(dto(&task, false, None)).unwrap();
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
        let value = serde_json::to_value(dto(&task, false, None)).unwrap();
        assert_eq!(value["statusSince"], "2026-08-01T09:00:00Z");
    }

    #[test]
    fn bare_task_has_no_due_or_liveness_metadata() {
        let task = Task::new("Ship the deck", TaskStatus::Todo);
        let value = serde_json::to_value(dto(&task, false, None)).unwrap();
        assert!(value["dueAt"].is_null());
        assert!(value["checkedAt"].is_null());
        assert!(value["liveness"].is_null());
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
