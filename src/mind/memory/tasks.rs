//! Durable tasks: one lifecycle, stored as ordinary facets.
//!
//! A task lives at `memory/facets/tasks/<subject>/facet.md`. The machine-readable
//! shape is deliberately small:
//!
//! ```text
//! status: todo | doing | serving | done | cancelled
//! created_at: <RFC3339>
//! ```
//!
//! `todo` and `doing` promise an ending; `serving` promises presence. A duty being kept
//! up — a watch, a listener, a backup that runs — never finishes, so judging it by how
//! long it has been open says nothing, and offering to mark it done says the wrong thing.
//! It closes by being stood down, not by being completed.
//!
//! `due_at`, `checked_at`, `completed_at`, and `cancelled_at` are optional lifecycle
//! timestamps. A `serving` task may also carry a liveness contract (`verify`, `restart`,
//! `owner`, `start_key`) describing how to tell its machinery is still alive. That
//! contract is how a duty is checked, not what makes it a duty: the status is.
//!
//! Older records using `kind` plus `state` remain readable. New writes emit only the
//! status taxonomy.

use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, TimeZone, Utc};

use super::episodes::{frontmatter_field, jstr, strip_frontmatter};
use super::{facets, layout};

pub const DIMENSION: &str = "tasks";
pub const PROJECTED_TASKS: usize = 12;
const PROJECTED_LINE_CHARS: usize = 120;

/// The complete task lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Todo,
    Doing,
    Serving,
    Done,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Doing => "doing",
            Self::Serving => "serving",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }

    /// Unknown values remain active and visible.
    fn parse(s: &str) -> Self {
        match s.trim() {
            "doing" => Self::Doing,
            "serving" => Self::Serving,
            "done" => Self::Done,
            "cancelled" => Self::Cancelled,
            _ => Self::Todo,
        }
    }

    /// A duty is never closed by standing there being kept — `serving` is as open as
    /// `doing`, and dropping it from the active set is how a watch goes unwatched.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Todo | Self::Doing | Self::Serving)
    }
}

/// Optional health instructions for a `serving` task backed by running machinery.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Liveness {
    pub verify: Option<String>,
    pub restart: Option<String>,
    pub owner: Option<String>,
    pub start_key: Option<String>,
}

impl Liveness {
    pub fn is_empty(&self) -> bool {
        self.verify.is_none()
            && self.restart.is_none()
            && self.owner.is_none()
            && self.start_key.is_none()
    }
}

#[derive(Debug, Clone)]
pub struct Task {
    pub subject: String,
    pub status: TaskStatus,
    pub title: String,
    /// Always present on tasks created by current writers. Legacy records may not
    /// have one; reads do not invent a creation instant.
    pub created_at: Option<DateTime<Utc>>,
    /// Optional because most tasks have no user-set due time.
    pub due_at: Option<DateTime<Utc>>,
    pub liveness: Liveness,
    /// Last successful liveness verification, not merely the last attempt.
    pub checked_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub body: String,
}

impl Task {
    pub fn new(title: &str, status: TaskStatus) -> Self {
        let now = Utc::now();
        Self {
            subject: facets::slug(title),
            status,
            title: title.trim().to_owned(),
            created_at: Some(now),
            due_at: None,
            liveness: Liveness::default(),
            checked_at: None,
            completed_at: (status == TaskStatus::Done).then_some(now),
            cancelled_at: (status == TaskStatus::Cancelled).then_some(now),
            body: String::new(),
        }
    }

    pub fn facet_ref(&self) -> String {
        format!("{DIMENSION}/{}", self.subject)
    }

    /// Apply a lifecycle transition and keep its closing timestamps coherent.
    pub fn set_status(&mut self, status: TaskStatus, at: DateTime<Utc>) {
        if self.status == status {
            return;
        }
        self.status = status;
        match status {
            TaskStatus::Todo | TaskStatus::Doing | TaskStatus::Serving => {
                self.completed_at = None;
                self.cancelled_at = None;
            }
            TaskStatus::Done => {
                self.completed_at = Some(at);
                self.cancelled_at = None;
            }
            TaskStatus::Cancelled => {
                self.completed_at = None;
                self.cancelled_at = Some(at);
            }
        }
    }

