//! The session directory — what ran, and where its frames are.
//!
//! The switchboard ([`super::Registry`]) is live by construction: an entry exists between
//! `register` and `unregister`, and a session whose drive task has ended is simply absent.
//! That is the right shape for "what is running now" and the wrong one for the two
//! questions that kept going unanswered:
//!
//! - **What just ended?** `gaps.md` §2 records the agent calling a price watch
//!   "挂着呢,一直在盯" while `GET /api/sessions` showed zero workers. The roster was
//!   right; the reader had no way to see the watch that had already died.
//! - **Where are that session's frames?** [`WireTap`](crate::foundation::codex::WireTap)
//!   has kept every JSON-RPC line per session since it was given a data dir, under
//!   [`session_frames_path`](crate::mind::memory::layout::session_frames_path). Nothing
//!   read them back, and nothing could: the path is keyed by `(run, session)`, session slugs
//!   restart at 1 every boot ([`crate::foundation::run`]), and a session that has ended is
//!   gone from the switchboard — so the id needed to name its own frame log died with it.
//!   **The frames outlived the session; the index did not exist.**
//!
//! So: one append-only file next to the frame logs it points at, and an in-memory list of
//! recent ends seeded from it at boot. Journal-seeded rather than authoritative-in-memory,
//! the same shape [`docs/arch/text-transcript.md`](../../../../docs/arch/text-transcript.md)
//! uses for the conversation and for the same reason — a restart must not be the thing
//! that erases the evidence of the restart.
//!
//! **A `closed` record carries everything a reader needs, deliberately duplicating its
//! `opened`.** Recency is the only order anyone asks for, so the common read is a scan of
//! `closed` lines and must not have to pair each one with an `opened` line somewhere
//! earlier in the file — a long-lived rung's `opened` sits at the top and its `closed` at
//! the bottom, so folding would mean reading all of it. The duplication costs ~150 bytes
//! per session and buys a read that never grows a fold.
//!
//! **What `opened` is still for: the sessions that never got a `closed`.** An `opened`
//! with no matching `closed`, in a run that is not the current one, is a session the
//! process died underneath — exactly `server.log`'s `worker report dropped; reaction loop
//! gone worker=9`. Those are reported as [`EndedHow::Restart`] rather than quietly
//! omitted, because a worker that vanished mid-flight is the single most useful row on the
//! page.
//!
//! Not a retention story. Nothing prunes this file or the frame logs beside it — see
//! [`crate::mind::memory::layout::is_signal_dir`], which exists so the forgetting pass
//! skips `sessions/` entirely. Bounding it is open work, not something this module quietly
//! decides.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::SessionSlug;
use crate::identity::Role;
use crate::mind::memory::layout;

/// How many recent ends the in-memory list holds.
///
/// A reader asking "what just ended" is asking about the last few minutes; a page showing
/// five hundred is a page nobody scrolls. The file keeps everything regardless, so raising
/// this is a display decision and never a data-loss one.
const RECENT_CAP: usize = 500;

/// How much of the tail of the index file to read at boot.
///
/// Bounded because the file is append-only and unpruned, so an install that has been up
/// for months must not pay a full read to answer a question about this afternoon. A
/// truncated first line is dropped by [`fold`] rather than guessed at.
const SEED_TAIL_BYTES: u64 = 2 * 1024 * 1024;

/// One line in the index. Tagged, so a future kind can be added without the reader
/// mistaking it for one of these.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Record {
    /// A session joined the switchboard. Written from `register`.
    Opened {
        run: String,
        session: SessionSlug,
        /// The ledger subject this session serves — see [`Ended::subject`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
        at: DateTime<Utc>,
        /// [`Role::as_str`] — the tool surface, the same word `GET /api/workers` and the
        /// `X-HI-Role` header use.
        role: String,
        /// Which specialism, for the five sessions that share the `worker` surface;
        /// `None` for a rung.
        #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
        worker_type: Option<String>,
        /// The one line this session shows up as — see [`super::Status::title`]. Old rows
        /// carry it under its former name, when it held the whole brief a worker was sent;
        /// the alias is how those still read, and their line is simply the long one.
        #[serde(alias = "task", skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        owner: Option<SessionSlug>,
    },
    /// The codex thread behind a session, once it exists. Written from
    /// [`Registry::note_thread`](super::Registry::note_thread).
    ///
    /// **Its own record rather than a field on `opened`, because the thread does not exist
    /// yet when `opened` is written.** A session registers before its subprocess is
    /// spawned — deliberately, so an address exists before anything can be told to use it
    /// — and `thread/start` answers some hundreds of milliseconds later. Folding the id
    /// into `opened` would mean delaying that write until the thread is up, which is
    /// exactly the window a crash falls into and the reason `opened` is written on the way
    /// in at all.
    ///
    /// The id is codex's, not ours: `thread/start` takes no path, so where the rollout
    /// lands is its choice and `(run, session)` cannot address it. Recorded, never derived.
    Thread {
        run: String,
        session: SessionSlug,
        at: DateTime<Utc>,
        thread_id: String,
    },
    /// A session left the switchboard. Written from `unregister`, and self-contained on
    /// purpose — see the module doc.
    Closed {
        run: String,
        session: SessionSlug,
        /// The ledger subject this session served — see [`Ended::subject`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
        at: DateTime<Utc>,
        started: DateTime<Utc>,
        role: String,
        #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
        worker_type: Option<String>,
        /// See [`Record::Opened::title`].
        #[serde(alias = "task", skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        owner: Option<SessionSlug>,
        turns: u64,
        /// Whether this session still had work in hand when it stopped — see
        /// [`Ended::interrupted`]. Absent on rows written before it was recorded, which
        /// read as `false`: the sessions those rows describe are long past being resumable
        /// anyway, so guessing `true` for them would only reopen furniture.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        interrupted: bool,
        /// How many delivered messages it never got to read — see [`Ended::unread`].
        #[serde(default, skip_serializing_if = "is_zero")]
        unread: u32,
    },
}

