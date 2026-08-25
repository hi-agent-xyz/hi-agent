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

use chrono::{DateTime, NaiveDate, SecondsFormat, TimeZone, Utc};

use super::episodes::{frontmatter_field, jstr, strip_frontmatter};
use super::{facets, layout, task_history};

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

/// The heading the running record lives under, and the only body structure this schema
/// owns. Everything above it is the writer's own prose and passes through untouched.
const TIMELINE_HEADING: &str = "## Timeline";

/// What a line in a task's running record is saying.
///
/// **Five words, because a sixth is a paragraph.** This record is read by the person
/// catching up on their own errand, not by an auditor: what they asked for, what landed,
/// what is in the way, what was checked, and where the row stands. Anything a mind wants
/// to say that is none of those belongs in the prose above the timeline, which has room
/// for it and is not read line by line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineKind {
    /// What would make this right, in the person's words. Written once, at open, by the
    /// rung that was in the conversation — see `cognition.md`. It is a **reading, not a
    /// gate**: nothing waits on it and no task is held open against it.
    Asked,
    /// Something was delivered. The milestone.
    Landed,
    /// Something is in the way, and what it is.
    Blocked,
    /// A verification and what came back — including when what came back was wrong.
    Checked,
    /// A status transition. **Written by the store, never by a mind**, on the same pass
    /// that stamps `status_since`: a mind that has to remember to record its own
    /// transition is a mind that will sometimes not.
    Moved,
    /// A line this schema does not recognise, kept exactly as it was written. The
    /// frontmatter rule one level down: a writer that does not understand a line is not
    /// thereby entitled to drop it.
    Note,
}

impl TimelineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Asked => "asked",
            Self::Landed => "landed",
            Self::Blocked => "blocked",
            Self::Checked => "checked",
            Self::Moved => "moved",
            Self::Note => "note",
        }
    }

    /// `None` for anything else, so an unrecognised first word stays part of the text
    /// rather than being eaten as a kind.
    fn parse(word: &str) -> Option<Self> {
        match word.trim().trim_end_matches([':', ',']).to_ascii_lowercase().as_str() {
            "asked" => Some(Self::Asked),
            "landed" => Some(Self::Landed),
            "blocked" => Some(Self::Blocked),
            "checked" => Some(Self::Checked),
            "moved" => Some(Self::Moved),
            "note" => Some(Self::Note),
            _ => None,
        }
    }
}

/// One line of a task's running record.
///
/// `at` is optional because a hand-written line carrying no instant is still something
/// somebody meant to record, and a reader that drops what it cannot date is exactly the
/// writer [`Task::extra`] exists to forbid. Nothing here invents a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineEntry {
    pub at: Option<DateTime<Utc>>,
    pub kind: TimelineKind,
    pub text: String,
}

impl TimelineEntry {
    pub fn new(kind: TimelineKind, at: DateTime<Utc>, text: impl Into<String>) -> Self {
        Self { at: Some(at), kind, text: text.into() }
    }

    /// The one entry the store writes for itself.
    fn moved(before: TaskStatus, after: TaskStatus, at: DateTime<Utc>) -> Self {
        Self::new(
            TimelineKind::Moved,
            at,
            format!("{} \u{2192} {}", before.as_str(), after.as_str()),
        )
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
    /// Prose above the timeline: the long-form account, written whole by whoever is
    /// keeping it. Not read line by line and not rendered on a card.
    pub body: String,
    /// The running record, oldest first — **append-only, and that is the whole design.**
    ///
    /// The body used to be one prose blob three writers shared, and a rewrite by any of
    /// them silently replaced the other two. Dated lines do not fix a clobber, but they
    /// make one *visible*: an entry that disappears leaves a gap in a sequence, where a
    /// rewritten paragraph leaves nothing at all. Order is the file's order, never sorted
    /// — a record that appends is already chronological, and re-sorting one that is not
    /// would assert a history nobody wrote.
    pub timeline: Vec<TimelineEntry>,
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
            timeline: Vec::new(),
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
        let before = self.status;
        self.status = status;
        self.stamp_transition(before, at);
    }