    fn is_overdue(&self, now: DateTime<Utc>) -> bool {
        self.due_at.is_some_and(|due| due <= now)
    }

    fn is_serving(&self) -> bool {
        self.status == TaskStatus::Serving
    }

    /// A duty nobody has confirmed alive. Deliberately keyed on the status rather than on
    /// whether a `verify:` happens to be written: a duty with no recorded way to check it
    /// is the *worse* case, not an exempt one.
    fn unconfirmed(&self) -> bool {
        self.is_serving() && self.checked_at.is_none()
    }
}

pub async fn read_task(data_dir: &Path, subject: &str) -> anyhow::Result<Option<Task>> {
    let subject = facets::slug(subject);
    match facets::read_facet(data_dir, DIMENSION, &subject).await? {
        Some(content) => Ok(Some(parse(&subject, &content))),
        None => Ok(None),
    }
}

pub async fn write_task(data_dir: &Path, task: &Task) -> anyhow::Result<String> {
    let content = render(data_dir, task)?;
    facets::update_facet(data_dir, DIMENSION, &task.subject, &content).await
}

pub async fn fresh_subject(data_dir: &Path, title: &str) -> anyhow::Result<String> {
    let base = facets::slug(title);
    if base.is_empty() {
        anyhow::bail!("a task's title must contain a usable character");
    }
    let mut candidate = base.clone();
    for n in 2..1000 {
        let dir = facets::subject_dir(data_dir, DIMENSION, &candidate);
        if !tokio::fs::try_exists(&dir).await.unwrap_or(false) {
            return Ok(candidate);
        }
        candidate = format!("{base}-{n}");
    }
    anyhow::bail!("too many tasks already named like {base:?}; give this one its own title")
}

/// Todo, doing and serving tasks, sorted by subject.
pub async fn active_tasks(data_dir: &Path) -> anyhow::Result<Vec<Task>> {
    let mut all = scan(data_dir).await?;
    all.retain(|task| task.status.is_active());
    Ok(all)
}

pub async fn projection(data_dir: &Path) -> anyhow::Result<String> {
    Ok(render_projection(&active_tasks(data_dir).await?, Utc::now()))
}

