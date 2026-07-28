//! Durable tasks — the `tasks` dimension of the facet tree.
//!
//! A task is what the agent **owes**: a delivery left half-done by a restart, a
//! group it serves, a value it watches, a date it must act on, a staged job
//! suspended for approval. Those were five separate mechanisms; they are one thing
//! now, and that one thing is **a facet** — no new store, no new file format, no
//! new tools. A task lives exactly where a facet lives:
//!
//! ```text
//! <data_dir>/memory/facets/tasks/<subject>/facet.md
//! ```
//!
//! written and read with [`facets::update_facet`] / [`facets::read_facet`] like any
//! other subject. The dimension list was always open-ended ([`super::facets`]);
//! `tasks` is simply one more dimension, and the agent already has the tools for
//! it. The subject **is** the task's identity: one obligation, one directory,
//! regenerated whole (never patched) as it moves.
//!
//! ## What code reads, and what it doesn't
//!
//! The frontmatter is the machine-readable half — [`TaskKind`], [`TaskState`], an
//! optional `report_to` scene, an optional `due`, and the liveness contract. The
//! body below it is the agent's own prose and code never interprets it.
//!
//! **The liveness fields are prose written by an agent and read by an agent.** How
//! to verify a serving task is really alive is *a count, not "something is
//! running"*; how to restart it is instructions, not a command line this module
//! will ever execute. Nothing here spawns, shells out, or parses them into
//! actions — they exist so the next agent to look knows what "alive" means.
//!
//! ## Why code touches this at all
//!
//! Two queries, and only two:
//!
//! - [`due_before`] — the clock rebuilds every timer from this at startup, so it
//!   holds no durable state of its own.
//! - [`projection`] — a **code-bounded** rendering of what is open, for injection
//!   into every agent's window. Projected is what Reaction must know without
//!   reading; everything else is recall.
//!
//! Everything else about tasks — writing one, closing one, reasoning about one —
//! is the agent using the facet tools it already has.
//!
//! ## Keep-biased parsing
//!
//! A record whose `state` is missing or unrecognised reads as **open**, and a
//! subject under `tasks/` with no frontmatter at all is still a task. A missed duty
//! is a silently broken promise, so every ambiguity resolves toward "still owed" —
//! the failure mode we can afford is a stale line in the projection, not a dropped
//! one.
//!
//! ## The one place the facet shape chafes
//!
//! A facet subject is a **directory**, and here the subject is the task's identity —
//! so every task costs a directory, and a title used over and over accumulates
//! numbered siblings (`daily-digest`, `daily-digest-2`, …) that stay forever, since
//! a closed task is the record that it was closed and is never deleted. That suits
//! the durable obligations this is for and would suit a thousand tiny reminders
//! badly. If tasks ever churn at that rate, the answer is a coarser task, not a
//! second store beside `facets/`.

use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, TimeZone, Utc};

use super::episodes::{frontmatter_field, jstr, strip_frontmatter};
use super::{facets, layout};
use crate::types::Scene;

/// The facet dimension tasks live in. One more open-ended dimension beside
/// `people`/`projects`/…, not a special case in the store.
pub const DIMENSION: &str = "tasks";

/// How many open tasks [`projection`] lists in full before it collapses the rest
/// into one counted line.
///
/// **Twelve, and the bound is code's, not the agent's.** This text rides in *every*
/// agent's window — including Reaction's, which is a small model on a latency
/// budget — so its size has to be predictable no matter what the store holds. Two
/// hundred open tasks must render as a summary: a list that long stops being read
/// at all, and an unread projection is worse than a short one, because it looks
/// like it is doing its job. Twelve is about what a person can hold as "what I owe
/// right now"; past it, a count that says *there are more, go look* is the honest
/// rendering. Nothing is lost by the bound — the tail is still on disk, and
/// [`due_before`] and the facet tools reach all of it.
pub const PROJECTED_TASKS: usize = 12;

/// Per-line character cap in [`projection`], so one long title can't blow the
/// window the way a long list would. Chars, not bytes: a CJK title clips at the
/// same visual length and costs more bytes, which is the trade we want.
const PROJECTED_LINE_CHARS: usize = 120;

