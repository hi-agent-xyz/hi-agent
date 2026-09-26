//! The bridge between the MCP tool server and the reaction loop.
//!
//! The mind (and its workers) express side-effects as MCP tool calls over the
//! `/mcp` HTTP endpoint (see [`crate::foundation::mcp`]). Those calls arrive on a different
//! task than the reaction loop, so they cannot touch the loop's private state
//! directly. Instead each owning loop registers a [`ToolSink`] — a control-channel
//! sender — into the shared [`ToolRegistry`]. The MCP handler takes the sink for
//! the caller's role from there and forwards a [`LoopControl`]
//! the loop applies on its own turn, so the worker registry stays owned by the
//! loop with no locking.

use std::sync::Arc;


use tokio::sync::{Mutex, mpsc};
// The loop's clock, so a deadline set here can be handed straight to `sleep_until`
// there. `tokio::time::Instant` is also what a paused test clock advances.
use tokio::time::Instant;

use crate::foundation::registry::SessionSlug;

/// The size of one message: room for one matter said whole — a sentence, a paragraph, or
/// a conclusion and a few short paragraphs under it (`docs/arch/legibility.md`). Past that
/// it is a document, which belongs on screen or in a file. It was 240, sized for "three
/// short messages rather than one long one", and that shape split one matter across
/// several bubbles. 400 is a starting value, not a measurement.
pub(super) const SAY_MAX_CHARS: usize = 400;