fn render_projection(active: &[Task], now: DateTime<Utc>) -> String {
    use std::fmt::Write as _;

    if active.is_empty() {
        return String::new();
    }

    let mut decorated: Vec<(OrderKey<'_>, &Task)> =
        active.iter().map(|task| (order_key(task, now), task)).collect();
    decorated.sort_by(|a, b| a.0.cmp(&b.0));
    let ordered: Vec<&Task> = decorated.into_iter().map(|(_, task)| task).collect();

    let shown = ordered.len().min(PROJECTED_TASKS);
    let mut out = String::from(
        "# Active tasks\n\n_What you owe right now. Full records: memory/facets/tasks/<subject>/facet.md_\n\n",
    );
    for task in &ordered[..shown] {
        out.push_str(&clip(&line(task, now), PROJECTED_LINE_CHARS));
        if let Some(note) = trailing_note(task, now) {
            let _ = write!(out, " · {note}");
        }
        out.push('\n');
    }

    let rest = &ordered[shown..];
    if !rest.is_empty() {
        let todo = rest.iter().filter(|task| task.status == TaskStatus::Todo).count();
        let doing = rest.iter().filter(|task| task.status == TaskStatus::Doing).count();
        let serving = rest.iter().filter(|task| task.is_serving()).count();
        let mut parts = Vec::new();
        if todo > 0 {
            parts.push(format!("{todo} todo"));
        }
        if doing > 0 {
            parts.push(format!("{doing} doing"));
        }
        if serving > 0 {
            parts.push(format!("{serving} serving"));
        }
        let overdue = rest.iter().filter(|task| task.is_overdue(now)).count();
        let unchecked = rest.iter().filter(|task| task.unconfirmed()).count();
        let _ = write!(
            out,
            "- ... and {} more active ({})",
            rest.len(),
            parts.join(", ")
        );
        if overdue > 0 {
            let _ = write!(out, ", {overdue} of them overdue");
        }
        if unchecked > 0 {
            let _ = write!(out, ", {unchecked} duties never confirmed alive");
        }
        out.push_str(". The whole list is memory/facets/tasks/.\n");
    }
    out
}

fn line(task: &Task, now: DateTime<Utc>) -> String {
    let mut head = task.status.as_str().to_owned();
    if let Some(due) = task.due_at {
        let when = due.format("%Y-%m-%d %H:%MZ");
        if due <= now {
            head = format!("{head}, overdue since {when}");
        } else {
            head = format!("{head}, due {when}");
        }
    }
    let title = task.title.trim().replace('\n', " ");
    let title = if title.is_empty() {
        task.subject.replace('-', " ")
    } else {
        title
    };
    format!("- [{head}] {title}")
}

/// The one trailing fact a projected line can afford, and which one it is depends on what
/// kind of task it is. A duty is judged by whether it is still alive, and its age says
/// nothing — a watch is *supposed* to be old. Plain work is the opposite: nothing in the
/// line ever said how long it had been sitting, so a delivery finished days ago and one
/// opened this morning read identically, and only one of them is work.
fn trailing_note(task: &Task, now: DateTime<Utc>) -> Option<String> {
    if task.is_serving() {
        return Some(match (task.checked_at, task.liveness.verify.is_some()) {
            (Some(at), _) => format!("last confirmed alive {}", ago(now, at)),
            (None, true) => "never checked".to_owned(),
            (None, false) => "never checked, and no recorded way to".to_owned(),
        });
    }
    // Below a day the number is noise on a list read many times a day, and a task with no
    // `created_at:` gets no note at all rather than a guessed one.
    let created = task.created_at?;
    let days = (now - created).num_days();
    (days >= 1).then(|| format!("open {days}d"))
}

fn ago(now: DateTime<Utc>, then: DateTime<Utc>) -> String {
    let mins = (now - then).num_minutes();
    match mins {
        m if m < 1 => "just now".to_owned(),
        m if m < 60 => format!("{m}m ago"),
        m if m < 60 * 24 => format!("{}h ago", m / 60),
        m => format!("{}d ago", m / (60 * 24)),
    }
}

/// Overdue first, then duties nobody has confirmed alive, then upcoming, then undated
/// doing work, then duties known to be up, then todo work.
///
/// A duty splits across two of those bands on purpose. A watch that has been confirmed
/// alive wants to be *seen* and not acted on, so it sits low; the same watch with nothing
/// to say it is running is the one line here that might already be silently dead, so it
/// sits at the top next to overdue work.
type OrderKey<'a> = (usize, i64, &'a str);

fn order_key(task: &Task, now: DateTime<Utc>) -> OrderKey<'_> {
    match (task.due_at, task.status) {
        (Some(due), _) if due <= now => (0, due.timestamp(), &task.subject),
        _ if task.unconfirmed() => (1, 0, &task.subject),
        (Some(due), _) => (2, due.timestamp(), &task.subject),
        (None, TaskStatus::Doing) => (3, 0, &task.subject),
        (None, TaskStatus::Serving) => (4, 0, &task.subject),
        (None, _) => (5, 0, &task.subject),
    }
}

