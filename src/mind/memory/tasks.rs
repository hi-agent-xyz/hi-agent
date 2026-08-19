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
//! Older records using `kind` plus `state` remain readable, and a record predating
//! `serving` — `doing` carrying a liveness contract — is read back as the duty it always
//! was (see [`coerce_duty`]). New writes emit only the status taxonomy.

use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, TimeZone, Utc};

use super::episodes::{frontmatter_field, jstr, strip_frontmatter};
use super::{facets, layout};

pub const DIMENSION: &str = "tasks";
pub const PROJECTED_TASKS: usize = 12;
const PROJECTED_LINE_CHARS: usize = 120;

/// How long work may sit in `doing` before the projection stops reporting its age and
/// starts asking for a disposition.
///
/// Two days, because the thing this catches is not slow work — it is work that finished
/// and never got filed, usually because the last conceivable proof belonged to the person
/// and they never said anything. That wait has no end of its own: the record this was
/// measured against sat four days past its own delivery, and would have sat there still.
/// Long enough that a genuinely two-day job is not nagged, short enough that a finished one
/// is not forgotten.
const IDLE_BOUNDARY_HOURS: i64 = 48;

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
    /// When the status last changed — the clock [`IDLE_BOUNDARY_HOURS`] reads.
    ///
    /// **Not "when was this touched".** A ticket that sat in `doing` for four days was
    /// rewritten six times in five hours while it did so, each write another probe
    /// concluding the same thing, so the file's own mtime called it freshly tended right up
    /// to the day it was closed by hand. Churn is not movement, and the only movement a
    /// duty has is its status.
    ///
    /// Stamped by [`Task::set_status`], so every transition code performs records itself.
    /// A record without one falls back to `created_at`, which is older and therefore errs
    /// toward the boundary rather than hiding behind it.
    pub status_since: Option<DateTime<Utc>>,
    /// Frontmatter lines this schema does not know, verbatim and in order.
    ///
    /// The agent writes facets whole and has always kept its own record on a task —
    /// `report_to:`, and dated note keys running to tens of kilobytes. None of that is
    /// schema, so a status change used to read the six fields it understood and write back
    /// only those, silently deleting the rest of the ledger. **A writer that does not
    /// understand a line is not thereby entitled to drop it.**
    pub extra: Vec<String>,
    pub body: String,
}