/// One command the MCP tool server routes to the reaction loop.
///
/// Once there were four: two for dispatching work and two for a worker to reach
/// Reaction. The reaching ones are gone — a worker addresses its owner with the one verb
/// now, through the switchboard, which needs no channel of its own.
/// `Alarm` went with it: nothing in the host fires at a named time
/// ([`glancing up`](../../../docs/arch/host.md)), and the agent arranges its own
/// timing with the shell it already has. What is left is one variant, and it is
/// here because the loop owns the state it touches.
#[derive(Debug)]
pub enum LoopControl {
    /// Start a working session for `task` (the `create_worker` tool), owned by the
    /// session that asked.
    ///
    /// Creating a worker is the caller's decision but the loop's bookkeeping — the
    /// live-session map is the loop's own state, so this crosses on the control
    /// channel like everything else that touches it. `owner` is who the finished work
    /// answers to; a worker belongs to the session that created it, never to Reaction
    /// it happens to run in.
    ///
    /// `resume` is a codex thread the new session should pick back up instead of starting
    /// cold — an errand the last restart killed, named by the boot glance that offered it.
    /// It is a fresh session either way: a new id, a new registration, and the same prompt.
    /// Only where its mind starts differs.
    ///
    /// `subject` is the ledger task this errand serves, and it is what makes "is anyone on
    /// this task" a lookup instead of a reading. It rides the creation call because that is
    /// the only moment anyone knows the answer: the rung asking for the work is the ledger's
    /// writer, and by the time the session is running, the association exists nowhere else.
    ///
    /// `title` and `task` are the same errand at two lengths, and both travel because they
    /// land in different places: the title is what the switchboard registers and every
    /// reader of a roster sees ([`crate::foundation::registry::Status::title`]); the task is
    /// the brief that becomes the session's first prompt. Deriving either from the other is
    /// the thing this pair exists to stop.
    ///
    /// **It carries a `ready` reply now, and that is what makes the answer true.** This was
    /// the one dispatch verb that reported success from the *send* rather than from the
    /// deed: the tool queued the message, said `session <id> starting`, and returned — so
    /// for as long as the loop had not picked it up, every follow-up verb about that id
    /// (`hi_session_status`, `hi_send_message`, `hi_cancel_worker`, `hi_close_worker`) was
    /// asked about a session the switchboard had never heard of, and each answered
    /// confidently that there was nothing there. Observed 2026-08-17 in one reflection
    /// turn: create at 08:52:42, three "no live session" answers, `hi_close_worker` at
    /// 08:55:34 replying *"was already gone — nothing to close"*, and the session actually
    /// spawning at 08:55:45 — after which nobody could ever close it, because its owner had
    /// been told it was gone. A create that has not registered yet is indistinguishable
    /// from a create that never happened, so the caller has to wait for the difference.
    CreateWorker {
        id: SessionSlug,
        title: String,
        /// The brief, and `None` for the one caller that has none: the boot pass reopening a
        /// session that was *waiting* when the host stopped ([`super::reopen_interrupted`]).
        /// It was parked on its owner's next instruction and still is, so it comes back and
        /// goes straight back to waiting — a turn handed to it would be one spent on nothing.
        task: Option<String>,
        kind: crate::identity::WorkerType,
        /// Registered MCP servers this errand needs, by skill name. Empty for most work.
        /// The dispatching rung chooses, because it is the one that knows what the job is
        /// — see [`crate::foundation::agent`]'s thread config for why that is the whole of
        /// "which sessions get which tools".
        servers: Vec<String>,
        owner: Option<SessionSlug>,
        resume: Option<String>,
        subject: Option<String>,
        /// Whether this errand is for a step nobody has asked for yet — `agents.md`'s
        /// *Working ahead*. Carried for the record only: nothing downstream behaves
        /// differently, because a prepared errand is an ordinary errand that happens to be
        /// early, and a second class of worker would be a second thing to keep correct.
        /// See [`crate::foundation::observatory::EventKind::WorkerSpawned`] for why it can
        /// only ever undercount.
        ahead: bool,
        /// `Ok` once the session is registered, open and driving — the point from which
        /// its id answers. `Err` carries why it never opened, which used to reach nothing
        /// but a log line while the caller was told the errand had started.
        ready: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    /// Stop the turn a working session is running (the `cancel_worker` tool).
    ///
    /// Crosses on this channel for the same reason `CreateWorker` does — the live-session
    /// map is the loop's own state — and it is the symmetric half of it. Dispatch that
    /// can only hand work out is dispatch that cannot change its mind, and the one
    /// instruction a person gives most urgently is "stop".
    ///
    /// Unlike `CreateWorker` it carries a **reply**, because the two outcomes are not the
    /// same news. `true` means a running turn was cut and a report is coming; `false`
    /// means there was nothing to cut — already finished, or already gone — and no report
    /// will arrive. A caller that cannot tell them apart can only guess, and the guess it
    /// would make ("stopped") is the one that reproduces the bug this tool was added for.
    CancelWorker { id: SessionSlug, reply: tokio::sync::oneshot::Sender<bool> },
    /// End a working session for good (the `close_worker` tool).
    ///
    /// The third verb of dispatch, and the one that had no caller: `CreateWorker` hands
    /// work out, `CancelWorker` takes a turn back, and until now nothing *finished* with a
    /// session — a fifteen-minute idle timer did, on its own judgment, which turned out to
    /// be no judgment at all (see [`super::workers`]). A worker's lifetime belongs to the
    /// rung holding the errand, so it needs a way to say the errand is over.
    ///
    /// Carries a **reply** for the same reason `CancelWorker` does: "I closed it" and "it
    /// was already gone" are different facts about what is still running, and a caller
    /// that cannot tell them apart cannot keep an honest roster.
    CloseWorker { id: SessionSlug, reply: tokio::sync::oneshot::Sender<bool> },
}

/// The handle the MCP handler dispatches to. Cheap to clone. Carries `control` for
/// loop-applied side-effects (creating a worker) and, for the one rung that has one, the
/// `mouth` — where what Reaction prepares is set, for the floor to release.
#[derive(Clone)]
pub struct ToolSink {
    pub(super) control: mpsc::Sender<LoopControl>,
    /// Where expression goes — **`None` for a rung with no mouth.**
    ///
    /// Only Reaction has somewhere for speech to go. Cognition registers a sink so its
    /// workers have a home, and it has no sequencer, no audio, no screen; expressing
    /// there is not "blocked", it is undefined. Making that an `Option` states it once
    /// in the type instead of leaving it to two guards elsewhere agreeing — the tool
    /// list and the role check at dispatch — which is the kind of arrangement that
    /// holds until someone adds a third caller.
    pub(super) mouth: Option<Mouth>,
}

/// Where Reaction's expression goes: the prepared sets, and what `prepare` reads before it
/// sets one. Nothing here emits anything — the floor releases a set when the room reaches its
/// moment ([`super::prepared`]), so a call answers what was *prepared*, never what was said.
#[derive(Clone)]
pub(super) struct Mouth {
    /// What the running turn has seen of them, and whether they started it — stamped on each set.
    pub(super) floor: super::Floor,
    /// The pre-send check, which reads every line when it is prepared
    /// ([`super::legibility::check`]).
    pub(super) speech: Arc<super::legibility::Speech>,
    /// What Reaction has prepared, matter by matter ([`super::prepared`]).
    pub(super) prepared: Arc<super::prepared::Prepared>,
}

/// The standing loop that owns a tool sink.
///
/// Workers do not own loop state reached through MCP. Keeping this narrower than the
/// session-role enum makes an accidental registration for one impossible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolOwner {
    Reaction,
    Cognition,
    Reflection,
}

