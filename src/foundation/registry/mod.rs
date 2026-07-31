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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    Session(SessionId),
    Scene(Scene),
}

/// What happened to a message — **delivery, never a response.** `send` does not wait for
/// the target to read, act, or agree; it reports whether the message reached a mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// A session's inbox: one pending message, merged rather than queued.
///
/// Several messages landing while a session is mid-turn are **concatenated**, so it picks
/// all of them up in one prompt rather than running each as its own round-trip. No
/// LLM-smart merge — the receiving model reads the combined text.
#[derive(Default)]
struct Inbox {
    pending: Option<String>,
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
    /// Woken when something lands, so an idle session picks it up without polling.
    notify: std::sync::Arc<Notify>,
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
        let mut map = self.sessions.lock().unwrap();

        let target = match to {
            Address::Session(id) => *id,
            Address::Scene(scene) => {
                let found = map.iter().find(|(_, e)| {
                    e.role == Role::Reaction && e.scene.as_ref() == Some(scene)
                });
                match found {
                    Some((id, _)) => *id,
                    None => return Delivery::Unknown,
                }
            }
        };

        // A worker answers to whoever asked, and to nobody else.
        if let Some(sender) = map.get(&from)
            && sender.role == Role::Worker
            && sender.owner != Some(target)
        {
            return Delivery::NotPermitted;
        }

        let Some(entry) = map.get_mut(&target) else {
            return Delivery::Unknown;
        };
        if entry.inbox.closed {
            return Delivery::Unknown;
        }
        entry.inbox.pending = Some(match entry.inbox.pending.take() {
            Some(prev) => format!("{prev}\n\n{message}"),
            None => message,
        });
        entry.notify.notify_one();
        Delivery::Delivered
    }

    /// Take everything queued for `id`, if anything is. Marks the session busy — it is
    /// about to take a turn, and an agent with a turn in flight is not idle.
    pub fn take_pending(&self, id: SessionId) -> Option<String> {
        let mut map = self.sessions.lock().unwrap();
        let entry = map.get_mut(&id)?;
        let text = entry.inbox.pending.take()?;
        entry.busy = true;
        entry.turns += 1;
        Some(text)
    }

    /// Mark a turn finished.
    pub fn finish_turn(&self, id: SessionId) {
        if let Some(e) = self.sessions.lock().unwrap().get_mut(&id) {
            e.busy = false;
        }
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
            queued: e.inbox.pending.is_some(),
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
        assert_eq!(r.take_pending(b).as_deref(), Some("go"));
        assert_eq!(r.take_pending(b), None, "taking drains the inbox");
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
        assert_eq!(r.take_pending(b).as_deref(), Some("first\n\nsecond"));
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
        assert_eq!(r.take_pending(rx).as_deref(), Some("news"));
        assert_eq!(r.take_pending(dl), None, "the scene's address is its voice");
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
    fn ids_are_unique_process_wide() {
        let ids: Vec<SessionId> = (0..50).map(|_| mint()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
    }
}