    /// Write down that this record's **current** status was reached at `at`.
    ///
    /// Split out of [`Self::set_status`] because the two callers know the transition in
    /// different ways and must not disagree about what it implies. `set_status` is told —
    /// it changes the word itself. [`reconcile`] finds out — the word was already changed
    /// on disk by a mind that had no obligation to stamp anything, and all the pass can
    /// say is *this differs from what it said last time*. One body, so a transition means
    /// the same thing however it was learned.
    ///
    /// It writes the timeline's `moved` entry too, for the reason the stamps are here at
    /// all: a consequence of a decision already recorded is not something a mind should
    /// have to remember. `status_since` answers *how long has it been like this*; the
    /// entry answers *what happened, and when* — the second question the first one has
    /// never been able to reach, because a scalar is overwritten by the next transition.
    fn stamp_transition(&mut self, before: TaskStatus, at: DateTime<Utc>) {
        self.status_since = Some(at);
        self.timeline
            .push(TimelineEntry::moved(before, self.status, at));
        match self.status {
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

    /// Make the stamps agree with the status **without inventing an instant**.
    ///
    /// This is the cold path — no previous read to compare against, so *when* the status
    /// moved is not knowable. Two of the three repairs need no clock at all:
    ///
    /// - An **open** record cannot carry a closing stamp. `todo`/`doing`/`serving` mean
    ///   this has not ended, so a `completed_at` beside one is a leftover from a close
    ///   that was undone by hand, and dropping it asserts nothing new.
    /// - A **closed** record with a `status_since` already says when it last moved, and
    ///   for a closed record that move *is* the close. So the closing stamp is derived
    ///   from it rather than guessed.
    ///
    /// The third case — closed, and no `status_since` either — is left alone on purpose.
    /// `now` would be a lie with a plausible face: a task finished in June would read as
    /// finished today, and nothing downstream could tell that stamp from a real one. The
    /// same refusal the parser already makes for `created_at`, which it will not invent.
    /// Records in that state are the pass's to *report*, never to fill in.
    fn derive_stamps(&mut self) -> bool {
        let before = (self.completed_at, self.cancelled_at);
        match self.status {
            TaskStatus::Todo | TaskStatus::Doing | TaskStatus::Serving => {
                self.completed_at = None;
                self.cancelled_at = None;
            }
            TaskStatus::Done => {
                self.cancelled_at = None;
                if self.completed_at.is_none() {
                    self.completed_at = self.status_since;
                }
            }
            TaskStatus::Cancelled => {
                self.completed_at = None;
                if self.cancelled_at.is_none() {
                    self.cancelled_at = self.status_since;
                }
            }
        }
        before != (self.completed_at, self.cancelled_at)
    }

    /// Prose and running record as one text: the body exactly as the file carries it.
    ///
    /// The struct splits them because the two are read differently — the panel renders
    /// the timeline as lines and the prose as a block — but anything handed a task
    /// *whole* (a duty handler's brief, [`render`]) wants them joined, and joining them
    /// here is what keeps one source of truth for the bytes.
    pub fn record(&self) -> String {
        let mut out = self.body.trim().to_owned();
        if !self.timeline.is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(render_timeline(&self.timeline).trim_end());
        }
        out
    }

    /// What would make this right, in their words — the first thing a person catching up
    /// on their own errand wants, and the reason it is pinned rather than scrolled to.
    pub fn asked(&self) -> Option<&TimelineEntry> {
        self.timeline
            .iter()
            .find(|entry| entry.kind == TimelineKind::Asked)
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

/// Write a record, and tell [`reconcile`] this status is now the one on disk.
///
/// **The second half is what keeps the timeline from stuttering.** A status changed
/// through code has already recorded its own `moved` entry ([`Task::set_status`]); left
/// unsaid, the next pass would compare against a [`LAST_SEEN`] holding the *old* word,
/// witness a transition that had already been written down, and append a second entry
/// dated later than the first. So the two writers of a transition — code, which is told,
/// and the pass, which finds out — are kept from both claiming the same one.
pub async fn write_task(data_dir: &Path, task: &Task) -> anyhow::Result<String> {
    let content = render(task);
    let written = facets::update_facet(data_dir, DIMENSION, &task.subject, &content).await?;
    if let Ok(mut seen) = last_seen().lock() {
        let key = facets::subject_dir(data_dir, DIMENSION, &task.subject);
        seen.entry(key).or_default().status = Some(task.status);
    }
    Ok(written)
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

/// How many open rows a refusal offers back. Enough that the row being looked for is almost
/// always in the list, few enough that the list stays readable — one live store carries 108
/// subjects and twelve of them open.
const OFFERED_SUBJECTS: usize = 30;

/// What the ledger says about a subject somebody named.
#[derive(Debug)]
pub enum Named {
    /// A row exists, and this is the subject **as the ledger spells it**. The join between a
    /// task and the session working it is an exact match on the directory name, so a raw
    /// `Login failure` handed on would name a row that exists and match nothing.
    Row(String),
    /// Nothing is filed under that name — with the open rows rendered beside it, because a
    /// refusal that only says *no* is the one that gets answered by coining a near-duplicate.
    /// Empty when the ledger is empty.
    Missing { open: String },
}

/// Look `subject` up, and answer a miss with the ledger itself.
///
/// **Nothing here writes.** A row is a promise somebody made, and the one thing that keeps
/// the list worth reading is that every line on it was put there deliberately — an
/// auto-opened row is a row nobody decided to owe. So this reports, and a mind with a shell
/// does the opening ([`write_task`] is not the path; the writer edits the file).
///
/// **The list is the point of the miss.** Two workers on one job, a review filed as its own
/// task, a second row for work already tracked under another name — each of those is a
/// dispatcher that did not know what the ledger already held. It holds it right here, so the
/// answer to *no such row* arrives with the rows there are.
pub async fn named(data_dir: &Path, subject: &str) -> anyhow::Result<Named> {
    let subject = facets::slug(subject);
    if !subject.is_empty() && read_task(data_dir, &subject).await?.is_some() {
        return Ok(Named::Row(subject));
    }
    Ok(Named::Missing { open: open_rows(data_dir).await })
}

/// The open ledger, one line per row, for a reader choosing among them.
///
/// Never fails: this is the helpful half of a refusal that has already been decided, and a
/// ledger that cannot be read is a reason to say less, not a second error to raise.
async fn open_rows(data_dir: &Path) -> String {
    use std::fmt::Write as _;

    let mut open = match active_tasks(data_dir).await {
        Ok(open) => open,
        Err(error) => {
            tracing::warn!(%error, "could not read the ledger to offer its open rows");
            return String::new();
        }
    };
    open.sort_by(|a, b| {
        (a.status.as_str(), a.subject.as_str()).cmp(&(b.status.as_str(), b.subject.as_str()))
    });
    let mut out = String::new();
    for task in open.iter().take(OFFERED_SUBJECTS) {
        let _ = writeln!(out, "- `{}` [{}] {}", task.subject, task.status.as_str(), task.title);
    }
    if open.len() > OFFERED_SUBJECTS {
        let _ = writeln!(out, "- (and {} more open)", open.len() - OFFERED_SUBJECTS);
    }
    out
}

/// What each subject's status was the last time [`reconcile`] looked.
///
/// **In memory on purpose, and lossy on purpose.** Written down it would be a second
/// record of what is owed, and the one thing the ledger's design refuses is a second
/// record — a durable copy is free to disagree with the file and nothing could say which
/// was right. Held here it can only ever be *stale*, and stale degrades to the cold path,
/// which invents nothing. So a restart costs the pass its ability to date a transition it
/// did not witness, and costs it nothing else.
///
/// Keyed by the record's own path, not by its subject: a subject is unique within one
/// store and says nothing across two, and one process holding two stores is the ordinary
/// case under test.
static LAST_SEEN: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<PathBuf, Seen>>,
> = std::sync::OnceLock::new();

/// One subject directory as the pass last found it.
///
/// The status and the file marks are remembered together because they are learned in the
/// same read and are stale in the same way — a restart empties both, and both degrade to
/// the cold path rather than to a wrong answer.
#[derive(Debug, Default)]
struct Seen {
    /// The status word this pass last read off disk. `None` before it has ever looked, which
    /// is what makes a first sight after a restart a non-event rather than a transition.
    status: Option<TaskStatus>,
    /// What [`task_history::keep`] found in this directory, so an untouched file costs a
    /// `stat` and nothing more.
    files: task_history::DirState,
}

fn last_seen() -> &'static std::sync::Mutex<std::collections::HashMap<PathBuf, Seen>> {
    LAST_SEEN.get_or_init(Default::default)
}

/// Make every record's mechanical fields agree with its status, and re-emit legacy
/// spellings in the current one. Returns how many files were rewritten.
///
/// **This exists because the ledger has no code on its write path and should not get
/// one.** Every agent that may write a task has a shell, so a task is changed by editing
/// a file — the host does not see it happen, and a verb it could be *asked* to use is a
/// door beside an open wall, absent exactly when it is forgotten and silent about being
/// absent. What cannot be walked around is a pass that re-reads the bytes, because it
/// reads whatever is actually there however it got there. So the writer is left to decide
/// the one thing that is a judgment — the status word — and everything that follows
/// mechanically from that word is repaired here, on the read the window already does.
///
/// The repairs are deliberately of two grades. A transition this pass **witnessed** (the
/// status differs from [`LAST_SEEN`]) is dated `now`, because now is when it was seen to
/// move and that is a measurement. Everything else is [`Task::derive_stamps`], which
/// repairs only what needs no clock. Nothing here fabricates a time.
///
/// Idempotent, and that is load-bearing rather than merely nice: it runs on every window
/// build, so a pass that could not converge would rewrite the whole ledger forever and
/// make every task look freshly touched — the exact signal `status_since` exists to keep
/// honest. `render` is canonical, so the first pass over a hand-written store rewrites
/// what it normalises and every pass after it writes nothing.
pub async fn reconcile(data_dir: &Path) -> anyhow::Result<usize> {
    let mut rewritten = 0;
    for mut task in scan(data_dir).await? {
        let subject = task.subject.clone();
        let key = facets::subject_dir(data_dir, DIMENSION, &subject);

        // Both facts about the last look, taken under one lock. The marks are moved out
        // rather than borrowed because keeping this directory is `await`-ing IO and a
        // `std::sync` guard may not be held across it.
        let (previous, mut files) = match last_seen().lock() {
            Ok(mut seen) => {
                let entry = seen.entry(key.clone()).or_default();
                (entry.status, std::mem::take(&mut entry.files))
            }
            Err(_) => (None, task_history::DirState::default()),
        };

        // **Before the repair below can rewrite anything.** Whatever is on disk right now is
        // what some session last wrote, and the next writer — this pass, a worker, a second
        // worker that never knew about the first — is free to replace it whole with no error
        // and no copy. This is the copy. See [`task_history`] for why it rides on the read.
        let looked = task_history::keep(&key, &mut files, Utc::now()).await;
        if !looked.gone.is_empty() {
            // A file that was in a task's folder and is not any more. Unambiguous, unlike a
            // content change, which is ordinary work most of the time.
            tracing::warn!(
                task = %subject,
                files = %looked.gone.join(", "),
                "files vanished from a task folder; their last seen version is in .history",
            );
        }

        match previous {
            Some(before) if before != task.status => task.stamp_transition(before, Utc::now()),
            _ => {
                task.derive_stamps();
            }
        }

        // Compare rendered-against-stored rather than tracking what changed. A field this
        // pass does not know about is still a difference `render` would erase or reorder,
        // and the only honest question is whether the canonical form of this record is
        // already on disk.
        let wanted = render(&task);
        let stored = facets::read_facet(data_dir, DIMENSION, &subject).await?;
        if stored.as_deref() != Some(wanted.as_str()) {
            facets::update_facet(data_dir, DIMENSION, &subject, &wanted).await?;
            rewritten += 1;
        }

        if let Ok(mut seen) = last_seen().lock() {
            let entry = seen.entry(key).or_default();
            entry.status = Some(task.status);
            entry.files = files;
        }
    }
    Ok(rewritten)
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
    pub session: crate::foundation::registry::SessionSlug,
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
    /// The last stop cut its worker off mid-turn and the host is putting it back.
    ///
    /// Distinguished from a plain absence because the two call for different moves — and from
    /// [`OnIt::Lost`] because this one calls for **no move at all**. It exists to stop the
    /// ledger from reporting the seconds after every restart as though the work had been
    /// abandoned, and it clears itself the moment that session registers.
    Reopening,
    /// The last stop cut its worker off and its thread would not reopen.
    ///
    /// The one case where the restart really did take the work: there is no mind to go back
    /// to, so the errand has to be started again or written off, and either is a decision
    /// somebody has to make. Left unsaid it decays into an ordinary-looking "nobody on it",
    /// which reads exactly like a task being worked on.
    Lost,
}

/// The active ledger as the agent reads it, annotated with who is on each task.
///
/// `working` is keyed by task subject. An empty map means nobody is on anything, and every
/// `doing` task then reads as having nobody on it — the state this whole annotation exists to
/// make visible. Immediately after a restart that is *true but not the whole answer*, which is
/// what [`OnIt::Reopening`] carries: the switchboard is empty because the process died, not
/// because the work was abandoned, and the difference is the difference between an alarm and a
/// session that is already coming back.
pub async fn projection(
    data_dir: &Path,
    working: &std::collections::HashMap<String, OnIt>,
) -> anyhow::Result<String> {
    // **Repair before reading, and never fail the read for it.** A ledger that cannot be
    // tidied is still a ledger that has to be projected — dropping the window because a
    // stamp could not be written would turn a cosmetic fault into a missed duty, which is
    // the one failure this whole file is built against.
    if let Err(error) = reconcile(data_dir).await {
        tracing::warn!(%error, "task ledger could not be reconciled; projecting it as it stands");
    }
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
        Some(OnIt::Reopening) if task.status == TaskStatus::Doing => Some(
            "nobody on it this second — the restart cut its worker off and it is being \
             reopened on its own thread"
                .to_owned(),
        ),
        Some(OnIt::Lost) if task.status == TaskStatus::Doing => Some(
            "nobody on it — the restart cut its worker off and its session could not be \
             reopened, so this needs somebody put on it or writing off"
                .to_owned(),
        ),
        // Same rule as below: said only where nobody is a problem.
        Some(OnIt::Reopening) | Some(OnIt::Lost) => None,
        None if task.status == TaskStatus::Doing => Some("nobody on it".to_owned()),
        None => None,
    }
}

/// `ago` without the trailing word, for a note that already supplies its own verb.
fn ago_short(now: DateTime<Utc>, then: DateTime<Utc>) -> String {
    ago(now, then).trim_end_matches(" ago").to_owned()
}

/// How long ago, in the one spelling every ledger line and prompt uses.
pub(crate) fn ago(now: DateTime<Utc>, then: DateTime<Utc>) -> String {
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
    let (body, timeline) = split_timeline(strip_frontmatter(content));
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
        body,
        timeline,
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

/// The canonical text of a record — **infallible, and that is the point**.
///
/// It could fail on exactly one rule: a field carrying this machine's absolute data-dir
/// path was rejected, so the directory would stay portable. The rule was right and the
/// place was wrong, twice over. The system prompt hands every rung its directories as
/// *absolute* paths on purpose (`{data_dir}` and friends interpolate through
/// [`crate::identity`]'s `abs`, because the rungs do not share a working directory), so a
/// `verify:` naming a file the agent had just been told about was refused for quoting what
/// it was given. And because [`reconcile`] renders every record in one loop, one such
/// record aborted the **whole pass**: on the live store 61 of 68 tasks had never been
/// reconciled once — never stamped, never migrated off the legacy fields — behind a single
/// `verify:` line, under a warning that did not even name the task.
///
/// Portability is now a habit asked for where a mind can act on it (the `verify:` section
/// of `cognition.md`), and the hard half moved to the reader: **code that resolves a stored
/// path takes both forms** — absolute as given, relative against the data dir. Where a path
/// is genuinely a reference the code reads back, it is a typed one validated at parse, the
/// way [`crate::foundation::privacy::store`] does it. Prose stays prose.
fn render(task: &Task) -> String {
    use std::fmt::Write as _;

    let mut out = String::from("---\n");
    let _ = writeln!(out, "status: {}", task.status.as_str());
    let mut field = |key: &str, value: &str| {
        let _ = writeln!(out, "{key}: {}", jstr(value.trim()));
    };
    field("title", task.title.as_str());
    if let Some(created_at) = task.created_at {
        field("created_at", created_at.to_rfc3339().as_str());
    }
    if let Some(status_since) = task.status_since {
        field("status_since", status_since.to_rfc3339().as_str());
    }
    if let Some(due_at) = task.due_at {
        field("due_at", due_at.to_rfc3339().as_str());
    }
    if let Some(checked_at) = task.checked_at {
        field("checked_at", checked_at.to_rfc3339().as_str());
    }
    if let Some(completed_at) = task.completed_at {
        field("completed_at", completed_at.to_rfc3339().as_str());
    }
    if let Some(cancelled_at) = task.cancelled_at {
        field("cancelled_at", cancelled_at.to_rfc3339().as_str());
    }
    for (key, value) in [
        ("verify", &task.liveness.verify),
        ("restart", &task.liveness.restart),
        ("owner", &task.liveness.owner),
        ("start_key", &task.liveness.start_key),
    ] {
        if let Some(value) = value {
            field(key, value);
        }
    }
    for line in &task.extra {
        let _ = writeln!(out, "{line}");
    }
    out.push_str("---\n\n");
    out.push_str(task.record().trim());
    out.push('\n');
    out
}

/// Split a stored body into the prose above the running record and the record itself.
///
/// **Nothing is dropped, at any level.** Prose keeps its lines verbatim. A line under the
/// heading that is not a bullet continues the entry above it, so a wrapped note survives a
/// round trip. A bullet with no instant, or none this schema can date, is a
/// [`TimelineKind::Note`] carrying its whole text. And more than one `## Timeline` heading
/// — what two writers appending independently produce — merges into one section in
/// document order, which heals on the next pass rather than accumulating.
fn split_timeline(body: &str) -> (String, Vec<TimelineEntry>) {
    let mut prose = String::new();
    let mut entries: Vec<TimelineEntry> = Vec::new();
    let mut inside = false;
    for line in body.lines() {
        if line.trim_start().starts_with('#') {
            inside = is_timeline_heading(line);
            if inside {
                continue;
            }
        }
        if !inside {
            prose.push_str(line);
            prose.push('\n');
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        match parse_entry(line) {
            Some(entry) => entries.push(entry),
            None => match entries.last_mut() {
                Some(last) => {
                    last.text.push('\n');
                    last.text.push_str(line.trim());
                }
                // Loose text under the heading with no entry above it to belong to.
                None => entries.push(TimelineEntry {
                    at: None,
                    kind: TimelineKind::Note,
                    text: line.trim().to_owned(),
                }),
            },
        }
    }
    (prose.trim().to_owned(), entries)
}

/// `## Timeline` in either spelling the store might meet, and nothing that merely starts
/// with the word — `## Timeline of the outage` is somebody's prose section.
fn is_timeline_heading(line: &str) -> bool {
    let text = line.trim();
    if !text.starts_with('#') {
        return false;
    }
    let text = text.trim_start_matches('#').trim();
    text.eq_ignore_ascii_case("timeline") || text == "\u{65f6}\u{95f4}\u{7ebf}"
}

fn take_token(s: &str) -> (&str, &str) {
    match s.split_once(char::is_whitespace) {
        Some((head, tail)) => (head, tail.trim_start()),
        None => (s, ""),
    }
}

/// A separator a mind is likely to put between the kind and what it is saying. Optional
/// on the way in, canonical on the way out.
fn strip_separator(s: &str) -> &str {
    let s = s.trim_start();
    for sep in ["\u{2014}", "\u{2013}", "\u{b7}", "\u{ff1a}", ":", "-"] {
        if let Some(rest) = s.strip_prefix(sep) {
            return rest.trim_start();
        }
    }
    s
}

/// `- <instant?> <kind?> <separator?> <text>`, every part after the bullet optional.
///
/// Forgiving on purpose: this is written by minds with a shell, in two languages, and a
/// strict grammar here would mean a line silently failing to be a line. `None` only for
/// something that is not a bullet at all, which the caller reads as a continuation.
fn parse_entry(line: &str) -> Option<TimelineEntry> {
    let trimmed = line.trim();
    let rest = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))?
        .trim();
    let (head, tail) = take_token(rest);
    let (at, rest) = match parse_timestamp(head) {
        Some(at) => (Some(at), tail),
        None => (None, rest),
    };
    let (head, tail) = take_token(rest);
    let (kind, rest) = match TimelineKind::parse(head) {
        Some(kind) => (kind, tail),
        None => (TimelineKind::Note, rest),
    };
    let text = if kind == TimelineKind::Note {
        rest.trim().to_owned()
    } else {
        strip_separator(rest).trim().to_owned()
    };
    Some(TimelineEntry { at, kind, text })
}

/// The canonical text of a running record. A [`TimelineKind::Note`] is re-emitted without
/// a kind word, so a line somebody wrote by hand comes back looking the way they wrote it.
fn render_timeline(entries: &[TimelineEntry]) -> String {
    let mut out = String::from(TIMELINE_HEADING);
    out.push_str("\n\n");
    for entry in entries {
        out.push_str("- ");
        if let Some(at) = entry.at {
            out.push_str(&at.to_rfc3339_opts(SecondsFormat::Secs, true));
            out.push(' ');
        }
        if entry.kind != TimelineKind::Note {
            out.push_str(entry.kind.as_str());
            out.push_str(" \u{2014} ");
        }
        let mut lines = entry.text.lines();
        out.push_str(lines.next().unwrap_or_default());
        out.push('\n');
        // Wrapped lines ride indented, which is exactly what `split_timeline` reads back
        // as a continuation of this entry rather than as a new one.
        for line in lines {
            out.push_str("  ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
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

    /// **A subject that names a row resolves to it, spelled as the ledger spells it.** The
    /// join is an exact string match on the directory name, so a raw `Login Failure!` carried
    /// on to the session would name a row that exists and match nothing — a worker linked in
    /// its own record and absent from the task's line.
    #[tokio::test]
    async fn a_named_row_comes_back_as_the_ledger_spells_it() {
        let dir = tempfile::tempdir().unwrap();
        hand_write(dir.path(), "login-failure", "status: doing\ntitle: \"Chase the login failure\"\n", "")
            .await;

        let Named::Row(subject) = named(dir.path(), "  Login Failure!  ").await.unwrap() else {
            panic!("the row is there");
        };
        assert_eq!(subject, "login-failure");
    }

    /// **A miss answers with the ledger, because a refusal that only says *no* is the one that
    /// gets answered by coining a near-duplicate.** The list is what lets a review name the
    /// task it is a review *of* instead of opening a sibling row beside it.
    #[tokio::test]
    async fn a_miss_offers_the_open_rows() {
        let dir = tempfile::tempdir().unwrap();
        hand_write(dir.path(), "ship-the-flash-cards", "status: todo\ntitle: \"Ship the flash cards\"\n", "").await;
        hand_write(dir.path(), "watch-the-ops-group", "status: serving\ntitle: \"Watch the ops group\"\n", "").await;
        hand_write(dir.path(), "old-thing", "status: done\ntitle: \"Old thing\"\n", "").await;

        let Named::Missing { open } = named(dir.path(), "review-the-flash-cards").await.unwrap()
        else {
            panic!("nothing is filed under that name");
        };
        assert!(open.contains("`ship-the-flash-cards` [todo] Ship the flash cards"), "{open}");
        assert!(open.contains("`watch-the-ops-group` [serving]"), "{open}");
        assert!(!open.contains("old-thing"), "a closed row is not what someone is looking for: {open}");
    }

    /// **Nothing in this path writes.** The whole reason the lookup refuses instead of opening
    /// is that a row nobody decided to owe is what turns the list into a place nobody reads.
    #[tokio::test]
    async fn a_lookup_never_opens_a_row() {
        let dir = tempfile::tempdir().unwrap();
        let _ = named(dir.path(), "something-brand-new").await.unwrap();
        assert!(
            !facets::subject_dir(dir.path(), DIMENSION, "something-brand-new").exists(),
            "a lookup must not have filed anything"
        );
    }

    /// A subject of nothing but punctuation slugs to the empty string, which names no row and
    /// could never be a directory. It is a miss like any other — the list comes back, and the
    /// caller says what a subject looks like.
    #[tokio::test]
    async fn a_subject_that_slugs_to_nothing_is_a_miss() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(named(dir.path(), "///").await.unwrap(), Named::Missing { .. }));
    }

    /// Write a facet exactly as a hand-editing agent would — straight to the file, no
    /// `render`, no stamps it forgot. Every `reconcile` test starts from one of these,
    /// because a record produced by `write_task` is already canonical and would prove
    /// nothing.
    async fn hand_write(dir: &Path, subject: &str, frontmatter: &str, body: &str) {
        let d = facets::subject_dir(dir, DIMENSION, subject);
        tokio::fs::create_dir_all(&d).await.unwrap();
        tokio::fs::write(d.join(facets::FACET_FILE), format!("---\n{frontmatter}---\n\n{body}\n"))
            .await
            .unwrap();
    }

    async fn stored(dir: &Path, subject: &str) -> String {
        facets::read_facet(dir, DIMENSION, subject).await.unwrap().unwrap()
    }

    /// **The property the pass lives or dies on.** It runs on every window build, so if it
    /// could not converge it would rewrite the whole ledger forever — and `status_since`,
    /// the one field that says whether a task has actually moved, would be reset on every
    /// turn by the machinery meant to keep it honest. Churn is not movement.
    #[tokio::test]
    async fn reconcile_converges_and_then_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        hand_write(
            dir.path(),
            "converge-a",
            "status: done\ntitle: shipped it\nstatus_since: 2026-08-01T09:00:00+08:00\n",
            "prose",
        )
        .await;
        hand_write(dir.path(), "converge-b", "kind: wip\nstate: open\ntitle: legacy one\n", "prose").await;

        assert_eq!(reconcile(dir.path()).await.unwrap(), 2, "both records were off-canonical");
        assert_eq!(reconcile(dir.path()).await.unwrap(), 0, "a second pass must write nothing");
        assert_eq!(reconcile(dir.path()).await.unwrap(), 0, "and stay that way");
    }

    /// A transition the pass **watched happen** is dated, because `now` is a measurement
    /// here rather than a guess: the status was one thing last read and is another now.
    #[tokio::test]
    async fn a_witnessed_transition_is_stamped_now() {
        let dir = tempfile::tempdir().unwrap();
        hand_write(dir.path(), "witnessed", "status: doing\ntitle: in flight\n", "prose").await;
        reconcile(dir.path()).await.unwrap();

        // The agent edits the file by hand and stamps nothing, which is the whole problem.
        hand_write(dir.path(), "witnessed", "status: done\ntitle: in flight\n", "prose").await;
        reconcile(dir.path()).await.unwrap();

        let task = read_task(dir.path(), "witnessed").await.unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Done);
        let completed = task.completed_at.expect("a watched close is dated");
        assert!((Utc::now() - completed).num_seconds().abs() < 5);
        assert_eq!(task.status_since, Some(completed), "one moment, not two");
    }

    /// Cold — nothing to compare against — so the close is derived from the one instant the
    /// record already carries, never from the clock.
    #[tokio::test]
    async fn a_cold_close_is_derived_from_status_since() {
        let dir = tempfile::tempdir().unwrap();
        hand_write(
            dir.path(),
            "cold-derive",
            "status: done\ntitle: filed late\nstatus_since: 2026-06-02T11:30:00+00:00\n",
            "prose",
        )
        .await;
        reconcile(dir.path()).await.unwrap();

        let task = read_task(dir.path(), "cold-derive").await.unwrap().unwrap();
        assert_eq!(task.completed_at, task.status_since);
        assert_eq!(task.completed_at.unwrap().to_rfc3339(), "2026-06-02T11:30:00+00:00");
    }

    /// **The refusal.** Closed, and no instant anywhere to derive from — so the field stays
    /// empty. `now` would read as "finished today" on a task finished in June, and nothing
    /// downstream could tell that stamp from a real one.
    #[tokio::test]
    async fn a_close_with_no_instant_anywhere_is_left_empty() {
        let dir = tempfile::tempdir().unwrap();
        hand_write(dir.path(), "cold-refuse", "status: done\ntitle: undated\n", "prose").await;
        reconcile(dir.path()).await.unwrap();

        let task = read_task(dir.path(), "cold-refuse").await.unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Done);
        assert!(task.completed_at.is_none(), "an instant that is not known is not invented");
    }

