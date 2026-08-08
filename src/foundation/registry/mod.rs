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
use tokio::sync::{watch, Notify};


/// Handle for one agent session, unique process-wide.
///
/// It names a *session*, not a role: a role has many sessions over a run, and a
/// Deliberation replaced after a failure is a second session of one role. One namespace
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

/// Which rung a session is. Only [`Role::Worker`] is restricted here; the rest differ by
/// prompt and tool surface, which are not this module's business.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Reaction,
    Deliberation,
    Cognition,
    Reflection,
    Worker,
}

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
    inbox: Inbox,
    /// Bounded tail of what this session has said, for `SessionMessages`.
    output: String,
    /// Woken when something lands, so an idle session picks it up without polling.
    notify: std::sync::Arc<Notify>,
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
}

impl Default for Registry {
    fn default() -> Self {
        let (activity, _) = watch::channel(0);
        Self {
            sessions: Mutex::new(HashMap::new()),
            activity,
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
        {
            let mut map = self.sessions.lock().unwrap();
            map.insert(
                id,
                Entry {
                    role,
                    owner,
                    task,
                    busy: false,
                    turns: 0,
                    started: Utc::now(),
                    inbox: Inbox::default(),
                    output: String::new(),
                    notify: notify.clone(),
                },
            );
        }
        self.note_activity();
        notify
    }

    /// Drop a session. Anything still in its inbox goes with it — undelivered is the
    /// honest outcome, and the sender was told `Delivered` about a mailbox, never about
    /// an outcome.
    pub fn unregister(&self, id: SessionId) {
        let removed = if let Some(mut e) = self.sessions.lock().unwrap().remove(&id) {
            e.inbox.closed = true;
            true
        } else {
            false
        };
        if removed {
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
                && sender.role == Role::Worker
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
            Role::Worker => {
                if let Some(owner) = me.owner {
                    out.push(("the session that asked for this work".to_string(), owner));
                }
            }
            // The voice's rungs hand work up, and that is all they address.
            Role::Reaction | Role::Deliberation => {
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
    /// Deliberation is registered and warmed before it receives its first real task,
    /// so the switchboard entry must move from its startup placeholder to the work the
    /// the voice actually handed down.
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

    /// What a session has recently said. Costs context — which is exactly why it is a
    /// different call from [`status`](Self::status).
    pub fn messages(&self, id: SessionId) -> Option<String> {
        let map = self.sessions.lock().unwrap();
        map.get(&id).map(|e| e.output.clone())
    }

    /// Metadata for one session. Cheap by construction — no content crosses.
    pub fn status(&self, id: SessionId) -> Option<Status> {
        let map = self.sessions.lock().unwrap();
        let e = map.get(&id)?;
        Some(Status {
            id,
            role: e.role,
            owner: e.owner,
            task: e.task.clone(),
            busy: e.busy,
            queued: !e.inbox.pending.is_empty(),
            turns: e.turns,
            started: e.started,
        })
    }

    /// Metadata for every live session, ordered by id.
    pub fn statuses(&self) -> Vec<Status> {
        let map = self.sessions.lock().unwrap();
        let mut rows: Vec<Status> = map
            .iter()
            .map(|(&id, e)| Status {
                id,
                role: e.role,
                owner: e.owner,
                task: e.task.clone(),
                busy: e.busy,
                queued: !e.inbox.pending.is_empty(),
                turns: e.turns,
                started: e.started,
            })
            .collect();
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

    fn reg() -> Registry {
        Registry::new()
    }

    #[test]
    fn a_message_reaches_the_target_inbox() {
        let r = reg();
        let (a, b) = (mint(), mint());
        r.register(a, Role::Cognition, None, "thinking".into());
        r.register(b, Role::Worker, Some(a), "the errand".into());

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
        r.register(b, Role::Worker, Some(a), String::new());

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
        r.register(b, Role::Worker, Some(a), String::new());

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
        r.register(owner, Role::Deliberation, None, String::new());
        r.register(w, Role::Worker, Some(owner), String::new());

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
        r.register(gone, Role::Worker, Some(a), String::new());
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
        r.register(worker, Role::Worker, Some(owner), String::new());

        assert_eq!(r.send(worker, owner, "done".into()), Delivery::Delivered);
        assert_eq!(
            r.send(worker, other, "psst".into()),
            Delivery::NotPermitted
        );
    }

    /// The projection that replaced name-a-destination addressing. The voice's rungs are
    /// offered the shared
    /// brain and nothing else, because handing work up is the only edge it has.
    #[test]
    fn the_voices_rungs_are_offered_the_shared_brain() {
        let r = reg();
        let (dl, cog) = (mint(), mint());
        r.register(dl, Role::Deliberation, None, String::new());
        r.register(cog, Role::Cognition, None, "thinking".into());

        let who = r.reachable(dl);
        assert_eq!(who.len(), 1, "{who:?}");
        assert_eq!(who[0].1, cog);
        assert!(who[0].0.contains("shared brain"), "{who:?}");

        // And the id it was handed is one it can actually send to.
        assert_eq!(r.send(dl, who[0].1, "a real errand".into()), Delivery::Delivered);
        assert_eq!(r.take_pending(cog).expect("delivered")[0].text, "a real errand");
    }

    /// A cold rung is simply absent from the list, which is the point: the asker learns
    /// there is nobody there *before* sending, instead of guessing a name and being told
    /// `Unknown` after the fact.
    #[test]
    fn a_rung_that_is_not_up_is_not_offered() {
        let r = reg();
        let dl = mint();
        r.register(dl, Role::Deliberation, None, String::new());
        assert!(r.reachable(dl).is_empty());
    }

    /// Cognition is offered the live voice, because that is the one way anything it
    /// works out reaches the person.
    #[test]
    fn cognition_is_offered_the_voice_and_its_own_workers() {
        let r = reg();
        let (cog, rx, dl, w) = (mint(), mint(), mint(), mint());
        r.register(cog, Role::Cognition, None, "thinking".into());
        r.register(rx, Role::Reaction, None, String::new());
        r.register(dl, Role::Deliberation, None, String::new());
        r.register(w, Role::Worker, Some(cog), "file the receipts".into());

        let who = r.reachable(cog);
        let ids: Vec<SessionId> = who.iter().map(|(_, id)| *id).collect();
        assert!(ids.contains(&rx), "the voice: {who:?}");
        assert!(ids.contains(&w), "its own worker: {who:?}");
        assert!(!ids.contains(&dl), "deliberation is reached through the voice: {who:?}");
    }

    /// A worker is offered its owner and nothing else — which is also the only thing the
    /// routing rule would let it send to, so the list and the rule agree.
    #[test]
    fn a_worker_is_offered_only_its_owner() {
        let r = reg();
        let (owner, worker, other) = (mint(), mint(), mint());
        r.register(owner, Role::Cognition, None, String::new());
        r.register(other, Role::Reaction, None, String::new());
        r.register(worker, Role::Worker, Some(owner), String::new());

        let who = r.reachable(worker);
        assert_eq!(who.len(), 1, "{who:?}");
        assert_eq!(who[0].1, owner);
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
        r.register(owner, Role::Deliberation, None, String::new());
        assert!(!r.has_live_children(owner));

        r.register(child, Role::Worker, Some(owner), String::new());
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
        r.register(b, Role::Worker, Some(a), "file the receipts".into());

        let s = r.status(b).expect("registered");
        assert_eq!(s.role, Role::Worker);
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

    #[test]
    fn a_prewarmed_session_can_replace_its_placeholder_task() {
        let r = reg();
        let id = mint();
        r.register(
            id,
            Role::Deliberation,
            None,
            "waiting for the first question".into(),
        );

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
        r.register(a, Role::Worker, None, String::new());
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