impl ToolOwner {
    pub fn from_role(role: Option<&str>) -> Option<Self> {
        match role {
            Some("reaction") => Some(Self::Reaction),
            Some("cognition") => Some(Self::Cognition),
            Some("reflection") => Some(Self::Reflection),
            _ => None,
        }
    }
}

impl ToolSink {
    /// Forward one control command to the reaction loop. Returns an error only if
    /// the loop is gone (channel closed).
    pub async fn send(&self, control: LoopControl) -> anyhow::Result<()> {
        self.control
            .send(control)
            .await
            .map_err(|_| anyhow::anyhow!("reaction loop gone; control dropped"))
    }

    /// Prepare a matter (the `hi_prepare` tool): `args` carries the matter and its whole set,
    /// which is read and checked action by action ([`super::prepared::parse`]) and replaces
    /// whatever was set for that matter. Returns the literal the tool answers with; a set that
    /// does not parse is an error, naming where, and nothing is set.
    ///
    /// **Every line is read by the pre-send check before the call answers**, all at once: a
    /// `finished` line as the reply it is ([`super::legibility::Speech::review`], which spends
    /// the turn's one send-back), a `paused` or condition line as the prepared line it is
    /// ([`super::legibility::Speech::review_prepared`], which holds up nobody and spends
    /// nothing). A branch whose line is sent back is left out whole, and the answer says which
    /// and why. Nothing is said here: what was set goes when the floor releases it.
    pub async fn prepare(&self, args: &serde_json::Value, data_dir: &std::path::Path) -> anyhow::Result<String> {
        use super::prepared::When;
        let mouth = self
            .mouth
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("this rung has no mouth; there is nothing to prepare"))?;
        let arrived = Instant::now();
        let (matter, branches) = super::prepared::parse(args, data_dir, SAY_MAX_CHARS)
            .await
            .map_err(anyhow::Error::msg)?;
        // The warm-up and the seed prime a session and answer nobody; what they prepared would
        // otherwise be released by the floor to a person who asked for nothing.
        if !mouth.floor.in_turn() {
            return Ok("not prepared — this is not a turn anyone is waiting on, so nothing you \
                       prepare here would reach them"
                .into());
        }
        let heard = mouth.floor.seen();
        let answering = mouth.floor.answering();
        if branches.is_empty() {
            mouth.prepared.set(&matter, branches, Vec::new(), heard, answering).await;
            return Ok(format!("cleared — nothing is prepared for \"{matter}\""));
        }
        let conditions = branches.iter().any(|b| matches!(b.when, When::Condition(_)));
        if conditions && !super::prepared::enabled() {
            return Ok("not prepared — condition branches are switched off on this install; \
                       prepare `finished` or `paused` only"
                .into());
        }
        if conditions && !crate::body::capabilities::decision::available() {
            return Ok("not prepared — nothing is configured to read their messages against a \
                       condition, so it could never run; prepare `finished` or `paused` only"
                .into());
        }
        let reads = branches.iter().enumerate().flat_map(|(i, b)| {
            b.lines().map(move |text| (i, b.when.clone(), text.to_string()))
        });
        let reads = futures::future::join_all(reads.map(|(i, when, text)| {
            let speech = mouth.speech.clone();
            async move {
                let review = match &when {
                    When::Finished => speech.review(&text, arrived).await,
                    other => speech.review_prepared(&moment(other), &text).await,
                };
                (i, review)
            }
        }))
        .await;
        let mut sent_back = std::collections::BTreeMap::new();
        for (i, review) in reads {
            if let super::legibility::check::Review::SendBack(note) = review {
                sent_back.entry(i).or_insert(note);
            }
        }
        let notes: Vec<String> = sent_back
            .iter()
            .map(|(i, note)| format!("the line for `{}` was sent back ({note})", branches[*i].when.as_str()))
            .collect();
        let kept: Vec<_> = branches
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !sent_back.contains_key(i))
            .map(|(_, b)| b)
            .collect();
        let whens: Vec<String> = kept.iter().map(|b| b.when.as_str().to_string()).collect();
        let evicted = mouth
            .prepared
            .set(&matter, kept, mouth.speech.said_since_their_last(), heard, answering)
            .await;
        if !whens.is_empty() {
            mouth.prepared.after_set(answering);
        }
        let others: Vec<String> = mouth.prepared.matters().into_iter().filter(|m| m != &matter).collect();
        let mut ack = if whens.is_empty() {
            format!("not prepared — every line was sent back, so nothing is prepared for \"{matter}\"")
        } else {
            format!(
                "prepared \"{matter}\" — {}. Nothing has been said yet: it goes when the room reaches \
                 that moment and it still fits, and you are told what happened with your next wake",
                whens.join("; ")
            )
        };
        if !notes.is_empty() {
            ack.push_str(&format!(". {} — so that branch is left out; prepare again to change it", notes.join("; ")));
        }
        if !evicted.is_empty() {
            ack.push_str(&format!(
                ". To make room, the conditions prepared for {} are gone",
                evicted.iter().map(|m| format!("\"{m}\"")).collect::<Vec<_>>().join(", ")
            ));
        }
        // What else is ready, at the moment of writing — a turn sees the list when it starts,
        // not what it prepared since. One matter under two names is how the same answer went
        // out twice on 09-24.
        if !others.is_empty() && !whens.is_empty() {
            ack.push_str(&format!(
                ". Also ready: {} — if this is a new thought on one of those, it is a revision of that one: \
                 prepare it under that matter's name, merged, and nothing goes out twice",
                others.iter().map(|m| format!("\"{m}\"")).collect::<Vec<_>>().join(", ")
            ));
        }
        Ok(ack)
    }
}