/// Frontmatter keys [`parse`] consumes, current and legacy. These are the only lines
/// [`render`] may re-author, because it re-emits each one canonically; every other line is
/// somebody else's and passes through untouched.
const SCHEMA_KEYS: [&str; 16] = [
    "status",
    "title",
    "created_at",
    "status_since",
    "due_at",
    "checked_at",
    "completed_at",
    "cancelled_at",
    "verify",
    "restart",
    "owner",
    "start_key",
    // Legacy, re-emitted in current form.
    "kind",
    "state",
    "due",
    "checked",
];

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
            status_since: Some(now),
            extra: Vec::new(),
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
        self.status_since = Some(at);
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

    /// Work that has been underway long enough that continuing to hold it open is itself
    /// the thing to answer for.
    ///
    /// `doing` only. `serving` is exempt because a duty being old is what a duty looks
    /// like, and `todo` because not-started-yet is a state it may sit in on purpose. The
    /// one status that promises an ending is the one that can fail to reach it.
    fn past_idle_boundary(&self, now: DateTime<Utc>) -> bool {
        self.status == TaskStatus::Doing
            && self
                .status_since
                .is_some_and(|since| (now - since).num_hours() >= IDLE_BOUNDARY_HOURS)
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

/// What the switchboard says is working a task **right now**.
///
/// Handed in rather than looked up: `mind::memory` does not depend on the registry, and more
/// to the point this is not memory's fact to hold. Who is on a task is a question about the
/// present, answered by whatever is actually registered — so it is computed at the join
/// ([`super::snapshot::agent_window`]) each turn and written down nowhere. A facet field
/// naming its worker would be a second copy, free to disagree with the switchboard, and wrong
/// by construction after a restart: it would still name a session that no longer exists.
#[derive(Debug, Clone)]
pub struct WorkingOnIt {
    pub session: crate::foundation::registry::SessionId,
    pub busy: bool,
    /// The last thing it was seen doing, already clipped by the registry.
    pub doing: Option<String>,
    /// When it last changed state, for "how long has it been like this".
    pub since: DateTime<Utc>,
    /// How its last finished turn ended, when the switchboard has seen one end.
    ///
    /// Carried because "who is on this task" and "is that going well" are one question to
    /// the reader of a ledger line, and until this field the line could only answer the
    /// first: a worker whose turn died reported as `idle`, which is what a worker between
    /// instructions reports, and the two are opposite situations.
    pub last_turn: Option<crate::foundation::registry::TurnOutcome>,
}

/// What the switchboard says about a task's worker — there is one, or the last restart took
/// the one there was.
///
/// **One map, not two lookups, because a task can only be in one of these states.** The thing
/// that makes a subject live is exactly the thing that drains its cut-off entry: a worker
/// registering under it ([`crate::foundation::registry::Registry::register`]). Two maps would
/// be free to say both, and the reader would have to pick.
#[derive(Debug, Clone)]
pub enum OnIt {
    /// A session is registered under this subject right now.
    Live(WorkingOnIt),
    /// The last restart killed its worker and nothing has picked the errand back up.
    ///
    /// Worth distinguishing from a plain absence because the two call for different moves.
    /// An errand nobody ever started has to be started; this one has a **mind still on the
    /// boot offer** ([`crate::foundation::registry::Registry::lost_workers`]), so the cheap
    /// answer is to resume it. It also stops the ledger from reporting the first minute of
    /// every restart as though the work had been abandoned.
    CutOff,
}

/// The active ledger as the agent reads it, annotated with who is on each task.
///
/// `working` is keyed by task subject. An empty map means nobody is on anything, and every
/// `doing` task then reads as having nobody on it — the state this whole annotation exists to
/// make visible. Immediately after a restart that is *true but not the whole answer*, which
/// is what [`OnIt::CutOff`] carries: the switchboard is empty because the process died, not
/// because the work was abandoned, and the difference is the difference between an alarm and
/// a resume.
pub async fn projection(
    data_dir: &Path,
    working: &std::collections::HashMap<String, OnIt>,
) -> anyhow::Result<String> {
    Ok(render_projection(&active_tasks(data_dir).await?, Utc::now(), working))
}

/// The ledger as it is sent, paired with the form a window should compare on.
///
/// **A worker's activity tail is liveness, not news.** `doing` is the last thing a session
/// was seen at — mid-shell-command, mid-edit — and it moves on every rung of every live
/// worker, so it lands in the ledger line of every task anybody is on. Measured across one
/// live thread of 188 turns, blanking it drops this block's distinct states from 149 to 87:
/// two of every five re-sends of the whole ledger were saying that a worker had gone from
/// thinking to running `ls`, and each re-send costs a permanent copy of itself in a finite
/// window. The field earns its place in the *text* — it exists so a silent worker cannot be
/// mistaken for a dead one ([`worker_note`]) — and that is a thing to read when you look,
/// never a thing to be told. So it rides out and is blanked in the comparison, exactly as
/// elapsed quantities already are by [`without_elapsed`].
///
/// **Rendered twice rather than stripped by pattern**, and that is the load-bearing part: a
/// worker running a shell command puts the command in that tail, commas and all, so no
/// regex over the finished line can tell it apart from the `last turn FAILED: …` note it
/// must *not* blank. Suppressing it at the source is the only way to be sure which is which.
pub async fn projection_and_comparable(
    data_dir: &Path,
    working: &std::collections::HashMap<String, OnIt>,
) -> anyhow::Result<(String, String)> {
    let active = active_tasks(data_dir).await?;
    let now = Utc::now();
    let sent = render_projection(&active, now, working);
    let compared = without_elapsed(&render_projection(&active, now, &without_activity(working)));
    Ok((sent, compared))
}

/// The same switchboard reading with every live worker's activity tail dropped.
fn without_activity(
    working: &std::collections::HashMap<String, OnIt>,
) -> std::collections::HashMap<String, OnIt> {
    working
        .iter()
        .map(|(subject, on_it)| {
            let quieted = match on_it {
                OnIt::Live(w) => {
                    let mut w = w.clone();
                    w.doing = None;
                    OnIt::Live(w)
                }
                other => other.clone(),
            };
            (subject.clone(), quieted)
        })
        .collect()
}

fn render_projection(
    active: &[Task],
    now: DateTime<Utc>,
    working: &std::collections::HashMap<String, OnIt>,
) -> String {
    use std::fmt::Write as _;

    // **An empty ledger says so out loud**, and that is not decoration. The window is sent
    // on change now, and a block that renders to nothing is skipped rather than sent — so a
    // silent empty meant the last duty could close and Reaction would go on believing it
    // was owed. Nothing else tells it: a task is closed by a file edit, not by a message.
    // Sixty characters, once, against a silently broken promise.
    if active.is_empty() {
        return "# Active tasks\n\n_Nothing open right now._\n".to_owned();
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
        if let Some(note) = worker_note(task, working.get(&task.subject), now) {
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
        let stalled = rest.iter().filter(|task| task.past_idle_boundary(now)).count();
        if stalled > 0 {
            let _ = write!(out, ", {stalled} waiting on a disposition from you");
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
///
/// Past [`IDLE_BOUNDARY_HOURS`] the age stops being the fact worth carrying and the
/// disposition does. An age is something to note; this is something to answer, and it names
/// the three answers there are so that a seventh probe cannot pass for one of them.
fn trailing_note(task: &Task, now: DateTime<Utc>) -> Option<String> {
    if task.past_idle_boundary(now) {
        let since = task.status_since?;
        return Some(format!(
            "last moved {} — close it with what you did verify, or ask once, or cancel it; \
             checking it again is not one of the three",
            ago(now, since)
        ));
    }
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

/// Who is on this task, or — where that is the alarming answer — that nobody is.
///
/// **"Nobody" is only said where nobody is a problem.** A `todo` with no worker is what a
/// `todo` *is*, and a `serving` duty spends most of its life with no live handler because a
/// handler is spawned per burst and idles out. Printing "nobody on it" on those would put the
/// phrase on most of the list and teach the reader to skip it — and then it would be skipped
/// on the one line where it means something.
///
/// That line is `doing`. `doing` is a claim that work is in flight, and the failure this
/// exists to end is the claim outliving the worker: a restart, a crash, a session that idled
/// out, an errand nobody ever started. From the outside those are indistinguishable from work
/// in progress, and stay that way until someone happens to look.
///
/// A live worker is reported wherever there is one, `todo` and `serving` included, because
/// that is positive information and cannot be a false alarm.
///
/// **A restart gets its own answer, and it is still "nobody on it".** The phrase is not
/// softened, because nobody is on it — but bare, it invites the reading that the errand was
/// dropped, and for the first minute or two of every boot that is wrong in a way the reader
/// cannot check: the process died, Cognition has not finished dispositioning the offer yet,
/// and the dead worker's mind is sitting there resumable. Naming the cause is what separates
/// "start a second worker on this" from "resume the one you have" — and, once the offer has
/// been taken up or the errand written off, the entry drains and the line goes back to the
/// bare phrase, which by then means what it says.
fn worker_note(task: &Task, on_it: Option<&OnIt>, now: DateTime<Utc>) -> Option<String> {
    match on_it {
        Some(OnIt::Live(w)) => {
            let state = if w.busy { "busy" } else { "idle" };
            let mut note = format!("worker {} — {state} {}", w.session, ago_short(now, w.since));
            // **`idle` after a turn that died says the wrong thing loudest.** The word is
            // the same one a worker waiting for its next instruction reports, so the line
            // that should read as an alarm reads as patience — and the move it invites is
            // "get on with it" rather than "it fell over". Only on a quiet worker: while it
            // is busy, what it is doing now is the answer and last turn's ending is stale.
            if let Some(outcome) =
                w.last_turn.as_ref().filter(|_| !w.busy).filter(|o| o.is_trouble())
            {
                match outcome.error() {
                    Some(err) => {
                        note.push_str(", last turn FAILED: ");
                        note.push_str(err);
                    }
                    None => note.push_str(", last turn was stopped"),
                }
            }
            // What it is *doing*, not what it has said: a worker four minutes into a shell
            // command has produced no output, and its silence is the thing most easily
            // mistaken for death.
            if let Some(doing) = w.doing.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
                note.push_str(", ");
                note.push_str(doing);
            }
            Some(note)
        }
        Some(OnIt::CutOff) if task.status == TaskStatus::Doing => Some(
            "nobody on it — the restart cut its worker off, and its thread is still on the \
             boot offer"
                .to_owned(),
        ),
        // Same rule as below: said only where nobody is a problem.
        Some(OnIt::CutOff) => None,
        None if task.status == TaskStatus::Doing => Some("nobody on it".to_owned()),
        None => None,
    }
}

/// `ago` without the trailing word, for a note that already supplies its own verb.
fn ago_short(now: DateTime<Utc>, then: DateTime<Utc>) -> String {
    ago(now, then).trim_end_matches(" ago").to_owned()
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

/// The ledger with every elapsed quantity blanked — **for comparing two turns, never for
/// reading.**
///
/// A ledger line carries how long something has been the way it is, and that number moves
/// on its own. Sixty-five of the ninety-two times the projection "changed" across one live
/// thread, the only difference was a clock: `last confirmed alive 1h ago` became `2h ago`,
/// and the whole 431-character ledger was sent again to say so. The window is finite and
/// a re-send costs a permanent copy of itself in it (`docs/arch/data.md`).
///
/// **What it blanks is the quantity, not the category**, which is the whole care here.
/// `never checked` still differs from `last confirmed alive 2h ago`; a task crossing the
/// idle boundary still reads as a change, because its note stops being an age and becomes
/// something to answer. Only *how long* is dropped, and how long is the one part nobody
/// needs told again.
///
/// It lives beside [`ago`] because it is the inverse of it, and the test below fails if
/// the two ever stop agreeing about what an elapsed quantity looks like.
pub fn without_elapsed(projection: &str) -> String {
    let mut out = String::with_capacity(projection.len());
    for (i, line) in projection.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let mut rest = line;
        while let Some(cut) = rest.find(|c: char| c.is_ascii_digit()) {
            let (head, tail) = rest.split_at(cut);
            let digits = tail.len() - tail.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            let (number, after) = tail.split_at(digits);
            // `3d ago`, `12h ago`, `5m ago`, `open 2d` — a count and a unit, and nothing
            // else in a ledger line is a bare number followed by one of these letters.
            let elapsed = matches!(after.as_bytes().first(), Some(b'm' | b'h' | b'd'))
                && after
                    .as_bytes()
                    .get(1)
                    .is_none_or(|c| !c.is_ascii_alphanumeric());
            out.push_str(head);
            if elapsed {
                out.push('#');
                rest = &after[1..];
            } else {
                out.push_str(number);
                rest = after;
            }
        }
        out.push_str(rest);
    }
    out
}

/// Overdue first, then duties nobody has confirmed alive, then work past the idle
/// boundary, then upcoming, then undated doing work, then duties known to be up, then todo
/// work.
///
/// A duty splits across two of those bands on purpose. A watch that has been confirmed
/// alive wants to be *seen* and not acted on, so it sits low; the same watch with nothing
/// to say it is running is the one line here that might already be silently dead, so it
/// sits at the top next to overdue work.
///
/// Work past the boundary rises for a reason the projection can't otherwise supply: only
/// [`PROJECTED_TASKS`] lines are printed, and the thing being fixed is a task that quietly
/// stops being read. A line that needs a decision cannot be allowed to fall off the bottom
/// of the list, and the longest-stuck goes first within the band. It sits *below* an
/// unconfirmed duty, which might be dead right now, and *above* a future due date, which is
/// not a problem yet.
type OrderKey<'a> = (usize, i64, &'a str);

fn order_key(task: &Task, now: DateTime<Utc>) -> OrderKey<'_> {
    match (task.due_at, task.status) {
        (Some(due), _) if due <= now => (0, due.timestamp(), &task.subject),
        _ if task.unconfirmed() => (1, 0, &task.subject),
        _ if task.past_idle_boundary(now) => (
            2,
            task.status_since.map_or(0, |since| since.timestamp()),
            &task.subject,
        ),
        (Some(due), _) => (3, due.timestamp(), &task.subject),
        (None, TaskStatus::Doing) => (4, 0, &task.subject),
        (None, TaskStatus::Serving) => (5, 0, &task.subject),
        (None, _) => (6, 0, &task.subject),
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
    let liveness = Liveness {
        verify: field("verify"),
        restart: field("restart"),
        owner: field("owner"),
        start_key: field("start_key"),
    };
    let created_at = field("created_at").and_then(|value| parse_timestamp(&value));
    Task {
        subject: subject.to_owned(),
        status: coerce_duty(status, &liveness),
        title: field("title").unwrap_or_else(|| subject.replace('-', " ")),
        created_at,
        due_at: field("due_at")
            .or_else(|| field("due"))
            .and_then(|value| parse_timestamp(&value)),
        liveness,
        checked_at: field("checked_at")
            .or_else(|| field("checked"))
            .and_then(|value| parse_timestamp(&value)),
        completed_at: field("completed_at").and_then(|value| parse_timestamp(&value)),
        cancelled_at: field("cancelled_at").and_then(|value| parse_timestamp(&value)),
        status_since: field("status_since")
            .and_then(|value| parse_timestamp(&value))
            .or(created_at),
        extra: foreign_frontmatter(content),
        body: strip_frontmatter(content).trim().to_owned(),
    }
}

/// Every task written before `serving` existed recorded its duties as `doing` carrying a
/// liveness contract, and that is the shape this reads back into place. It is a fallback
/// for a record, never the definition: a duty is `serving` because someone said so, and a
/// `serving` task with no contract stays one. The reverse — `doing` plus `verify:` — has no
/// legitimate reading left, since plain work has no business carrying those fields, so the
/// same rule covers a record written wrong today.
///
/// **What makes a record a duty is a way back, not a `verify:`.** A `verify:` alone is the
/// commonest thing plain work carries by mistake — an acceptance test the agent set itself,
/// filed in the liveness field because that is where checks go — and reading that as a duty
/// converts a finished delivery into something with no ending, which is the same camouflage
/// this file removed from the projection, one layer down. The `google-login-hi-agent-xyz`
/// record was exactly it: `verify:` plus an `owner:` note, no `restart:`, no `start_key:`,
/// a delivery that had shipped four days earlier. The Feishu watcher, a real duty, carries
/// all four. So a contract counts here only if it says how to bring the thing back —
/// nothing you cannot relaunch is machinery.
///
/// Two cases it cannot reach, both needing the agent to re-read its own ledger: a legacy
/// duty that recorded no contract at all, and one that recorded only how to check it.
fn coerce_duty(status: TaskStatus, liveness: &Liveness) -> TaskStatus {
    let has_way_back = liveness.restart.is_some() || liveness.start_key.is_some();
    if status == TaskStatus::Doing && has_way_back {
        return TaskStatus::Serving;
    }
    status
}

/// Every frontmatter line [`parse`] did not consume, verbatim and in order.
///
/// Line-based like [`frontmatter_field`] itself, with one allowance: an indented line
/// continues the key above it, so it travels with that key — dropped when the key was
/// schema and re-emitted canonically, kept when the key was somebody else's.
fn foreign_frontmatter(content: &str) -> Vec<String> {
    let Some(fm) = content.strip_prefix("---\n") else {
        return Vec::new();
    };
    let Some(end) = fm.find("\n---\n") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut under_schema_key = false;
    for line in fm[..end].lines() {
        if line.starts_with([' ', '\t']) {
            if !under_schema_key {
                out.push(line.to_owned());
            }
            continue;
        }
        match line.split_once(':') {
            Some((key, _)) if SCHEMA_KEYS.contains(&key.trim()) => under_schema_key = true,
            _ => {
                under_schema_key = false;
                out.push(line.to_owned());
            }
        }
    }
    out
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
    if let Some(status_since) = task.status_since {
        field("status_since", status_since.to_rfc3339().as_str())?;
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
    for line in &task.extra {
        let _ = writeln!(out, "{line}");
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

    /// The common case for the projection tests that predate the join: nothing registered, so
    /// every task is unattended. `doing` rows then carry "nobody on it", which is correct.
    fn nobody() -> std::collections::HashMap<String, OnIt> {
        std::collections::HashMap::new()
    }

    fn on_it(session: u64, busy: bool, doing: Option<&str>, since: DateTime<Utc>) -> WorkingOnIt {
        WorkingOnIt {
            session: session.into(),
            busy,
            doing: doing.map(str::to_string),
            since,
            last_turn: None,
        }
    }
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
        // An hour before `now()`: a fixture has just moved, so nothing is at the idle
        // boundary unless a test puts it there.
        task.status_since = Some(now() - Duration::hours(1));
        task
    }

    fn staffed(task: &Task, w: WorkingOnIt) -> std::collections::HashMap<String, OnIt> {
        std::collections::HashMap::from([(task.subject.clone(), OnIt::Live(w))])
    }

    fn cut_off(task: &Task) -> std::collections::HashMap<String, OnIt> {
        std::collections::HashMap::from([(task.subject.clone(), OnIt::CutOff)])
    }

    /// **The line this whole join exists for.** `doing` claims work is in flight; when the
    /// worker is gone — a restart, a crash, an idle-out, or an errand nobody ever started —
    /// the claim outlives it and reads exactly like work in progress. Nothing said so before,
    /// and the only way to find out was to notice.
    #[test]
    fn a_doing_task_with_no_worker_says_nobody_is_on_it() {
        let owed = task("Ship the multilingual fix", TaskStatus::Doing);
        let text = render_projection(std::slice::from_ref(&owed), now(), &nobody());
        assert!(text.contains("nobody on it"), "{text}");
    }

    /// **The first minute of a restart is not an abandoned errand, and used to read as one.**
    /// The switchboard is empty at boot by construction, so every `doing` task said "nobody
    /// on it" — true, and indistinguishable from work dropped days ago. On 2026-08-17 the
    /// voice read that ledger 67 seconds after a boot and reported five tasks as "只是开放任务,
    /// 不是在执行"; the workers were up 16 seconds later. The phrase stays, the cause goes
    /// with it.
    #[test]
    fn a_task_the_restart_unstaffed_says_what_happened_to_it() {
        let owed = task("Ship the multilingual fix", TaskStatus::Doing);
        let text = render_projection(std::slice::from_ref(&owed), now(), &cut_off(&owed));
        assert!(text.contains("nobody on it"), "the phrase is not softened: {text}");
        assert!(text.contains("restart cut its worker off"), "{text}");
        assert!(text.contains("boot offer"), "there is a mind to resume, and that is the move: {text}");
    }

    /// And it is the same rule as the bare phrase about *where* it may be said. A `todo` or a
    /// `serving` duty whose last handler died in the restart is not a problem to raise — the
    /// phrase would be on most of the list again, cause and all.
    #[test]
    fn a_cut_off_todo_or_duty_is_still_not_flagged() {
        for status in [TaskStatus::Todo, TaskStatus::Serving] {
            let owed = task("Watch the ops group", status);
            let text = render_projection(std::slice::from_ref(&owed), now(), &cut_off(&owed));
            assert!(!text.contains("nobody on it"), "{status:?}: {text}");
            assert!(!text.contains("restart"), "{status:?}: {text}");
        }
    }

    /// And when someone *is* on it, the line carries the three facts that separate working
    /// from wedged: which session, whether it is mid-turn, and how long it has been that way.
    #[test]
    fn a_staffed_task_names_its_worker_and_how_long_it_has_been_like_that() {
        let owed = task("Ship the multilingual fix", TaskStatus::Doing);
        let text = render_projection(
            std::slice::from_ref(&owed),
            now(),
            &staffed(&owed, on_it(9, true, Some("$ docker push harbor/ktv"), now() - Duration::minutes(4))),
        );
        assert!(text.contains("worker 9"), "{text}");
        assert!(text.contains("busy 4m"), "{text}");
        assert!(text.contains("$ docker push harbor/ktv"), "the tool line, not the output tail: {text}");
        assert!(!text.contains("nobody on it"), "{text}");
    }

    /// A worker idle for forty minutes is the shape of a wedge, and it must not read as busy.
    #[test]
    fn an_idle_worker_is_not_reported_as_working() {
        let owed = task("Ship the multilingual fix", TaskStatus::Doing);
        let text = render_projection(
            std::slice::from_ref(&owed),
            now(),
            &staffed(&owed, on_it(9, false, None, now() - Duration::minutes(40))),
        );
        assert!(text.contains("idle 40m"), "{text}");
        assert!(!text.contains("busy"), "{text}");
    }

    /// **And an idle worker whose turn died must not read like one waiting for orders.**
    /// They are the same word — `idle` — and on 2026-08-18 that cost three workers a real
    /// recovery: they had each fallen over on a 429, the ledger line said `idle`, and what
    /// it got in reply was "Continue now; do not leave this idle".
    #[test]
    fn an_idle_worker_whose_turn_failed_says_so_with_the_reason() {
        let owed = task("Ship the multilingual fix", TaskStatus::Doing);
        let mut w = on_it(9, false, Some("$ cargo test"), now() - Duration::minutes(3));
        w.last_turn = Some(crate::foundation::registry::TurnOutcome::Failed(
            "exceeded retry limit, last status: 429 Too Many Requests".into(),
        ));
        let text = render_projection(std::slice::from_ref(&owed), now(), &staffed(&owed, w));
        assert!(text.contains("idle 3m"), "{text}");
        assert!(text.contains("last turn FAILED"), "{text}");
        assert!(text.contains("429 Too Many Requests"), "the reason travels: {text}");
    }

    /// Said on a quiet worker only, and only about an ending worth chasing. Mid-turn, what
    /// it is doing now is the answer; and a line that announced every clean turn would be on
    /// most of the list, which is how a line stops being read.
    #[test]
    fn a_busy_worker_or_a_clean_ending_says_nothing_about_the_last_turn() {
        let owed = task("Ship the multilingual fix", TaskStatus::Doing);

        let mut busy = on_it(9, true, None, now() - Duration::minutes(1));
        busy.last_turn = Some(crate::foundation::registry::TurnOutcome::Failed("429".into()));
        let text = render_projection(std::slice::from_ref(&owed), now(), &staffed(&owed, busy));
        assert!(!text.contains("last turn"), "it is working now: {text}");

        let mut clean = on_it(9, false, None, now() - Duration::minutes(1));
        clean.last_turn = Some(crate::foundation::registry::TurnOutcome::Completed);
        let text = render_projection(std::slice::from_ref(&owed), now(), &staffed(&owed, clean));
        assert!(!text.contains("last turn"), "a clean ending is not news: {text}");
    }

    /// **"Nobody" is only said where nobody is a problem.** A `todo` with no worker is what a
    /// `todo` is, and a `serving` duty spends most of its life between handler bursts. Saying
    /// it on those would put the phrase on most of the list, and a phrase on most of the list
    /// is one the reader stops seeing — including on the `doing` line where it means
    /// something.
    #[test]
    fn an_unattended_todo_or_duty_is_not_flagged() {
        for status in [TaskStatus::Todo, TaskStatus::Serving] {
            let owed = task("Watch the ops group", status);
            let text = render_projection(std::slice::from_ref(&owed), now(), &nobody());
            assert!(!text.contains("nobody on it"), "{status:?}: {text}");
        }
    }

    /// A live worker is reported wherever there is one, though — that is positive information
    /// and cannot be a false alarm. A duty with a handler up is worth seeing.
    #[test]
    fn a_live_handler_shows_on_a_duty() {
        let duty = task("Watch the ops group", TaskStatus::Serving);
        let text = render_projection(
            std::slice::from_ref(&duty),
            now(),
            &staffed(&duty, on_it(12, true, None, now() - Duration::seconds(30))),
        );
        assert!(text.contains("worker 12"), "{text}");
    }

    #[tokio::test]
    async fn current_schema_round_trips_without_kind_or_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut task = task("File the Feishu digest", TaskStatus::Serving);
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
        assert!(raw.contains("status: serving"));
        assert!(raw.contains("created_at:"));
        assert!(raw.contains("due_at:"));
        assert!(!raw.contains("\nkind:"));
        assert!(!raw.contains("\nstate:"));

        let got = read_task(dir.path(), "File the Feishu digest").await.unwrap().unwrap();
        assert_eq!(got.status, TaskStatus::Serving);
        assert_eq!(got.created_at, Some(at(1, 9)));
        assert_eq!(got.due_at, Some(at(30, 9)));
        assert_eq!(got.checked_at, Some(at(28, 10)));
        assert_eq!(got.body, "Boss asked for a daily digest of the ops group.");
    }

    /// The ledger a real task carries is mostly not schema: `report_to:`, and dated note
    /// keys running to tens of kilobytes. Closing one used to write back only the fields
    /// this file understands, so the button that files a task deleted its record — the
    /// live `google-login-hi-agent-xyz` facet stood at 53KB of such keys against a 4KB
    /// body. A status change must move the status and nothing else.
    #[tokio::test]
    async fn a_status_change_keeps_the_notes_it_does_not_understand() {
        let dir = tempfile::tempdir().unwrap();
        facets::update_facet(
            dir.path(),
            DIMENSION,
            "google-login",
            "---\n\
             kind: wip\n\
             state: open\n\
             title: \"Google login\"\n\
             report_to: prdo8qht\n\
             checked: 2026-08-11T10:27:00+08:00\n\
             SHIPPED_20260807: \"deployed, waiting on his own retry\"\n\
             wrapped: |\n\
             \x20 a value that runs on\n\
             \x20 to a second line\n\
             ---\n\n\
             What he asked for, in his words.\n",
        )
        .await
        .unwrap();

        let mut task = read_task(dir.path(), "google-login").await.unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Doing, "legacy wip reads as doing");
        task.set_status(TaskStatus::Done, at(11, 10));
        write_task(dir.path(), &task).await.unwrap();

        let raw = facets::read_facet(dir.path(), DIMENSION, "google-login")
            .await
            .unwrap()
            .unwrap();
        assert!(raw.contains("status: done"), "{raw}");
        assert!(raw.contains("completed_at:"), "{raw}");
        assert!(raw.contains("report_to: prdo8qht"), "{raw}");
        assert!(
            raw.contains("SHIPPED_20260807: \"deployed, waiting on his own retry\""),
            "the note key survived verbatim: {raw}"
        );
        assert!(raw.contains("wrapped: |\n  a value that runs on\n  to a second line"), "{raw}");
        assert!(raw.contains("What he asked for, in his words."), "{raw}");
        // The legacy pair is re-authored, not carried alongside the status it became.
        assert!(!raw.contains("\nkind:"), "{raw}");
        assert!(!raw.contains("\nstate:"), "{raw}");
        assert!(!raw.contains("\nchecked:"), "legacy `checked` became `checked_at`: {raw}");
        assert_eq!(raw.matches("title:").count(), 1, "{raw}");
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

    /// The ledger that existed the day before `serving` did, in the shape it is actually
    /// in: a duty there is a `doing` task that says how to bring its machinery back.
    /// Nothing rewrites those files, so reading them wrong would have cost each of them the
    /// one line that says whether it is still up.
    ///
    /// Counted over the live ledger, 17 records carry a liveness field: `verify:` alone
    /// appears on 13 of them, `status: todo` and `kind: staged` records included, and
    /// `owner:` on 16 — both of those are where a note lands, not a duty. Only the two real
    /// duties record a `restart:` or a `start_key:`.
    #[tokio::test]
    async fn a_duty_written_before_serving_existed_reads_back_as_one() {
        let dir = tempfile::tempdir().unwrap();
        facets::update_facet(
            dir.path(),
            DIMENSION,
            "watch-the-ops-group",
            "---\nstatus: doing\ntitle: \"Watch the ops group\"\n\
             checked_at: \"2026-07-28T10:00:00Z\"\nverify: \"latest row is under 30m old\"\n\
             start_key: watch-the-ops-group\n---\n",
        )
        .await
        .unwrap();
        let got = read_task(dir.path(), "watch-the-ops-group").await.unwrap().unwrap();
        assert_eq!(got.status, TaskStatus::Serving);

        let text = render_projection(std::slice::from_ref(&got), now(), &nobody());
        assert!(text.contains("- [serving] Watch the ops group"), "{text}");
        assert!(text.contains("last confirmed alive 2h ago"), "{text}");

        // And the correction persists the moment anything writes the task back.
        write_task(dir.path(), &got).await.unwrap();
        let raw = facets::read_facet(dir.path(), DIMENSION, "watch-the-ops-group")
            .await
            .unwrap()
            .unwrap();
        assert!(raw.contains("status: serving"), "{raw}");
    }

    /// Only that one shape moves. A duty is `serving` because someone said so, and plain
    /// work stays plain work.
    #[tokio::test]
    async fn coercion_does_not_reach_past_the_shape_it_is_for() {
        assert_eq!(
            coerce_duty(TaskStatus::Serving, &Liveness::default()),
            TaskStatus::Serving,
            "a duty with no contract recorded is still a duty"
        );
        assert_eq!(
            coerce_duty(TaskStatus::Doing, &Liveness::default()),
            TaskStatus::Doing
        );
        for closed in [TaskStatus::Done, TaskStatus::Cancelled, TaskStatus::Todo] {
            let contract = Liveness {
                verify: Some("a row landed today".into()),
                ..Liveness::default()
            };
            assert_eq!(coerce_duty(closed, &contract), closed, "{closed:?}");
        }
    }

    /// The clock is time in status, not time since the task was written down. Work opened a
    /// month ago and picked up an hour ago is fresh; the same record left in `doing` is not.
    #[test]
    fn the_boundary_reads_time_in_status_not_age_since_creation() {
        let mut fresh = task("Ship Google login", TaskStatus::Doing);
        fresh.created_at = Some(now() - Duration::days(30));
        let text = render_projection(std::slice::from_ref(&fresh), now(), &nobody());
        assert!(text.contains("· open 30d"), "{text}");
        assert!(!text.contains("close it with"), "{text}");

        let mut stuck = fresh.clone();
        stuck.status_since = Some(now() - Duration::days(4));
        let text = render_projection(std::slice::from_ref(&stuck), now(), &nobody());
        assert!(text.contains("last moved 4d ago"), "{text}");
        assert!(
            text.contains("close it with what you did verify, or ask once, or cancel it"),
            "{text}"
        );
        assert!(text.contains("checking it again is not one of the three"), "{text}");
        assert!(!text.contains("open 30d"), "the age is no longer the fact: {text}");

        // The boundary itself: an hour short of it says nothing.
        let mut short = fresh.clone();
        short.status_since = Some(now() - Duration::hours(IDLE_BOUNDARY_HOURS - 1));
        let text = render_projection(std::slice::from_ref(&short), now(), &nobody());
        assert!(!text.contains("close it with"), "{text}");
    }

    /// Only the status that promises an ending can fail to reach one. A duty is meant to be
    /// old, and `todo` is a status work can sit in on purpose.
    #[test]
    fn only_work_that_promises_an_ending_meets_the_boundary() {
        let old = now() - Duration::days(9);
        for exempt in [TaskStatus::Serving, TaskStatus::Todo] {
            let mut task = task("Watch the ops group", exempt);
            task.status_since = Some(old);
            task.liveness.verify = Some("latest row is under 30m old".into());
            task.liveness.start_key = Some("watch-the-ops-group".into());
            let text = render_projection(std::slice::from_ref(&task), now(), &nobody());
            assert!(!text.contains("close it with"), "{exempt:?}: {text}");
        }

        let mut work = task("Draft the report", TaskStatus::Doing);
        work.status_since = Some(old);
        let text = render_projection(std::slice::from_ref(&work), now(), &nobody());
        assert!(text.contains("close it with"), "{text}");
    }

    /// A transition restamps the clock, a write persists it, and a record that predates the
    /// field falls back to `created_at` — older, so it errs toward the boundary rather than
    /// hiding behind it.
    #[tokio::test]
    async fn a_transition_restamps_the_clock_and_an_old_record_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut task = task("Ship Google login", TaskStatus::Doing);
        task.status_since = Some(now() - Duration::days(4));
        assert!(task.past_idle_boundary(now()));

        task.set_status(TaskStatus::Done, now());
        assert_eq!(task.status_since, Some(now()), "the clock moves with the status");
        write_task(dir.path(), &task).await.unwrap();
        let got = read_task(dir.path(), "ship-google-login").await.unwrap().unwrap();
        assert_eq!(got.status_since, Some(now()));

        facets::update_facet(
            dir.path(),
            DIMENSION,
            "older-than-the-field",
            "---\nstatus: doing\ntitle: \"Older than the field\"\n\
             created_at: \"2026-07-01T09:00:00Z\"\n---\n",
        )
        .await
        .unwrap();
        let legacy = read_task(dir.path(), "older-than-the-field").await.unwrap().unwrap();
        assert_eq!(legacy.status_since, Some(at(1, 9)));
        assert!(legacy.past_idle_boundary(now()), "it is caught, not exempted");
    }

    /// The projection prints [`PROJECTED_TASKS`] lines, and the failure being fixed is a
    /// task that stops being read. So a line needing a decision has to be one of the twelve,
    /// however long the list gets.
    #[test]
    fn work_past_the_boundary_cannot_fall_off_the_projection() {
        let mut tasks: Vec<Task> = (0..PROJECTED_TASKS + 8)
            .map(|i| task(&format!("routine {i:02}"), TaskStatus::Doing))
            .collect();
        let mut stuck = task("zzz last alphabetically", TaskStatus::Doing);
        stuck.status_since = Some(now() - Duration::days(4));
        tasks.push(stuck);

        let text = render_projection(&tasks, now(), &nobody());
        let first = text.lines().find(|line| line.starts_with("- [")).unwrap();
        assert!(first.contains("zzz last alphabetically"), "{text}");
        assert!(first.contains("close it with"), "{text}");
    }

    /// The two shapes plain work carries by mistake, and the reason the rule asks for a way
    /// back rather than for any liveness field at all. A delivery that set itself an
    /// acceptance test, and one whose `owner:` holds a note — read either as a duty and a
    /// finished piece of work becomes something with no ending, exempt from the boundary
    /// that would have caught it.
    #[test]
    fn an_acceptance_test_is_not_machinery_and_neither_is_an_owner_note() {
        let acceptance = Liveness {
            verify: Some("the two OAuth routes exist and key on Google's `sub`".into()),
            owner: Some("cognition shipped it; person-blocked on his own retry".into()),
            ..Liveness::default()
        };
        assert_eq!(
            coerce_duty(TaskStatus::Doing, &acceptance),
            TaskStatus::Doing,
            "the google-login record: shipped work, not a watch"
        );

        for way_back in [
            Liveness { restart: Some("launchctl kickstart the label".into()), ..Liveness::default() },
            Liveness { start_key: Some("feishu-it-group-watcher".into()), ..Liveness::default() },
        ] {
            assert_eq!(coerce_duty(TaskStatus::Doing, &way_back), TaskStatus::Serving);
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
        let text = render_projection(&[plain, late], now(), &nobody());
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
        let text = render_projection(std::slice::from_ref(&plain), now(), &nobody());
        assert!(!text.contains("checked"), "{text}");

        let mut duty = task("Watch the queue", TaskStatus::Serving);
        duty.liveness.verify = Some("latest ledger row is under 30m old".into());
        let text = render_projection(std::slice::from_ref(&duty), now(), &nobody());
        assert!(text.contains("- [serving] Watch the queue"), "{text}");
        assert!(text.contains("never checked"), "{text}");

        duty.checked_at = Some(now() - Duration::hours(3));
        let text = render_projection(std::slice::from_ref(&duty), now(), &nobody());
        assert!(text.contains("last confirmed alive 3h ago"), "{text}");

        // A duty with nothing recorded to check it says so, rather than going quiet.
        let mut unverifiable = task("Watch the queue", TaskStatus::Serving);
        unverifiable.liveness = Liveness::default();
        let text = render_projection(std::slice::from_ref(&unverifiable), now(), &nobody());
        assert!(text.contains("never checked, and no recorded way to"), "{text}");

        // Carrying the fields is not what makes it a duty; the status is.
        duty.status = TaskStatus::Doing;
        let text = render_projection(std::slice::from_ref(&duty), now(), &nobody());
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

        let text = render_projection(&[confirmed, work, unconfirmed], now(), &nobody());
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
        let text = render_projection(&tasks, now(), &nobody());
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
        let text = render_projection(std::slice::from_ref(&task), now(), &nobody());
        let line = text.lines().find(|line| line.starts_with("- ")).unwrap();
        // The trailing note is appended after the clip, so only the title half is bounded.
        let (title, _note) = line.split_once(" · ").unwrap();
        assert_eq!(title.chars().count(), PROJECTED_LINE_CHARS);
    }

    #[test]
    fn plain_work_carries_how_long_it_has_been_open() {
        let mut fresh = task("Draft the report", TaskStatus::Doing);
        fresh.created_at = Some(now() - Duration::hours(5));
        let text = render_projection(std::slice::from_ref(&fresh), now(), &nobody());
        assert!(!text.contains("open "), "{text}");

        let mut stale = task("Draft the report", TaskStatus::Doing);
        stale.created_at = Some(now() - Duration::days(6));
        let text = render_projection(std::slice::from_ref(&stale), now(), &nobody());
        assert!(text.contains("· open 6d"), "{text}");

        // A watch is meant to be old, so its line stays about whether it is still alive.
        let mut duty = stale.clone();
        duty.status = TaskStatus::Serving;
        duty.liveness.verify = Some("latest ledger row is under 30m old".into());
        let text = render_projection(std::slice::from_ref(&duty), now(), &nobody());
        assert!(!text.contains("open 6d"), "{text}");
        assert!(text.contains("never checked"), "{text}");

        // A record with no `created_at:` gets no note rather than a guessed one.
        let mut undated = stale.clone();
        undated.created_at = None;
        let text = render_projection(std::slice::from_ref(&undated), now(), &nobody());
        assert!(!text.contains("open "), "{text}");
    }
}

#[cfg(test)]
mod without_elapsed_tests {
    use super::*;
    use chrono::Duration;

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, day, hour, 0, 0).unwrap()
    }

    /// Pinned against [`ago`] itself: every shape it can produce must come out blanked, so
    /// the two cannot drift apart silently.
    #[test]
    fn every_shape_ago_produces_is_blanked() {
        let then = at(20, 8);
        for mins in [0, 5, 59, 60, 60 * 23, 60 * 24, 60 * 24 * 9] {
            let now = then + Duration::minutes(mins);
            let line = format!("- [serving] Watch it · last confirmed alive {}", ago(now, then));
            let blanked = without_elapsed(&line);
            assert!(
                !blanked.chars().any(|c| c.is_ascii_digit()),
                "{mins}m: `{line}` left digits in `{blanked}`"
            );
        }
    }

    /// The quantity goes; the category stays. This is the line between "nothing happened"
    /// and "something did", and getting it wrong in either direction is a real cost: one way
    /// the ledger is re-sent for nothing, the other way a duty changes and nobody is told.
    #[test]
    fn the_category_survives_and_only_the_number_goes() {
        let quiet = "- [serving] Watch it · last confirmed alive 1h ago";
        let louder = "- [serving] Watch it · last confirmed alive 9h ago";
        assert_eq!(without_elapsed(quiet), without_elapsed(louder));

        // Never checked is not "checked a while ago", and crossing the idle boundary
        // replaces an age with something to answer.
        let never = "- [serving] Watch it · never checked";
        assert_ne!(without_elapsed(quiet), without_elapsed(never));
        let moved = "- [doing] Ship it · last moved 3d ago — close it with what you did verify";
        let open = "- [doing] Ship it · open 3d";
        assert_ne!(without_elapsed(moved), without_elapsed(open));
    }

    /// A date is not an elapsed quantity, and neither is a number in a title. Both stay, so
    /// a task going overdue still reads as a change.
    #[test]
    fn dates_and_titles_are_left_alone() {
        let overdue = "- [doing, overdue since 2026-07-20 08:00Z] Renew the domain";
        assert_eq!(without_elapsed(overdue), overdue);
        assert_eq!(without_elapsed("- [todo] KT8-059 timestamp control"), "- [todo] KT8-059 timestamp control");
    }
}