/// What kind of thing is owed. Five mechanisms that used to be separate; the kind
/// is a label on one record, not a different store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    /// A half-finished delivery — typically interrupted by a restart.
    Wip,
    /// A standing duty: watch a group, file what arrives, reply in thread.
    Serving,
    /// A value, a baseline, a threshold, a cadence.
    Watch,
    /// A date and what to do at it.
    Deadline,
    /// A multi-stage job suspended for approval.
    Staged,
}

impl TaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wip => "wip",
            Self::Serving => "serving",
            Self::Watch => "watch",
            Self::Deadline => "deadline",
            Self::Staged => "staged",
        }
    }

    /// Absent or unrecognised reads as [`TaskKind::Wip`] — the generic "something
    /// is unfinished", the reading that keeps a malformed record visible.
    fn parse(s: &str) -> Self {
        match s.trim() {
            "serving" => Self::Serving,
            "watch" => Self::Watch,
            "deadline" => Self::Deadline,
            "staged" => Self::Staged,
            _ => Self::Wip,
        }
    }

    /// Ordering weight for tasks carrying no due time: resume what was interrupted,
    /// then unblock what is waiting on someone, then the standing duties.
    fn rank(self) -> usize {
        match self {
            Self::Wip => 0,
            Self::Staged => 1,
            Self::Serving => 2,
            Self::Watch => 3,
            Self::Deadline => 4,
        }
    }

    /// Every kind, in [`TaskKind::rank`] order — the order the projection's summary
    /// counts them in, so the line is stable across renders.
    const ALL: [TaskKind; 5] =
        [Self::Wip, Self::Staged, Self::Serving, Self::Watch, Self::Deadline];
}

/// Where a task stands. Only `open` is projected; `done` and `dropped` are history
/// and stay on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Open,
    Done,
    Dropped,
}

impl TaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Done => "done",
            Self::Dropped => "dropped",
        }
    }

    /// Only the two explicit closings close a task; everything else — absent,
    /// blank, misspelt — is [`TaskState::Open`]. See the module's keep-biased note.
    fn parse(s: &str) -> Self {
        match s.trim() {
            "done" => Self::Done,
            "dropped" => Self::Dropped,
            _ => Self::Open,
        }
    }

    pub fn is_open(self) -> bool {
        matches!(self, Self::Open)
    }
}

/// The liveness contract of a serving task: **not an existence check**. Every field
/// is prose an agent wrote for the next agent to read — how to tell it is *really*
/// alive (a count of what it has actually done, not "a process exists"), how to
/// bring it back, and who owns it or what key makes starting it twice a no-op, so
/// two scenes cannot both relaunch it.
///
/// This module never executes, resolves, or schedules any of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Liveness {
    /// How to verify it is really alive — a count, not "something is running".
    pub verify: Option<String>,
    /// How to bring it back if the check says it isn't.
    pub restart: Option<String>,
    /// Who owns the running copy.
    pub owner: Option<String>,
    /// An idempotent start key, so a second start is a no-op rather than a
    /// duplicate.
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

/// One durable task: a facet under [`DIMENSION`], parsed.
#[derive(Debug, Clone)]
pub struct Task {
    /// The facet subject — the directory name. `tasks/<subject>` is its ref, and
    /// the subject is the task's identity.
    pub subject: String,
    pub kind: TaskKind,
    pub state: TaskState,
    /// Human prose title. Falls back to the subject when a record carries none.
    pub title: String,
    /// Optional scene to report into. A task is global — created in one scene and
    /// delivered in another — so this is a preference, never a scope.
    pub report_to: Option<Scene>,
    /// When it comes due, if it ever does. The clock rebuilds its timers from these
    /// ([`due_before`]).
    pub due: Option<DateTime<Utc>>,
    pub liveness: Liveness,
    /// The agent's own prose, below the frontmatter. Code never interprets it.
    pub body: String,
}

impl Task {
    /// A new open task with `subject` derived from `title`. Fields are public —
    /// set `due`, `report_to` and `liveness` directly. Use [`fresh_subject`] first
    /// when the title might already name an existing task.
    pub fn new(title: &str, kind: TaskKind) -> Self {
        Self {
            subject: facets::slug(title),
            kind,
            state: TaskState::Open,
            title: title.trim().to_owned(),
            report_to: None,
            due: None,
            liveness: Liveness::default(),
            body: String::new(),
        }
    }

    /// `tasks/<subject>` — the same ref shape [`facets::update_facet`] returns and
    /// [`facets::facet_subject_index`] lists.
    pub fn facet_ref(&self) -> String {
        format!("{DIMENSION}/{}", self.subject)
    }