    /// An open record cannot carry a closing stamp — that is a leftover from a close undone
    /// by hand, and clearing it asserts nothing the status does not already say.
    #[tokio::test]
    async fn reopening_by_hand_drops_the_stale_closing_stamp() {
        let dir = tempfile::tempdir().unwrap();
        hand_write(
            dir.path(),
            "reopened-by-hand",
            "status: doing\ntitle: back open\ncompleted_at: 2026-07-01T00:00:00+00:00\n",
            "prose",
        )
        .await;
        reconcile(dir.path()).await.unwrap();

        let task = read_task(dir.path(), "reopened-by-hand").await.unwrap().unwrap();
        assert!(task.completed_at.is_none());
        assert!(task.cancelled_at.is_none());
    }

    /// A record on the retired `kind:`/`state:` spelling is re-emitted in the current one
    /// by being touched at all — eight of these sat in one live store, and every read was
    /// re-interpreting them rather than fixing them.
    #[tokio::test]
    async fn a_legacy_record_is_re_emitted_in_the_current_spelling() {
        let dir = tempfile::tempdir().unwrap();
        hand_write(dir.path(), "legacy-one", "kind: wip\nstate: done\ntitle: old shape\n", "prose").await;
        reconcile(dir.path()).await.unwrap();

        let raw = stored(dir.path(), "legacy-one").await;
        assert!(raw.contains("status: done"), "{raw}");
        assert!(!raw.contains("kind:"), "{raw}");
        assert!(!raw.contains("state:"), "{raw}");
    }

