//! The agent session registry — the switchboard, in code, with no model in it.
//!
//! Agents do not hold references to each other. They hold **addresses**, and this
//! resolves them. That is what makes "the switchboard is the host"
//! ([`docs/arch/foundation.md`](../../../docs/arch/foundation.md#the-agent-session-registry))
//! a mechanism rather than an aspiration: every agent-to-agent edge passes through here,
//! so routing, queueing and liveness live in one place that cannot be slow, confused, or
//! dead.
//!
//! There is **one verb**: [`Registry::send`]. One direction, no reply, queued. A reply is
//! the same verb going the other way — which is why the sender's identity is stamped here
//! and never passed in by the caller. An agent that names itself can name someone else.
//!
//! Nothing in this module talks to the agent wire or to a model. It owns addresses, mailboxes and
//! metadata; who drains a mailbox and what they do with it belongs to the caller.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};

use crate::identity::Role;
use tokio::sync::{watch, Notify};


/// Handle for one agent session, unique process-wide.
///
/// It names a *session*, not a role: a role has many sessions over a run, and a
/// Cognition replaced after a failure is a second session of one role. One namespace
/// for every rung and every worker, because ownership crosses rungs — an owner holds
/// sessions no per-rung counter could
/// name without collision.
pub type SessionId = u64;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// How much of a session's recent output the registry keeps for `SessionMessages`.
///
/// A live tail, not an archive: enough for "how's it going?" without turning the
/// switchboard into a second transcript store. The durable copy is the log, and anything
/// older is replayed from the protocol's own session load.
const OUTPUT_TAIL_CHARS: usize = 4_000;

/// How long a "what is it doing" line may be before it is cut.
///
/// It renders as one line on a roster beside the task, and the frame it renders in may be
/// the window minus a ~400px conversation rail (`docs/arch/stage.md`), so a line that wraps
/// three times pushes every other session off the page.
const ACTIVITY_LINE_CHARS: usize = 120;

pub mod index;

/// The process's registry. One switchboard, as the design says.
pub fn global() -> &'static Registry {
    static G: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
    G.get_or_init(Registry::new)
}

/// Mint the next session id. Called before the underlying session is opened, because the
/// tool surface identifies its caller by this id in a request header — so it cannot be an
/// id the protocol assigns later.
pub fn mint() -> SessionId {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

// Which role a session is running comes from [`crate::identity::Role`] — the one
// namespace for all nine, rungs and worker types alike. This module kept its own
// five-variant copy until now, on the reasoning that prompt and tool surface "are not
// this module's business". They still aren't; the *identity* of the session is, because
// routing turns on it (a worker may address only its owner) and `GET /api/workers`
// reports it. Splitting the type is what left the switchboard unable to say which kind
// of worker a session was.
//
// Only workers are restricted here. That predicate is [`Role::is_worker`], which stays
// correct as worker types are added because they nest inside one variant.

/// What happened to a message — **delivery, never a response.** `send` does not wait for
/// the target to read, act, or agree; it reports whether the message reached a mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// In the target's mailbox. It will be picked up whole on its next prompt.
    Delivered,
    /// No live session at that address. The caller decides what that means — for a report
    /// whose owner has shut down, falling back one rung beats losing finished work.
    Unknown,
    /// A worker addressing something other than its owner. Routing, not policy: whether a
    /// thing is worth saying is judgment and lives in prompts; who may be reached is a
    /// fact and lives here.
    NotPermitted,
}

/// What a session is and how it is doing — **metadata only, no content.**
///
/// Separate from reading its messages on purpose. "Is it still going?" and "what did it
/// find?" are asked at completely different rates, and only the second should cost
/// context.
#[derive(Debug, Clone)]
pub struct Status {
    pub id: SessionId,
    pub role: Role,
    /// The session that created this one and to which its work answers.
    pub owner: Option<SessionId>,
    /// What it is working on, in its own words.
    pub task: String,
    /// Mid-turn right now, versus idle and waiting.
    pub busy: bool,
    /// Whether anything is queued for its next turn.
    pub queued: bool,
    pub turns: u64,
    pub started: DateTime<Utc>,
    /// When this session last **changed state** — opened a turn, finished one, or had mail
    /// land on a quiet inbox.
    ///
    /// `started` cannot answer the question a reader is actually asking. A session that has
    /// been quiet for five minutes and one that finished a turn two seconds ago have the
    /// same `started`, so the only clock on the roster measured the wrong thing: uptime,
    /// when what says whether anything is wrong is *how long it has been like this*. A turn
    /// running for 12 seconds is working; one running for 40 minutes is stuck, and until
    /// this field there was nothing on the wire that could tell them apart.
    pub state_since: DateTime<Utc>,
    /// The last thing this session was seen **doing** — a tool call, a shell command, a
    /// thought — as one short line, or `None` before it has done anything.
    ///
    /// Distinct from its output tail ([`Registry::messages`]), and the distinction is the
    /// point. `output` is what a session has *said*, which is what its owner reads to learn
    /// what it found; this is what it is *doing*, which is what a person reads to learn
    /// whether it is alive. Folding the two would put tool noise into `SessionMessages` and
    /// make a report unreadable — and leaving `doing` out is why a worker four minutes into
    /// a shell command showed a blank line on the roster, which is the same "silence read as
    /// health" this whole surface exists to end.
    pub doing: Option<String>,
    /// When [`doing`](Self::doing) was last replaced, or `None` alongside a `None` `doing`.
    ///
    /// A line with no age says a session is alive and nothing more. `$ cargo test` four
    /// minutes in is working; the same line forty minutes in is hung, and those are the two
    /// answers a reader wants from a busy row. Without this the roster could not distinguish
    /// them — which is the same shape as the `tail`/`doing` split one level down: it is not
    /// enough to know a thing happened, you have to know when.
    pub doing_at: Option<DateTime<Utc>>,
}

/// One message in flight, with the return address the registry stamped on it.
///
/// **`from` travels with the text and is not optional.** A reply is just a message going
/// the other way, so a message that arrives anonymously is one that cannot be answered —
/// and "answer whoever asked" is the whole of a worker's addressing rule.
#[derive(Debug, Clone)]
pub struct Message {
    /// Who to answer — or `None` when the **host** put this here rather than another
    /// agent. A follow-up the reaction loop hands down is not a message from a colleague,
    /// and rendering it with a return address would put a second voice in a room that
    /// has only one.
    pub from: Option<SessionId>,
    pub text: String,
}

