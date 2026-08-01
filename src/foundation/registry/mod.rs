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
//! Nothing in this module talks to ACP or to a model. It owns addresses, mailboxes and
//! metadata; who drains a mailbox and what they do with it belongs to the caller.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use tokio::sync::Notify;

use crate::types::Scene;

/// Handle for one agent session, unique process-wide.
///
/// It names a *session*, not a role: a role has many sessions over a run, and two scenes'
/// Deliberations are two sessions of one role. Process-wide rather than per-scene because
/// ownership crosses scenes — a sceneless owner holds sessions no per-scene counter could
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

/// Who a message is for.
///
/// A **session id** names a live agent and dies with the process, so nothing durable may
/// hold one — a task holds a scene. A **scene** names a conversation and is stable; it
/// resolves to that scene's Reaction, because a scene is where a person is spoken to and
/// Reaction is the only thing that speaks there.
///
/// A **rung** names one of the sceneless agents. It exists because the other two forms
/// cannot reach one: a sceneless rung has no scene by definition, and its session id is
/// minted fresh every boot, so nothing durable and nothing in a prompt can hold it. That
/// left the design's own "Deliberation hands up to Cognition" describable and impossible.
/// There is at most one live session per sceneless role, which is what makes the role a
/// sufficient address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    Session(SessionId),
    Scene(Scene),
    Rung(Role),
}

/// The one rung reachable by name today. See [`Address::parse`] for why the list is not
/// simply "every sceneless role".
const ADDRESSABLE_RUNGS: [(&str, Role); 1] = [("cognition", Role::Cognition)];

impl Address {
    /// Parse an agent-facing address string — what an agent typed into `send_message`.
    ///
    /// Lives here rather than at the tool surface because this is routing, and routing
    /// lives in one place; the host also reaches the registry by paths that never touch
    /// MCP, and they must resolve a name the same way.
    ///
    /// **Reflection is deliberately not addressable**, though it is registered sceneless
    /// and would resolve. Its pass is one `prompt` then `wait`, so nothing drains its
    /// inbox: a send would report `Delivered` and the message would die with the pass.
    /// A verb that reports success and delivers nothing is the exact failure `9e6ae45`
    /// was written to remove. It joins this list when it has a reader.
    ///
    /// The cost, stated: a scene literally named `cognition` cannot be addressed. Same
    /// shape as [`crate::mind::memory::layout::SESSIONS_DIR`] — a reserved word in a
    /// namespace people also use — and the same answer: name it, own it, move on.
    pub fn parse(to: &str) -> Address {
        let to = to.trim();
        if let Ok(id) = to.parse::<SessionId>() {
            return Address::Session(id);
        }
        for (name, role) in ADDRESSABLE_RUNGS {
            if to.eq_ignore_ascii_case(name) {
                return Address::Rung(role);
            }
        }
        Address::Scene(Scene(to.to_string()))
    }
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
    /// The scene this belongs to, if any. Cognition and Reflection have none.
    pub scene: Option<Scene>,
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
    /// agent. A follow-up the scene loop hands down is not a message from a colleague,
    /// and rendering it with a return address would put a second voice in a room that
    /// has only one.
    pub from: Option<SessionId>,
    pub text: String,
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
    scene: Option<Scene>,
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
/// The scene loop leaves by several paths — inbound closed, closed mid-settle, shutdown
/// — and a registration released at only some of them is how a scene ends up with more
/// than one voice, `Address::Scene` resolving to an arbitrary dead one. Rather than
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
    scene: Option<Scene>,
    owner: Option<SessionId>,
    task: String,
) -> Registration {
    let mail = global().register(id, role, scene, owner, task);
    Registration { id, mail }
}