async fn scan(data_dir: &Path) -> anyhow::Result<Vec<Task>> {
    let root = tasks_dir(data_dir);
    let mut rd = match tokio::fs::read_dir(&root).await {
        Ok(rd) => rd,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let mut out = Vec::new();
    while let Some(entry) = rd.next_entry().await? {
        if !entry.file_type().await?.is_dir() {
            continue;
        }
        let Ok(subject) = entry.file_name().into_string() else {
            continue;
        };
        if subject.is_empty() || subject.starts_with('.') {
            continue;
        }
        let Ok(content) =
            tokio::fs::read_to_string(entry.path().join(facets::FACET_FILE)).await
        else {
            continue;
        };
        out.push(parse(&subject, &content));
    }
    out.sort_by(|a, b| a.subject.cmp(&b.subject));
    Ok(out)
}

fn tasks_dir(data_dir: &Path) -> PathBuf {
    layout::facets_dir(data_dir).join(DIMENSION)
}

fn parse(subject: &str, content: &str) -> Task {
    let field = |key: &str| frontmatter_field(content, key).filter(|v| !v.trim().is_empty());
    let status = match field("status") {
        Some(value) => TaskStatus::parse(&value),
        None => legacy_status(field("kind").as_deref(), field("state").as_deref()),
    };
    Task {
        subject: subject.to_owned(),
        status,
        title: field("title").unwrap_or_else(|| subject.replace('-', " ")),
        created_at: field("created_at").and_then(|value| parse_timestamp(&value)),
        due_at: field("due_at")
            .or_else(|| field("due"))
            .and_then(|value| parse_timestamp(&value)),
        liveness: Liveness {
            verify: field("verify"),
            restart: field("restart"),
            owner: field("owner"),
            start_key: field("start_key"),
        },
        checked_at: field("checked_at")
            .or_else(|| field("checked"))
            .and_then(|value| parse_timestamp(&value)),
        completed_at: field("completed_at").and_then(|value| parse_timestamp(&value)),
        cancelled_at: field("cancelled_at").and_then(|value| parse_timestamp(&value)),
        body: strip_frontmatter(content).trim().to_owned(),
    }
}

/// Compatibility for the retired `kind` + `state` schema.
fn legacy_status(kind: Option<&str>, state: Option<&str>) -> TaskStatus {
    match state.map(str::trim) {
        Some("done") => TaskStatus::Done,
        Some("dropped") => TaskStatus::Cancelled,
        _ => match kind.map(str::trim) {
            Some("staged") | Some("deadline") => TaskStatus::Todo,
            // The retired schema drew this same line and called it `kind`; these two are
            // the duties, and they land where they always meant to.
            Some("serving") | Some("watch") => TaskStatus::Serving,
            // `wip`, absent, and malformed kinds represented work the old system
            // considered underway.
            _ => TaskStatus::Doing,
        },
    }
}

fn parse_timestamp(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    Some(Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0)?))
}

fn render(data_dir: &Path, task: &Task) -> anyhow::Result<String> {
    use std::fmt::Write as _;

    let mut out = String::from("---\n");
    let _ = writeln!(out, "status: {}", task.status.as_str());
    let mut field = |key: &str, value: &str| -> anyhow::Result<()> {
        reject_host_path(data_dir, key, value)?;
        let _ = writeln!(out, "{key}: {}", jstr(value.trim()));
        Ok(())
    };
    field("title", task.title.as_str())?;
    if let Some(created_at) = task.created_at {
        field("created_at", created_at.to_rfc3339().as_str())?;
    }
    if let Some(due_at) = task.due_at {
        field("due_at", due_at.to_rfc3339().as_str())?;
    }
    if let Some(checked_at) = task.checked_at {
        field("checked_at", checked_at.to_rfc3339().as_str())?;
    }
    if let Some(completed_at) = task.completed_at {
        field("completed_at", completed_at.to_rfc3339().as_str())?;
    }
    if let Some(cancelled_at) = task.cancelled_at {
        field("cancelled_at", cancelled_at.to_rfc3339().as_str())?;
    }
    for (key, value) in [
        ("verify", &task.liveness.verify),
        ("restart", &task.liveness.restart),
        ("owner", &task.liveness.owner),
        ("start_key", &task.liveness.start_key),
    ] {
        if let Some(value) = value {
            field(key, value)?;
        }
    }
    out.push_str("---\n\n");
    out.push_str(task.body.trim());
    out.push('\n');
    Ok(out)
}