    /// **The pass is not entitled to tidy what it does not understand.** The agent keeps its
    /// own ledger in frontmatter keys this schema never defined, and a repair that dropped
    /// them would destroy more than it fixed.
    #[tokio::test]
    async fn a_repair_keeps_frontmatter_the_schema_never_defined() {
        let dir = tempfile::tempdir().unwrap();
        hand_write(
            dir.path(),
            "foreign-keys",
            "status: done\ntitle: has a ledger of its own\nreport_to: prdo8qht\nCHECK_20260818: \"still up\"\n",
            "the body",
        )
        .await;
        reconcile(dir.path()).await.unwrap();

        let raw = stored(dir.path(), "foreign-keys").await;
        assert!(raw.contains("report_to: prdo8qht"), "{raw}");
        assert!(raw.contains("CHECK_20260818:"), "{raw}");
        assert!(raw.contains("the body"), "{raw}");
    }

    /// The pass must never mint a creation instant a record does not have — the parser
    /// already refuses to, and a repair that quietly supplied one would make every legacy
    /// record look created the day it was first tidied.
    #[tokio::test]
    async fn a_repair_does_not_mint_a_creation_instant() {
        let dir = tempfile::tempdir().unwrap();
        hand_write(dir.path(), "no-birthday", "kind: wip\nstate: open\ntitle: undated\n", "prose").await;
        reconcile(dir.path()).await.unwrap();

        assert!(read_task(dir.path(), "no-birthday").await.unwrap().unwrap().created_at.is_none());
    }