impl Record {
    /// Only for the writer's error log — a record that would not serialize still has to be
    /// reportable as *which* one.
    fn session(&self) -> SessionSlug {
        match self {
            Record::Opened { session, .. }
            | Record::Closed { session, .. }
            | Record::Thread { session, .. } => session.clone(),
        }
    }
}

/// How a session stopped running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndedHow {
    /// It unregistered — the drive task finished, or its scope was dropped.
    Closed,
    /// It has an `opened` and no `closed`, in a run that is over. The process died
    /// underneath it, so whatever it was doing was lost rather than reported.
    Restart,
}

/// One session that is no longer live, as a reader reads it.
#[derive(Debug, Clone, Serialize)]
pub struct Ended {
    /// Which run it belonged to. Required to address its frame log, because session slugs
    /// repeat every boot — the rungs always, a worker whenever the same errand comes round.
    pub run: String,
    pub session: SessionSlug,
    pub role: String,
    #[serde(rename = "type")]
    pub worker_type: Option<String>,
    /// The one line it ran under — see [`super::Status::title`].
    pub title: Option<String>,
    /// The ledger subject this session served, if it was created against a task.
    ///
    /// The live join lives in the switchboard ([`super::Status::subject`]) and dies with the
    /// process, which is right — "who is on this task" is a question about now. This is the
    /// half that has to outlive the run: without it a restart-killed errand can be offered
    /// back only as its title, and the boot glance cannot say which ledger entry it belonged
    /// to. Recorded on the way *in*, because the way out is what a crash skips.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub owner: Option<SessionSlug>,
    pub started: Option<DateTime<Utc>>,
    /// When it closed. `None` for a [`EndedHow::Restart`] row — nothing recorded an end,
    /// which is the whole point of that variant; the reader orders those by `started`.
    pub ended: Option<DateTime<Utc>>,
    pub how: EndedHow,
    pub turns: Option<u64>,
    /// The codex thread this session ran on, if one was ever opened. `None` for a session
    /// that died before `thread/start` answered, and for every row written before threads
    /// were recorded.
    ///
    /// This is what makes a dead session's *mind* addressable, the way `run` + `session`
    /// already makes its frames addressable. A resident rung's is what the next boot
    /// resumes; a worker's is what the boot glance offers Cognition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// Whether work was still in hand when this session ended — a turn running, or mail it
    /// never got to read.
    ///
    /// **This is the whole difference between an errand a stop cut off and one that was
    /// merely never closed.** A worker holds its subprocess until its owner says
    /// `hi_close_worker`, so most of what is alive at a stop has already reported and has
    /// nothing to finish; the 2026-08-21 quit closed sixteen workers of which four were
    /// mid-turn. Reopening all sixteen would spend twelve subprocesses on sessions nobody
    /// is going to brief.
    ///
    /// Read from the switchboard at [`Registry::unregister`](super::Registry::unregister),
    /// which is the last moment anyone knows. It is `true` for **every**
    /// [`EndedHow::Restart`] row, because a crash records nothing and the alternative to
    /// guessing is dropping work that was in flight — the direction of the error is chosen,
    /// not overlooked.
    ///
    /// One case it still misses: a worker holding a task after a 402, whose retry lives in
    /// the drive loop's own `next_task` and is invisible here. It reads as idle and will not
    /// be reopened.
    pub interrupted: bool,
    /// How many messages had been delivered to it and not yet read when it stopped.
    ///
    /// **The mail itself does not survive, and this is what says so out loud.**
    /// [`Registry::unregister`](super::Registry::unregister) drops the inbox with the entry —
    /// undelivered is the honest outcome, and the sender was told `Delivered` about a mailbox
    /// and never about an outcome. Reopening the session does not bring those messages back:
    /// they never reached its thread, because a message only enters a prompt when
    /// `take_pending` renders it.
    ///
    /// So the fact travels to the one party that still holds the text — the sender — and it
    /// decides whether the instruction still applies forty minutes later. Re-posting it here
    /// would put a pre-restart instruction in front of a session whose whole first act is to
    /// find out what has changed since.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unread: u32,
}

/// `skip_serializing_if` for a count that is almost always zero.
fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl Ended {
    /// What to sort by. A restart row has no end, so it takes its start — a session the
    /// process died under is at least as recent as the run that killed it.
    fn recency(&self) -> Option<DateTime<Utc>> {
        self.ended.or(self.started)
    }
}

/// `<memory>/raw/sessions/index.jsonl` — the directory, beside the per-session frame logs
/// it indexes.
pub fn index_path(data_dir: &Path) -> PathBuf {
    layout::raw_root(data_dir).join(layout::SESSIONS_DIR).join("index.jsonl")
}

// ── writing ───────────────────────────────────────────────────────────────────

