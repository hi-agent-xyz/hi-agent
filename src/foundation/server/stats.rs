//! Aggregate, read-only statistics over the durable local record.
//!
//! `GET /api/stats?range=7d|30d|90d|all` is intentionally derived on read. Session
//! frames, the signal journal, task facets, and the current broker snapshot already own
//! the facts; a second counters database would be free to drift from every one of them.
//! The response carries coverage alongside totals so legacy cumulative token records and
//! unreadable lines are visible rather than silently presented as exact.
//!
//! Derived on read only pays if the read is proportional to what was asked for, and one
//! input grows without bound: the frame logs, which keep every JSON-RPC line in both
//! directions forever. Three things keep the scan proportional.
//!
//! 1. **A log older than the range is never opened.** A log's mtime is its last frame, so
//!    `mtime < from` means none of its frames are in the window. Skipped logs are counted
//!    in [`Coverage::frame_logs_out_of_window`] rather than silently dropped. `range=all`
//!    has no `from` and so still reads everything — that is what it asks for.
//! 2. **One cheap pass per line, not two full ones.** The scan wants a handful of fields,
//!    not the folded message. Deltas — most of every log, and the reason a log gets large
//!    — are recognised by method and dropped without their payload being parsed at all.
//! 3. **Logs are scanned concurrently**, on the blocking pool, and merged. The work is
//!    JSON parsing rather than I/O wait, so the useful width is the machine's cores.
//!
//! What it counts is [`crate::foundation::codex::messages::fold`]'s reading of the same
//! log minus the text: one message per item id, kind from the first frame carrying that
//! id and status from the last. Both readings must agree about what an item *is*, which
//! is why [`messages::kind_of`] is shared rather than copied.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Days, Duration, NaiveDate, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::foundation::broker::KEY_BROKER_CHECKED_AT;
use crate::foundation::codex::messages;
use crate::foundation::credentials::{Credentials, Mode, get_setting};
use crate::foundation::registry::{self, index::Record};
use crate::foundation::server::{AppState, tools};
use crate::mind::memory::{facets, journal, layout, tasks};
use crate::types::{Channel, JournalEntry};

const DEFAULT_RANGE: &str = "30d";
const CONVERSATION_GAP_MINUTES: i64 = 30;
const STALLED_TASK_HOURS: i64 = 48;