    /// The common case for the projection tests that predate the join: nothing registered, so
    /// **The line the scalars could never write.** `status_since` is overwritten by the
    /// next transition, so a record could say how long it had been in `done` and never
    /// what it had been before. The pass that already witnesses the move writes it down.
    #[tokio::test]
    async fn a_witnessed_transition_writes_one_line_in_the_running_record() {
        let dir = tempfile::tempdir().unwrap();
        hand_write(dir.path(), "moved-once", "status: doing\ntitle: in flight\n", "prose").await;
        reconcile(dir.path()).await.unwrap();

        hand_write(dir.path(), "moved-once", "status: done\ntitle: in flight\n", "prose").await;
        reconcile(dir.path()).await.unwrap();

        let task = read_task(dir.path(), "moved-once").await.unwrap().unwrap();
        let moves: Vec<_> = task
            .timeline
            .iter()
            .filter(|entry| entry.kind == TimelineKind::Moved)
            .collect();
        assert_eq!(moves.len(), 1, "{:?}", task.timeline);
        assert_eq!(moves[0].text, "doing \u{2192} done");
        // One moment, said twice at different precisions: the stamp keeps what the clock
        // gave it, and the line is dated to the second because a person reads it.
        assert_eq!(
            moves[0].at.unwrap().timestamp(),
            task.status_since.unwrap().timestamp()
        );
        assert_eq!(task.body, "prose", "the prose above it is untouched");
    }