    /// Due at or before `now`.
    fn is_overdue(&self, now: DateTime<Utc>) -> bool {
        self.due.is_some_and(|d| d <= now)
    }
}

/// One task by subject, or `None` if nothing is written there. A thin read over
/// [`facets::read_facet`] — same file, same layout.
pub async fn read_task(data_dir: &Path, subject: &str) -> anyhow::Result<Option<Task>> {
    let subject = facets::slug(subject);
    match facets::read_facet(data_dir, DIMENSION, &subject).await? {
        Some(content) => Ok(Some(parse(&subject, &content))),
        None => Ok(None),
    }
}

/// Write `task` as the whole record for its subject — regenerate, don't patch,
/// exactly the facet convention (and the same atomic temp-then-rename, so a reader
/// never sees a torn task). Returns the `tasks/<subject>` ref.
///
/// This is a **write to that subject**: it advances the task already there, it does
/// not create a second one beside it. To add a genuinely new task whose title may
/// already be taken, take a [`fresh_subject`] first.
pub async fn write_task(data_dir: &Path, task: &Task) -> anyhow::Result<String> {
    let content = render(data_dir, task)?;
    facets::update_facet(data_dir, DIMENSION, &task.subject, &content).await
}

/// A subject not yet taken: `slug(title)`, else `-2`, `-3`, … — the same escalation
/// [`super::episodes`] uses for a colliding episode name.
///
/// It steps past *any* existing subject, closed ones included: a finished task is
/// the record that it was finished, and quietly writing over it would lose the only
/// evidence the promise was ever kept. The cost is that a title used often
/// accumulates numbered siblings — the module's one noted awkwardness.
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

/// Every open task, sorted by subject. The base both queries read from — one
/// directory listing plus one small file per task, which is why the projection can
/// afford to be rebuilt rather than cached.
///
/// Never deletes, closes, or rewrites anything: this module only ever *reads* an
/// open task.
pub async fn open_tasks(data_dir: &Path) -> anyhow::Result<Vec<Task>> {
    let mut all = scan(data_dir).await?;
    all.retain(|t| t.state.is_open());
    Ok(all)
}

/// Open tasks carrying a due time at or before `cutoff`, soonest first.
///
/// **This is what the clock is rebuilt from.** The clock keeps no durable state of
/// its own: at startup it calls this once, arms a timer per row (the first row is
/// the next one to fire), and re-reads after anything writes a task. Pass
/// `DateTime::<Utc>::MAX_UTC` to sweep every future due date, or `Utc::now()` for
/// just what is already overdue — a restart that slept through a deadline finds it
/// here rather than losing it.
///
/// Tasks with no `due` are absent by construction, and closed ones are filtered
/// out, so a timer is never armed for something already delivered.
pub async fn due_before(data_dir: &Path, cutoff: DateTime<Utc>) -> anyhow::Result<Vec<Task>> {
    let mut out: Vec<Task> = open_tasks(data_dir)
        .await?
        .into_iter()
        .filter(|t| t.due.is_some_and(|d| d <= cutoff))
        .collect();
    // Due first (both are Some here), subject as the tiebreak so the order is
    // stable across runs and two tasks due at the same instant never swap.
    out.sort_by(|a, b| a.due.cmp(&b.due).then_with(|| a.subject.cmp(&b.subject)));
    Ok(out)
}

/// What is open, rendered for injection into an agent's window — bounded by
/// [`PROJECTED_TASKS`], not by anyone's judgment.
///
/// Empty string when nothing is open, so a caller joining sections drops it (the
/// same contract as the other sections [`super::snapshot::window`] joins).
pub async fn projection(data_dir: &Path) -> anyhow::Result<String> {
    Ok(render_projection(&open_tasks(data_dir).await?, Utc::now()))
}