/// The writer half: an unbounded channel drained by one task.
///
/// Unbounded and non-blocking for the same reason the wire tap's is
/// ([`WireTap::with_durable_log`](crate::foundation::codex::WireTap::with_durable_log)):
/// `register` and `unregister` are synchronous and run on paths that must not touch the
/// filesystem — `unregister` is called from `Registration::drop`, where an await is not
/// available and a blocking write would stall whatever is unwinding.
pub struct Writer {
    tx: mpsc::UnboundedSender<Msg>,
}

/// What crosses to the append task. A flush travels as a message rather than on its own
/// channel so that it takes its place in the queue: the loop is FIFO, so a reply to a
/// `Flush` proves every record queued before it is already on disk.
enum Msg {
    Record(Record),
    Flush(tokio::sync::oneshot::Sender<()>),
}

impl Writer {
    /// Start the index writer for `data_dir`, spawning its append task.
    pub fn start(data_dir: PathBuf) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(append_loop(data_dir, rx));
        Self { tx }
    }

    /// Queue one record. Never blocks; a dead writer is logged once per record and
    /// otherwise ignored, because losing the directory must not take the agent down.
    pub fn write(&self, record: Record) {
        if self.tx.send(Msg::Record(record)).is_err() {
            tracing::warn!("session index writer is gone; a session went unrecorded");
        }
    }

    /// Wait until everything queued so far has reached the disk.
    ///
    /// **The shutdown path cannot do without this.** Closing the switchboard queues one
    /// `closed` record per live session and the process then exits at once; with no flush
    /// the runtime drops the append task while those records are still in the channel, and a
    /// clean stop would leave exactly the `opened`-with-no-`closed` pattern that means
    /// *crashed*. The one moment this file has to be durable is the moment the process ends.
    pub async fn flush(&self) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if self.tx.send(Msg::Flush(tx)).is_err() {
            return;
        }
        // A dropped sender means the loop is gone, which is as flushed as this will get.
        let _ = rx.await;
    }
}

/// Append records as they arrive, batching whatever is already queued into one write.
///
/// Failures are logged and the loop continues, matching the wire tap: a disk that has
/// stopped accepting writes is not something a retry here can fix, and the index is
/// evidence, never a dependency.
async fn append_loop(data_dir: PathBuf, mut rx: mpsc::UnboundedReceiver<Msg>) {
    let path = index_path(&data_dir);
    let mut batch: Vec<Record> = Vec::new();
    // Flush replies are held until after the write below, never answered on receipt — a
    // reply that outran the `write_all` would be a promise this cannot keep.
    let mut waiting: Vec<tokio::sync::oneshot::Sender<()>> = Vec::new();

    while let Some(first) = rx.recv().await {
        match first {
            Msg::Record(r) => batch.push(r),
            Msg::Flush(tx) => waiting.push(tx),
        }
        while let Ok(more) = rx.try_recv() {
            match more {
                Msg::Record(r) => batch.push(r),
                Msg::Flush(tx) => waiting.push(tx),
            }
        }

        let mut buf = String::new();
        for record in batch.drain(..) {
            match serde_json::to_string(&record) {
                Ok(line) => {
                    buf.push_str(&line);
                    buf.push('\n');
                }
                Err(err) => tracing::error!(
                    error = %err,
                    session = %record.session(),
                    "a session index record would not serialize"
                ),
            }
        }

        if !buf.is_empty() {
            append(&path, &buf).await;
        }

        // **Always**, on every path out of the write — including a failed one and an empty
        // batch. A `flush` that is never answered parks the caller forever, and its one
        // caller is the shutdown path, so an unanswered reply is a process that will not
        // exit. A failed write is reported by the log above; the flush only ever promised
        // that the attempt is over.
        for tx in waiting.drain(..) {
            let _ = tx.send(());
        }
    }
}

/// Append `buf` to `path`, creating the directory and file if needed.
///
/// Failures are logged and swallowed: a disk that has stopped accepting writes is not
/// something a retry here can fix, and the directory is evidence, never a dependency.
async fn append(path: &Path, buf: &str) {
    use tokio::io::AsyncWriteExt as _;

    if let Some(parent) = path.parent()
        && let Err(err) = tokio::fs::create_dir_all(parent).await
    {
        tracing::error!(error = %err, path = %parent.display(), "cannot make the session index dir");
        return;
    }
    match tokio::fs::OpenOptions::new().create(true).append(true).open(path).await {
        Ok(mut f) => {
            if let Err(err) = f.write_all(buf.as_bytes()).await {
                tracing::error!(error = %err, path = %path.display(), "session index write failed");
            }
        }
        Err(err) => {
            tracing::error!(error = %err, path = %path.display(), "cannot open the session index");
        }
    }
}

// ── reading ───────────────────────────────────────────────────────────────────

/// Read the tail of the index and fold it into recent ends, most recent first.
///
/// `current_run` is excluded from restart detection: its sessions have no `closed` yet
/// because they are still running, which is the switchboard's answer, not this one's.
pub async fn seed(data_dir: &Path, current_run: &str) -> Vec<Ended> {
    let path = index_path(data_dir);
    let text = match read_tail(&path, SEED_TAIL_BYTES).await {
        Ok(text) => text,
        // A fresh install has no index. That is not a failure and must not log as one.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "cannot read the session index");
            return Vec::new();
        }
    };
    fold(&text, current_run)
}