    /// A pass that ran forever would append forever. The record is the one part of a task
    /// that grows, so the thing that must not repeat is a transition already written down.
    #[tokio::test]
    async fn reconcile_does_not_re_record_a_transition_it_already_wrote() {
        let dir = tempfile::tempdir().unwrap();
        hand_write(dir.path(), "no-stutter", "status: doing\ntitle: in flight\n", "prose").await;
        reconcile(dir.path()).await.unwrap();
        hand_write(dir.path(), "no-stutter", "status: done\ntitle: in flight\n", "prose").await;
        for _ in 0..3 {
            reconcile(dir.path()).await.unwrap();
        }

        let task = read_task(dir.path(), "no-stutter").await.unwrap().unwrap();
        assert_eq!(task.timeline.len(), 1, "{:?}", task.timeline);
    }

    /// The other writer of a transition is code, and it records its own. Left unsaid, the
    /// next pass would compare against a stale reading, witness the same move a second
    /// time, and date it later than the moment it actually happened.
    #[tokio::test]
    async fn a_transition_made_through_code_is_recorded_once_too() {
        let dir = tempfile::tempdir().unwrap();
        hand_write(dir.path(), "via-code", "status: doing\ntitle: in flight\n", "prose").await;
        reconcile(dir.path()).await.unwrap();

        let mut task = read_task(dir.path(), "via-code").await.unwrap().unwrap();
        task.set_status(TaskStatus::Done, Utc::now());
        write_task(dir.path(), &task).await.unwrap();
        reconcile(dir.path()).await.unwrap();

        let task = read_task(dir.path(), "via-code").await.unwrap().unwrap();
        assert_eq!(task.timeline.len(), 1, "{:?}", task.timeline);
        assert_eq!(task.timeline[0].text, "doing \u{2192} done");
    }