/// The projection's pure half: order, clip, and summarise. Split out so the bound
/// can be tested at any scale without writing that many files.
fn render_projection(open: &[Task], now: DateTime<Utc>) -> String {
    use std::fmt::Write as _;

    if open.is_empty() {
        return String::new();
    }

    // Overdue first (most overdue leading), then what is coming, then the
    // undated by kind. Subject breaks every tie, so the same store always renders
    // the same text — and so the bound below always drops the *least* urgent tail.
    let mut decorated: Vec<(OrderKey<'_>, &Task)> =
        open.iter().map(|t| (order_key(t, now), t)).collect();
    decorated.sort_by(|a, b| a.0.cmp(&b.0));
    let ordered: Vec<&Task> = decorated.into_iter().map(|(_, t)| t).collect();

    let shown = ordered.len().min(PROJECTED_TASKS);
    let mut s = String::from(
        "# Open tasks\n\n_What you owe right now — projected, never fetched. Full records: memory/facets/tasks/<subject>/facet.md_\n\n",
    );
    for t in &ordered[..shown] {
        s.push_str(&clip(&line(t, now), PROJECTED_LINE_CHARS));
        s.push('\n');
    }

    let rest = &ordered[shown..];
    if !rest.is_empty() {
        let mut parts: Vec<String> = Vec::new();
        for kind in TaskKind::ALL {
            let n = rest.iter().filter(|t| t.kind == kind).count();
            if n > 0 {
                parts.push(format!("{n} {}", kind.as_str()));
            }
        }
        let overdue = rest.iter().filter(|t| t.is_overdue(now)).count();
        let _ = write!(s, "- … and {} more open ({})", rest.len(), parts.join(", "));
        if overdue > 0 {
            // Say it even though these lines were cut: the bound may hide a task,
            // it must never hide that a task is late.
            let _ = write!(s, ", {overdue} of them overdue");
        }
        s.push_str(". The whole list is memory/facets/tasks/.\n");
    }
    s
}

/// One projected line: kind, lateness, title, and where it reports.
fn line(t: &Task, now: DateTime<Utc>) -> String {
    let mut head = t.kind.as_str().to_string();
    if let Some(due) = t.due {
        let when = due.format("%Y-%m-%d %H:%MZ");
        if due <= now {
            head = format!("{head}, overdue since {when}");
        } else {
            head = format!("{head}, due {when}");
        }
    }
    let title = t.title.trim().replace('\n', " ");
    let title = if title.is_empty() { t.subject.replace('-', " ") } else { title };
    match &t.report_to {
        Some(scene) => format!("- [{head}] {title} → {}", scene.0),
        None => format!("- [{head}] {title}"),
    }
}

/// (group, due-or-zero, subject). Group 0 is overdue, 1 is upcoming, and 2+ are the
/// undated ranked by kind — so urgency, not alphabet, decides who survives the
/// bound.
type OrderKey<'a> = (usize, i64, &'a str);

fn order_key(t: &Task, now: DateTime<Utc>) -> OrderKey<'_> {
    match t.due {
        Some(due) if due <= now => (0, due.timestamp(), &t.subject),
        Some(due) => (1, due.timestamp(), &t.subject),
        None => (2 + t.kind.rank(), 0, &t.subject),
    }
}

/// Every task under the dimension, open or not, sorted by subject. A stray file
/// (not a directory) or a subject with no `facet.md` is skipped — a subject "exists"
/// once it has prose, the same rule [`facets::facet_subject_index`] applies.
async fn scan(data_dir: &Path) -> anyhow::Result<Vec<Task>> {
    let root = tasks_dir(data_dir);
    let mut rd = match tokio::fs::read_dir(&root).await {
        Ok(rd) => rd,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let mut out = Vec::new();
    while let Some(ent) = rd.next_entry().await? {
        if !ent.file_type().await?.is_dir() {
            continue;
        }
        let Ok(subject) = ent.file_name().into_string() else {
            continue;
        };
        if subject.is_empty() || subject.starts_with('.') {
            continue;
        }
        let Ok(content) = tokio::fs::read_to_string(ent.path().join(facets::FACET_FILE)).await
        else {
            continue;
        };
        out.push(parse(&subject, &content));
    }
    out.sort_by(|a, b| a.subject.cmp(&b.subject));
    Ok(out)
}

/// `<memory>/facets/tasks` — the dimension directory. Derived from
/// [`layout::facets_dir`]; tasks add no path of their own.
fn tasks_dir(data_dir: &Path) -> PathBuf {
    layout::facets_dir(data_dir).join(DIMENSION)
}

/// Read one record. Forgiving on purpose — an agent writes these by hand through
/// the ordinary facet tool, so an unknown key is ignored, a bare value is taken
/// as-is, and anything missing takes the keep-biased default.
fn parse(subject: &str, content: &str) -> Task {
    let field = |k: &str| frontmatter_field(content, k).filter(|v| !v.trim().is_empty());
    let title = field("title").unwrap_or_else(|| subject.replace('-', " "));
    Task {
        subject: subject.to_owned(),
        kind: field("kind").map_or(TaskKind::Wip, |v| TaskKind::parse(&v)),
        state: field("state").map_or(TaskState::Open, |v| TaskState::parse(&v)),
        title,
        report_to: field("report_to").map(Scene),
        due: field("due").and_then(|v| parse_due(&v)),
        liveness: Liveness {
            verify: field("verify"),
            restart: field("restart"),
            owner: field("owner"),
            start_key: field("start_key"),
        },
        body: strip_frontmatter(content).trim().to_owned(),
    }
}

/// A due time as RFC3339, or a bare `YYYY-MM-DD` read as that date's UTC midnight —
/// a date is what an agent writing by hand most often means. Anything else is
/// `None`: an unparseable due leaves the task open and projected, just untimed,
/// rather than silently arming nothing.
fn parse_due(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    Some(Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0)?))
}