#[derive(Debug, Deserialize, Default)]
pub struct StatsQuery {
    range: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Range {
    Days(u64, &'static str),
    All,
}

impl Range {
    fn parse(value: Option<&str>) -> Option<Self> {
        match value
            .unwrap_or(DEFAULT_RANGE)
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "7d" => Some(Self::Days(7, "7d")),
            "30d" => Some(Self::Days(30, "30d")),
            "90d" => Some(Self::Days(90, "90d")),
            "all" => Some(Self::All),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Days(_, label) => label,
            Self::All => "all",
        }
    }

    fn from(self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let Self::Days(days, _) = self else {
            return None;
        };
        let start = now
            .date_naive()
            .checked_sub_days(Days::new(days.saturating_sub(1)))
            .unwrap_or(NaiveDate::MIN);
        Some(Utc.from_utc_datetime(&start.and_hms_opt(0, 0, 0).expect("midnight is valid")))
    }
}

#[derive(Debug, Serialize)]
struct StatsResponse {
    period: Period,
    coverage: Coverage,
    summary: Summary,
    series: Vec<DailyPoint>,
    breakdowns: Breakdowns,
    inventory: Inventory,
}

#[derive(Debug, Serialize)]
struct Period {
    range: &'static str,
    from: String,
    to: String,
    generated_at: String,
    timezone: &'static str,
    granularity: &'static str,
}

#[derive(Debug, Default, Serialize)]
struct Coverage {
    session_index_records: usize,
    session_index_unreadable: usize,
    frame_logs: usize,
    /// Of those, the ones whose last frame predates the range and so were never opened.
    /// Zero for `range=all`, which asks for everything.
    frame_logs_out_of_window: usize,
    frame_logs_unreadable: usize,
    unreadable_frames: usize,
    journal_entries: usize,
    journal_readable: bool,
    task_records: usize,
    task_unreadable: usize,
    token_sessions: usize,
    token_turns: usize,
    token_updates: usize,
    estimated_token_updates: usize,
    legacy_estimated: bool,
    task_history_current_state_only: bool,
}

#[derive(Debug, Default, Serialize)]
struct Summary {
    tokens: TokenSummary,
    sessions: SessionSummary,
    conversation: ConversationSummary,
    tools: ToolSummary,
    tasks: TaskSummary,
    energy: EnergySummary,
}

#[derive(Debug, Default, Serialize)]
struct TokenSummary {
    total: u64,
    input: u64,
    output: u64,
    cached_input: u64,
    reasoning_output: u64,
    estimated: bool,
}

#[derive(Debug, Default, Serialize)]
struct SessionSummary {
    started: u64,
    live: u64,
    clean_closed: u64,
    restart_lost: u64,
    worker: SessionGroup,
    resident: SessionGroup,
}

#[derive(Debug, Default, Serialize)]
struct SessionGroup {
    started: u64,
    live: u64,
    clean_closed: u64,
    restart_lost: u64,
    average_duration_seconds: Option<f64>,
    median_duration_seconds: Option<f64>,
    duration_samples: u64,
    average_turns: Option<f64>,
    turn_samples: u64,
    #[serde(skip)]
    durations: Vec<u64>,
    #[serde(skip)]
    turns_total: u64,
}

#[derive(Debug, Default, Serialize)]
struct ConversationSummary {
    human_text_messages: u64,
    human_audio_messages: u64,
    handed_files: u64,
    human_messages: u64,
    agent_text_replies: u64,
    active_days: u64,
    conversation_windows: u64,
    window_gap_minutes: i64,
}

#[derive(Debug, Default, Serialize)]
struct ToolSummary {
    calls: u64,
    failed_calls: u64,
    commands: u64,
    edits: u64,
    web_searches: u64,
    context_compactions: u64,
    available_distinct: u64,
    available_assignments: u64,
}

#[derive(Debug, Default, Serialize)]
struct TaskSummary {
    total: u64,
    created: u64,
    completed: u64,
    cancelled: u64,
    overdue: u64,
    stalled_doing: u64,
    serving_never_confirmed: u64,
    average_completion_seconds: Option<f64>,
    median_completion_seconds: Option<f64>,
    completion_samples: u64,
}

#[derive(Debug, Default, Serialize)]
struct EnergySummary {
    mode: String,
    available: bool,
    remaining: Option<i64>,
    total: Option<i64>,
    resets_at: Option<String>,
    tier: Option<String>,
    observed_out_of_energy: bool,
    checked_at: Option<String>,
    historical_usage_available: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
struct DailyPoint {
    date: String,
    tokens: u64,
    turns: u64,
    sessions: u64,
    tool_calls: u64,
    user_messages: u64,
    agent_replies: u64,
    task_completions: u64,
}

#[derive(Debug, Default, Serialize)]
struct Breakdowns {
    sessions_by_role: Vec<SessionBreakdown>,
    sessions_by_worker_type: Vec<SessionBreakdown>,
    tools_by_name: Vec<ToolBreakdown>,
    tools_by_role: Vec<CountBreakdown>,
    tools_by_worker_type: Vec<CountBreakdown>,
    tool_statuses: Vec<CountBreakdown>,
    available_tools_by_role: Vec<CountBreakdown>,
    tasks_by_status: Vec<CountBreakdown>,
    facets_by_dimension: Vec<CountBreakdown>,
}

#[derive(Debug, Serialize)]
struct CountBreakdown {
    name: String,
    count: u64,
}

#[derive(Debug, Serialize)]
struct ToolBreakdown {
    name: String,
    count: u64,
    failed: u64,
}

#[derive(Debug, Default, Serialize)]
struct SessionBreakdown {
    name: String,
    count: u64,
    turns: u64,
    average_duration_seconds: Option<f64>,
    #[serde(skip)]
    durations: Vec<u64>,
}

#[derive(Debug, Default, Serialize)]
struct Inventory {
    facets: u64,
    episodes: u64,
    skills: SkillInventory,
    people: PeopleInventory,
    drive: DriveInventory,
    custom_views: u64,
}

#[derive(Debug, Default, Serialize)]
struct SkillInventory {
    total: u64,
    learned: u64,
    factory: u64,
}

#[derive(Debug, Default, Serialize)]
struct PeopleInventory {
    clusters: u64,
    named: u64,
    recurring: u64,
    face_samples: u64,
    voice_samples: u64,
}

#[derive(Debug, Default, Serialize)]
struct DriveInventory {
    files: u64,
    bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionEnd {
    Closed,
    Restart,
    Live,
}

#[derive(Debug, Clone)]
struct SessionRow {
    run: String,
    session: String,
    role: String,
    worker_type: Option<String>,
    started: DateTime<Utc>,
    ended: Option<DateTime<Utc>>,
    turns: Option<u64>,
    end: SessionEnd,
}

#[derive(Debug, Default)]
struct FrameAggregation {
    tokens: TokenSummary,
    tools: ToolSummary,
    tool_names: HashMap<String, (u64, u64)>,
    tool_roles: HashMap<String, u64>,
    tool_worker_types: HashMap<String, u64>,
    tool_statuses: HashMap<String, u64>,
    /// Sessions that reported token usage in the window, and the turns inside them. Plain
    /// counts rather than sets of keys: a session's frames live in exactly one file, so a
    /// per-file count of distinct turns needs no cross-file de-duplication.
    token_sessions: usize,
    token_turns: usize,
    token_updates: usize,
    estimated_token_updates: usize,
    frame_logs: usize,
    frame_logs_out_of_window: usize,
    frame_logs_unreadable: usize,
    unreadable_frames: usize,
}

impl FrameAggregation {
    fn absorb(&mut self, other: Self) {
        Usage {
            total: other.tokens.total,
            input: other.tokens.input,
            output: other.tokens.output,
            cached_input: other.tokens.cached_input,
            reasoning_output: other.tokens.reasoning_output,
        }
        .add_to(&mut self.tokens);
        self.tools.calls += other.tools.calls;
        self.tools.failed_calls += other.tools.failed_calls;
        self.tools.commands += other.tools.commands;
        self.tools.edits += other.tools.edits;
        self.tools.web_searches += other.tools.web_searches;
        self.tools.context_compactions += other.tools.context_compactions;
        for (name, (count, failed)) in other.tool_names {
            let entry = self.tool_names.entry(name).or_insert((0, 0));
            entry.0 += count;
            entry.1 += failed;
        }
        for (name, count) in other.tool_roles {
            *self.tool_roles.entry(name).or_default() += count;
        }
        for (name, count) in other.tool_worker_types {
            *self.tool_worker_types.entry(name).or_default() += count;
        }
        for (name, count) in other.tool_statuses {
            *self.tool_statuses.entry(name).or_default() += count;
        }
        self.token_sessions += other.token_sessions;
        self.token_turns += other.token_turns;
        self.token_updates += other.token_updates;
        self.estimated_token_updates += other.estimated_token_updates;
        self.frame_logs += other.frame_logs;
        self.frame_logs_out_of_window += other.frame_logs_out_of_window;
        self.frame_logs_unreadable += other.frame_logs_unreadable;
        self.unreadable_frames += other.unreadable_frames;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Usage {
    total: u64,
    input: u64,
    output: u64,
    cached_input: u64,
    reasoning_output: u64,
}

impl Usage {
    fn add_to(self, summary: &mut TokenSummary) {
        summary.total = summary.total.saturating_add(self.total);
        summary.input = summary.input.saturating_add(self.input);
        summary.output = summary.output.saturating_add(self.output);
        summary.cached_input = summary.cached_input.saturating_add(self.cached_input);
        summary.reasoning_output = summary
            .reasoning_output
            .saturating_add(self.reasoning_output);
    }

    fn positive_delta(self, before: Self) -> Self {
        Self {
            total: self.total.saturating_sub(before.total),
            input: self.input.saturating_sub(before.input),
            output: self.output.saturating_sub(before.output),
            cached_input: self.cached_input.saturating_sub(before.cached_input),
            reasoning_output: self
                .reasoning_output
                .saturating_sub(before.reasoning_output),
        }
    }
}

#[derive(Debug, Clone)]
struct Window {
    range: Range,
    from: Option<DateTime<Utc>>,
    to: DateTime<Utc>,
}

impl Window {
    fn includes(&self, at: DateTime<Utc>) -> bool {
        self.from.is_none_or(|from| at >= from) && at <= self.to
    }
}

/// `GET /api/stats?range=7d|30d|90d|all`.
pub async fn get_stats(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StatsQuery>,
) -> Response {
    let Some(range) = Range::parse(query.range.as_deref()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "range must be 7d, 30d, 90d or all" })),
        )
            .into_response();
    };
    Json(build_stats(&state, range, Utc::now()).await).into_response()
}

async fn build_stats(state: &AppState, range: Range, now: DateTime<Utc>) -> StatsResponse {
    let window = Window {
        range,
        from: range.from(now),
        to: now,
    };
    let mut summary = Summary::default();
    let mut coverage = Coverage {
        journal_readable: true,
        task_history_current_state_only: true,
        ..Coverage::default()
    };
    let mut breakdowns = Breakdowns::default();
    let mut series: BTreeMap<NaiveDate, DailyPoint> = BTreeMap::new();
    let mut earliest: Option<DateTime<Utc>> = None;

    let index_text = tokio::fs::read_to_string(registry::index::index_path(&state.data_dir))
        .await
        .unwrap_or_default();
    let (mut sessions, index_records, index_unreadable) =
        fold_session_index(&index_text, crate::foundation::run::id());
    coverage.session_index_records = index_records;
    coverage.session_index_unreadable = index_unreadable;
    sessions.extend(live_session_rows());

    let mut session_meta: HashMap<(String, String), (String, Option<String>)> = HashMap::new();
    let mut role_breakdown: HashMap<String, SessionBreakdown> = HashMap::new();
    let mut worker_type_breakdown: HashMap<String, SessionBreakdown> = HashMap::new();
    for row in &sessions {
        session_meta.insert(
            (row.run.clone(), row.session.clone()),
            (row.role.clone(), row.worker_type.clone()),
        );
        note_earliest(&mut earliest, row.started);
        apply_session(
            row,
            &window,
            &mut summary.sessions,
            &mut role_breakdown,
            &mut worker_type_breakdown,
            &mut series,
        );
    }
    finish_session_group(&mut summary.sessions.worker);
    finish_session_group(&mut summary.sessions.resident);
    breakdowns.sessions_by_role = finish_session_breakdowns(role_breakdown);
    breakdowns.sessions_by_worker_type = finish_session_breakdowns(worker_type_breakdown);

    let frame_aggregation = aggregate_frames(
        &state.data_dir,
        &window,
        &session_meta,
        &mut series,
        &mut earliest,
    )
    .await;
    summary.tokens = frame_aggregation.tokens;
    summary.tools = frame_aggregation.tools;
    coverage.frame_logs = frame_aggregation.frame_logs;
    coverage.frame_logs_out_of_window = frame_aggregation.frame_logs_out_of_window;
    coverage.frame_logs_unreadable = frame_aggregation.frame_logs_unreadable;
    coverage.unreadable_frames = frame_aggregation.unreadable_frames;
    coverage.token_sessions = frame_aggregation.token_sessions;
    coverage.token_turns = frame_aggregation.token_turns;
    coverage.token_updates = frame_aggregation.token_updates;
    coverage.estimated_token_updates = frame_aggregation.estimated_token_updates;
    coverage.legacy_estimated = frame_aggregation.estimated_token_updates > 0;
    summary.tokens.estimated = coverage.legacy_estimated;
    breakdowns.tools_by_name = sorted_tools(frame_aggregation.tool_names);
    breakdowns.tools_by_role = sorted_counts(frame_aggregation.tool_roles);
    breakdowns.tools_by_worker_type = sorted_counts(frame_aggregation.tool_worker_types);
    breakdowns.tool_statuses = sorted_counts(frame_aggregation.tool_statuses);

    let journal_since = window
        .from
        .unwrap_or_else(|| DateTime::from_timestamp(0, 0).expect("unix epoch is valid"));
    match state.memory.journal.recent(journal_since, usize::MAX).await {
        Ok(entries) => {
            coverage.journal_entries = entries.len();
            apply_journal(
                &entries,
                &window,
                &mut summary.conversation,
                &mut series,
                &mut earliest,
            );
        }
        Err(error) => {
            coverage.journal_readable = false;
            tracing::warn!(error = %error, "stats could not read the signal journal");
        }
    }

    let facet_index = match facets::facet_subject_index(&state.data_dir).await {
        Ok(index) => index,
        Err(error) => {
            tracing::warn!(error = %error, "stats could not read the facet index");
            Vec::new()
        }
    };
    apply_facets(&facet_index, &mut breakdowns, &mut summary.tasks);
    apply_tasks(
        &state.data_dir,
        &facet_index,
        &window,
        now,
        &mut summary.tasks,
        &mut coverage,
        &mut breakdowns,
        &mut series,
        &mut earliest,
    )
    .await;

    let (available_distinct, available_assignments, available_by_role) = available_tools();
    summary.tools.available_distinct = available_distinct;
    summary.tools.available_assignments = available_assignments;
    breakdowns.available_tools_by_role = available_by_role;
    summary.energy = energy_summary(&state.data_dir);

    let (episodes, skills, people, drive, custom_views) = tokio::join!(
        count_episodes(&state.data_dir),
        count_skills(&state.data_dir),
        count_people(&state.data_dir),
        count_drive(&state.data_dir),
        count_custom_views(&state.data_dir),
    );
    let inventory = Inventory {
        facets: facet_index.len() as u64,
        episodes,
        skills,
        people,
        drive,
        custom_views,
    };

    let period_from = window.from.unwrap_or_else(|| {
        earliest
            .map(start_of_day)
            .unwrap_or_else(|| start_of_day(now))
    });
    fill_series(&mut series, period_from.date_naive(), now.date_naive());

    StatsResponse {
        period: Period {
            range: window.range.label(),
            from: stamp(period_from),
            to: stamp(now),
            generated_at: stamp(now),
            timezone: "UTC",
            granularity: "day",
        },
        coverage,
        summary,
        series: series.into_values().collect(),
        breakdowns,
        inventory,
    }
}

fn fold_session_index(text: &str, current_run: &str) -> (Vec<SessionRow>, usize, usize) {
    #[derive(Clone)]
    struct Opened {
        run: String,
        session: String,
        role: String,
        worker_type: Option<String>,
        started: DateTime<Utc>,
    }

    let mut opened: HashMap<(String, String), Opened> = HashMap::new();
    let mut closed: HashSet<(String, String)> = HashSet::new();
    let mut rows = Vec::new();
    let mut records = 0;
    let mut unreadable = 0;

    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(record) = serde_json::from_str::<Record>(line) else {
            unreadable += 1;
            continue;
        };
        records += 1;
        match record {
            Record::Opened {
                run,
                session,
                at,
                role,
                worker_type,
                ..
            } => {
                let session = session.to_string();
                opened.insert(
                    (run.clone(), session.clone()),
                    Opened {
                        run,
                        session,
                        role,
                        worker_type,
                        started: at,
                    },
                );
            }
            Record::Closed {
                run,
                session,
                at,
                started,
                role,
                worker_type,
                turns,
                ..
            } => {
                let session = session.to_string();
                let key = (run.clone(), session.clone());
                if closed.insert(key) {
                    rows.push(SessionRow {
                        run,
                        session,
                        role,
                        worker_type,
                        started,
                        ended: Some(at),
                        turns: Some(turns),
                        end: SessionEnd::Closed,
                    });
                }
            }
            Record::Thread { .. } => {}
        }
    }