/// The moment a prepared line is for, as the pre-send check reads it above the line.
fn moment(when: &super::prepared::When) -> String {
    match when {
        super::prepared::When::Paused => "they stop with more to come — a short acknowledgment".into(),
        other => format!("their next message goes this way: {}", other.as_str()),
    }
}

/// The shared role-specific sink slots. Created once in `lib.rs`, shared
/// (cloneable handle) between the HTTP front's `/mcp` handler and the loops that
/// register sinks.
#[derive(Clone, Default)]
pub struct ToolRegistry {
    inner: Arc<Mutex<ToolSinks>>,
}

#[derive(Default)]
struct ToolSinks {
    reaction: Option<ToolSink>,
    cognition: Option<ToolSink>,
    reflection: Option<ToolSink>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register (or replace) one role's sink. Called before that role's session
    /// opens and can issue any tool call.
    pub async fn register(&self, owner: ToolOwner, sink: ToolSink) {
        let mut sinks = self.inner.lock().await;
        match owner {
            ToolOwner::Reaction => sinks.reaction = Some(sink),
            ToolOwner::Cognition => sinks.cognition = Some(sink),
            ToolOwner::Reflection => sinks.reflection = Some(sink),
        }
    }

    /// The role's registered sink, or `None` before its loop has stood itself up.
    pub async fn get(&self, owner: ToolOwner) -> Option<ToolSink> {
        let sinks = self.inner.lock().await;
        match owner {
            ToolOwner::Reaction => sinks.reaction.clone(),
            ToolOwner::Cognition => sinks.cognition.clone(),
            ToolOwner::Reflection => sinks.reflection.clone(),
        }
    }

    // There is intentionally no fallback slot. A missing role owner is an error;
    // routing a call to another loop would transfer worker ownership silently.
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A mouth whose check reads nothing.
    fn mouth() -> ToolSink {
        let (control, _ctl) = mpsc::channel(8);
        ToolSink {
            control,
            mouth: Some(Mouth {
                floor: crate::body::reaction::Floor::new(),
                speech: Arc::new(crate::body::reaction::legibility::Speech::off()),
                prepared: Arc::new(super::super::prepared::Prepared::new(
                    crate::foundation::observatory::Observatory::new(None),
                    None,
                )),
            }),
        }
    }