/// The switchboard. One per process.
#[derive(Default)]
pub struct Registry {
    sessions: Mutex<HashMap<SessionId, Entry>>,
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
        scene: Option<Scene>,
        owner: Option<SessionId>,
        task: String,
    ) -> std::sync::Arc<Notify> {
        let notify = std::sync::Arc::new(Notify::new());
        let mut map = self.sessions.lock().unwrap();
        map.insert(
            id,
            Entry {
                role,
                scene,
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
        notify
    }

    /// Drop a session. Anything still in its inbox goes with it — undelivered is the
    /// honest outcome, and the sender was told `Delivered` about a mailbox, never about
    /// an outcome.
    pub fn unregister(&self, id: SessionId) {
        if let Some(mut e) = self.sessions.lock().unwrap().remove(&id) {
            e.inbox.closed = true;
        }
    }

    /// Send `message` to `to`, from `from`.
    ///
    /// **`from` is supplied by the host, not by the calling agent.** The host knows who is
    /// calling; letting an agent name itself is letting it impersonate another.
    ///
    /// One direction, no reply. The return value says whether it reached a mailbox — a
    /// reply, if there is one, arrives later as its own `send` in the other direction.
    pub fn send(&self, from: SessionId, to: &Address, message: String) -> Delivery {
        self.send_traced(from, to, message).0
    }

    /// [`send`](Self::send), also reporting **which session** it resolved to.
    ///
    /// For anything that wants to *observe* an edge rather than travel it. An
    /// `Address::Scene` names a conversation, and resolving it to the session that
    /// actually received the message happens under this lock — so a caller cannot ask
    /// afterwards without racing, and cannot name the target from the address alone.
    ///
    /// The observatory could not simply be called from in here: `record` is async and
    /// this holds a `std::sync::Mutex`. Handing the resolved id back instead keeps the
    /// switchboard synchronous and free of any handle to a debug surface, which is
    /// what lets it stay the one thing that "cannot be slow, confused, or dead".
    ///
    /// `None` accompanies every non-`Delivered` outcome, and also `NotPermitted` —
    /// there was a target, but naming it would leak an address the sender may not have.
    pub fn send_traced(
        &self,
        from: SessionId,
        to: &Address,
        message: String,
    ) -> (Delivery, Option<SessionId>) {
        let mut map = self.sessions.lock().unwrap();

        let target = match to {
            Address::Session(id) => *id,
            Address::Scene(scene) => {
                let found = map.iter().find(|(_, e)| {
                    e.role == Role::Reaction && e.scene.as_ref() == Some(scene)
                });
                match found {
                    Some((id, _)) => *id,
                    None => return (Delivery::Unknown, None),
                }
            }
            // `scene.is_none()` is the whole discriminator, and it is not redundant with
            // the role: a role alone would also match the per-scene rungs if one were
            // ever added to the list. Sceneless is what makes there be exactly one.
            Address::Rung(role) => {
                let found =
                    map.iter().find(|(_, e)| e.role == *role && e.scene.is_none());
                match found {
                    Some((id, _)) => *id,
                    None => return (Delivery::Unknown, None),
                }
            }
        };

        // A worker answers to whoever asked, and to nobody else.
        if let Some(sender) = map.get(&from)
            && sender.role == Role::Worker
            && sender.owner != Some(target)
        {
            return (Delivery::NotPermitted, None);
        }

        let Some(entry) = map.get_mut(&target) else {
            return (Delivery::Unknown, None);
        };
        if entry.inbox.closed {
            return (Delivery::Unknown, None);
        }
        entry.inbox.pending.push(Message { from: Some(from), text: message });
        entry.notify.notify_one();
        (Delivery::Delivered, Some(target))
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
    }

    /// Take everything queued for `id`, if anything is. Marks the session busy — it is
    /// about to take a turn, and an agent with a turn in flight is not idle.
    pub fn take_pending(&self, id: SessionId) -> Option<Vec<Message>> {
        let mut map = self.sessions.lock().unwrap();
        let entry = map.get_mut(&id)?;
        if entry.inbox.pending.is_empty() {
            return None;
        }
        entry.busy = true;
        entry.turns += 1;
        Some(std::mem::take(&mut entry.inbox.pending))
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
        let mut map = self.sessions.lock().unwrap();
        let Some(entry) = map.get_mut(&id) else {
            return None;
        };
        if entry.inbox.pending.is_empty() {
            entry.inbox.closed = true;
            return None;
        }
        entry.busy = true;
        entry.turns += 1;
        Some(std::mem::take(&mut entry.inbox.pending))
    }

    /// The handle woken when mail lands for `id`, for a loop that wants to wait on its
    /// own inbox without polling. Same `Notify` [`register`](Self::register) returned.
    pub fn notifier(&self, id: SessionId) -> Option<std::sync::Arc<Notify>> {
        self.sessions.lock().unwrap().get(&id).map(|e| e.notify.clone())
    }

    /// Mark a turn finished.
    pub fn finish_turn(&self, id: SessionId) {
        if let Some(e) = self.sessions.lock().unwrap().get_mut(&id) {
            e.busy = false;
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
            scene: e.scene.clone(),
            owner: e.owner,
            task: e.task.clone(),
            busy: e.busy,
            queued: !e.inbox.pending.is_empty(),
            turns: e.turns,
            started: e.started,
        })
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
        r.register(a, Role::Cognition, None, None, "thinking".into());
        r.register(b, Role::Worker, None, Some(a), "the errand".into());

        assert_eq!(r.send(a, &Address::Session(b), "go".into()), Delivery::Delivered);
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
        r.register(a, Role::Cognition, None, None, String::new());
        r.register(b, Role::Worker, None, Some(a), String::new());

        r.send(a, &Address::Session(b), "first".into());
        r.send(a, &Address::Session(b), "second".into());
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
        r.register(a, Role::Cognition, None, None, String::new());
        r.register(b, Role::Worker, None, Some(a), String::new());

        // Mail present: it is taken, and the inbox stays open for more.
        r.send(a, &Address::Session(b), "one more thing".into());
        let mail = r.take_pending_or_close(b).expect("mail wins over the close");
        assert_eq!(mail[0].text, "one more thing");
        assert_eq!(
            r.send(a, &Address::Session(b), "and another".into()),
            Delivery::Delivered,
            "taking mail must not close the mailbox"
        );

        // Drain, then find it empty: now it closes, and later sends are told so.
        r.take_pending(b);
        assert!(r.take_pending_or_close(b).is_none());
        assert_eq!(
            r.send(a, &Address::Session(b), "too late".into()),
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
        r.register(owner, Role::Deliberation, None, None, String::new());
        r.register(w, Role::Worker, None, Some(owner), String::new());

        // A worker may not address itself as an agent — that is not its owner.
        assert_eq!(r.send(w, &Address::Session(w), "self".into()), Delivery::NotPermitted);
        // The host posting the same follow-up is fine, and arrives anonymous.
        assert_eq!(r.post(w, "keep going".into()), Delivery::Delivered);
        let mail = r.take_pending(w).expect("posted");
        assert_eq!(mail[0].from, None);
        assert_eq!(mail[0].text, "keep going");

        r.unregister(w);
        assert_eq!(r.post(w, "too late".into()), Delivery::Unknown);
    }

    /// The bug this exists to make impossible: a scene loop leaves by several paths, and
    /// a registration released at only some of them leaves a second voice behind for the
    /// same scene — after which `Address::Scene` resolves to whichever the map hands back
    /// first, which may be the dead one.
    #[test]
    fn a_scoped_registration_ends_with_its_scope() {
        let scene = Scene("boss".into());
        let sender = mint();
        global().register(sender, Role::Cognition, None, None, String::new());

        let id = {
            let voice =
                register_scoped(mint(), Role::Reaction, Some(scene.clone()), None, String::new());
            let id = voice.id();
            assert_eq!(
                global().send(sender, &Address::Scene(scene.clone()), "hi".into()),
                Delivery::Delivered
            );
            id
        };

        assert!(global().status(id).is_none(), "leaving the scope closed the registration");
        assert_eq!(
            global().send(sender, &Address::Scene(scene), "hi again".into()),
            Delivery::Unknown,
            "no stale voice is left answering for the scene"
        );
        global().unregister(sender);
    }

    #[test]
    fn a_notifier_is_reachable_after_registration() {
        let r = reg();
        let a = mint();
        r.register(a, Role::Reaction, Some(Scene("boss".into())), None, String::new());
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
        r.register(a, Role::Cognition, None, None, String::new());
        assert_eq!(r.send(a, &Address::Session(9_999), "hello".into()), Delivery::Unknown);

        let gone = mint();
        r.register(gone, Role::Worker, None, Some(a), String::new());
        r.unregister(gone);
        assert_eq!(r.send(a, &Address::Session(gone), "hello".into()), Delivery::Unknown);
    }

    /// Routing, not policy: a worker answers whoever asked and cannot reach past them —
    /// not a sibling, and not the conversation.
    #[test]
    fn a_worker_may_address_only_its_owner() {
        let r = reg();
        let (owner, other, worker) = (mint(), mint(), mint());
        r.register(owner, Role::Cognition, None, None, String::new());
        r.register(other, Role::Reaction, Some(Scene("boss".into())), None, String::new());
        r.register(worker, Role::Worker, None, Some(owner), String::new());

        assert_eq!(r.send(worker, &Address::Session(owner), "done".into()), Delivery::Delivered);
        assert_eq!(
            r.send(worker, &Address::Session(other), "psst".into()),
            Delivery::NotPermitted
        );
        assert_eq!(
            r.send(worker, &Address::Scene(Scene("boss".into())), "psst".into()),
            Delivery::NotPermitted,
            "a scene is where a person is spoken to; a worker has no business there"
        );
    }

    /// Everything above a worker may address a scene, and it lands on the one thing that
    /// speaks there.
    #[test]
    fn a_scene_resolves_to_its_reaction() {
        let r = reg();
        let (cog, rx, dl) = (mint(), mint(), mint());
        let scene = Scene("boss".into());
        r.register(cog, Role::Cognition, None, None, String::new());
        r.register(rx, Role::Reaction, Some(scene.clone()), None, String::new());
        r.register(dl, Role::Deliberation, Some(scene.clone()), None, String::new());

        assert_eq!(r.send(cog, &Address::Scene(scene), "news".into()), Delivery::Delivered);
        assert_eq!(r.take_pending(rx).expect("delivered")[0].text, "news");
        assert!(r.take_pending(dl).is_none(), "the scene's address is its voice");
    }

    /// The gap that made "Deliberation hands up to Cognition" describable and impossible:
    /// a sceneless rung has no scene, and its id is minted fresh each boot, so neither of
    /// the other two address forms can reach it.
    #[test]
    fn a_rung_resolves_to_the_sceneless_session_of_that_role() {
        let r = reg();
        let (dl, cog) = (mint(), mint());
        let scene = Scene("boss".into());
        r.register(dl, Role::Deliberation, Some(scene), None, String::new());
        r.register(cog, Role::Cognition, None, None, "thinking".into());

        assert_eq!(
            r.send(dl, &Address::Rung(Role::Cognition), "a real errand".into()),
            Delivery::Delivered
        );
        assert_eq!(r.take_pending(cog).expect("delivered")[0].text, "a real errand");
    }

    /// Nothing live at that rung reads as `Unknown`, the same as an absent scene — the
    /// sender is told, rather than the message vanishing into a name that looks routable.
    #[test]
    fn a_rung_with_no_live_session_is_unknown() {
        let r = reg();
        let a = mint();
        r.register(a, Role::Deliberation, Some(Scene("boss".into())), None, String::new());
        assert_eq!(
            r.send(a, &Address::Rung(Role::Cognition), "anyone?".into()),
            Delivery::Unknown
        );
    }

    /// A per-scene session must never answer a rung address, or "one sceneless brain"
    /// would silently become "whichever Deliberation the map yielded first".
    #[test]
    fn a_scened_session_never_answers_a_rung_address() {
        let r = reg();
        let (a, scened) = (mint(), mint());
        r.register(a, Role::Reaction, Some(Scene("boss".into())), None, String::new());
        // Same role, but it has a scene — so it is not the sceneless one.
        r.register(scened, Role::Cognition, Some(Scene("boss".into())), None, String::new());
        assert_eq!(
            r.send(a, &Address::Rung(Role::Cognition), "hi".into()),
            Delivery::Unknown
        );
    }

    #[test]
    fn parse_reads_ids_rungs_and_scenes() {
        assert_eq!(Address::parse("42"), Address::Session(42));
        assert_eq!(Address::parse(" 42 "), Address::Session(42));
        assert_eq!(Address::parse("cognition"), Address::Rung(Role::Cognition));
        assert_eq!(Address::parse("Cognition"), Address::Rung(Role::Cognition));
        assert_eq!(Address::parse("boss"), Address::Scene(Scene("boss".into())));
        assert_eq!(
            Address::parse("alice@phone"),
            Address::Scene(Scene("alice@phone".into()))
        );
    }

    /// Reflection is registered sceneless and *would* resolve — but its pass is one
    /// `prompt` then `wait`, so nothing drains its inbox. Naming it would make
    /// `send_message` answer "delivered" and drop the message, which is the write-only
    /// verb `9e6ae45` removed. It becomes addressable when it has a reader, not before.
    #[test]
    fn reflection_is_not_addressable_while_nothing_drains_it() {
        assert_eq!(
            Address::parse("reflection"),
            Address::Scene(Scene("reflection".into())),
            "a scene by that name at worst; never a rung"
        );
    }

    #[test]
    fn an_unknown_scene_is_unknown_not_a_panic() {
        let r = reg();
        let a = mint();
        r.register(a, Role::Cognition, None, None, String::new());
        assert_eq!(
            r.send(a, &Address::Scene(Scene("nobody-here".into())), "hi".into()),
            Delivery::Unknown
        );
    }

    /// The rule that keeps owners from being reaped out from under running work.
    #[test]
    fn an_owner_with_live_children_is_not_idle() {
        let r = reg();
        let (owner, child) = (mint(), mint());
        r.register(owner, Role::Deliberation, None, None, String::new());
        assert!(!r.has_live_children(owner));

        r.register(child, Role::Worker, None, Some(owner), String::new());
        assert!(r.has_live_children(owner));
        assert_eq!(r.children(owner), vec![child]);

        r.unregister(child);
        assert!(!r.has_live_children(owner), "a closed child stops holding its owner open");
    }

    #[test]
    fn status_carries_meta_and_never_content() {
        let r = reg();
        let (a, b) = (mint(), mint());
        r.register(a, Role::Cognition, None, None, String::new());
        r.register(b, Role::Worker, Some(Scene("boss".into())), Some(a), "file the receipts".into());

        let s = r.status(b).expect("registered");
        assert_eq!(s.role, Role::Worker);
        assert_eq!(s.owner, Some(a));
        assert_eq!(s.task, "file the receipts");
        assert!(!s.busy && !s.queued && s.turns == 0);

        r.send(a, &Address::Session(b), "go".into());
        assert!(r.status(b).unwrap().queued);

        r.take_pending(b);
        let s = r.status(b).unwrap();
        assert!(s.busy && !s.queued && s.turns == 1);

        r.finish_turn(b);
        assert!(!r.status(b).unwrap().busy);
    }

    #[test]
    fn output_is_a_bounded_tail_not_an_archive() {
        let r = reg();
        let a = mint();
        r.register(a, Role::Worker, None, None, String::new());
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