    rows.extend(
        opened
            .into_iter()
            .filter(|(key, _)| key.0 != current_run && !closed.contains(key))
            .map(|(_, opened)| SessionRow {
                run: opened.run,
                session: opened.session,
                role: opened.role,
                worker_type: opened.worker_type,
                started: opened.started,
                ended: None,
                turns: None,
                end: SessionEnd::Restart,
            }),
    );
    rows.sort_by(|a, b| a.started.cmp(&b.started).then(a.session.cmp(&b.session)));
    (rows, records, unreadable)
}

fn live_session_rows() -> Vec<SessionRow> {
    let run = crate::foundation::run::id().to_string();
    registry::global()
        .statuses()
        .into_iter()
        .map(|status| SessionRow {
            run: run.clone(),
            session: status.id.to_string(),
            role: status.role.as_str().to_string(),
            worker_type: status
                .role
                .worker_type()
                .map(|kind| kind.as_str().to_string()),
            started: status.started,
            ended: None,
            turns: Some(status.turns),
            end: SessionEnd::Live,
        })
        .collect()
}

fn apply_session(
    row: &SessionRow,
    window: &Window,
    summary: &mut SessionSummary,
    roles: &mut HashMap<String, SessionBreakdown>,
    worker_types: &mut HashMap<String, SessionBreakdown>,
    series: &mut BTreeMap<NaiveDate, DailyPoint>,
) {
    let worker = row.role == "worker";
    let group = if worker {
        &mut summary.worker
    } else {
        &mut summary.resident
    };
    if row.end == SessionEnd::Live {
        summary.live += 1;
        group.live += 1;
    }
    if !window.includes(row.started) {
        return;
    }

    summary.started += 1;
    group.started += 1;
    point(series, row.started).sessions += 1;
    match row.end {
        SessionEnd::Closed => {
            summary.clean_closed += 1;
            group.clean_closed += 1;
        }
        SessionEnd::Restart => {
            summary.restart_lost += 1;
            group.restart_lost += 1;
        }
        SessionEnd::Live => {}
    }
    if let Some(turns) = row.turns {
        group.turns_total = group.turns_total.saturating_add(turns);
        group.turn_samples += 1;
    }
    let duration = (row.end == SessionEnd::Closed)
        .then(|| {
            row.ended
                .map(|ended| (ended - row.started).num_seconds().max(0) as u64)
        })
        .flatten();
    if let Some(seconds) = duration {
        group.durations.push(seconds);
    }

    let role = roles
        .entry(row.role.clone())
        .or_insert_with(|| SessionBreakdown {
            name: row.role.clone(),
            ..SessionBreakdown::default()
        });
    role.count += 1;
    role.turns = role.turns.saturating_add(row.turns.unwrap_or(0));
    if let Some(seconds) = duration {
        role.durations.push(seconds);
    }
    if let Some(kind) = &row.worker_type {
        let worker_type = worker_types
            .entry(kind.clone())
            .or_insert_with(|| SessionBreakdown {
                name: kind.clone(),
                ..SessionBreakdown::default()
            });
        worker_type.count += 1;
        worker_type.turns = worker_type.turns.saturating_add(row.turns.unwrap_or(0));
        if let Some(seconds) = duration {
            worker_type.durations.push(seconds);
        }
    }
}

