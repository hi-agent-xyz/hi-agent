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
use crate::mind::memory::tasks::{self, Task, TaskStatus, TimelineEntry};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskDto {
    subject: String,
    title: String,
    status: &'static str,
    created_at: Option<String>,
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

fn dto(task: &Task, malformed: bool) -> TaskDto {
    TaskDto {
        subject: task.subject.clone(),
        title: task.title.clone(),
        status: task.status.as_str(),
        created_at: task.created_at.map(rfc3339),
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
        rows.push((sort_key(&task), dto(&task, malformed)));
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
    Json(serde_json::json!({ "ok": true, "task": dto(&task, false) })).into_response()
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
        let value = serde_json::to_value(dto(&got, false)).unwrap();
        assert_eq!(value["status"], "serving");
        assert_eq!(value["createdAt"], "2026-08-01T09:00:00Z");
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
        let value = serde_json::to_value(dto(&got, false)).unwrap();
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
        let value = serde_json::to_value(dto(&task, false)).unwrap();
        assert_eq!(value["timeline"], serde_json::json!([]));
    }

    #[test]
    fn bare_task_has_no_due_or_liveness_metadata() {
        let task = Task::new("Ship the deck", TaskStatus::Todo);
        let value = serde_json::to_value(dto(&task, false)).unwrap();
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