/// Render a batch of mail as the text that goes into the recipient's next prompt.
///
/// **One renderer, here, because there is one mailbox.** There were three — one per
/// driver — and they had already drifted into three different strings, three different
/// separators, and one that forgot to trim. Turning an inbox into a prompt is the
/// switchboard's job, not something each rung reinvents; a rung decides what to *do* with
/// its mail, never what it looks like.
///
/// A `from` is a return address, so it is shown: whoever reads this can answer. Host-posted
/// mail (`from: None`) renders bare — the host is not a colleague, and giving it a return
/// address would put a second voice in a room that has one.
pub fn render(batch: &[Message]) -> String {
    batch
        .iter()
        .map(|m| match m.from {
            Some(from) => format!("(from session {from}) {}", m.text.trim()),
            None => m.text.trim().to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Render `reachable` as the block that goes into an agent's window.
///
/// Empty when there is nobody — an empty section is worse than none, because a heading
/// with nothing under it reads as a load that failed rather than as an honest "no one".
pub fn render_reachable(who: &[(String, SessionId)]) -> String {
    if who.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "## Who you can reach right now\nSend with `send_message`, using the number.\n",
    );
    for (label, id) in who {
        s.push_str(&format!("- `{id}` — {label}\n"));
    }
    s
}

/// A session's inbox: messages merged rather than queued.
///
/// Several landing while a session is mid-turn are picked up **together**, so it reads
/// all of them in one prompt rather than running each as its own round-trip. No
/// LLM-smart merge — they are handed over in arrival order and the receiving model
/// reads them as one batch.
#[derive(Default)]
struct Inbox {
    pending: Vec<Message>,
    closed: bool,
}

struct Entry {
    role: Role,
    owner: Option<SessionId>,
    task: String,
    busy: bool,
    turns: u64,
    started: DateTime<Utc>,
    /// When `busy`, or the emptiness of `inbox.pending`, last changed — see
    /// [`Status::state_since`].
    state_since: DateTime<Utc>,
    inbox: Inbox,
    /// Bounded tail of what this session has said, for `SessionMessages`.
    output: String,
    /// The last thing it was seen doing — see [`Status::doing`].
    doing: Option<String>,
    /// When `doing` was last replaced — see [`Status::doing_at`].
    doing_at: Option<DateTime<Utc>>,
    /// The codex thread hosting this session, once `thread/start` has answered. `None`
    /// between registration and that moment — the session exists first, deliberately.
    thread: Option<String>,
    /// Woken when something lands, so an idle session picks it up without polling.
    notify: std::sync::Arc<Notify>,
}

impl Entry {
    /// This entry as the metadata a reader gets. One place, because there are three
    /// callers ([`Registry::status`], [`Registry::session_of_role`],
    /// [`Registry::statuses`]) and they were three copies of the same nine-field literal —
    /// so a field added to `Status` had three chances to be forgotten in one.
    fn status(&self, id: SessionId) -> Status {
        Status {
            id,
            role: self.role,
            owner: self.owner,
            task: self.task.clone(),
            busy: self.busy,
            queued: !self.inbox.pending.is_empty(),
            turns: self.turns,
            started: self.started,
            state_since: self.state_since,
            doing: self.doing.clone(),
            doing_at: self.doing_at,
        }
    }

    /// Stamp a state change, if this is one.
    ///
    /// Every transition goes through here rather than each call site writing `Utc::now()`,
    /// because the field only means anything if *all* of them stamp it: one path that
    /// changes `busy` without moving the clock reports a turn as older than it is, and the
    /// reading it exists to give — how long has it been like this — is silently wrong
    /// exactly on the path that skipped it.
    fn note_state_change(&mut self, changed: bool) {
        if changed {
            self.state_since = Utc::now();
        }
    }

    /// Whether this entry is quiet: no turn in flight and nothing waiting. Mail landing on
    /// a quiet session is a state change (idle → waiting); mail landing on a busy or
    /// already-queued one is not.
    fn is_quiet(&self) -> bool {
        !self.busy && self.inbox.pending.is_empty()
    }
}

/// A registration that ends when it goes out of scope.
///
/// The reaction loop leaves by several paths — inbound closed, closed mid-settle, shutdown
/// — and a registration released at only some of them is how the agent ends up with more
/// than one voice, `reachable` then offering an arbitrary dead one. Rather than
/// remember every exit, hold this: the exits are then not something anyone has to get
/// right again, including whoever adds the next one.
pub struct Registration {
    id: SessionId,
    /// Woken when mail lands. Cloneable and outlives nothing — dropping the handle is
    /// what closes the registration, not dropping this.
    pub mail: std::sync::Arc<Notify>,
}

impl Registration {
    pub fn id(&self) -> SessionId {
        self.id
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        global().unregister(self.id);
    }
}

/// Register a session in the process switchboard, releasing it when the returned handle
/// is dropped. The scope-bound form of [`Registry::register`]; prefer it for anything
/// whose lifetime is a scope rather than a task.
pub fn register_scoped(
    id: SessionId,
    role: Role,
    owner: Option<SessionId>,
    task: String,
) -> Registration {
    let mail = global().register(id, role, owner, task);
    Registration { id, mail }
}

/// The switchboard. One per process.
pub struct Registry {
    sessions: Mutex<HashMap<SessionId, Entry>>,
    activity: watch::Sender<u64>,
    /// The durable session directory, once [`Registry::attach_index`] has been called at
    /// boot. Absent in tests and anywhere without a data dir, in which case the switchboard
    /// behaves exactly as it did before — live-only, nothing kept.
    index: std::sync::OnceLock<index::Writer>,
    /// Sessions that are no longer live, newest first — seeded from the directory at boot
    /// and appended to as sessions close.
    ///
    /// **In memory because the read is per-poll and the file is unpruned.** A roster
    /// refreshing every few seconds must not re-read a months-old append-only log to answer
    /// a question about this afternoon; the file is the durable copy and this is the working
    /// set, which is the same split [`Registry::messages`] already makes against the frame
    /// log.
    recent: Mutex<Vec<index::Ended>>,
    /// The thread each resident rung resumes at boot, keyed by role, seeded once by
    /// [`Registry::attach_index`] and **taken** rather than read.
    ///
    /// Take-once is the discard rule, expressed as a data structure. The first session a
    /// rung opens in a run is the resume; every later one — and every reopen after a turn
    /// fails — finds the slot empty and opens cold. So a thread wedged badly enough to
    /// break a turn cannot be handed back to the session replacing it, and a thread that
    /// crashed the host is resumed exactly once before the next boot starts fresh.
    resumable: Mutex<HashMap<String, String>>,
    /// The errands the last restart killed, for the boot glance to offer Cognition.
    ///
    /// Snapshotted at [`Registry::attach_index`] rather than derived from `recent` on demand,
    /// because `recent` grows this run's own ends as sessions close and
    /// [`index::lost_workers`] reads the head of the list to decide which run "the previous
    /// one" was. A read taken after the first session closes would answer about this run and
    /// find nothing.
    ///
    /// Read, never taken: unlike `resumable` there is no discard rule to express here, since
    /// the only reader is the one-shot boot note.
    lost: Mutex<Vec<index::Ended>>,
}

impl Default for Registry {
    fn default() -> Self {
        let (activity, _) = watch::channel(0);
        Self {
            sessions: Mutex::new(HashMap::new()),
            activity,
            index: std::sync::OnceLock::new(),
            recent: Mutex::new(Vec::new()),
            resumable: Mutex::new(HashMap::new()),
            lost: Mutex::new(Vec::new()),
        }
    }
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Announce a live session. `id` comes from [`mint`], claimed before the session was
    /// opened.
    pub fn register(
        &self,
        id: SessionId,
        role: Role,
        owner: Option<SessionId>,
        task: String,
    ) -> std::sync::Arc<Notify> {
        let notify = std::sync::Arc::new(Notify::new());
        let started = Utc::now();
        {
            let mut map = self.sessions.lock().unwrap();
            map.insert(
                id,
                Entry {
                    role,
                    owner,
                    task: task.clone(),
                    busy: false,
                    turns: 0,
                    started,
                    // A fresh session is idle, and has been since it existed.
                    state_since: started,
                    inbox: Inbox::default(),
                    output: String::new(),
                    doing: None,
                    doing_at: None,
                    thread: None,
                    notify: notify.clone(),
                },
            );
        }
        // Recorded on the way in, not only on the way out, because the way out is exactly
        // what a crash skips. An `opened` with no `closed` is how a restart-killed session
        // becomes visible at all — see [`index::EndedHow::Restart`].
        if let Some(writer) = self.index.get() {
            writer.write(index::opened_record(
                crate::foundation::run::id(),
                id,
                role,
                owner,
                &task,
                started,
            ));
        }
        self.note_activity();
        notify
    }

    /// Start recording sessions to the durable directory under `data_dir`, and seed the
    /// recent-ends list from what previous runs left there.
    ///
    /// Called once at boot. Idempotent: a second call is ignored, because the writer owns a
    /// spawned task and two of them would interleave lines into one file.
    pub async fn attach_index(&self, data_dir: std::path::PathBuf) {
        let run = crate::foundation::run::id();
        let seeded = index::seed(&data_dir, run).await;
        if self.index.set(index::Writer::start(data_dir)).is_err() {
            tracing::warn!("the session index is already attached; ignoring");
            return;
        }
        let found = seeded.len();
        let lost = seeded.iter().filter(|e| e.how == index::EndedHow::Restart).count();
        let resumable = index::resumable(&seeded);
        let resuming = resumable.len();
        // Before `recent` takes the list: `lost_workers` reads its head to decide which run
        // was the previous one, and this run's first close would move that head.
        let offered = index::lost_workers(&seeded);
        let offering = offered.len();
        *self.lost.lock().unwrap() = offered;
        *self.resumable.lock().unwrap() = resumable;
        *self.recent.lock().unwrap() = seeded;
        // Worth a line at boot: `lost` is the count of sessions a previous run died
        // underneath, which nothing used to be able to say out loud. `resuming` is how many
        // resident rungs are picking their previous thread back up — zero on a fresh
        // install, and zero for any rung whose last run predates thread recording. `offering`
        // is the subset of `lost` that is a worker with a thread, which the boot glance hands
        // to Cognition to pick from — never resumed by the host, only offered.
        tracing::info!(
            run,
            recent = found,
            lost,
            resuming,
            offering,
            "session directory attached"
        );
    }

    /// Record the codex thread a session opened, in memory and in the directory.
    ///
    /// Called once per session, right after `thread/start` answers. A session that never
    /// gets one (the spawn failed, the process died first) simply has no thread on its row,
    /// which reads as "there is no mind to go back to" rather than as missing data.
    pub fn note_thread(&self, id: SessionId, thread_id: &str) {
        if let Some(e) = self.sessions.lock().unwrap().get_mut(&id) {
            e.thread = Some(thread_id.to_string());
        }
        if let Some(writer) = self.index.get() {
            writer.write(index::thread_record(
                crate::foundation::run::id(),
                id,
                thread_id,
                Utc::now(),
            ));
        }
    }

    /// Take the thread `role` should resume, leaving nothing behind — see
    /// [`Registry::resumable`](#structfield.resumable) for why taking rather than reading is
    /// the discard rule.
    pub fn take_resumable(&self, role: Role) -> Option<String> {
        self.resumable.lock().unwrap().remove(role.as_str())
    }

    /// The errands the last restart killed, newest first — see [`index::lost_workers`].
    ///
    /// Offered, not resumed: the caller is the boot glance, and what it does with these is
    /// put them in front of Cognition.
    pub fn lost_workers(&self) -> Vec<index::Ended> {
        self.lost.lock().unwrap().clone()
    }

    /// Unregister every live session, in id order.
    ///
    /// For the shutdown path: a graceful stop genuinely ends these, and the host is still
    /// alive to say so. That is the whole difference between the directory reporting
    /// [`index::EndedHow::Closed`] and [`index::EndedHow::Restart`] — the latter means
    /// nothing got to record an end, and it only carries a warning if a clean exit does not
    /// produce it.
    ///
    /// Ids are collected before unregistering because [`Registry::unregister`] takes the
    /// same lock a snapshot would still be holding.
    pub fn close_all(&self) {
        let ids: Vec<SessionId> = {
            let map = self.sessions.lock().unwrap();
            let mut ids: Vec<SessionId> = map.keys().copied().collect();
            ids.sort_unstable();
            ids
        };
        if ids.is_empty() {
            return;
        }
        tracing::info!(sessions = ids.len(), "closing the switchboard");
        for id in ids {
            self.unregister(id);
        }
    }

    /// Wait until every record queued for the session directory has reached the disk.
    ///
    /// Pairs with [`close_all`](Self::close_all) on the shutdown path — see
    /// [`index::Writer::flush`] for why closing without flushing would write the crash
    /// signature on a clean exit. A no-op when no index is attached.
    pub async fn flush_index(&self) {
        if let Some(writer) = self.index.get() {
            writer.flush().await;
        }
    }

    /// Sessions that are no longer live, newest first, at most `limit`.
    pub fn recent_ended(&self, limit: usize) -> Vec<index::Ended> {
        let recent = self.recent.lock().unwrap();
        recent.iter().take(limit).cloned().collect()
    }

    /// Drop a session. Anything still in its inbox goes with it — undelivered is the
    /// honest outcome, and the sender was told `Delivered` about a mailbox, never about
    /// an outcome.
    pub fn unregister(&self, id: SessionId) {
        let removed = if let Some(mut e) = self.sessions.lock().unwrap().remove(&id) {
            e.inbox.closed = true;
            Some(index::ended_now(
                crate::foundation::run::id(),
                id,
                e.role,
                e.owner,
                &e.task,
                e.turns,
                e.started,
                e.thread.clone(),
            ))
        } else {
            None
        };
        if let Some(ended) = removed {
            // The file and the in-memory list get the same row. The list is what a poll
            // reads; the file is what survives this process.
            if let Some(writer) = self.index.get() {
                writer.write(index::closed_record(&ended));
            }
            index::push_recent(&mut self.recent.lock().unwrap(), ended);
            self.note_activity();
        }
    }

    /// Send `message` to `to`, from `from`.
    ///
    /// **`from` is supplied by the host, not by the calling agent.** The host knows who is
    /// calling; letting an agent name itself is letting it impersonate another.
    ///
    /// One direction, no reply. The return value says whether it reached a mailbox — a
    /// reply, if there is one, arrives later as its own `send` in the other direction.
    /// **`to` is a session id, and that is the only address there is.**
    ///
    /// A worker's id comes back from `CreateWorker`; a standing rung's is projected into
    /// the window of whoever may reach it ([`Registry::reachable`]). What this replaced —
    /// letting an agent name a destination by some other string and searching for the
    /// session behind it — was
    /// retrieval, and a retrieval that misses is indistinguishable from nobody being
    /// there. Being told who is live, every turn, is strictly more information than being
    /// allowed to guess, and it turns this from a scan into a map lookup.
    pub fn send(&self, from: SessionId, to: SessionId, message: String) -> Delivery {
        let delivery = {
            let mut map = self.sessions.lock().unwrap();

            // A worker answers to whoever asked, and to nobody else.
            if let Some(sender) = map.get(&from)
                && sender.role.is_worker()
                && sender.owner != Some(to)
            {
                return Delivery::NotPermitted;
            }

            let Some(entry) = map.get_mut(&to) else {
                return Delivery::Unknown;
            };
            if entry.inbox.closed {
                return Delivery::Unknown;
            }
            entry.note_state_change(entry.is_quiet());
            entry.inbox.pending.push(Message { from: Some(from), text: message });
            entry.notify.notify_one();
            Delivery::Delivered
        };
        self.note_activity();
        delivery
    }

    /// Who `asker` may reach right now, as `(label, id)` — the projection that replaced
    /// name-a-destination addressing.
    ///
    /// Deliberately narrow, and narrow **per asker**, because this is the whole of what an
    /// agent knows about the rest of the agent: what it is offered here is what it can
    /// do. A worker gets its owner and nothing else, which is also the only thing the
    /// routing rule would let it send to; the voice's rungs get the shared brain; Cognition
    /// gets the voice, because that is the one way anything reaches the person.
    ///
    /// Rebuilt every turn by the caller. There is no cache and should not be: the answer
    /// is only true for as long as those sessions are up, and a stale id is worse than no
    /// id — it sends somewhere real.
    pub fn reachable(&self, asker: SessionId) -> Vec<(String, SessionId)> {
        let map = self.sessions.lock().unwrap();
        let Some(me) = map.get(&asker) else { return Vec::new() };

        let mut out: Vec<(String, SessionId)> = Vec::new();
        match me.role {
            // Its owner, which the routing rule already limits it to.
            Role::Worker(_) => {
                if let Some(owner) = me.owner {
                    out.push(("the session that asked for this work".to_string(), owner));
                }
            }
            // The voice hands work up, and that is all it addresses.
            Role::Reaction => {
                if let Some((id, _)) = map.iter().find(|(_, e)| e.role == Role::Cognition) {
                    out.push(("cognition — the shared brain".to_string(), *id));
                }
            }
            // The voice, so anything worth saying has somewhere to land, plus whatever
            // this rung has running. A voice that is cold simply is not here, which is
            // the fact Cognition needs before it decides to hold a result rather than
            // send at it.
            Role::Cognition | Role::Reflection => {
                for (id, e) in map.iter() {
                    if e.role == Role::Reaction {
                        out.push(("the voice — what reaches the person".to_string(), *id));
                    }
                }
                for (id, e) in map.iter() {
                    if e.owner == Some(asker) {
                        out.push((format!("your worker: {}", e.task.trim()), *id));
                    }
                }
            }
        }
        out.sort_by_key(|(_, id)| *id);
        out
    }

    /// Put `text` in `id`'s inbox **on the host's own behalf** — no sender, and none of
    /// the addressing rules that govern one agent reaching another.
    ///
    /// The rules exist to stop an agent talking somewhere it has no business; the host
    /// is not an agent and is the thing that enforces them. This is how a follow-up
    /// reaches a warm session, and it answers the one question the caller actually has:
    /// is that session still able to take work, or has it closed and does this need a
    /// fresh one?
    pub fn post(&self, id: SessionId, text: String) -> Delivery {
        let delivery = {
            let mut map = self.sessions.lock().unwrap();
            let Some(entry) = map.get_mut(&id) else {
                return Delivery::Unknown;
            };
            if entry.inbox.closed {
                return Delivery::Unknown;
            }
            entry.note_state_change(entry.is_quiet());
            entry.inbox.pending.push(Message { from: None, text });
            entry.notify.notify_one();
            Delivery::Delivered
        };
        self.note_activity();
        delivery
    }

    /// Mark a session's turn as running.
    ///
    /// `take_pending` already performs this transition for mailbox-driven turns.
    /// Directly-driven turns (Reaction's queue and a worker's initial task) use this
    /// method so every status reader observes the same lifecycle.
    pub fn start_turn(&self, id: SessionId) {
        let changed = {
            let mut map = self.sessions.lock().unwrap();
            let Some(entry) = map.get_mut(&id) else {
                return;
            };
            if entry.busy {
                false
            } else {
                entry.busy = true;
                entry.turns += 1;
                entry.note_state_change(true);
                true
            }
        };
        if changed {
            self.note_activity();
        }
    }

    /// Take everything queued for `id`, if anything is. Marks the session busy — it is
    /// about to take a turn, and an agent with a turn in flight is not idle.
    pub fn take_pending(&self, id: SessionId) -> Option<Vec<Message>> {
        let batch = {
            let mut map = self.sessions.lock().unwrap();
            let entry = map.get_mut(&id)?;
            if entry.inbox.pending.is_empty() {
                return None;
            }
            // Only when it was not already mid-turn: emptying the inbox of a *busy* session
            // leaves it running, which is the state it was already in.
            entry.note_state_change(!entry.busy);
            if !entry.busy {
                entry.busy = true;
                entry.turns += 1;
            }
            std::mem::take(&mut entry.inbox.pending)
        };
        self.note_activity();
        Some(batch)
    }

    /// Drain queued mail without opening a turn.
    ///
    /// Reaction folds this mailbox into its separate input queue, then starts one
    /// combined turn after the settle window. Marking a turn here would create a
    /// false busy/idle edge before that real turn begins.
    pub fn drain_pending(&self, id: SessionId) -> Option<Vec<Message>> {
        let batch = {
            let mut map = self.sessions.lock().unwrap();
            let entry = map.get_mut(&id)?;
            if entry.inbox.pending.is_empty() {
                return None;
            }
            // Waiting → idle. A busy session was, and stays, running.
            entry.note_state_change(!entry.busy);
            std::mem::take(&mut entry.inbox.pending)
        };
        self.note_activity();
        Some(batch)
    }

    /// Take everything queued for `id` — or, finding nothing, **close the inbox** and
    /// report that by returning `None` with the mailbox now shut.
    ///
    /// One call because it is one decision under one lock. A session that has idled out
    /// wants to stop; a message racing that decision must either be taken or must find
    /// the mailbox already closed and spawn its own fresh session. Split into a peek and
    /// a close, the message that lands between them is lost — silently, and only under
    /// load, which is the worst way to find out.
    pub fn take_pending_or_close(&self, id: SessionId) -> Option<Vec<Message>> {
        let batch = {
            let mut map = self.sessions.lock().unwrap();
            let Some(entry) = map.get_mut(&id) else {
                return None;
            };
            if entry.inbox.pending.is_empty() {
                entry.inbox.closed = true;
                return None;
            }
            entry.note_state_change(!entry.busy);
            if !entry.busy {
                entry.busy = true;
                entry.turns += 1;
            }
            std::mem::take(&mut entry.inbox.pending)
        };
        self.note_activity();
        Some(batch)
    }

    /// The handle woken when mail lands for `id`, for a loop that wants to wait on its
    /// own inbox without polling. Same `Notify` [`register`](Self::register) returned.
    pub fn notifier(&self, id: SessionId) -> Option<std::sync::Arc<Notify>> {
        self.sessions.lock().unwrap().get(&id).map(|e| e.notify.clone())
    }

    /// Mark a turn finished.
    pub fn finish_turn(&self, id: SessionId) {
        let changed = {
            let mut map = self.sessions.lock().unwrap();
            if let Some(e) = map.get_mut(&id) {
                let changed = e.busy;
                e.busy = false;
                e.note_state_change(changed);
                changed
            } else {
                false
            }
        };
        if changed {
            self.note_activity();
        }
    }

    /// Replace the human-readable task attached to a live session.
    ///
    /// A session is registered before it receives its first real task, so the
    /// switchboard entry must be able to move from a startup placeholder to the work it
    /// was actually handed.
    pub fn set_task(&self, id: SessionId, task: String) {
        if let Some(e) = self.sessions.lock().unwrap().get_mut(&id) {
            e.task = task;
        }
    }

    /// Append to a session's visible output, keeping only the recent tail.
    pub fn record_output(&self, id: SessionId, chunk: &str) {
        if let Some(e) = self.sessions.lock().unwrap().get_mut(&id) {
            e.output.push_str(chunk);
            let n = e.output.chars().count();
            if n > OUTPUT_TAIL_CHARS {
                e.output = e.output.chars().skip(n - OUTPUT_TAIL_CHARS).collect();
            }
        }
    }

    /// Note the last thing a session was seen **doing** — see [`Status::doing`].
    ///
    /// One line, replaced rather than appended: this answers "is it alive and on what",
    /// which only the newest answer serves. Long lines are cut, because the caller is
    /// summarizing a tool call and a shell command can be a screenful.
    pub fn record_activity(&self, id: SessionId, what: &str) {
        let what = what.trim();
        if what.is_empty() {
            return;
        }
        let line: String = match what.char_indices().nth(ACTIVITY_LINE_CHARS) {
            Some((cut, _)) => format!("{}…", &what[..cut]),
            None => what.to_string(),
        };
        if let Some(e) = self.sessions.lock().unwrap().get_mut(&id) {
            e.doing = Some(line);
            e.doing_at = Some(Utc::now());
        }
    }

    /// What a session has recently said. Costs context — which is exactly why it is a
    /// different call from [`status`](Self::status).
    pub fn messages(&self, id: SessionId) -> Option<String> {
        let map = self.sessions.lock().unwrap();
        map.get(&id).map(|e| e.output.clone())
    }

    /// Metadata for one session. Cheap by construction — no content crosses.
    pub fn status(&self, id: SessionId) -> Option<Status> {
        let map = self.sessions.lock().unwrap();
        Some(map.get(&id)?.status(id))
    }

    /// The live session holding `role`, if there is one — for the **singleton** rungs,
    /// where "the Cognition" names a thing rather than a category.
    ///
    /// Lowest id wins if two are somehow up, which is a tie that should not happen and is
    /// resolved deterministically rather than arbitrarily: a `HashMap` iteration order
    /// would make the answer differ between two calls in one turn, and a caller asking
    /// "is it busy" twice and getting two sessions is worse than a caller consistently
    /// reading the older one.
    ///
    /// **Only meaningful for a rung.** Asking for `Role::Worker(_)` gets an arbitrary
    /// worker, which is never a useful question — a worker is addressed by the id its
    /// creator holds.
    pub fn session_of_role(&self, role: Role) -> Option<Status> {
        let map = self.sessions.lock().unwrap();
        let id = map.iter().filter(|(_, e)| e.role == role).map(|(&id, _)| id).min()?;
        Some(map.get(&id)?.status(id))
    }

    /// Metadata for every live session, ordered by id.
    pub fn statuses(&self) -> Vec<Status> {
        let map = self.sessions.lock().unwrap();
        let mut rows: Vec<Status> = map.iter().map(|(&id, e)| e.status(id)).collect();
        rows.sort_by_key(|status| status.id);
        rows
    }

    /// Subscribe to changes that can affect live activity projection.
    pub fn subscribe_activity(&self) -> watch::Receiver<u64> {
        self.activity.subscribe()
    }

    /// Every session `owner` created, oldest id first.
    pub fn children(&self, owner: SessionId) -> Vec<SessionId> {
        let map = self.sessions.lock().unwrap();
        let mut ids: Vec<SessionId> = map
            .iter()
            .filter(|(_, e)| e.owner == Some(owner))
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        ids
    }

    /// Whether `id` owns anything still live.
    ///
    /// **An agent with live children is not idle.** Reaping an owner out from under
    /// running work is what creates orphans; the fix is to not call it idle in the first
    /// place, so whatever decides to close a session asks this first.
    pub fn has_live_children(&self, id: SessionId) -> bool {
        let map = self.sessions.lock().unwrap();
        map.values().any(|e| e.owner == Some(id))
    }

    fn note_activity(&self) {
        self.activity.send_modify(|version| *version = version.wrapping_add(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::WorkerType;

    fn reg() -> Registry {
        Registry::new()
    }

    /// The offer survives the trip through `attach_index`, and survives this run's own
    /// sessions closing on top of it.
    ///
    /// The ordering inside `attach_index` is the part worth pinning: [`index::lost_workers`]
    /// reads the head of the ends list to decide which run was the previous one, and `recent`
    /// grows this run's ends as sessions close. Snapshot it after `recent` takes the list — or
    /// derive it on demand — and the first session to close moves that head to *this* run,
    /// where there are no lost errands, and the offer silently empties. Which is exactly the
    /// shape of bug that only shows up on a boot where something closed early.
    #[tokio::test]
    async fn the_offer_is_snapshotted_at_boot_and_outlives_this_runs_own_ends() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_path_buf();
        let path = index::index_path(&data_dir);
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();

        let at = Utc::now();
        let mut text = String::new();
        for record in [
            index::opened_record(
                "run-prev",
                4,
                Role::Worker(WorkerType::General),
                Some(3),
                "chase the deploy",
                at,
            ),
            index::thread_record("run-prev", 4, "th-errand", at),
        ] {
            text.push_str(&format!("{}\n", serde_json::to_string(&record).unwrap()));
        }
        tokio::fs::write(&path, text).await.unwrap();

        let r = reg();
        r.attach_index(data_dir).await;

        let offered = r.lost_workers();
        assert_eq!(offered.len(), 1, "the previous run's unfinished errand");
        assert_eq!(offered[0].thread.as_deref(), Some("th-errand"));
        assert_eq!(offered[0].task.as_deref(), Some("chase the deploy"));

        // Now let this run close a session of its own, which pushes onto `recent`.
        let id = mint();
        r.register(id, Role::Cognition, None, "the shared brain".into());
        r.unregister(id);

        assert_eq!(
            r.lost_workers().len(),
            1,
            "a close in this run must not empty the offer"
        );
    }

    #[test]
    fn a_message_reaches_the_target_inbox() {
        let r = reg();
        let (a, b) = (mint(), mint());
        r.register(a, Role::Cognition, None, "thinking".into());
        r.register(b, Role::Worker(WorkerType::General), Some(a), "the errand".into());

        assert_eq!(r.send(a, b, "go".into()), Delivery::Delivered);
        let mail = r.take_pending(b).expect("delivered");
        assert_eq!(mail.len(), 1);
        assert_eq!(mail[0].text, "go");
        assert_eq!(mail[0].from, Some(a), "the return address rides with the message");
        assert!(r.take_pending(b).is_none(), "taking drains the inbox");
    }

    /// Several messages arriving while a session is mid-turn must cost one turn, not
    /// several: the point of merging is that a burst reads as one prompt.
    #[test]
    fn messages_landing_together_merge_into_one_prompt() {
        let r = reg();
        let (a, b) = (mint(), mint());
        r.register(a, Role::Cognition, None, String::new());
        r.register(b, Role::Worker(WorkerType::General), Some(a), String::new());

        r.send(a, b, "first".into());
        r.send(a, b, "second".into());
        let mail = r.take_pending(b).expect("both delivered");
        assert_eq!(
            mail.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            ["first", "second"],
            "a burst is taken together, in arrival order"
        );
        assert_eq!(r.status(b).unwrap().turns, 1, "a burst costs one turn, not several");
    }

    /// The race the atomic take-or-close exists for: a session idling out and a message
    /// landing are one decision, so exactly one of them wins and neither is lost.
    #[test]
    fn taking_or_closing_never_loses_the_racing_message() {
        let r = reg();
        let (a, b) = (mint(), mint());
        r.register(a, Role::Cognition, None, String::new());
        r.register(b, Role::Worker(WorkerType::General), Some(a), String::new());

        // Mail present: it is taken, and the inbox stays open for more.
        r.send(a, b, "one more thing".into());
        let mail = r.take_pending_or_close(b).expect("mail wins over the close");
        assert_eq!(mail[0].text, "one more thing");
        assert_eq!(
            r.send(a, b, "and another".into()),
            Delivery::Delivered,
            "taking mail must not close the mailbox"
        );

        // Drain, then find it empty: now it closes, and later sends are told so.
        r.take_pending(b);
        assert!(r.take_pending_or_close(b).is_none());
        assert_eq!(
            r.send(a, b, "too late".into()),
            Delivery::Unknown,
            "a closed inbox reports Unknown so the sender starts something fresh"
        );
    }

    /// The host is not an agent: it may hand work to any live session, and what it
    /// hands over carries no return address because there is nobody to answer.
    #[test]
    fn the_host_can_post_without_being_a_sender() {
        let r = reg();
        let (owner, w) = (mint(), mint());
        r.register(owner, Role::Cognition, None, String::new());
        r.register(w, Role::Worker(WorkerType::General), Some(owner), String::new());

        // A worker may not address itself as an agent — that is not its owner.
        assert_eq!(r.send(w, w, "self".into()), Delivery::NotPermitted);
        // The host posting the same follow-up is fine, and arrives anonymous.
        assert_eq!(r.post(w, "keep going".into()), Delivery::Delivered);
        let mail = r.take_pending(w).expect("posted");
        assert_eq!(mail[0].from, None);
        assert_eq!(mail[0].text, "keep going");

        r.unregister(w);
        assert_eq!(r.post(w, "too late".into()), Delivery::Unknown);
    }

    /// The bug this exists to make impossible: the reaction loop leaves by several paths, and
    /// a registration released at only some of them leaves a second voice behind for the
    /// one role — which `reachable` would then offer, and a sender would send at.
    #[test]
    fn a_scoped_registration_ends_with_its_scope() {
        let sender = mint();
        global().register(sender, Role::Cognition, None, String::new());

        let id = {
            let voice =
                register_scoped(mint(), Role::Reaction, None, String::new());
            let id = voice.id();
            assert_eq!(
                global().send(sender, id, "hi".into()),
                Delivery::Delivered
            );
            id
        };

        assert!(global().status(id).is_none(), "leaving the scope closed the registration");
        assert_eq!(
            global().send(sender, id, "hi again".into()),
            Delivery::Unknown,
            "no stale voice is left registered"
        );
        global().unregister(sender);
    }

    #[test]
    fn a_notifier_is_reachable_after_registration() {
        let r = reg();
        let a = mint();
        r.register(a, Role::Reaction, None, String::new());
        assert!(r.notifier(a).is_some());
        assert!(r.notifier(9_999).is_none());
    }

    /// The sender must be able to tell the difference between "it arrived" and "there was
    /// nobody there" — a report whose owner has gone needs to fall back rather than be
    /// silently dropped.
    #[test]
    fn an_absent_target_is_reported_not_swallowed() {
        let r = reg();
        let a = mint();
        r.register(a, Role::Cognition, None, String::new());
        assert_eq!(r.send(a, 9_999, "hello".into()), Delivery::Unknown);

        let gone = mint();
        r.register(gone, Role::Worker(WorkerType::General), Some(a), String::new());
        r.unregister(gone);
        assert_eq!(r.send(a, gone, "hello".into()), Delivery::Unknown);
    }

    /// Routing, not policy: a worker answers whoever asked and cannot reach past them —
    /// not a sibling, and not the conversation.
    #[test]
    fn a_worker_may_address_only_its_owner() {
        let r = reg();
        let (owner, other, worker) = (mint(), mint(), mint());
        r.register(owner, Role::Cognition, None, String::new());
        r.register(other, Role::Reaction, None, String::new());
        r.register(worker, Role::Worker(WorkerType::General), Some(owner), String::new());

        assert_eq!(r.send(worker, owner, "done".into()), Delivery::Delivered);
        assert_eq!(
            r.send(worker, other, "psst".into()),
            Delivery::NotPermitted
        );
    }

    /// The projection that replaced name-a-destination addressing. The voice is offered
    /// the shared brain and nothing else, because handing work up is the only edge it has.
    #[test]
    fn the_voice_is_offered_the_shared_brain() {
        let r = reg();
        let (rx, cog) = (mint(), mint());
        r.register(rx, Role::Reaction, None, String::new());
        r.register(cog, Role::Cognition, None, "thinking".into());

        let who = r.reachable(rx);
        assert_eq!(who.len(), 1, "{who:?}");
        assert_eq!(who[0].1, cog);
        assert!(who[0].0.contains("shared brain"), "{who:?}");

        // And the id it was handed is one it can actually send to.
        assert_eq!(r.send(rx, who[0].1, "a real errand".into()), Delivery::Delivered);
        assert_eq!(r.take_pending(cog).expect("delivered")[0].text, "a real errand");
    }

    /// A cold rung is simply absent from the list, which is the point: the asker learns
    /// there is nobody there *before* sending, instead of guessing a name and being told
    /// `Unknown` after the fact.
    #[test]
    fn a_rung_that_is_not_up_is_not_offered() {
        let r = reg();
        let rx = mint();
        r.register(rx, Role::Reaction, None, String::new());
        assert!(r.reachable(rx).is_empty());
    }

    /// Cognition is offered the live voice, because that is the one way anything it
    /// works out reaches the person.
    #[test]
    fn cognition_is_offered_the_voice_and_its_own_workers() {
        let r = reg();
        let (cog, rx, other, w) = (mint(), mint(), mint(), mint());
        r.register(cog, Role::Cognition, None, "thinking".into());
        r.register(rx, Role::Reaction, None, String::new());
        r.register(other, Role::Worker(WorkerType::General), Some(rx), String::new());
        r.register(w, Role::Worker(WorkerType::General), Some(cog), "file the receipts".into());

        let who = r.reachable(cog);
        let ids: Vec<SessionId> = who.iter().map(|(_, id)| *id).collect();
        assert!(ids.contains(&rx), "the voice: {who:?}");
        assert!(ids.contains(&w), "its own worker: {who:?}");
        assert!(!ids.contains(&other), "someone else's worker is not offered: {who:?}");
    }

    /// The lookup the hand-down rides on. Reaction has no id for Cognition — nothing
    /// hands it one, and `reachable` rebuilds its list per turn precisely because a
    /// stored id goes stale — so the host asks the switchboard by role at the moment it
    /// posts.
    #[test]
    fn a_singleton_rung_can_be_found_by_its_role() {
        let r = reg();
        assert!(r.session_of_role(Role::Cognition).is_none(), "nothing is up yet");

        let (rx, cog) = (mint(), mint());
        r.register(rx, Role::Reaction, None, String::new());
        r.register(cog, Role::Cognition, None, "thinking".into());

        let found = r.session_of_role(Role::Cognition).expect("cognition is up");
        assert_eq!(found.id, cog);
        assert_eq!(found.task, "thinking");
        assert_eq!(r.session_of_role(Role::Reaction).map(|s| s.id), Some(rx));
    }

    /// A rung that has gone is *absent*, not stale — the caller must be able to tell
    /// "nobody to hand to" from "handed and waiting", because only one of those means
    /// the person is owed an answer that is coming.
    #[test]
    fn a_rung_that_unregistered_is_no_longer_found_by_role() {
        let r = reg();
        let cog = mint();
        r.register(cog, Role::Cognition, None, String::new());
        r.unregister(cog);
        assert!(r.session_of_role(Role::Cognition).is_none());
    }

    /// Two of one rung should not happen; if it does, the answer must not depend on hash
    /// order. A caller that asks twice in one turn and gets two different sessions would
    /// post the request to one and then read the other's status.
    #[test]
    fn two_of_one_rung_resolve_to_the_same_session_every_time() {
        let r = reg();
        let (first, second) = (mint(), mint());
        r.register(second, Role::Cognition, None, "second".into());
        r.register(first, Role::Cognition, None, "first".into());

        let id = r.session_of_role(Role::Cognition).map(|s| s.id);
        assert_eq!(id, Some(first.min(second)));
        for _ in 0..8 {
            assert_eq!(r.session_of_role(Role::Cognition).map(|s| s.id), id);
        }
    }

    /// A worker is offered its owner and nothing else — which is also the only thing the
    /// routing rule would let it send to, so the list and the rule agree.
    #[test]
    fn a_worker_is_offered_only_its_owner() {
        let r = reg();
        let (owner, worker, other) = (mint(), mint(), mint());
        r.register(owner, Role::Cognition, None, String::new());
        r.register(other, Role::Reaction, None, String::new());
        r.register(worker, Role::Worker(WorkerType::General), Some(owner), String::new());

        let who = r.reachable(worker);
        assert_eq!(who.len(), 1, "{who:?}");
        assert_eq!(who[0].1, owner);
    }

    /// **Take-once is the discard rule.** The first session a rung opens in a run gets the
    /// previous thread; every later one — which is what a reopen after a failed turn is —
    /// gets nothing and opens cold. Without this a thread wedged badly enough to break a
    /// turn would be handed straight back to the session replacing it, and "turn it off and
    /// on again" would stop working.
    #[test]
    fn a_resumable_thread_is_handed_out_exactly_once() {
        let r = Registry::new();
        r.resumable
            .lock()
            .unwrap()
            .insert(Role::Cognition.as_str().to_string(), "th-1".to_string());

        assert_eq!(r.take_resumable(Role::Cognition).as_deref(), Some("th-1"));
        assert_eq!(r.take_resumable(Role::Cognition), None, "the second open is cold");
    }

    /// A rung with nothing seeded — a fresh install, or one whose last run predates thread
    /// recording — simply opens cold rather than erroring.
    #[test]
    fn an_unseeded_rung_has_nothing_to_resume() {
        assert_eq!(Registry::new().take_resumable(Role::Reaction), None);
    }

    /// The thread lands on the live entry, so the `closed` row written when the session ends
    /// carries it too — a rung that quit cleanly must be as resumable as one that crashed.
    #[test]
    fn noting_a_thread_puts_it_on_the_live_session() {
        let r = Registry::new();
        let id = mint();
        r.register(id, Role::Reaction, None, "the voice".into());
        r.note_thread(id, "th-voice");

        assert_eq!(
            r.sessions.lock().unwrap().get(&id).and_then(|e| e.thread.clone()).as_deref(),
            Some("th-voice"),
        );
    }

    #[test]
    fn an_empty_reach_renders_as_nothing_at_all() {
        assert_eq!(render_reachable(&[]), "", "a heading with nothing under it reads as a failure");
    }

    /// One renderer, because there is one mailbox. The three that preceded it had already
    /// drifted — different strings, different separators, one missing a trim.
    #[test]
    fn mail_renders_with_a_return_address_and_host_posts_without_one() {
        let batch = vec![
            Message { from: Some(7), text: "  did you see this?  ".into() },
            Message { from: None, text: "  a follow-up  ".into() },
        ];
        assert_eq!(render(&batch), "(from session 7) did you see this?\n\na follow-up");
    }

    /// The rule that keeps owners from being reaped out from under running work.
    #[test]
    fn an_owner_with_live_children_is_not_idle() {
        let r = reg();
        let (owner, child) = (mint(), mint());
        r.register(owner, Role::Cognition, None, String::new());
        assert!(!r.has_live_children(owner));

        r.register(child, Role::Worker(WorkerType::General), Some(owner), String::new());
        assert!(r.has_live_children(owner));
        assert_eq!(r.children(owner), vec![child]);

        r.unregister(child);
        assert!(!r.has_live_children(owner), "a closed child stops holding its owner open");
    }

    #[test]
    fn status_carries_meta_and_never_content() {
        let r = reg();
        let (a, b) = (mint(), mint());
        r.register(a, Role::Cognition, None, String::new());
        r.register(b, Role::Worker(WorkerType::General), Some(a), "file the receipts".into());

        let s = r.status(b).expect("registered");
        assert_eq!(s.role, Role::Worker(WorkerType::General));
        assert_eq!(s.owner, Some(a));
        assert_eq!(s.task, "file the receipts");
        assert!(!s.busy && !s.queued && s.turns == 0);

        r.send(a, b, "go".into());
        assert!(r.status(b).unwrap().queued);

        r.take_pending(b);
        let s = r.status(b).unwrap();
        assert!(s.busy && !s.queued && s.turns == 1);

        r.finish_turn(b);
        assert!(!r.status(b).unwrap().busy);
    }

    /// Every transition moves `state_since`, and the ones that are *not* transitions leave
    /// it alone. The field is only worth having if all of them stamp: one path that flips
    /// `busy` without moving the clock reports a turn as older than it is, on exactly the
    /// path that skipped it.
    #[test]
    fn every_state_change_moves_its_clock_and_nothing_else_does() {
        let r = reg();
        let id = mint();
        r.register(id, Role::Worker(WorkerType::General), None, String::new());
        let registered = r.status(id).unwrap();
        assert_eq!(registered.state_since, registered.started, "idle since it existed");

        // idle → waiting
        r.post(id, "go".into());
        let waiting = r.status(id).unwrap();
        assert!(waiting.queued && !waiting.busy);
        assert!(waiting.state_since > registered.state_since);

        // A second letter onto an already-queued inbox is not a state change.
        r.post(id, "and also".into());
        assert_eq!(r.status(id).unwrap().state_since, waiting.state_since, "still waiting");

        // waiting → running
        r.take_pending(id);
        let running = r.status(id).unwrap();
        assert!(running.busy);
        assert!(running.state_since > waiting.state_since);

        // Mail landing mid-turn leaves it running, so the clock holds.
        r.post(id, "one more".into());
        assert_eq!(r.status(id).unwrap().state_since, running.state_since, "still running");

        // running → idle
        r.finish_turn(id);
        let done = r.status(id).unwrap();
        assert!(!done.busy);
        assert!(done.state_since > running.state_since);

        // A second finish is not a transition.
        r.finish_turn(id);
        assert_eq!(r.status(id).unwrap().state_since, done.state_since);
    }

    /// `doing` without an age says a session is alive and nothing more — the line reads the
    /// same four minutes in and forty minutes in.
    #[test]
    fn doing_carries_when_it_was_last_seen() {
        let r = reg();
        let id = mint();
        r.register(id, Role::Worker(WorkerType::General), None, String::new());
        assert!(r.status(id).unwrap().doing_at.is_none(), "nothing done, no clock");

        r.record_activity(id, "$ cargo test");
        let first = r.status(id).unwrap().doing_at.expect("stamped");

        r.record_activity(id, "hi-agent/send_message");
        let second = r.status(id).unwrap();
        assert_eq!(second.doing.as_deref(), Some("hi-agent/send_message"));
        assert!(second.doing_at.unwrap() >= first, "replaced, so re-stamped");

        // A blank line is not activity and must not refresh the clock — a session that has
        // gone quiet would otherwise look busy forever.
        r.record_activity(id, "   ");
        assert_eq!(r.status(id).unwrap().doing_at, second.doing_at);
    }

    #[test]
    fn a_prewarmed_session_can_replace_its_placeholder_task() {
        let r = reg();
        let id = mint();
        r.register(id, Role::Cognition, None, "waiting for the first question".into());

        r.set_task(id, "review the restart behavior".into());

        assert_eq!(
            r.status(id).expect("registered").task,
            "review the restart behavior"
        );
    }

    #[test]
    fn output_is_a_bounded_tail_not_an_archive() {
        let r = reg();
        let a = mint();
        r.register(a, Role::Worker(WorkerType::General), None, String::new());
        r.record_output(a, "hello ");
        r.record_output(a, "world");
        assert_eq!(r.messages(a).as_deref(), Some("hello world"));

        r.record_output(a, &"x".repeat(OUTPUT_TAIL_CHARS + 500));
        let kept = r.messages(a).unwrap();
        assert_eq!(kept.chars().count(), OUTPUT_TAIL_CHARS, "the tail is capped");
        assert!(kept.ends_with('x'), "it is the *recent* tail that survives");
    }

    #[test]
    fn ids_are_unique_process_wide() {
        let ids: Vec<SessionId> = (0..50).map(|_| mint()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
    }
}