fn finish_session_group(group: &mut SessionGroup) {
    group.duration_samples = group.durations.len() as u64;
    group.average_duration_seconds = average(&group.durations);
    group.median_duration_seconds = median(&mut group.durations);
    group.average_turns =
        (group.turn_samples > 0).then(|| group.turns_total as f64 / group.turn_samples as f64);
}

fn finish_session_breakdowns(rows: HashMap<String, SessionBreakdown>) -> Vec<SessionBreakdown> {
    let mut rows: Vec<SessionBreakdown> = rows
        .into_values()
        .map(|mut row| {
            row.average_duration_seconds = average(&row.durations);
            row
        })
        .collect();
    rows.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
    rows
}

/// One session's frame log, named before anything is read.
struct FrameLog {
    path: PathBuf,
    /// Role and worker type from the session index, when the index knows this session.
    /// `None` falls back to the `role` the log's own first frame carries.
    meta: Option<(String, Option<String>)>,
}

/// What one frame log contributed, kept apart so logs can be scanned concurrently.
#[derive(Debug, Default)]
struct FrameScan {
    agg: FrameAggregation,
    series: BTreeMap<NaiveDate, DailyPoint>,
    earliest: Option<DateTime<Utc>>,
}

/// How many logs to have in flight. Each is a `spawn_blocking` doing JSON parsing rather
/// than waiting on I/O, so the useful width is the machine's parallelism — and the cap
/// bounds how many whole logs are resident at once.
fn scan_width() -> usize {
    std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(2, 16)
}

async fn aggregate_frames(
    data_dir: &Path,
    window: &Window,
    session_meta: &HashMap<(String, String), (String, Option<String>)>,
    series: &mut BTreeMap<NaiveDate, DailyPoint>,
    earliest: &mut Option<DateTime<Utc>>,
) -> FrameAggregation {
    use futures::stream::StreamExt as _;

    let root = layout::raw_root(data_dir).join(layout::SESSIONS_DIR);
    let mut out = FrameAggregation::default();
    let mut runs = match tokio::fs::read_dir(&root).await {
        Ok(runs) => runs,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return out,
        Err(error) => {
            tracing::warn!(error = %error, path = %root.display(), "stats could not read session logs");
            return out;
        }
    };
    // A log's mtime is when its last frame was appended, so a log that stopped before the
    // window began holds nothing the window wants. Cheaper than opening it to find out,
    // and the whole reason a 7-day question does not cost a year of frames.
    let cutoff = window.from.map(SystemTime::from);
    let mut pending: Vec<FrameLog> = Vec::new();
    while let Ok(Some(run_entry)) = runs.next_entry().await {
        let Ok(file_type) = run_entry.file_type().await else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let run = run_entry.file_name().to_string_lossy().into_owned();
        let mut logs = match tokio::fs::read_dir(run_entry.path()).await {
            Ok(logs) => logs,
            Err(_) => continue,
        };
        while let Ok(Some(log)) = logs.next_entry().await {
            let name = log.file_name().to_string_lossy().into_owned();
            let Some(session) = name.strip_suffix(".jsonl").map(str::to_string) else {
                continue;
            };
            let Ok(metadata) = log.metadata().await else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            out.frame_logs += 1;
            let last_written = metadata.modified().ok();
            if let (Some(cutoff), Some(last_written)) = (cutoff, last_written)
                && last_written < cutoff
            {
                out.frame_logs_out_of_window += 1;
                continue;
            }
            pending.push(FrameLog {
                meta: session_meta.get(&(run.clone(), session)).cloned(),
                path: log.path(),
            });
        }
    }

    let scans = futures::stream::iter(pending)
        .map(|log| {
            let window = window.clone();
            async move {
                tokio::task::spawn_blocking(move || scan_frame_log(&log, &window))
                    .await
                    .unwrap_or_default()
            }
        })
        .buffer_unordered(scan_width())
        .collect::<Vec<FrameScan>>()
        .await;
    for scan in scans {
        out.absorb(scan.agg);
        for (date, point) in scan.series {
            let into = series.entry(date).or_insert_with(|| DailyPoint {
                date: date.to_string(),
                ..DailyPoint::default()
            });
            merge_point(into, &point);
        }
        if let Some(at) = scan.earliest {
            note_earliest(earliest, at);
        }
    }
    out
}

fn merge_point(into: &mut DailyPoint, from: &DailyPoint) {
    into.tokens = into.tokens.saturating_add(from.tokens);
    into.turns += from.turns;
    into.sessions += from.sessions;
    into.tool_calls += from.tool_calls;
    into.user_messages += from.user_messages;
    into.agent_replies += from.agent_replies;
    into.task_completions += from.task_completions;
}

/// The envelope the tap writes around each wire line, read for the four fields the scan
/// wants. Everything else in the frame is skipped by the deserializer rather than built
/// into a tree, which is most of why a pass over a large log is cheap.
///
/// `ts`, `dir` and `role` borrow out of the line: the tap writes an RFC3339 stamp, a
/// direction word and a role word, none of which can contain a JSON escape. `raw` is a
/// JSON document inside a JSON string, so it always has escapes and always allocates —
/// which is fine, since anything that reads it needs it unescaped anyway.
#[derive(Deserialize)]
struct FrameEnvelope<'a> {
    #[serde(default, borrow)]
    ts: Option<&'a str>,
    #[serde(default, borrow)]
    dir: Option<&'a str>,
    #[serde(default, borrow)]
    role: Option<&'a str>,
    #[serde(default, borrow)]
    raw: Option<Cow<'a, str>>,
}

/// A wire line read for its method alone, to decide whether it is worth parsing properly.
#[derive(Deserialize)]
struct WireMethod<'a> {
    #[serde(default, borrow)]
    method: Option<Cow<'a, str>>,
}

/// One item, accumulated across the frames that carry its id — the same message
/// [`messages::fold`] would produce, reduced to what a count needs.
struct ItemTally {
    kind: &'static str,
    at: Option<DateTime<Utc>>,
    status: Option<String>,
    tool: Option<String>,
    errored: bool,
}

fn scan_frame_log(log: &FrameLog, window: &Window) -> FrameScan {
    let mut scan = FrameScan::default();
    let text = match std::fs::read_to_string(&log.path) {
        Ok(text) => text,
        Err(error) => {
            scan.agg.frame_logs_unreadable = 1;
            tracing::warn!(error = %error, path = %log.path.display(), "stats could not read a session log");
            return scan;
        }
    };
    scan_frame_text(log.meta.clone(), &text, window, &mut scan);
    scan
}