/// Render a task to its `facet.md` text: JSON scalars in the frontmatter (a colon,
/// quote or newline in a title can never break the block), the agent's prose below.
///
/// Refuses to persist a frontmatter value carrying this install's own absolute data
/// directory: `data/` has to stay a directory you can move to another machine, and
/// a `restart:` line naming `/Users/someone/…` is exactly what would break that.
/// The guard is deliberately narrow — it knows one absolute path, its own — because
/// anything wider would start rejecting ordinary prose.
fn render(data_dir: &Path, task: &Task) -> anyhow::Result<String> {
    use std::fmt::Write as _;

    let mut s = String::from("---\n");
    let _ = writeln!(s, "kind: {}", task.kind.as_str());
    let _ = writeln!(s, "state: {}", task.state.as_str());
    let mut field = |key: &str, value: &str| -> anyhow::Result<()> {
        reject_host_path(data_dir, key, value)?;
        let _ = writeln!(s, "{key}: {}", jstr(value.trim()));
        Ok(())
    };
    field("title", task.title.as_str())?;
    if let Some(scene) = &task.report_to {
        field("report_to", scene.0.as_str())?;
    }
    if let Some(due) = task.due {
        field("due", due.to_rfc3339().as_str())?;
    }
    for (key, value) in [
        ("verify", &task.liveness.verify),
        ("restart", &task.liveness.restart),
        ("owner", &task.liveness.owner),
        ("start_key", &task.liveness.start_key),
    ] {
        if let Some(v) = value {
            field(key, v.as_str())?;
        }
    }
    s.push_str("---\n\n");
    s.push_str(task.body.trim());
    s.push('\n');
    Ok(s)
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

/// Clip to `max` **characters** (never a byte boundary), marking the cut.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A July-2026 instant, so `now()` sits between the "past" and "future" fixtures.
    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, day, hour, 0, 0).unwrap()
    }

    fn now() -> DateTime<Utc> {
        at(28, 12)
    }

    fn task(title: &str, kind: TaskKind) -> Task {
        Task::new(title, kind)
    }

    #[tokio::test]
    async fn round_trips_through_the_facet_layout() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = Task::new("File the Feishu digest", TaskKind::Serving);
        t.report_to = Some(Scene("boss".into()));
        t.due = Some(at(30, 9));
        t.liveness = Liveness {
            verify: Some("count today's rows in drive/ledgers/feishu.jsonl — one per filed message".into()),
            restart: Some("re-open the group listener; see skills/feishu-watch.md".into()),
            start_key: Some("feishu-digest".into()),
            ..Default::default()
        };
        t.body = "Boss asked for a daily digest of the ops group.".into();

        let r = write_task(dir.path(), &t).await.unwrap();
        assert_eq!(r, "tasks/file-the-feishu-digest");

        // It is a facet: the ordinary facet reader and the subject index see it.
        assert!(
            facets::read_facet(dir.path(), DIMENSION, "file-the-feishu-digest")
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            facets::facet_subject_index(dir.path())
                .await
                .unwrap()
                .contains(&"tasks/file-the-feishu-digest".to_string())
        );

        let got = read_task(dir.path(), "File the Feishu digest").await.unwrap().unwrap();
        assert_eq!(got.subject, "file-the-feishu-digest");
        assert_eq!(got.facet_ref(), "tasks/file-the-feishu-digest");
        assert_eq!(got.kind, TaskKind::Serving);
        assert_eq!(got.state, TaskState::Open);
        assert_eq!(got.title, "File the Feishu digest");
        assert_eq!(got.report_to.as_ref().map(|s| s.0.as_str()), Some("boss"));
        assert_eq!(got.due, Some(at(30, 9)));
        assert_eq!(got.liveness.start_key.as_deref(), Some("feishu-digest"));
        assert!(got.liveness.verify.as_deref().unwrap().contains("count"));
        assert!(got.liveness.owner.is_none());
        assert_eq!(got.body, "Boss asked for a daily digest of the ops group.");
    }

    #[tokio::test]
    async fn read_missing_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_task(dir.path(), "nothing").await.unwrap().is_none());
        assert!(open_tasks(dir.path()).await.unwrap().is_empty());
        assert!(projection(dir.path()).await.unwrap().is_empty());
    }

    /// A record written by hand through the plain facet tool, with a colon in a
    /// value, an unknown key, and no `state` — it must still read as an open task.
    #[tokio::test]
    async fn hand_written_record_parses_keep_biased() {
        let dir = tempfile::tempdir().unwrap();
        facets::update_facet(
            dir.path(),
            DIMENSION,
            "renew the domain",
            "---\nkind: deadline\ntitle: Renew xiaoyuanzhu.com: expires soon\ndue: 2026-08-01\nnote: whatever\n---\n\nCard on file.\n",
        )
        .await
        .unwrap();
        // And one with no frontmatter at all.
        facets::update_facet(dir.path(), DIMENSION, "half a slide deck", "Got three slides in.")
            .await
            .unwrap();

        let still_open = open_tasks(dir.path()).await.unwrap();
        assert_eq!(still_open.len(), 2);
        assert_eq!(still_open[0].subject, "half-a-slide-deck");
        assert_eq!(still_open[0].kind, TaskKind::Wip);
        assert_eq!(still_open[0].state, TaskState::Open);
        assert_eq!(still_open[0].title, "half a slide deck");
        assert_eq!(still_open[0].body, "Got three slides in.");
        assert_eq!(still_open[1].title, "Renew xiaoyuanzhu.com: expires soon");
        assert_eq!(still_open[1].kind, TaskKind::Deadline);
        assert_eq!(still_open[1].due, Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).single());
    }

    #[tokio::test]
    async fn done_and_dropped_leave_the_open_set() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = task("ship the deck", TaskKind::Wip);
        let mut b = task("chase the invoice", TaskKind::Wip);
        let c = task("watch the build", TaskKind::Watch);
        a.state = TaskState::Done;
        b.state = TaskState::Dropped;
        for t in [&a, &b, &c] {
            write_task(dir.path(), t).await.unwrap();
        }

        let still_open = open_tasks(dir.path()).await.unwrap();
        assert_eq!(still_open.len(), 1);
        assert_eq!(still_open[0].subject, "watch-the-build");
        // Closed ones are history, not deletions — still on disk, still readable.
        assert_eq!(
            read_task(dir.path(), "ship the deck").await.unwrap().unwrap().state,
            TaskState::Done
        );
    }

    #[tokio::test]
    async fn due_before_is_open_only_and_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let mut soon = task("soon", TaskKind::Deadline);
        soon.due = Some(at(29, 8));
        let mut later = task("later", TaskKind::Deadline);
        later.due = Some(at(31, 8));
        let mut past = task("past", TaskKind::Deadline);
        past.due = Some(at(20, 8));
        let mut closed = task("closed", TaskKind::Deadline);
        closed.due = Some(at(29, 9));
        closed.state = TaskState::Done;
        let undated = task("undated", TaskKind::Serving);
        for t in [&soon, &later, &past, &closed, &undated] {
            write_task(dir.path(), t).await.unwrap();
        }

        // The clock's startup sweep: everything still owed that has a time.
        let all = due_before(dir.path(), DateTime::<Utc>::MAX_UTC).await.unwrap();
        let subjects: Vec<&str> = all.iter().map(|t| t.subject.as_str()).collect();
        assert_eq!(subjects, vec!["past", "soon", "later"]);

        // A horizon: only what falls inside it.
        let near = due_before(dir.path(), at(30, 0)).await.unwrap();
        assert_eq!(near.len(), 2);

        // Just what a restart slept through.
        let overdue = due_before(dir.path(), now()).await.unwrap();
        assert_eq!(overdue.len(), 1);
        assert_eq!(overdue[0].subject, "past");
    }

    #[test]
    fn projection_is_empty_when_nothing_is_open() {
        assert!(render_projection(&[], now()).is_empty());
    }

    #[test]
    fn projection_leads_with_the_overdue() {
        let mut wip = task("finish the deck", TaskKind::Wip);
        wip.title = "Finish the deck".into();
        let mut late = task("renew the domain", TaskKind::Deadline);
        late.title = "Renew the domain".into();
        late.due = Some(at(20, 8));
        late.report_to = Some(Scene("boss".into()));
        let mut coming = task("quarterly filing", TaskKind::Deadline);
        coming.title = "Quarterly filing".into();
        coming.due = Some(at(31, 8));
        let watching = task("watch the build", TaskKind::Watch);

        let text = render_projection(&[watching, coming, wip, late], now());
        let lines: Vec<&str> = text.lines().filter(|l| l.starts_with("- ")).collect();
        assert_eq!(
            lines,
            vec![
                "- [deadline, overdue since 2026-07-20 08:00Z] Renew the domain → boss",
                "- [deadline, due 2026-07-31 08:00Z] Quarterly filing",
                "- [wip] Finish the deck",
                "- [watch] watch the build",
            ]
        );
    }

    /// The reason this store exists in code at all: 200 open tasks must render as a
    /// summary, not a list — and the bound must not be able to hide lateness.
    #[test]
    fn two_hundred_open_tasks_render_as_a_summary() {
        let mut tasks = Vec::new();
        for i in 0..60 {
            tasks.push(task(&format!("watch value {i}"), TaskKind::Watch));
        }
        for i in 0..60 {
            tasks.push(task(&format!("serve group {i}"), TaskKind::Serving));
        }
        for i in 0..60 {
            tasks.push(task(&format!("finish thing {i}"), TaskKind::Wip));
        }
        for i in 0..20 {
            let mut t = task(&format!("deadline {i}"), TaskKind::Deadline);
            // Half already blown, half still ahead.
            t.due = Some(if i % 2 == 0 { at(20, 8) } else { at(31, 8) });
            tasks.push(t);
        }
        assert_eq!(tasks.len(), 200);

        let text = render_projection(&tasks, now());
        let listed: Vec<&str> =
            text.lines().filter(|l| l.starts_with("- ") && !l.starts_with("- …")).collect();
        assert_eq!(listed.len(), PROJECTED_TASKS);

        // The ten overdue ones all fit inside the bound and lead it.
        assert_eq!(listed.iter().filter(|l| l.contains("overdue")).count(), 10);
        assert!(listed[0].contains("overdue"));

        // The tail is counted, not dropped, and says so by kind.
        let summary = text.lines().find(|l| l.starts_with("- …")).expect("a summary line");
        assert!(summary.contains("188 more open"), "{summary}");
        assert!(summary.contains("60 wip"), "{summary}");
        assert!(summary.contains("60 serving"), "{summary}");
        assert!(summary.contains("60 watch"), "{summary}");
        assert!(summary.contains("memory/facets/tasks/"), "{summary}");

        // And the whole thing stays small enough to sit in every window.
        assert!(text.chars().count() < 2_500, "{} chars", text.chars().count());
    }

    /// Lateness must survive the bound even when the overdue tasks are cut from the
    /// list: more overdue than [`PROJECTED_TASKS`] means the count is stated.
    #[test]
    fn summary_states_overdue_it_could_not_list() {
        let mut tasks = Vec::new();
        for i in 0..30 {
            let mut t = task(&format!("late {i:02}"), TaskKind::Deadline);
            t.due = Some(at(20, 8));
            tasks.push(t);
        }
        let text = render_projection(&tasks, now());
        let summary = text.lines().find(|l| l.starts_with("- …")).expect("a summary line");
        assert!(summary.contains("18 more open"), "{summary}");
        assert!(summary.contains("18 of them overdue"), "{summary}");
    }

    #[test]
    fn a_long_title_cannot_blow_the_line() {
        let mut t = task("long", TaskKind::Wip);
        t.title = "x".repeat(1_000);
        let text = render_projection(std::slice::from_ref(&t), now());
        let line = text.lines().find(|l| l.starts_with("- ")).unwrap();
        assert_eq!(line.chars().count(), PROJECTED_LINE_CHARS);
        assert!(line.ends_with('…'));

        // Chars, not bytes: a CJK title clips without splitting a character.
        t.title = "任务".repeat(200);
        let text = render_projection(std::slice::from_ref(&t), now());
        let line = text.lines().find(|l| l.starts_with("- ")).unwrap();
        assert_eq!(line.chars().count(), PROJECTED_LINE_CHARS);
    }

    #[tokio::test]
    async fn projection_reads_through_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = task("Water the plants", TaskKind::Wip);
        t.title = "Water the plants".into();
        write_task(dir.path(), &t).await.unwrap();
        let text = projection(dir.path()).await.unwrap();
        assert!(text.contains("- [wip] Water the plants"), "{text}");
    }

    #[tokio::test]
    async fn fresh_subject_never_takes_an_existing_one() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(fresh_subject(dir.path(), "Daily digest").await.unwrap(), "daily-digest");

        let t = task("Daily digest", TaskKind::Serving);
        write_task(dir.path(), &t).await.unwrap();
        assert_eq!(fresh_subject(dir.path(), "Daily digest").await.unwrap(), "daily-digest-2");

        // Even a closed one keeps its name — a finished task is the record that it
        // was finished.
        let mut done = task("Daily digest", TaskKind::Serving);
        done.state = TaskState::Done;
        write_task(dir.path(), &done).await.unwrap();
        assert_eq!(fresh_subject(dir.path(), "Daily digest").await.unwrap(), "daily-digest-2");

        assert!(fresh_subject(dir.path(), "???").await.is_err());
    }

    /// `data/` must stay portable: nothing this module persists may carry this
    /// machine's absolute paths.
    #[tokio::test]
    async fn no_absolute_host_path_is_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let host = dir.path().display().to_string();

        let mut t = task("Serve the group", TaskKind::Serving);
        t.title = "Serve the group".into();
        t.liveness.verify = Some("count rows in drive/ledgers/group.jsonl".into());
        t.liveness.restart = Some("re-run skills/group-watch.md".into());
        t.body = "Filed 12 messages so far.".into();
        write_task(dir.path(), &t).await.unwrap();

        let path = facets::subject_dir(dir.path(), DIMENSION, "serve-the-group")
            .join(facets::FACET_FILE);
        let written = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(!written.contains(&host), "{written}");
        assert!(!written.lines().any(|l| l.contains(": \"/")), "{written}");

        // And an absolute one is refused rather than quietly written.
        let mut bad = t.clone();
        bad.liveness.restart = Some(format!("run {host}/bin/watch"));
        let err = write_task(dir.path(), &bad).await.unwrap_err().to_string();
        assert!(err.contains("restart"), "{err}");
        assert!(err.contains("relative"), "{err}");

        // The refusal did not touch what was already there.
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), written);
    }

    /// Reflection may not prune an open task, and neither may this module: no read
    /// path here removes, closes, or rewrites one.
    #[tokio::test]
    async fn nothing_here_removes_an_open_task() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = task("Deliver the report", TaskKind::Wip);
        t.due = Some(at(20, 8));
        write_task(dir.path(), &t).await.unwrap();
        let path = facets::subject_dir(dir.path(), DIMENSION, "deliver-the-report")
            .join(facets::FACET_FILE);
        let before = tokio::fs::read_to_string(&path).await.unwrap();

        for _ in 0..3 {
            open_tasks(dir.path()).await.unwrap();
            due_before(dir.path(), DateTime::<Utc>::MAX_UTC).await.unwrap();
            due_before(dir.path(), now()).await.unwrap();
            projection(dir.path()).await.unwrap();
            read_task(dir.path(), "deliver-the-report").await.unwrap();
            fresh_subject(dir.path(), "Deliver the report").await.unwrap();
        }

        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), before);
        let still = read_task(dir.path(), "deliver-the-report").await.unwrap().unwrap();
        assert!(still.state.is_open());
    }

    #[test]
    fn due_accepts_rfc3339_and_a_bare_date() {
        assert_eq!(parse_due("2026-08-01T09:30:00Z"), Utc.with_ymd_and_hms(2026, 8, 1, 9, 30, 0).single());
        assert_eq!(parse_due("2026-08-01"), Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).single());
        assert_eq!(parse_due("next tuesday"), None);
        assert_eq!(parse_due(""), None);
    }
}