/// Read at most `limit` bytes from the end of `path`, starting at a line boundary.
///
/// Seeking into the middle of a line is expected — the first partial line is dropped by
/// starting after the first newline. A file shorter than `limit` is read whole and keeps
/// its first line.
async fn read_tail(path: &Path, limit: u64) -> std::io::Result<String> {
    use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};

    let mut f = tokio::fs::File::open(path).await?;
    let len = f.metadata().await?.len();
    let whole = len <= limit;
    if !whole {
        f.seek(std::io::SeekFrom::Start(len - limit)).await?;
    }
    let mut buf = Vec::with_capacity(limit.min(len) as usize);
    f.read_to_end(&mut buf).await?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    if whole {
        return Ok(text);
    }
    // Drop the partial first line.
    Ok(match text.find('\n') {
        Some(i) => text[i + 1..].to_string(),
        None => String::new(),
    })
}

/// Fold index lines into recent ends, most recent first.
///
/// Unparseable lines are skipped rather than failing the read: the first line of a
/// tail-read is routinely a fragment, and one corrupt line must not blank the page.
fn fold(text: &str, current_run: &str) -> Vec<Ended> {
    let mut ends: Vec<Ended> = Vec::new();
    let mut opened: Vec<Ended> = Vec::new();
    let mut closed: HashSet<(String, SessionSlug)> = HashSet::new();
    // Threads are folded in a second pass rather than as they arrive: a `thread` line
    // always follows its `opened` (the thread cannot exist before the session that opens
    // it) but may precede or follow the `closed`, and a tail-read can begin between any
    // two of the three. Collecting first and attaching after makes the order irrelevant.
    let mut threads: std::collections::HashMap<(String, SessionSlug), String> =
        std::collections::HashMap::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Record>(line) else { continue };
        match record {
            Record::Closed {
                run,
                session,
                subject,
                at,
                started,
                role,
                worker_type,
                title,
                owner,
                turns,
                interrupted,
                unread,
            } => {
                closed.insert((run.clone(), session.clone()));
                ends.push(Ended {
                    run,
                    session,
                    role,
                    worker_type,
                    title,
                    subject,
                    owner,
                    started: Some(started),
                    ended: Some(at),
                    how: EndedHow::Closed,
                    turns: Some(turns),
                    thread: None,
                    interrupted,
                    unread,
                });
            }
            Record::Thread { run, session, thread_id, .. } => {
                threads.insert((run, session), thread_id);
            }
            Record::Opened { run, session, subject, at, role, worker_type, title, owner } => {
                // A session in the current run with no close is *live*, not lost — the
                // switchboard reports it, and claiming it here would double-count it.
                if run == current_run {
                    continue;
                }
                opened.push(Ended {
                    run,
                    session,
                    role,
                    worker_type,
                    title,
                    subject,
                    owner,
                    started: Some(at),
                    ended: None,
                    how: EndedHow::Restart,
                    turns: None,
                    thread: None,
                    // Nothing recorded what this one was doing, because recording it is
                    // exactly what a crash skips — see [`Ended::interrupted`].
                    interrupted: true,
                    // And nothing recorded what was in its inbox either. Zero is not a claim
                    // that it was empty; it is the absence of a count.
                    unread: 0,
                });
            }
        }
    }

    // An `opened` is only evidence of a restart-killed session once the whole window has
    // been read and no `closed` for it turned up.
    ends.extend(
        opened.into_iter().filter(|e| !closed.contains(&(e.run.clone(), e.session.clone()))),
    );

    // Attach each session's thread now that every line has been seen.
    for end in &mut ends {
        end.thread = threads.get(&(end.run.clone(), end.session.clone())).cloned();
    }

    // Most recent first, and stable on the id so two ends in the same second keep a
    // deterministic order rather than shuffling between polls. The tie-break is a name
    // compare now that ids are slugs, not "the later session first" it read as while they
    // were ordinals — deterministic either way, which is all this line is for.
    ends.sort_by(|a, b| b.recency().cmp(&a.recency()).then(b.session.cmp(&a.session)));
    ends.truncate(RECENT_CAP);
    ends
}

/// Push one just-closed session onto a seeded list, newest first, holding the cap.
pub fn push_recent(recent: &mut Vec<Ended>, ended: Ended) {
    recent.insert(0, ended);
    recent.truncate(RECENT_CAP);
}

/// Build the [`Ended`] row for a session leaving the switchboard.
pub fn ended_now(
    run: &str,
    session: &SessionSlug,
    role: Role,
    owner: Option<SessionSlug>,
    title: &str,
    subject: Option<&str>,
    turns: u64,
    started: DateTime<Utc>,
    thread: Option<String>,
    interrupted: bool,
    unread: u32,
) -> Ended {
    Ended {
        run: run.to_string(),
        session: session.clone(),
        role: role.as_str().to_string(),
        worker_type: role.worker_type().map(|t| t.as_str().to_string()),
        title: Some(title.to_string()).filter(|t| !t.is_empty()),
        subject: subject.map(str::to_string),
        owner,
        started: Some(started),
        ended: Some(Utc::now()),
        how: EndedHow::Closed,
        turns: Some(turns),
        thread,
        interrupted,
        unread,
    }
}

/// The `closed` record for the same event, for the file.
pub fn closed_record(ended: &Ended) -> Record {
    Record::Closed {
        run: ended.run.clone(),
        session: ended.session.clone(),
        subject: ended.subject.clone(),
        at: ended.ended.unwrap_or_else(Utc::now),
        started: ended.started.unwrap_or_else(Utc::now),
        role: ended.role.clone(),
        worker_type: ended.worker_type.clone(),
        title: ended.title.clone(),
        owner: ended.owner.clone(),
        turns: ended.turns.unwrap_or(0),
        interrupted: ended.interrupted,
        unread: ended.unread,
    }
}