fn scan_frame_text(
    meta: Option<(String, Option<String>)>,
    text: &str,
    window: &Window,
    scan: &mut FrameScan,
) {
    let out = &mut scan.agg;
    let mut items: HashMap<String, ItemTally> = HashMap::new();
    let mut turn = 0u64;
    let mut token_turns: HashSet<u64> = HashSet::new();
    let mut previous_total: Option<Usage> = None;
    let mut role = meta.as_ref().map(|(role, _)| role.clone());
    let worker_type = meta.and_then(|(_, worker_type)| worker_type);
    let mut first_ts_seen = false;

    for (index, line) in text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
    {
        let Ok(envelope) = serde_json::from_str::<FrameEnvelope>(line) else {
            out.unreadable_frames += 1;
            continue;
        };
        let at = envelope.ts.and_then(parse_ts);
        // Frames are appended in order, so the first datable one is this log's earliest —
        // one parse rather than one per frame.
        if !first_ts_seen && let Some(at) = at {
            first_ts_seen = true;
            scan.earliest = Some(scan.earliest.map_or(at, |current| current.min(at)));
        }
        if role.is_none() {
            role = envelope.role.map(str::to_string);
        }
        // stderr is not JSON-RPC — it is whatever the subprocess printed. `fold` reads it
        // as a message; nothing here counts one, and it is not unreadable either.
        if envelope.dir == Some("stderr") {
            continue;
        }
        let raw = envelope.raw.as_deref().unwrap_or_default();
        let Ok(wire) = serde_json::from_str::<WireMethod>(raw) else {
            out.unreadable_frames += 1;
            continue;
        };
        match wire.method.as_deref().unwrap_or_default() {
            "turn/started" => {
                turn += 1;
                if let Some(at) = at
                    && window.includes(at)
                {
                    point(&mut scan.series, at).turns += 1;
                }
            }
            "thread/tokenUsage/updated" => {
                let Ok(body) = serde_json::from_str::<Value>(raw) else {
                    continue;
                };
                let params = body.get("params").unwrap_or(&Value::Null);
                let usage_root = params
                    .get("tokenUsage")
                    .or_else(|| params.get("usage"))
                    .unwrap_or(params);
                let total = usage_root.get("total").and_then(parse_usage);
                let exact = usage_root.get("last").and_then(parse_usage).or_else(|| {
                    (usage_root.get("inputTokens").is_some()
                        || usage_root.get("outputTokens").is_some())
                    .then(|| parse_usage(usage_root))
                    .flatten()
                });
                let (usage, estimated) = match (exact, total) {
                    (Some(usage), _) => (usage, false),
                    (None, Some(total)) => {
                        let delta =
                            previous_total.map_or(total, |before| total.positive_delta(before));
                        (delta, true)
                    }
                    (None, None) => continue,
                };
                if let Some(total) = total {
                    previous_total = Some(total);
                }
                let Some(at) = at else { continue };
                if !window.includes(at) {
                    continue;
                }
                usage.add_to(&mut out.tokens);
                let daily = point(&mut scan.series, at);
                daily.tokens = daily.tokens.saturating_add(usage.total);
                token_turns.insert(turn);
                out.token_updates += 1;
                if estimated {
                    out.estimated_token_updates += 1;
                }
            }
            "item/started" | "item/updated" | "item/completed" => {
                let Ok(body) = serde_json::from_str::<Value>(raw) else {
                    continue;
                };
                let Some(item) = body.get("params").and_then(|params| params.get("item")) else {
                    continue;
                };
                // An item with no id cannot be folded onto later, so it is keyed by the
                // frame it arrived on — one message, never merged, exactly as `fold` does.
                let key = item
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("seq:{index}"));
                let tally = items.entry(key).or_insert_with(|| ItemTally {
                    kind: messages::kind_of(item),
                    at,
                    status: None,
                    tool: None,
                    errored: false,
                });
                if let Some(status) = item.get("status").and_then(Value::as_str) {
                    tally.status = Some(status.to_string());
                }
                if let Some(tool) = item.get("tool").and_then(Value::as_str) {
                    tally.tool = Some(tool.to_string());
                }
                tally.errored |= item.get("error").is_some_and(|error| !error.is_null());
            }
            // Deltas and protocol housekeeping. A delta's payload is never parsed: it is
            // a fragment of an item whose own frames already say everything a count needs,
            // and there are more of them than of everything else put together.
            _ => {}
        }
    }

    let role = role.unwrap_or_else(|| "unknown".to_string());
    for tally in items.into_values() {
        let Some(at) = tally.at else { continue };
        if !window.includes(at) {
            continue;
        }
        match tally.kind {
            "tool" => {
                out.tools.calls += 1;
                point(&mut scan.series, at).tool_calls += 1;
                let name = tally
                    .tool
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| "unknown".to_string());
                let failed = tally.status.as_deref() == Some("failed") || tally.errored;
                if failed {
                    out.tools.failed_calls += 1;
                }
                let entry = out.tool_names.entry(name).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += u64::from(failed);
                *out.tool_roles.entry(role.clone()).or_default() += 1;
                if let Some(worker_type) = &worker_type {
                    *out.tool_worker_types.entry(worker_type.clone()).or_default() += 1;
                }
                let status = tally.status.unwrap_or_else(|| "unknown".to_string());
                *out.tool_statuses.entry(status).or_default() += 1;
            }
            "command" => out.tools.commands += 1,
            "edit" => out.tools.edits += 1,
            "search" => out.tools.web_searches += 1,
            "compaction" => out.tools.context_compactions += 1,
            _ => {}
        }
    }
    out.token_turns = token_turns.len();
    out.token_sessions = usize::from(!token_turns.is_empty());
}

fn parse_usage(value: &Value) -> Option<Usage> {
    let input = number(value, &["inputTokens", "input_tokens"]).unwrap_or(0);
    let output = number(value, &["outputTokens", "output_tokens"]).unwrap_or(0);
    let cached_input = number(value, &["cachedInputTokens", "cached_input_tokens"]).unwrap_or(0);
    let reasoning_output =
        number(value, &["reasoningOutputTokens", "reasoning_output_tokens"]).unwrap_or(0);
    let total = number(value, &["totalTokens", "total_tokens"]).unwrap_or(input + output);
    (total > 0 || input > 0 || output > 0 || cached_input > 0 || reasoning_output > 0).then_some(
        Usage {
            total,
            input,
            output,
            cached_input,
            reasoning_output,
        },
    )
}

fn number(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_u64))
}

fn apply_journal(
    entries: &[JournalEntry],
    window: &Window,
    summary: &mut ConversationSummary,
    series: &mut BTreeMap<NaiveDate, DailyPoint>,
    earliest: &mut Option<DateTime<Utc>>,
) {
    let mut active_days = BTreeSet::new();
    let mut human_times = Vec::new();
    for entry in entries {
        let at = journal::entry_ts(entry);
        note_earliest(earliest, at);
        if !window.includes(at) {
            continue;
        }
        match entry {
            JournalEntry::SignalIn { channel, .. } => {
                let human = match channel {
                    Channel::Text => {
                        summary.human_text_messages += 1;
                        true
                    }
                    Channel::Audio => {
                        summary.human_audio_messages += 1;
                        true
                    }
                    Channel::File => {
                        summary.handed_files += 1;
                        true
                    }
                    _ => false,
                };
                if human {
                    summary.human_messages += 1;
                    human_times.push(at);
                    active_days.insert(at.date_naive());
                    point(series, at).user_messages += 1;
                }
            }
            JournalEntry::SignalOut {
                channel: Channel::Text,
                ..
            } => {
                summary.agent_text_replies += 1;
                active_days.insert(at.date_naive());
                point(series, at).agent_replies += 1;
            }
            JournalEntry::SignalOut { .. } => {}
        }
    }
    human_times.sort();
    summary.conversation_windows = human_times
        .iter()
        .enumerate()
        .filter(|(index, at)| {
            *index == 0
                || **at - human_times[index - 1] > Duration::minutes(CONVERSATION_GAP_MINUTES)
        })
        .count() as u64;
    summary.active_days = active_days.len() as u64;
    summary.window_gap_minutes = CONVERSATION_GAP_MINUTES;
}