    /// Cold — no previous reading — so nothing is known about when it moved, and nothing
    /// is written. The same refusal `derive_stamps` already makes about the clock: a
    /// history invented on the first pass after a restart is indistinguishable from one
    /// somebody recorded.
    #[tokio::test]
    async fn a_cold_pass_records_no_history_it_did_not_witness() {
        let dir = tempfile::tempdir().unwrap();
        hand_write(dir.path(), "cold-record", "status: done\ntitle: filed late\n", "prose").await;
        reconcile(dir.path()).await.unwrap();

        let task = read_task(dir.path(), "cold-record").await.unwrap().unwrap();
        assert!(task.timeline.is_empty(), "{:?}", task.timeline);
    }

    /// Prose above, record below, and a round trip through `render` that changes neither.
    #[tokio::test]
    async fn prose_and_running_record_survive_a_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        hand_write(
            dir.path(),
            "round-trip",
            "status: doing\ntitle: the digest\n",
            "The long account, written whole.\n\n\
             ## Timeline\n\n\
             - 2026-08-24T06:16:17Z asked \u{2014} it goes to the Feishu group, not to me\n\
             - 2026-08-24T07:02:00Z blocked: the app has no im:chat scope\n",
        )
        .await;

        let task = read_task(dir.path(), "round-trip").await.unwrap().unwrap();
        assert_eq!(task.body, "The long account, written whole.");
        assert_eq!(task.timeline.len(), 2);
        assert_eq!(task.timeline[0].kind, TimelineKind::Asked);
        assert_eq!(task.timeline[0].text, "it goes to the Feishu group, not to me");
        assert_eq!(task.timeline[1].kind, TimelineKind::Blocked);
        assert_eq!(task.timeline[1].text, "the app has no im:chat scope");
        assert_eq!(
            task.asked().map(|entry| entry.text.as_str()),
            Some("it goes to the Feishu group, not to me")
        );