    /// **Preparing says nothing, and the answer says so.** A reply is set for its moment; the
    /// call answering "sent" would be the old verb's promise, and it would be false.
    #[tokio::test]
    async fn a_reply_is_prepared_not_said() {
        let sink = mouth();
        sink.mouth.as_ref().unwrap().floor.note_turn_started(1, true);
        let ack = sink
            .prepare(
                &serde_json::json!({"matter": "Attention", "branches": [
                    {"when": "finished", "actions": [{"do": "say", "text": "对，平方的是那一层。"}]},
                    {"when": "paused", "actions": [{"do": "say", "text": "收到，还有要补的吗？"}]}
                ]}),
                &std::env::temp_dir(),
            )
            .await
            .unwrap();
        assert!(ack.starts_with("prepared \"Attention\" — finished; paused"), "{ack}");
        assert!(ack.contains("Nothing has been said yet"), "{ack}");
        assert!(sink.mouth.as_ref().unwrap().prepared.render().contains("- finished → say"));
    }

    /// The warm-up and the seed run outside a turn: what they prepare would reach a person who
    /// asked for nothing, so it is refused.
    #[tokio::test]
    async fn nothing_is_prepared_outside_a_turn() {
        let sink = mouth();
        let set = serde_json::json!({"matter": "hello", "branches": [{"when": "finished", "actions": [{"do": "say", "text": "hi"}]}]});
        let ack = sink.prepare(&set, &std::env::temp_dir()).await.unwrap();
        assert!(ack.starts_with("not prepared"), "{ack}");
        assert!(sink.mouth.as_ref().unwrap().prepared.render().contains("(nothing prepared)"));
        let floor = &sink.mouth.as_ref().unwrap().floor;
        floor.note_turn_started(1, false);
        floor.note_turn_ended();
        assert!(sink.prepare(&set, &std::env::temp_dir()).await.unwrap().starts_with("not prepared"), "after it ends, too");
    }

    #[tokio::test]
    async fn clearing_a_matter_says_it_is_cleared() {
        let sink = mouth();
        sink.mouth.as_ref().unwrap().floor.note_turn_started(1, true);
        let ack = sink.prepare(&serde_json::json!({"matter": "x", "branches": []}), &std::env::temp_dir()).await.unwrap();
        assert!(ack.starts_with("cleared"), "{ack}");
    }

    #[tokio::test]
    async fn a_rung_with_no_mouth_cannot_prepare() {
        let (control, _ctl) = mpsc::channel(8);
        let sink = ToolSink { control, mouth: None };
        assert!(sink.prepare(&serde_json::json!({"matter": "x", "branches": []}), &std::env::temp_dir()).await.is_err());
    }

    #[tokio::test]
    async fn role_registrations_do_not_replace_each_other() {
        fn sink() -> (ToolSink, mpsc::Receiver<LoopControl>) {
            let (control, rx) = mpsc::channel(1);
            (
                ToolSink {
                    control,
                    mouth: None,
                },
                rx,
            )
        }

        let registry = ToolRegistry::new();
        let (reaction, mut reaction_rx) = sink();
        let (cognition, mut cognition_rx) = sink();
        let (reflection, mut reflection_rx) = sink();

        registry.register(ToolOwner::Reaction, reaction).await;
        registry.register(ToolOwner::Cognition, cognition).await;
        registry.register(ToolOwner::Reflection, reflection).await;

        for (owner, id) in [
            (ToolOwner::Reaction, "general-one"),
            (ToolOwner::Cognition, "general-two"),
            (ToolOwner::Reflection, "general-three"),
        ] {
            registry
                .get(owner)
                .await
                .unwrap()
                .send(LoopControl::CreateWorker {
                    id: id.parse().unwrap(),
                    title: format!("errand-{id}"),
                    task: Some(format!("task-{id}")),
                    kind: crate::identity::WorkerType::default(),
                    servers: Vec::new(),
                    owner: None,
                    resume: None,
                    subject: None,
                    ahead: false,
                    ready: tokio::sync::oneshot::channel().0,
                })
                .await
                .unwrap();
        }

        let landed = |control| match control {
            Some(LoopControl::CreateWorker { id, .. }) => id.to_string(),
            other => panic!("expected a CreateWorker, got {other:?}"),
        };
        assert_eq!(landed(reaction_rx.recv().await), "general-one");
        assert_eq!(landed(cognition_rx.recv().await), "general-two");
        assert_eq!(landed(reflection_rx.recv().await), "general-three");
    }
}