/// The `opened` record for a session joining the switchboard.
pub fn opened_record(
    run: &str,
    session: &SessionSlug,
    role: Role,
    owner: Option<SessionSlug>,
    title: &str,
    subject: Option<&str>,
    at: DateTime<Utc>,
) -> Record {
    Record::Opened {
        run: run.to_string(),
        session: session.clone(),
        subject: subject.map(str::to_string),
        at,
        role: role.as_str().to_string(),
        worker_type: role.worker_type().map(|t| t.as_str().to_string()),
        title: Some(title.to_string()).filter(|t| !t.is_empty()),
        owner,
    }
}

/// The `thread` record binding a session to the codex thread it opened.
pub fn thread_record(
    run: &str,
    session: &SessionSlug,
    thread_id: &str,
    at: DateTime<Utc>,
) -> Record {
    Record::Thread {
        run: run.to_string(),
        session: session.clone(),
        at,
        thread_id: thread_id.to_string(),
    }
}

/// The roles whose thread the next boot resumes, and the only ones.
///
/// **Residency is the whole criterion.** Both are one session for the life of the process,
/// so "the last one" is unambiguous and picking it needs no judgment. Reflection is excluded
/// because a pass that died is re-driven by the frontier cursor, which already points where
/// it stopped; workers are excluded because whether a dead errand is still worth finishing is
/// a judgment, and `agents.md` gives it to Cognition rather than to a list in code. Both
/// still *have* threads — a worker's is on its row for the boot glance to offer. This governs
/// what resumes **by itself**.
const RESUMED_AT_BOOT: [Role; 2] = [Role::Reaction, Role::Cognition];

/// The errands the last restart killed — what the boot glance offers Cognition.
///
/// The counterpart to [`resumable`], and now the same shape: both answer "which sessions come
/// back", and neither asks anyone. A rung's thread is taken because "the last one" needs no
/// judgment; an errand's is taken because the judgment it needs — is this half-done state still
/// worth finishing — is a question about what already landed, and the only party holding that
/// is the session that was doing it. `agents.md` used to give the call to Cognition, which
/// holds a title, a subject and a timestamp; the session holding the answer was dead on disk
/// one `thread/resume` away.
///
/// **Interrupted, not merely alive.** A worker holds its subprocess until its owner closes it,
/// so most of what a stop catches has already reported and has nothing to finish — sixteen live
/// at the 2026-08-21 quit, four of them mid-turn. Reopening the other twelve would spend a
/// subprocess each on sessions nobody is going to brief. [`Ended::interrupted`] is the bit that
/// tells them apart, and a crash row carries it by construction.
///
/// **Only the previous run's, and that is the whole staleness rule.** The directory is
/// append-only and unpruned, so a filter of "every worker that ever died" would reopen a
/// three-week-old errand at every boot. The run of the newest end row is the run before this
/// one, and an errand that did not die in it is not what the person just restarted out from
/// under.
///
/// Call with the ends **as seeded at boot**, before this run appends any of its own; a row from
/// the current run at the head would empty the list entirely. [`Registry::attach_index`]
/// snapshots it there for exactly that reason.
///
/// An errand with no thread is dropped rather than listed: there is no mind to reopen, and a
/// cold session it cannot tell apart from a resumed one is the one outcome `agents.md` rules
/// out — it would be handed "check what landed before continuing" knowing nothing about what it
/// was doing.
pub fn interrupted_workers(ends: &[Ended]) -> Vec<Ended> {
    let Some(previous_run) = ends.first().map(|end| end.run.clone()) else {
        return Vec::new();
    };
    ends.iter()
        .filter(|end| end.run == previous_run)
        .filter(|end| is_worker_row(end))
        .filter(|end| end.interrupted && end.thread.is_some())
        .cloned()
        .collect()
}

/// Whether a row is a working session's, read off the role name the row was written with.
///
/// Asked through [`Role`] rather than against a `"worker"` literal for the reason
/// [`resumable`] does the same: the spelling lives in one place ([`Role::as_str`], where all
/// five specialisms collapse onto one wire name), so a row and a filter cannot drift apart.
fn is_worker_row(end: &Ended) -> bool {
    Role::ALL.iter().any(|role| role.is_worker() && role.as_str() == end.role)
}