        write_task(dir.path(), &task).await.unwrap();
        let again = read_task(dir.path(), "round-trip").await.unwrap().unwrap();
        assert_eq!(again.body, task.body);
        assert_eq!(again.timeline, task.timeline);
        assert_eq!(reconcile(dir.path()).await.unwrap(), 0, "canonical already");
    }

    /// **Nothing under the heading is dropped**, whatever shape it is in — the frontmatter
    /// rule one level down. An undated bullet keeps its whole text; a wrapped line stays
    /// on the entry it belongs to instead of becoming an entry of its own.
    #[tokio::test]
    async fn a_line_this_schema_cannot_read_is_still_kept() {
        let dir = tempfile::tempdir().unwrap();
        hand_write(
            dir.path(),
            "kept-verbatim",
            "status: doing\ntitle: keep it all\n",
            "## Timeline\n\n\
             - \u{6536}\u{5230}\u{8d75}\u{529b}\u{7684}\u{8865}\u{5145}\u{8bf4}\u{660e}\n\
             - 2026-08-24T09:00:00Z landed \u{2014} first half shipped\n\
               and the second half is in review\n",
        )
        .await;

        let task = read_task(dir.path(), "kept-verbatim").await.unwrap().unwrap();
        assert_eq!(task.timeline.len(), 2, "{:?}", task.timeline);
        assert_eq!(task.timeline[0].kind, TimelineKind::Note);
        assert!(task.timeline[0].at.is_none());
        assert_eq!(task.timeline[0].text, "\u{6536}\u{5230}\u{8d75}\u{529b}\u{7684}\u{8865}\u{5145}\u{8bf4}\u{660e}");
        assert_eq!(
            task.timeline[1].text,
            "first half shipped\nand the second half is in review"
        );

        write_task(dir.path(), &task).await.unwrap();
        let again = read_task(dir.path(), "kept-verbatim").await.unwrap().unwrap();
        assert_eq!(again.timeline, task.timeline, "and a round trip keeps it");
    }

    /// Two writers appending independently produce two headings. Merging them on read is
    /// what stops that from compounding — the alternative is a record that grows a section
    /// every time somebody does the obvious thing.
    #[tokio::test]
    async fn two_running_records_in_one_file_merge_into_one() {
        let dir = tempfile::tempdir().unwrap();
        hand_write(
            dir.path(),
            "two-sections",
            "status: doing\ntitle: merged\n",
            "## Timeline\n\n\
             - 2026-08-24T09:00:00Z landed \u{2014} first\n\n\
             ## Notes\n\n\
             kept prose\n\n\
             ## Timeline\n\n\
             - 2026-08-24T10:00:00Z checked \u{2014} second\n",
        )
        .await;

        let task = read_task(dir.path(), "two-sections").await.unwrap().unwrap();
        assert_eq!(task.timeline.len(), 2, "{:?}", task.timeline);
        assert_eq!(task.timeline[0].text, "first");
        assert_eq!(task.timeline[1].text, "second");
        assert!(task.body.contains("## Notes"), "the other section is prose: {:?}", task.body);
        assert!(task.body.contains("kept prose"));

        write_task(dir.path(), &task).await.unwrap();
        let raw = stored(dir.path(), "two-sections").await;
        assert_eq!(raw.matches("## Timeline").count(), 1, "{raw}");
    }

    /// A prose section that merely starts with the word is somebody's writing, not the
    /// schema's section.
    #[test]
    fn only_the_heading_itself_opens_the_running_record() {
        assert!(is_timeline_heading("## Timeline"));
        assert!(is_timeline_heading("###   timeline  "));
        assert!(!is_timeline_heading("## Timeline of the outage"));
        assert!(!is_timeline_heading("Timeline"));
    }

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

    fn reopening(task: &Task) -> std::collections::HashMap<String, OnIt> {
        std::collections::HashMap::from([(task.subject.clone(), OnIt::Reopening)])
    }

    fn lost(task: &Task) -> std::collections::HashMap<String, OnIt> {
        std::collections::HashMap::from([(task.subject.clone(), OnIt::Lost)])
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
        let text = render_projection(std::slice::from_ref(&owed), now(), &reopening(&owed));
        assert!(text.contains("nobody on it"), "the phrase is not softened: {text}");
        assert!(text.contains("restart cut its worker off"), "{text}");
        assert!(text.contains("being reopened"), "and that nothing is wanted from the reader: {text}");
    }

    /// **The two restart lines must not read alike, because only one of them wants a move.**
    /// A worker on its way back wants nothing; a thread that would not reopen is work that has
    /// to be started again or written off. Reading the second as the first is how a task sits
    /// in `doing` forever waiting for a session that is never coming.
    #[test]
    fn an_errand_that_could_not_be_reopened_asks_for_a_decision() {
        let owed = task("Ship the multilingual fix", TaskStatus::Doing);
        let text = render_projection(std::slice::from_ref(&owed), now(), &lost(&owed));
        assert!(text.contains("nobody on it"), "{text}");
        assert!(text.contains("could not be reopened"), "{text}");
        assert!(!text.contains("being reopened"), "nothing is coming back: {text}");
    }

    /// And it is the same rule as the bare phrase about *where* it may be said. A `todo` or a
    /// `serving` duty whose last handler died in the restart is not a problem to raise — the
    /// phrase would be on most of the list again, cause and all.
    #[test]
    fn a_cut_off_todo_or_duty_is_still_not_flagged() {
        for status in [TaskStatus::Todo, TaskStatus::Serving] {
            let owed = task("Watch the ops group", status);
            for join in [reopening(&owed), lost(&owed)] {
                let text = render_projection(std::slice::from_ref(&owed), now(), &join);
                assert!(!text.contains("nobody on it"), "{status:?}: {text}");
                assert!(!text.contains("restart"), "{status:?}: {text}");
            }
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

    /// **One record cannot stop the pass**, and the path it quotes is kept as written.
    ///
    /// The store used to refuse a field carrying this machine's absolute data-dir path, to
    /// keep `data/` portable. Refusing it here could only ever fail closed: `reconcile`
    /// renders every record in one loop, so the refusal aborted the loop, and every subject
    /// sorting after that one went un-reconciled forever — 61 of 68 on the live store,
    /// none of them stamped or migrated, all of it behind a warning that did not say which
    /// task. The habit is asked for in `cognition.md` now, where a mind can act on it.
    #[tokio::test]
    async fn a_quoted_host_path_is_kept_and_does_not_stop_the_pass() {
        let dir = tempfile::tempdir().unwrap();
        let host = dir.path().display().to_string();
        let verify = format!("{host}/bin/watch printed a row in the last hour");
        hand_write(
            dir.path(),
            "aaa-quotes-its-host-path",
            &format!("status: serving\ntitle: watch the queue\nverify: {}\n", jstr(&verify)),
            "prose",
        )
        .await;
        hand_write(
            dir.path(),
            "zzz-sorts-after-it",
            "kind: wip\nstate: open\ntitle: the one that never got reconciled\n",
            "prose",
        )
        .await;

        reconcile(dir.path()).await.unwrap();

        let quoting = read_task(dir.path(), "aaa-quotes-its-host-path").await.unwrap().unwrap();
        assert_eq!(
            quoting.liveness.verify.as_deref(),
            Some(verify.as_str()),
            "the path is stored exactly as the mind wrote it"
        );

        let after = read_task(dir.path(), "zzz-sorts-after-it").await.unwrap().unwrap();
        assert_eq!(after.status, TaskStatus::Doing, "a record sorting after it is still read");
        let text = stored(dir.path(), "zzz-sorts-after-it").await;
        assert!(text.contains("status: doing"), "and canonicalised: {text}");
        assert!(!text.contains("kind:"), "off the legacy fields: {text}");
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