fn apply_facets(index: &[String], breakdowns: &mut Breakdowns, tasks: &mut TaskSummary) {
    let mut dimensions: HashMap<String, u64> = HashMap::new();
    for facet_ref in index {
        let Some((dimension, _)) = facet_ref.split_once('/') else {
            continue;
        };
        *dimensions.entry(dimension.to_string()).or_default() += 1;
    }
    tasks.total = dimensions.get(tasks::DIMENSION).copied().unwrap_or(0);
    breakdowns.facets_by_dimension = sorted_counts(dimensions);
}

#[allow(clippy::too_many_arguments)]
async fn apply_tasks(
    data_dir: &Path,
    index: &[String],
    window: &Window,
    now: DateTime<Utc>,
    summary: &mut TaskSummary,
    coverage: &mut Coverage,
    breakdowns: &mut Breakdowns,
    series: &mut BTreeMap<NaiveDate, DailyPoint>,
    earliest: &mut Option<DateTime<Utc>>,
) {
    let prefix = format!("{}/", tasks::DIMENSION);
    let mut statuses: HashMap<String, u64> = HashMap::new();
    let mut completion_durations = Vec::new();
    for facet_ref in index {
        let Some(subject) = facet_ref.strip_prefix(&prefix) else {
            continue;
        };
        let task = match tasks::read_task(data_dir, subject).await {
            Ok(Some(task)) => task,
            Ok(None) => continue,
            Err(error) => {
                coverage.task_unreadable += 1;
                tracing::warn!(error = %error, subject, "stats could not read a task");
                continue;
            }
        };
        coverage.task_records += 1;
        *statuses
            .entry(task.status.as_str().to_string())
            .or_default() += 1;
        if let Some(created) = task.created_at {
            note_earliest(earliest, created);
            if window.includes(created) {
                summary.created += 1;
            }
        }
        if let Some(completed) = task.completed_at {
            note_earliest(earliest, completed);
            if window.includes(completed) {
                summary.completed += 1;
                point(series, completed).task_completions += 1;
                if let Some(created) = task.created_at {
                    completion_durations.push((completed - created).num_seconds().max(0) as u64);
                }
            }
        }
        if let Some(cancelled) = task.cancelled_at {
            note_earliest(earliest, cancelled);
            if window.includes(cancelled) {
                summary.cancelled += 1;
            }
        }
        if task.status.is_active() && task.due_at.is_some_and(|due| due <= now) {
            summary.overdue += 1;
        }
        if task.status == tasks::TaskStatus::Doing
            && task
                .status_since
                .is_some_and(|since| now - since >= Duration::hours(STALLED_TASK_HOURS))
        {
            summary.stalled_doing += 1;
        }
        if task.status == tasks::TaskStatus::Serving && task.checked_at.is_none() {
            summary.serving_never_confirmed += 1;
        }
    }
    summary.completion_samples = completion_durations.len() as u64;
    summary.average_completion_seconds = average(&completion_durations);
    summary.median_completion_seconds = median(&mut completion_durations);
    breakdowns.tasks_by_status = sorted_counts(statuses);
}

fn available_tools() -> (u64, u64, Vec<CountBreakdown>) {
    let mut distinct = BTreeSet::new();
    let mut assignments = 0u64;
    let mut by_role = Vec::new();
    for role in tools::ROLES {
        let role_arg = (role != "other").then_some(role);
        let declarations = crate::foundation::mcp::tools_for_role(role_arg);
        for declaration in &declarations {
            if let Some(name) = declaration.get("name").and_then(Value::as_str) {
                distinct.insert(name.to_string());
            }
        }
        assignments += declarations.len() as u64;
        by_role.push(CountBreakdown {
            name: role.to_string(),
            count: declarations.len() as u64,
        });
    }
    (distinct.len() as u64, assignments, by_role)
}

fn energy_summary(data_dir: &Path) -> EnergySummary {
    let credentials = Credentials::load(data_dir);
    let managed = credentials.mode == Mode::Xiaoyuanzhu;
    let energy = managed.then_some(credentials.energy).flatten();
    EnergySummary {
        mode: match credentials.mode {
            Mode::Byok => "byok",
            Mode::Xiaoyuanzhu => "xiaoyuanzhu",
        }
        .to_string(),
        available: energy.is_some(),
        remaining: energy.as_ref().map(|energy| energy.remaining),
        total: energy.as_ref().map(|energy| energy.total),
        resets_at: energy
            .as_ref()
            .map(|energy| energy.resets_at.trim().to_string())
            .filter(|value| !value.is_empty()),
        tier: energy
            .as_ref()
            .map(|energy| energy.tier.trim().to_string())
            .filter(|value| !value.is_empty()),
        observed_out_of_energy: crate::foundation::energy_state::is_out(),
        checked_at: get_setting(data_dir, KEY_BROKER_CHECKED_AT),
        historical_usage_available: false,
    }
}

async fn count_episodes(data_dir: &Path) -> u64 {
    let root = layout::episodes_dir(data_dir);
    let mut entries = match tokio::fs::read_dir(root).await {
        Ok(entries) => entries,
        Err(_) => return 0,
    };
    let mut count = 0;
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.file_type().await.is_ok_and(|kind| kind.is_dir())
            && tokio::fs::try_exists(entry.path().join("episode.md"))
                .await
                .unwrap_or(false)
        {
            count += 1;
        }
    }
    count
}

async fn count_skills(data_dir: &Path) -> SkillInventory {
    let root = crate::mind::skills::skills_dir(data_dir);
    let mut out = SkillInventory::default();
    let mut stack = vec![(root, false)];
    while let Some((dir, in_factory)) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type().await else {
                continue;
            };
            if kind.is_dir() {
                stack.push((entry.path(), in_factory || name == "factory"));
            } else if kind.is_file() && entry.path().extension().is_some_and(|ext| ext == "md") {
                out.total += 1;
                if in_factory {
                    out.factory += 1;
                } else {
                    out.learned += 1;
                }
            }
        }
    }
    out
}

async fn count_people(data_dir: &Path) -> PeopleInventory {
    let Ok(people) = crate::mind::memory::people_vectors::list_clusters(data_dir).await else {
        return PeopleInventory::default();
    };
    PeopleInventory {
        clusters: people.len() as u64,
        named: people.iter().filter(|person| person.named).count() as u64,
        recurring: people.iter().filter(|person| person.occasions >= 2).count() as u64,
        face_samples: people
            .iter()
            .map(|person| person.face_stems.len() as u64)
            .sum(),
        voice_samples: people
            .iter()
            .map(|person| person.voice_stems.len() as u64)
            .sum(),
    }
}

async fn count_drive(data_dir: &Path) -> DriveInventory {
    let root = crate::mind::memory::media::drive_root(data_dir);
    let mut out = DriveInventory::default();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type().await else {
                continue;
            };
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file()
                && let Ok(metadata) = entry.metadata().await
            {
                out.files += 1;
                out.bytes = out.bytes.saturating_add(metadata.len());
            }
        }
    }
    out
}

async fn count_custom_views(data_dir: &Path) -> u64 {
    let root = data_dir.join("views");
    let mut count = 0;
    let mut stack = vec![(root, String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let Ok(kind) = entry.file_type().await else {
                continue;
            };
            if kind.is_dir() {
                if rel == "factory" || rel == "_compiled" {
                    continue;
                }
                stack.push((entry.path(), rel));
            } else if kind.is_file() && entry.path().extension().is_some_and(|ext| ext == "jsx") {
                count += 1;
            }
        }
    }
    count
}