fn reject_host_path(data_dir: &Path, key: &str, value: &str) -> anyhow::Result<()> {
    let dir = data_dir.to_string_lossy();
    if data_dir.is_absolute() && !dir.is_empty() && value.contains(&*dir) {
        anyhow::bail!(
            "task field `{key}` carries this machine's absolute data-dir path; \
             write it relative to the data dir so the directory stays portable"
        );
    }
    Ok(())
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max.saturating_sub(3)).collect();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, day, hour, 0, 0).unwrap()
    }

    fn now() -> DateTime<Utc> {
        at(28, 12)
    }

    fn task(title: &str, status: TaskStatus) -> Task {
        let mut task = Task::new(title, status);
        task.created_at = Some(at(1, 9));
        task
    }

    #[tokio::test]
    async fn current_schema_round_trips_without_kind_or_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut task = task("File the Feishu digest", TaskStatus::Doing);
        task.due_at = Some(at(30, 9));
        task.checked_at = Some(at(28, 10));
        task.liveness.verify =
            Some("count today's rows in drive/ledgers/feishu.jsonl".into());
        task.body = "Boss asked for a daily digest of the ops group.".into();

        write_task(dir.path(), &task).await.unwrap();
        let raw = facets::read_facet(dir.path(), DIMENSION, "file-the-feishu-digest")
            .await
            .unwrap()
            .unwrap();
        assert!(raw.contains("status: doing"));
        assert!(raw.contains("created_at:"));
        assert!(raw.contains("due_at:"));
        assert!(!raw.contains("\nkind:"));
        assert!(!raw.contains("\nstate:"));

        let got = read_task(dir.path(), "File the Feishu digest").await.unwrap().unwrap();
        assert_eq!(got.status, TaskStatus::Doing);
        assert_eq!(got.created_at, Some(at(1, 9)));
        assert_eq!(got.due_at, Some(at(30, 9)));
        assert_eq!(got.checked_at, Some(at(28, 10)));
        assert_eq!(got.body, "Boss asked for a daily digest of the ops group.");
    }

    #[tokio::test]
    async fn legacy_kind_and_state_map_into_the_five_statuses() {
        let dir = tempfile::tempdir().unwrap();
        for (subject, frontmatter, expected) in [
            ("queued", "kind: staged\nstate: open", TaskStatus::Todo),
            ("deadline", "kind: deadline\nstate: open", TaskStatus::Todo),
            ("wip", "kind: wip\nstate: open", TaskStatus::Doing),
            ("watch", "kind: watch\nstate: open", TaskStatus::Serving),
            ("serving", "kind: serving\nstate: open", TaskStatus::Serving),
            ("done", "kind: serving\nstate: done", TaskStatus::Done),
            ("dropped", "kind: wip\nstate: dropped", TaskStatus::Cancelled),
        ] {
            facets::update_facet(
                dir.path(),
                DIMENSION,
                subject,
                &format!("---\n{frontmatter}\ntitle: {subject}\n---\n"),
            )
            .await
            .unwrap();
            let got = read_task(dir.path(), subject).await.unwrap().unwrap();
            assert_eq!(got.status, expected, "{subject}");
        }
    }

    #[tokio::test]
    async fn done_and_cancelled_leave_the_active_set() {
        let dir = tempfile::tempdir().unwrap();
        for task in [
            task("todo", TaskStatus::Todo),
            task("doing", TaskStatus::Doing),
            task("serving", TaskStatus::Serving),
            task("done", TaskStatus::Done),
            task("cancelled", TaskStatus::Cancelled),
        ] {
            write_task(dir.path(), &task).await.unwrap();
        }
        let active = active_tasks(dir.path()).await.unwrap();
        assert_eq!(
            active.iter().map(|task| task.subject.as_str()).collect::<Vec<_>>(),
            vec!["doing", "serving", "todo"]
        );
    }

    #[test]
    fn transitions_stamp_and_clear_closing_times() {
        let mut task = task("ship", TaskStatus::Doing);
        task.set_status(TaskStatus::Done, at(28, 9));
        assert_eq!(task.completed_at, Some(at(28, 9)));
        assert!(task.cancelled_at.is_none());

        task.set_status(TaskStatus::Todo, at(28, 10));
        assert!(task.completed_at.is_none());
        assert!(task.cancelled_at.is_none());

        task.set_status(TaskStatus::Cancelled, at(28, 11));
        assert_eq!(task.cancelled_at, Some(at(28, 11)));
        assert!(task.completed_at.is_none());

        // Standing a duty up reopens it as surely as `todo` or `doing` does.
        task.set_status(TaskStatus::Serving, at(28, 12));
        assert!(task.cancelled_at.is_none());
        assert!(task.completed_at.is_none());
    }

    #[test]
    fn projection_uses_status_and_only_mentions_due_when_set() {
        let plain = task("Write the brief", TaskStatus::Todo);
        let mut late = task("Renew the domain", TaskStatus::Doing);
        late.due_at = Some(at(20, 8));
        let text = render_projection(&[plain, late], now());
        assert!(text.contains("# Active tasks"));
        assert!(text.contains("- [todo] Write the brief"));
        assert!(text.contains("- [doing, overdue since 2026-07-20 08:00Z] Renew the domain"));
        let plain_line = text.lines().find(|line| line.contains("Write the brief")).unwrap();
        assert!(!plain_line.contains("due"));
    }

    /// The status decides which fact the line carries, not whether a `verify:` was written.
    #[test]
    fn a_duty_is_judged_by_liveness_and_plain_work_never_is() {
        let plain = task("Draft the report", TaskStatus::Doing);
        let text = render_projection(std::slice::from_ref(&plain), now());
        assert!(!text.contains("checked"), "{text}");

        let mut duty = task("Watch the queue", TaskStatus::Serving);
        duty.liveness.verify = Some("latest ledger row is under 30m old".into());
        let text = render_projection(std::slice::from_ref(&duty), now());
        assert!(text.contains("- [serving] Watch the queue"), "{text}");
        assert!(text.contains("never checked"), "{text}");

        duty.checked_at = Some(now() - Duration::hours(3));
        let text = render_projection(std::slice::from_ref(&duty), now());
        assert!(text.contains("last confirmed alive 3h ago"), "{text}");

        // A duty with nothing recorded to check it says so, rather than going quiet.
        let mut unverifiable = task("Watch the queue", TaskStatus::Serving);
        unverifiable.liveness = Liveness::default();
        let text = render_projection(std::slice::from_ref(&unverifiable), now());
        assert!(text.contains("never checked, and no recorded way to"), "{text}");

        // Carrying the fields is not what makes it a duty; the status is.
        duty.status = TaskStatus::Doing;
        let text = render_projection(std::slice::from_ref(&duty), now());
        assert!(!text.contains("confirmed alive"), "{text}");
    }

    /// An unconfirmed duty outranks the work below it: it is the one line that might
    /// already be dead. A confirmed one drops under that work — seen, not acted on.
    #[test]
    fn unconfirmed_duties_rise_and_confirmed_ones_settle() {
        let mut confirmed = task("aaa confirmed watch", TaskStatus::Serving);
        confirmed.checked_at = Some(now() - Duration::minutes(20));
        let unconfirmed = task("zzz silent watch", TaskStatus::Serving);
        let work = task("mmm plain work", TaskStatus::Doing);

        let text = render_projection(&[confirmed, work, unconfirmed], now());
        let order: Vec<&str> = text
            .lines()
            .filter(|line| line.starts_with("- ["))
            .collect();
        assert!(order[0].contains("zzz silent watch"), "{text}");
        assert!(order[1].contains("mmm plain work"), "{text}");
        assert!(order[2].contains("aaa confirmed watch"), "{text}");
    }

    #[test]
    fn three_hundred_active_tasks_are_bounded_and_counted_by_status() {
        let mut tasks = Vec::new();
        for i in 0..100 {
            tasks.push(task(&format!("todo {i}"), TaskStatus::Todo));
            tasks.push(task(&format!("doing {i}"), TaskStatus::Doing));
            tasks.push(task(&format!("serving {i}"), TaskStatus::Serving));
        }
        let text = render_projection(&tasks, now());
        let listed = text
            .lines()
            .filter(|line| line.starts_with("- ["))
            .count();
        assert_eq!(listed, PROJECTED_TASKS);
        let summary = text.lines().find(|line| line.starts_with("- ...")).unwrap();
        assert!(summary.contains("288 more active"), "{summary}");
        assert!(summary.contains("100 todo"), "{summary}");
        assert!(summary.contains("100 doing"), "{summary}");
        assert!(summary.contains("88 serving"), "{summary}");
        // None of these duties has ever been confirmed, and the tail has to say so —
        // a count of duties is not a count of duties that are up.
        assert!(summary.contains("88 duties never confirmed alive"), "{summary}");
        assert!(text.chars().count() < 2_500);
    }

    #[tokio::test]
    async fn fresh_subject_never_reuses_closed_history() {
        let dir = tempfile::tempdir().unwrap();
        let done = task("Daily digest", TaskStatus::Done);
        write_task(dir.path(), &done).await.unwrap();
        assert_eq!(
            fresh_subject(dir.path(), "Daily digest").await.unwrap(),
            "daily-digest-2"
        );
    }

    #[tokio::test]
    async fn no_absolute_host_path_is_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let host = dir.path().display().to_string();
        let mut task = task("Watch the queue", TaskStatus::Serving);
        task.liveness.restart = Some(format!("run {host}/bin/watch"));
        let err = write_task(dir.path(), &task).await.unwrap_err().to_string();
        assert!(err.contains("restart"), "{err}");
        assert!(err.contains("relative"), "{err}");
    }

    #[test]
    fn timestamp_accepts_rfc3339_and_a_bare_date() {
        assert_eq!(
            parse_timestamp("2026-08-01T09:30:00Z"),
            Utc.with_ymd_and_hms(2026, 8, 1, 9, 30, 0).single()
        );
        assert_eq!(
            parse_timestamp("2026-08-01"),
            Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).single()
        );
        assert_eq!(parse_timestamp("next tuesday"), None);
    }

    #[test]
    fn long_titles_clip_on_character_boundaries() {
        let mut task = task("long", TaskStatus::Doing);
        task.title = "任务".repeat(200);
        let text = render_projection(std::slice::from_ref(&task), now());
        let line = text.lines().find(|line| line.starts_with("- ")).unwrap();
        // The trailing note is appended after the clip, so only the title half is bounded.
        let (title, _note) = line.split_once(" · ").unwrap();
        assert_eq!(title.chars().count(), PROJECTED_LINE_CHARS);
    }

    #[test]
    fn plain_work_carries_how_long_it_has_been_open() {
        let mut fresh = task("Draft the report", TaskStatus::Doing);
        fresh.created_at = Some(now() - Duration::hours(5));
        let text = render_projection(std::slice::from_ref(&fresh), now());
        assert!(!text.contains("open "), "{text}");

        let mut stale = task("Draft the report", TaskStatus::Doing);
        stale.created_at = Some(now() - Duration::days(6));
        let text = render_projection(std::slice::from_ref(&stale), now());
        assert!(text.contains("· open 6d"), "{text}");

        // A watch is meant to be old, so its line stays about whether it is still alive.
        let mut duty = stale.clone();
        duty.status = TaskStatus::Serving;
        duty.liveness.verify = Some("latest ledger row is under 30m old".into());
        let text = render_projection(std::slice::from_ref(&duty), now());
        assert!(!text.contains("open 6d"), "{text}");
        assert!(text.contains("never checked"), "{text}");

        // A record with no `created_at:` gets no note rather than a guessed one.
        let mut undated = stale.clone();
        undated.created_at = None;
        let text = render_projection(std::slice::from_ref(&undated), now());
        assert!(!text.contains("open "), "{text}");
    }
}