/// The thread each resident rung should resume, from the seeded ends — most recent first,
/// one per role.
///
/// Reads ends that are already sorted by recency, so the first row for a role is that
/// rung's last session, whether it was closed cleanly or lost to a crash. A rung with no
/// prior row (fresh install, or a run that never got a thread open) is simply absent, and
/// its session opens cold.
pub fn resumable(ends: &[Ended]) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for end in ends {
        let Some(thread) = &end.thread else { continue };
        if !RESUMED_AT_BOOT.iter().any(|r| r.as_str() == end.role) {
            continue;
        }
        out.entry(end.role.clone()).or_insert_with(|| thread.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::WorkerType;
    use chrono::TimeZone;

    fn ts(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 11, 4, minute, 0).unwrap()
    }

    fn line(record: &Record) -> String {
        format!("{}\n", serde_json::to_string(record).unwrap())
    }

    /// A thread line binds to its session's row whichever side of the close it lands on.
    /// The tail-read can begin between any two lines, so the fold may see `thread` before
    /// `closed` or after it, and a binding that only worked in one order would attach on
    /// some boots and not others.
    #[test]
    fn a_thread_binds_to_its_row_in_either_order() {
        let closed = closed_record(&ended_now("run-a", &7.into(), Role::Cognition, None, "", None, 3, ts(1), None, false, 0));
        let thread = thread_record("run-a", &7.into(), "th-cognition", ts(2));

        for text in [
            format!("{}{}", line(&closed), line(&thread)),
            format!("{}{}", line(&thread), line(&closed)),
        ] {
            let ends = fold(&text, "run-b");
            assert_eq!(ends.len(), 1, "the thread line is not a session of its own");
            assert_eq!(ends[0].thread.as_deref(), Some("th-cognition"));
        }
    }

    /// A session the process died under still carries its thread — that is the row the next
    /// boot resumes from, and the case the whole record exists for.
    #[test]
    fn a_restart_row_keeps_its_thread() {
        let opened = opened_record("run-a", &2.into(), Role::Reaction, None, "Reaction", None, ts(5));
        let thread = thread_record("run-a", &2.into(), "th-reaction", ts(5));
        let ends = fold(&format!("{}{}", line(&opened), line(&thread)), "run-b");
        assert_eq!(ends.len(), 1);
        assert_eq!(ends[0].how, EndedHow::Restart);
        assert_eq!(ends[0].thread.as_deref(), Some("th-reaction"));
    }

    /// Only the resident rungs are resumed by themselves. A worker keeps its thread on
    /// its row — that is what lets the boot glance offer it — but must never come back on
    /// its own, because whether a dead errand is still worth finishing is Cognition's
    /// judgment (`agents.md`).
    #[test]
    fn only_the_resident_rungs_resume_themselves() {
        let mut text = String::new();
        for (session, role) in [
            (1u64, Role::Reaction),
            (2, Role::Cognition),
            (3, Role::Reflection),
            (4, Role::Worker(WorkerType::General)),
        ] {
            let session = &SessionSlug::from(session);
            text.push_str(&line(&opened_record("run-a", session, role, None, "", None, ts(1))));
            text.push_str(&line(&thread_record("run-a", session, &format!("th-{session}"), ts(1))));
        }
        let plan = resumable(&fold(&text, "run-b"));

        assert_eq!(plan.get("reaction").map(String::as_str), Some("th-1"));
        assert_eq!(plan.get("cognition").map(String::as_str), Some("th-2"));
        assert!(plan.get("reflection").is_none(), "a dead pass is re-driven by the frontier cursor");
        assert!(plan.get("worker").is_none(), "picking an errand back up is Cognition's call");
    }

    /// A rung with several past sessions resumes its most recent, not whichever the fold
    /// happened to reach first.
    #[test]
    fn a_rung_resumes_its_latest_thread() {
        let mut text = String::new();
        for (session, at) in [(1u64, ts(1)), (2, ts(20)), (3, ts(10))] {
            let session = &SessionSlug::from(session);
            text.push_str(&line(&closed_record(&{
                let mut e = ended_now("run-a", session, Role::Cognition, None, "", None, 1, at, None, false, 0);
                e.ended = Some(at);
                e
            })));
            text.push_str(&line(&thread_record("run-a", session, &format!("th-{session}"), at)));
        }
        let plan = resumable(&fold(&text, "run-b"));
        assert_eq!(plan.get("cognition").map(String::as_str), Some("th-2"), "latest by recency");
    }

    /// The offer's contents: workers the process died under, and nothing else. A rung is
    /// excluded because the host resumes it without asking; a cleanly-closed worker is
    /// excluded because it finished and reported.
    #[test]
    fn only_a_workers_unfinished_thread_is_offered() {
        let mut text = String::new();
        for (session, role) in [
            (1u64, Role::Reaction),
            (2, Role::Cognition),
            (3, Role::Worker(WorkerType::General)),
        ] {
            let session = &SessionSlug::from(session);
            text.push_str(&line(&opened_record("run-a", session, role, None, "errand", Some("chase-harbor"), ts(1))));
            text.push_str(&line(&thread_record("run-a", session, &format!("th-{session}"), ts(1))));
        }
        // A fourth worker that finished properly in the same run.
        text.push_str(&line(&closed_record(&ended_now(
            "run-a",
            &4.into(),
            Role::Worker(WorkerType::General),
            None,
            "delivered",
            None,
            2,
            ts(1),
            None, false, 0))));
        text.push_str(&line(&thread_record("run-a", &4.into(), "th-4", ts(2))));

        let putting_back = interrupted_workers(&fold(&text, "run-b"));
        let threads: Vec<_> = putting_back.iter().filter_map(|e| e.thread.as_deref()).collect();
        assert_eq!(threads, vec!["th-3"], "only the errand the restart cut off");
    }

    /// **The staleness rule.** The directory is append-only and unpruned, so without this an
    /// errand killed three weeks ago would be reopened at every boot forever.
    #[test]
    fn only_the_previous_runs_errands_are_reopened() {
        let mut text = String::new();
        // An older run's lost errand…
        text.push_str(&line(&opened_record(
            "run-old",
            &9.into(),
            Role::Worker(WorkerType::General),
            None,
            "ancient",
            None,
            ts(1),
        )));
        text.push_str(&line(&thread_record("run-old", &9.into(), "th-ancient", ts(1))));
        // …and the run that just died, which is the one this boot came out from under.
        text.push_str(&line(&opened_record(
            "run-prev",
            &2.into(),
            Role::Worker(WorkerType::General),
            None,
            "current",
            None,
            ts(30),
        )));
        text.push_str(&line(&thread_record("run-prev", &2.into(), "th-current", ts(30))));

        let putting_back = interrupted_workers(&fold(&text, "run-now"));
        let threads: Vec<_> = putting_back.iter().filter_map(|e| e.thread.as_deref()).collect();
        assert_eq!(threads, vec!["th-current"], "the run before this one, and no further back");
    }

    /// An errand with no thread is not reopened at all. Without one there is no mind to go
    /// back to, and a cold session it cannot tell apart from a resumed one is the one outcome
    /// `agents.md` rules out.
    #[test]
    fn an_errand_without_a_thread_is_not_reopened() {
        let opened =
            opened_record("run-a", &1.into(), Role::Worker(WorkerType::General), None, "errand", None, ts(1));
        assert!(interrupted_workers(&fold(&line(&opened), "run-b")).is_empty());
    }

    /// A row from before threads were recorded has no thread, and must be skipped rather
    /// than resumed as an empty one — an upgrade's first boot is exactly this case.
    #[test]
    fn a_row_without_a_thread_is_not_resumable() {
        let closed = closed_record(&ended_now("run-a", &1.into(), Role::Reaction, None, "", None, 4, ts(1), None, false, 0));
        let plan = resumable(&fold(&line(&closed), "run-b"));
        assert!(plan.is_empty());
    }

    /// A close with both ends pinned. [`ended_now`] stamps the end with `Utc::now()`, which
    /// is its whole contract — it is called at the moment of unregistering — so a test about
    /// *ordering* has to set the end itself rather than pass a start and hope.
    fn closed_at(
        run: &str,
        session: u64,
        role: Role,
        started: DateTime<Utc>,
        ended: DateTime<Utc>,
    ) -> Record {
        let mut row = ended_now(run, &SessionSlug::from(session), role, None, "", None, 1, started, None, false, 0);
        row.ended = Some(ended);
        closed_record(&row)
    }

    /// A `closed` line alone is enough to render a row. This is the property the
    /// duplication buys: no pairing, so no full-file read.
    #[test]
    fn a_closed_line_needs_no_opened_line() {
        let closed = closed_record(&ended_now(
            "run-a",
            &7.into(),
            Role::Worker(WorkerType::ViewBuilder),
            Some(3.into()),
            "build the workers view",
            Some("workers-view"),
            4,
            ts(10),
            None, false, 0));
        let ends = fold(&line(&closed), "run-b");
        assert_eq!(ends.len(), 1);
        let e = &ends[0];
        assert_eq!((e.run.as_str(), e.session.to_string().as_str()), ("run-a", "7"));
        assert_eq!(e.role, "worker");
        assert_eq!(e.worker_type.as_deref(), Some("view-builder"));
        assert_eq!(e.title.as_deref(), Some("build the workers view"));
        assert_eq!(e.owner, Some(Into::into(3)));
        assert_eq!(e.turns, Some(4));
        assert_eq!(e.how, EndedHow::Closed);
    }

    /// The regression this module exists for: a session whose process died has an
    /// `opened` and no `closed`, and must be reported as lost rather than dropped.
    /// `server.log` 2026-08-03 — `worker report dropped; reaction loop gone worker=9`.
    #[test]
    fn an_opened_with_no_closed_from_a_dead_run_reads_as_a_restart() {
        let opened = opened_record(
            "run-a",
            &9.into(),
            Role::Worker(WorkerType::General),
            Some(3.into()),
            "watch the price",
            None,
            ts(5),
        );
        let ends = fold(&line(&opened), "run-b");
        assert_eq!(ends.len(), 1);
        assert_eq!(ends[0].how, EndedHow::Restart);
        assert_eq!(ends[0].session, 9.into());
        assert_eq!(ends[0].ended, None, "nothing recorded an end, so none is claimed");
        assert_eq!(ends[0].started, Some(ts(5)), "and the reader orders it by its start");
    }

    /// A session that opened *and* closed appears once, as a close — not twice, and not
    /// as a restart. The `closed` may arrive after the `opened` in the same window.
    #[test]
    fn an_opened_that_later_closed_is_one_closed_row() {
        let opened = opened_record("run-a", &7.into(), Role::Cognition, None, "", None, ts(1));
        let closed = closed_record(&ended_now("run-a", &7.into(), Role::Cognition, None, "", None, 12, ts(1), None, false, 0));
        let ends = fold(&format!("{}{}", line(&opened), line(&closed)), "run-b");
        assert_eq!(ends.len(), 1, "one session, one row");
        assert_eq!(ends[0].how, EndedHow::Closed);
        assert_eq!(ends[0].turns, Some(12));
    }

    /// A live session in the current run is the switchboard's answer, not this one's.
    /// Counting it here would show every running worker twice on the page.
    #[test]
    fn a_live_session_in_the_current_run_is_not_an_end() {
        let opened = opened_record("run-a", &4.into(), Role::Reflection, None, "", None, ts(2));
        assert!(fold(&line(&opened), "run-a").is_empty());
    }

    /// Same session slug, two runs — the case the run id exists for. Both rows survive and
    /// neither is confused for the other.
    #[test]
    fn the_same_session_id_in_two_runs_is_two_sessions() {
        let a = closed_at("run-a", 1, Role::Cognition, ts(1), ts(5));
        let b = closed_at("run-b", 1, Role::Cognition, ts(2), ts(9));
        let ends = fold(&format!("{}{}", line(&a), line(&b)), "run-c");
        assert_eq!(ends.len(), 2);
        assert_eq!(ends[0].run, "run-b", "most recent first");
        assert_eq!(ends[1].run, "run-a");
    }

    /// Most recent first, and a restart row sorts by its start against a close's end —
    /// otherwise a lost worker sinks below everything and is never seen.
    #[test]
    fn recency_orders_closes_and_restarts_together() {
        let early = closed_at("run-a", 1, Role::Cognition, ts(0), ts(1));
        let lost = opened_record("run-a", &2.into(), Role::Worker(WorkerType::General), Some(1.into()), "", None, ts(30));
        let late = closed_at("run-a", 3, Role::Cognition, ts(0), ts(20));
        let text = format!("{}{}{}", line(&early), line(&lost), line(&late));
        let ends = fold(&text, "run-b");
        assert_eq!(
            ends.iter().map(|e| e.session.to_string()).collect::<Vec<_>>(),
            vec!["2", "3", "1"],
            "lost at :30, closed at :20, closed at :01"
        );
    }

    /// A fragment — the routine first line of a tail read — is skipped, and the rest of
    /// the window still renders. One corrupt line must not blank the page.
    #[test]
    fn a_partial_or_corrupt_line_is_skipped_not_fatal() {
        let good = closed_record(&ended_now("run-a", &5.into(), Role::Cognition, None, "", None, 1, ts(1), None, false, 0));
        let text = format!("run\":\"run-a\",\"session\":4}}\n{}not json\n\n", line(&good));
        let ends = fold(&text, "run-b");
        assert_eq!(ends.len(), 1);
        assert_eq!(ends[0].session, 5.into());
    }

    /// The tail read starts at a line boundary and keeps the whole file when it is small
    /// enough — the two cases every poll takes.
    #[tokio::test]
    async fn a_tail_read_drops_the_partial_line_and_keeps_a_short_file_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.jsonl");
        tokio::fs::write(&path, "aaaa\nbbbb\ncccc\n").await.unwrap();

        assert_eq!(read_tail(&path, 1024).await.unwrap(), "aaaa\nbbbb\ncccc\n");
        // 10 bytes lands mid-`bbbb`; that line is dropped, not half-parsed.
        assert_eq!(read_tail(&path, 10).await.unwrap(), "cccc\n");
    }

    /// A fresh install has no index file, and that is not an error worth logging.
    #[tokio::test]
    async fn a_missing_index_seeds_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(seed(dir.path(), "run-a").await.is_empty());
    }

    /// Round trip through the file the writer actually writes — the serialize and
    /// deserialize halves are a pair here, unlike the observatory's journal.
    ///
    /// Also the flush contract: `flush().await` returning is the promise that everything
    /// queued before it is readable, with no polling and no sleep. The shutdown path has
    /// nothing else to wait on.
    #[tokio::test]
    async fn what_the_writer_writes_is_readable_the_moment_a_flush_returns() {
        let dir = tempfile::tempdir().unwrap();
        let writer = Writer::start(dir.path().to_path_buf());
        writer.write(opened_record("run-a", &2.into(), Role::Worker(WorkerType::DriveOrganizer), Some(1.into()), "file it", None, ts(3)));
        writer.write(closed_record(&ended_now(
            "run-a",
            &2.into(),
            Role::Worker(WorkerType::DriveOrganizer),
            Some(1.into()),
            "file it",
            None,
            2,
            ts(3),
            None, false, 0)));

        writer.flush().await;

        let ends = seed(dir.path(), "run-b").await;
        assert_eq!(ends.len(), 1, "opened + closed for one session is one row");
        assert_eq!(ends[0].worker_type.as_deref(), Some("drive-organizer"));
        assert_eq!(ends[0].how, EndedHow::Closed);
    }

    /// A flush with nothing queued still returns, and a second one after it does too.
    /// Both are the shutdown path: `close_all` on an empty switchboard queues no records,
    /// and a flush that only answers when it has work to do would park the process forever.
    #[tokio::test]
    async fn a_flush_with_nothing_to_write_still_returns() {
        let dir = tempfile::tempdir().unwrap();
        let writer = Writer::start(dir.path().to_path_buf());
        writer.flush().await;
        writer.write(opened_record("run-a", &1.into(), Role::Cognition, None, "", None, ts(1)));
        writer.flush().await;
        writer.flush().await;
        assert!(index_path(dir.path()).exists());
    }

    /// The cap is a display bound, applied after ordering — so it keeps the newest, never
    /// whatever happened to be parsed first.
    #[test]
    fn the_cap_keeps_the_newest() {
        let mut text = String::new();
        for i in 1..=(RECENT_CAP as u64 + 20) {
            let at = Utc.timestamp_opt(1_800_000_000 + i as i64 * 60, 0).unwrap();
            let mut e = ended_now("run-a", &SessionSlug::from(i), Role::Cognition, None, "", None, 1, at, None, false, 0);
            e.ended = Some(at);
            text.push_str(&line(&closed_record(&e)));
        }
        let ends = fold(&text, "run-b");
        assert_eq!(ends.len(), RECENT_CAP);
        assert_eq!(ends[0].session.to_string(), (RECENT_CAP + 20).to_string(), "newest survives");
    }
}