fn sorted_counts(counts: HashMap<String, u64>) -> Vec<CountBreakdown> {
    let mut rows: Vec<CountBreakdown> = counts
        .into_iter()
        .map(|(name, count)| CountBreakdown { name, count })
        .collect();
    rows.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
    rows
}

fn sorted_tools(tools: HashMap<String, (u64, u64)>) -> Vec<ToolBreakdown> {
    let mut rows: Vec<ToolBreakdown> = tools
        .into_iter()
        .map(|(name, (count, failed))| ToolBreakdown {
            name,
            count,
            failed,
        })
        .collect();
    rows.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
    rows
}

fn average(values: &[u64]) -> Option<f64> {
    (!values.is_empty()).then(|| {
        values
            .iter()
            .copied()
            .map(|value| value as f64)
            .sum::<f64>()
            / values.len() as f64
    })
}

fn median(values: &mut [u64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        Some((values[middle - 1] as f64 + values[middle] as f64) / 2.0)
    } else {
        Some(values[middle] as f64)
    }
}

fn parse_ts(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn start_of_day(at: DateTime<Utc>) -> DateTime<Utc> {
    Utc.from_utc_datetime(
        &at.date_naive()
            .and_hms_opt(0, 0, 0)
            .expect("midnight is valid"),
    )
}

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn note_earliest(earliest: &mut Option<DateTime<Utc>>, at: DateTime<Utc>) {
    *earliest = Some(earliest.map_or(at, |current| current.min(at)));
}

fn point(series: &mut BTreeMap<NaiveDate, DailyPoint>, at: DateTime<Utc>) -> &mut DailyPoint {
    let date = at.date_naive();
    series.entry(date).or_insert_with(|| DailyPoint {
        date: date.to_string(),
        ..DailyPoint::default()
    })
}

fn fill_series(series: &mut BTreeMap<NaiveDate, DailyPoint>, from: NaiveDate, through: NaiveDate) {
    let mut date = from;
    while date <= through {
        series.entry(date).or_insert_with(|| DailyPoint {
            date: date.to_string(),
            ..DailyPoint::default()
        });
        let Some(next) = date.succ_opt() else {
            break;
        };
        date = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, day, hour, 0, 0)
            .single()
            .unwrap()
    }

    fn frame(seq: u64, ts: DateTime<Utc>, body: Value) -> String {
        json!({
            "seq": seq,
            "ts": ts.to_rfc3339(),
            "role": "worker",
            "dir": "recv",
            "raw": body.to_string(),
        })
        .to_string()
    }

    /// One log's worth of frames, scanned the way [`aggregate_frames`] scans each file —
    /// with no session index, so the role comes from the frames themselves.
    fn scan(text: &str, window: &Window) -> FrameScan {
        let mut scan = FrameScan::default();
        scan_frame_text(None, text, window, &mut scan);
        scan
    }

    #[test]
    fn ranges_are_calendar_days_and_invalid_values_are_rejected() {
        let now = at(18, 14);
        assert_eq!(Range::parse(None), Some(Range::Days(30, "30d")));
        assert_eq!(Range::parse(Some("7D")), Some(Range::Days(7, "7d")));
        assert_eq!(
            Range::Days(7, "7d").from(now).unwrap(),
            Utc.with_ymd_and_hms(2026, 8, 12, 0, 0, 0).unwrap()
        );
        assert_eq!(Range::parse(Some("forever")), None);
    }

    #[test]
    fn cumulative_token_totals_are_differenced_not_summed() {
        let text = [
            frame(
                1,
                at(17, 10),
                json!({"method":"turn/started","params":{"turn":{}}}),
            ),
            frame(
                2,
                at(17, 10),
                json!({"method":"thread/tokenUsage/updated","params":{"tokenUsage":{
                    "total":{"inputTokens":80,"outputTokens":20,"totalTokens":100}
                }}}),
            ),
            frame(
                3,
                at(17, 11),
                json!({"method":"turn/started","params":{"turn":{}}}),
            ),
            frame(
                4,
                at(17, 11),
                json!({"method":"thread/tokenUsage/updated","params":{"tokenUsage":{
                    "total":{"inputTokens":200,"outputTokens":50,"totalTokens":250}
                }}}),
            ),
        ]
        .join("\n");
        let window = Window {
            range: Range::All,
            from: None,
            to: at(18, 0),
        };
        let scan = scan(&text, &window);
        assert_eq!(
            scan.agg.tokens.total, 250,
            "100 + (250 - 100), never 100 + 250"
        );
        assert_eq!(scan.agg.tokens.input, 200);
        assert_eq!(scan.agg.tokens.output, 50);
        assert_eq!(scan.agg.estimated_token_updates, 2);
        assert_eq!(scan.agg.token_turns, 2, "one per turn that reported usage");
        assert_eq!(scan.agg.token_sessions, 1);
    }

    #[test]
    fn exact_last_usage_wins_over_the_cumulative_total() {
        let text = frame(
            1,
            at(17, 10),
            json!({"method":"thread/tokenUsage/updated","params":{"tokenUsage":{
                "total":{"inputTokens":800,"outputTokens":200,"totalTokens":1000},
                "last":{"inputTokens":80,"outputTokens":20,"totalTokens":100}
            }}}),
        );
        let window = Window {
            range: Range::All,
            from: None,
            to: at(18, 0),
        };
        let scan = scan(&text, &window);
        assert_eq!(scan.agg.tokens.total, 100);
        assert_eq!(scan.agg.estimated_token_updates, 0);
    }

    /// The claim the whole scan rests on: it counts exactly what folding the same log and
    /// counting the messages would count. Asserted against [`messages::fold`] itself over
    /// a log carrying every shape that behaves differently — a streamed item, an item with
    /// no id, an item completed twice, stderr (a message to `fold`, nothing to count), and
    /// two lines that are not readable as frames at all.
    #[test]
    fn the_scan_counts_what_folding_the_same_log_would_count() {
        let item = |id: Option<&str>, kind: &str, status: &str| {
            let mut item = json!({"type": kind, "status": status});
            if let Some(id) = id {
                item["id"] = id.into();
            }
            if kind == "mcpToolCall" {
                item["tool"] = "hi_recall".into();
            }
            item
        };
        let mut lines = vec![
            frame(
                1,
                at(17, 10),
                json!({"method":"turn/started","params":{"turn":{}}}),
            ),
            frame(2, at(17, 10), json!({"method":"item/started","params":{"item": item(Some("a"), "mcpToolCall", "inProgress")}})),
            frame(3, at(17, 10), json!({"method":"item/agentMessage/delta","params":{"itemId":"a","delta":"frag"}})),
            frame(4, at(17, 10), json!({"method":"item/completed","params":{"item": item(Some("a"), "mcpToolCall", "completed")}})),
            frame(5, at(17, 10), json!({"method":"item/completed","params":{"item": item(Some("a"), "mcpToolCall", "completed")}})),
            frame(6, at(17, 11), json!({"method":"item/completed","params":{"item": item(None, "commandExecution", "completed")}})),
            frame(7, at(17, 11), json!({"method":"item/completed","params":{"item": item(None, "commandExecution", "failed")}})),
            frame(8, at(17, 11), json!({"method":"item/completed","params":{"item": item(Some("b"), "webSearch", "completed")}})),
            frame(9, at(17, 12), json!({"method":"item/completed","params":{"item": item(Some("c"), "fileChange", "completed")}})),
            frame(10, at(17, 12), json!({"method":"item/completed","params":{"item": item(Some("d"), "contextCompaction", "completed")}})),
            frame(11, at(17, 12), json!({"method":"item/completed","params":{"item": item(Some("e"), "agentMessage", "completed")}})),
            "{ not json at all".to_string(),
        ];
        // A frame whose `raw` is not JSON-RPC, which is what stderr is — and the same
        // shape with a direction that says it is not stderr, which is unreadable.
        lines.push(
            json!({"seq": 12, "ts": at(17, 12).to_rfc3339(), "role": "worker", "dir": "stderr", "raw": "thread 'main' panicked"})
                .to_string(),
        );
        lines.push(
            json!({"seq": 13, "ts": at(17, 12).to_rfc3339(), "role": "worker", "dir": "recv", "raw": "not json either"})
                .to_string(),
        );
        let text = lines.join("\n");
        let window = Window {
            range: Range::All,
            from: None,
            to: at(18, 0),
        };
        let scan = scan(&text, &window);

        // What the old reading was: fold the log, then count the messages by kind.
        let folded = messages::fold(&text);
        let count = |kind: &str| {
            folded
                .messages
                .iter()
                .filter(|message| message.kind == kind)
                .count() as u64
        };
        assert_eq!(scan.agg.tools.calls, count("tool"));
        assert_eq!(scan.agg.tools.commands, count("command"));
        assert_eq!(scan.agg.tools.edits, count("edit"));
        assert_eq!(scan.agg.tools.web_searches, count("search"));
        assert_eq!(scan.agg.tools.context_compactions, count("compaction"));
        assert_eq!(scan.agg.unreadable_frames, folded.unreadable);
        assert_eq!(
            scan.series.values().map(|point| point.turns).sum::<u64>(),
            folded.turns.len() as u64
        );
        // …and the counts are the ones a person would arrive at by hand.
        assert_eq!(scan.agg.tools.calls, 1, "one id, five frames, one call");
        assert_eq!(scan.agg.tools.commands, 2, "no id means never merged");
        assert_eq!(scan.agg.unreadable_frames, 2, "stderr is not unreadable");
    }

    /// The scan is `fold`'s reading minus the text, so the two must agree on how many
    /// items happened and what each one was — including that a streamed item whose
    /// fragments span hundreds of frames is one call, not hundreds.
    #[test]
    fn a_streamed_tool_call_counts_once_and_carries_its_name() {
        let item = |status: &str, error: Value| {
            json!({"id":"tc-1","type":"mcpToolCall","server":"hi","tool":"hi_say","status":status,"error":error})
        };
        let text = [
            frame(
                1,
                at(17, 10),
                json!({"method":"turn/started","params":{"turn":{}}}),
            ),
            frame(
                2,
                at(17, 10),
                json!({"method":"item/started","params":{"item": item("inProgress", Value::Null)}}),
            ),
            frame(
                3,
                at(17, 10),
                json!({"method":"item/agentMessage/delta","params":{"itemId":"tc-1","delta":"…"}}),
            ),
            frame(
                4,
                at(17, 10),
                json!({"method":"item/completed","params":{"item": item("failed", json!("boom"))}}),
            ),
            frame(
                5,
                at(17, 11),
                json!({"method":"item/completed","params":{"item":{"id":"cmd-1","type":"commandExecution","status":"completed"}}}),
            ),
        ]
        .join("\n");
        let window = Window {
            range: Range::All,
            from: None,
            to: at(18, 0),
        };
        let scan = scan(&text, &window);
        assert_eq!(scan.agg.tools.calls, 1, "one item id is one call");
        assert_eq!(scan.agg.tools.failed_calls, 1);
        assert_eq!(scan.agg.tools.commands, 1);
        assert_eq!(scan.agg.tool_names.get("hi_say").copied(), Some((1, 1)));
        assert_eq!(scan.agg.tool_roles.get("worker").copied(), Some(1));
        assert_eq!(
            messages::fold(&text)
                .messages
                .iter()
                .filter(|message| message.kind == "tool")
                .count(),
            1,
            "and `fold` reads the same log the same way"
        );
    }

    /// A log whose last frame predates the range is not opened at all — the reason a
    /// short range no longer costs the whole history.
    #[tokio::test]
    async fn a_log_older_than_the_range_is_skipped_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let run = layout::raw_root(dir.path())
            .join(layout::SESSIONS_DIR)
            .join("run-1");
        std::fs::create_dir_all(&run).unwrap();
        let text = frame(
            1,
            at(1, 10),
            json!({"method":"thread/tokenUsage/updated","params":{"tokenUsage":{
                "last":{"inputTokens":80,"outputTokens":20,"totalTokens":100}
            }}}),
        );
        let path = run.join("worker-a.jsonl");
        std::fs::write(&path, format!("{text}\n")).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::from(at(1, 11)))
            .unwrap();

        let window = Window {
            range: Range::Days(7, "7d"),
            from: Some(at(17, 0)),
            to: at(18, 0),
        };
        let out = aggregate_frames(
            dir.path(),
            &window,
            &HashMap::new(),
            &mut BTreeMap::new(),
            &mut None,
        )
        .await;
        assert_eq!(out.frame_logs, 1);
        assert_eq!(out.frame_logs_out_of_window, 1);
        assert_eq!(out.token_updates, 0, "never opened, so nothing was read");

        let all = Window {
            range: Range::All,
            from: None,
            to: at(18, 0),
        };
        let out = aggregate_frames(
            dir.path(),
            &all,
            &HashMap::new(),
            &mut BTreeMap::new(),
            &mut None,
        )
        .await;
        assert_eq!(out.frame_logs_out_of_window, 0, "`all` skips nothing");
        assert_eq!(out.tokens.total, 100);
    }

    #[test]
    fn a_closed_session_is_not_also_reported_as_restart_lost() {
        let opened = Record::Opened {
            run: "old-run".into(),
            session: "worker-a".parse().unwrap(),
            subject: None,
            at: at(17, 9),
            role: "worker".into(),
            worker_type: Some("general".into()),
            title: None,
            owner: None,
        };
        let closed = Record::Closed {
            run: "old-run".into(),
            session: "worker-a".parse().unwrap(),
            subject: None,
            at: at(17, 10),
            started: at(17, 9),
            role: "worker".into(),
            worker_type: Some("general".into()),
            title: None,
            owner: None,
            turns: 3,
        };
        let text = format!(
            "{}\n{}\n",
            serde_json::to_string(&opened).unwrap(),
            serde_json::to_string(&closed).unwrap()
        );
        let (rows, records, unreadable) = fold_session_index(&text, "current-run");
        assert_eq!(records, 2);
        assert_eq!(unreadable, 0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].end, SessionEnd::Closed);
    }

    #[test]
    fn conversation_windows_split_after_thirty_minutes() {
        let entry = |id: &str, ts| JournalEntry::SignalIn {
            id: id.into(),
            ts,
            channel: Channel::Text,
            body: "hi".into(),
            stream: None,
            media: None,
            origin: None,
            sender: None,
        };
        let entries = vec![
            entry("1", at(17, 9)),
            entry("2", at(17, 9) + Duration::minutes(30)),
            entry("3", at(17, 9) + Duration::minutes(61)),
        ];
        let window = Window {
            range: Range::All,
            from: None,
            to: at(18, 0),
        };
        let mut summary = ConversationSummary::default();
        apply_journal(
            &entries,
            &window,
            &mut summary,
            &mut BTreeMap::new(),
            &mut None,
        );
        assert_eq!(summary.human_messages, 3);
        assert_eq!(summary.conversation_windows, 2);
    }
}
