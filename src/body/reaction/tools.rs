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
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::{Mutex, mpsc};
// The loop's clock, so a deadline set here can be handed straight to `sleep_until`
// there. `tokio::time::Instant` is also what a paused test clock advances.
use tokio::time::Instant;

use crate::foundation::registry::SessionSlug;

use super::sequencer::Beat;

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

/// The handle the MCP handler dispatches to. Cheap to clone. Carries two
/// senders: `control` for loop-applied side-effects (creating a worker), and
/// `mouth` for output (say/show) that the sequencer renders directly
/// — output bypasses the turn loop so it streams while the prompt is still
/// running.
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

/// The outbound half: the sequencer to emit onto.
///
/// It used to carry a presence read as well, so `say` could answer where the words
/// landed. That answer is gone with the gate — a message is appended to the
/// conversation and keeps, so there is only one fate for an accepted utterance.
#[derive(Clone)]
pub(super) struct Mouth {
    pub(super) beats: mpsc::Sender<Beat>,
    /// How many utterances this mouth has accepted, ever. Only ever incremented.
    ///
    /// The turn loop reads it either side of a generation to answer one question the
    /// beat stream cannot answer until `TurnEnd`: *did this turn speak at all?* That
    /// answer paces the [check-in](super::render_check_in) floor — a check-in that
    /// produced speech is earning its cadence and one that came and went in silence
    /// doubles its gap. A counter rather than a flag, so nothing has to reset it and two
    /// turns cannot race over whose flag it was.
    pub(super) said: Arc<AtomicU64>,
    /// When Reaction next owes them a word. Written here, at the instant it makes the
    /// promise: it is a property of *this utterance*, and a turn can outlive the
    /// window that started it.
    /// Whether the room is Reaction's to speak into, asked at the instant the words
    /// are ready rather than when the turn that wrote them began.
    ///
    /// **The one thing between a produced utterance and the wire**, and it lives on
    /// the mouth because that is the last moment at which the question has a current
    /// answer — a turn takes seconds, and the room moves inside them. See
    /// [`super::floor`] for why the input-side settle could never answer it.
    pub(super) floor: super::Floor,
    /// The second reading a message may get before the floor is asked
    /// ([`super::legibility::check`]).
    pub(super) speech: Arc<super::legibility::Speech>,
    /// The messages sent since the person last sent one ([`super::unanswered`]).
    pub(super) unanswered: Arc<super::unanswered::Unanswered>,
    /// What Reaction has prepared, matter by matter ([`super::prepared`]). On the mouth
    /// because `prepare` sets it, and its checks are the mouth's.
    pub(super) prepared: Arc<super::prepared::Prepared>,
    /// The screen, asked by `show` whether a view goes in front of them or into their list
    /// ([`crate::foundation::server::ViewBus::claim`]).
    pub(super) views: crate::foundation::server::ViewBus,
    /// Whether they have been away since the page in front of them went up.
    pub(super) attachments: crate::body::attachments::Attachments,
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

/// What became of an utterance — the answer `say` hands back to Reaction.
///
/// **None of these is about the room's contents.** This used to distinguish
/// *voiced* / *on screen only* / *waiting for them to come back*, so Reaction could
/// read the answer and go quiet on an empty room. That whole axis is gone: an
/// accepted message is appended to the conversation, where it keeps and is read
/// whenever they look. Whether a speaker happened to be attached decides only
/// whether frames were synthesized, which is the host's business and not a thing to
/// reason about.
///
/// What remains are the facts the caller can act on, because only the caller can act
/// on them: the message was too long to be a message, enough had already gone out since
/// the person's last one, a second reading sent it back, or the floor was not Reaction's
/// to take ([`super::floor`]).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Spoken {
    /// Rejected: longer than a message. Nothing was sent.
    TooLong,
    /// Rejected: [`MAX_UNANSWERED`](super::unanswered::MAX_UNANSWERED) messages have gone out
    /// since the person last sent one ([`super::unanswered`]). Nothing was sent.
    Unanswered,
    /// Rejected: the words fit under the cap and the files handed over with them do not —
    /// each is a message of its own. Nothing was sent. Carries how many were attached.
    UnansweredWith(usize),
    /// Sent back by the pre-send check, with its note on where the line fails
    /// ([`super::legibility::check`]). Nothing was sent, and nothing about the room moved.
    NotSent(String),
    /// The floor was theirs. Nothing was sent, and nothing is queued to be sent
    /// later — see [`super::floor`] for why this is a refusal rather than a hold.
    NotSaid(super::Busy),
    /// Appended to the conversation.
    Sent,
}

impl Spoken {
    /// The literal `say` returns. Written as a plain statement of what happened,
    /// because it is read by a model deciding what to do next — not a status code.
    ///
    /// The three refusals say *what happened* and stop there. What to do about it —
    /// let the line go, keep listening, fold it into the next one — is `reaction.md`'s
    /// to say, and putting an instruction here would be the host writing character.
    pub fn ack(&self) -> String {
        match self {
            Spoken::TooLong => "too long for one message — nothing was sent".into(),
            Spoken::Unanswered => format!(
                "not sent — {} messages have already gone out since their last one",
                super::unanswered::MAX_UNANSWERED
            ),
            Spoken::UnansweredWith(files) => format!(
                "not sent — with the {files} it hands over this would be {} messages, and at \
                 most {} go out since their last one; each thing handed over is a message",
                files + 1,
                super::unanswered::MAX_UNANSWERED
            ),
            Spoken::NotSent(note) => format!("not sent — {note}"),
            Spoken::NotSaid(super::Busy::Speaking) => {
                "not said — they were still talking, so the floor was theirs".into()
            }
            Spoken::NotSaid(super::Busy::Unheard) => {
                "not said — they said something after this turn started that you haven't \
                 seen yet, so this reply is out of date"
                    .into()
            }
            Spoken::NotSaid(super::Busy::Typing) => {
                "not said — they are still typing a line they haven't sent, so the thought \
                 isn't finished"
                    .into()
            }
            // The one outcome that was a status code rather than a statement, and the
            // only one whose consequence the caller can still get wrong: a refusal is
            // plainly a refusal, where "sent" left *how final* to be inferred. Read
            // beside the two "not said" arms, which are sentences, it read as the
            // weaker answer of the three.
            Spoken::Sent => "sent — the message is in the conversation now, and stays there".into(),
        }
    }
}

/// What one `say` call did: where the words landed, and whether it also put Reaction
/// back on the hook for a named time.
///
/// Two facts rather than one enum arm each, because they are independent — an
/// utterance held for an empty room can still carry a promise, and most utterances
/// carry none.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Said {
    pub spoken: Spoken,
}

impl Said {
    /// The literal `say` returns — a plain statement of what happened, read by a model
    /// deciding what to do next.
    pub fn ack(&self) -> String {
        self.spoken.ack()
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

    /// Speak `text` (the `say` tool): queue it onto the output sequencer,
    /// which paces it to TTS. Acks immediately — never waits on synthesis.
    ///
    ///
    /// **It arms nothing.** This took `back_in` — the size Reaction had just put on a
    /// silence, held as a wake — and that was the last timer in the host. It fired a median
    /// 1.2 minutes before the work reported anyway, so what it produced was "still going,
    /// another five minutes" just ahead of the real answer.
    ///
    /// `hands` is what the message hands over — attachments, each one message of its own after
    /// the words, enqueued together as one arrival (`docs/arch/message.md` § *Both ends can hand
    /// over a file*). Empty for an ordinary message.
    pub async fn say(&self, text: String, hands: Vec<crate::types::FileRef>) -> anyhow::Result<Said> {
        let mouth = self
            .mouth
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("this rung has no mouth; there is nowhere to say it"))?;
        if text.chars().count() > SAY_MAX_CHARS {
            return Ok(Said { spoken: Spoken::TooLong });
        }
        // Read in the order they were written: the lock is held from here to the message's
        // fate, so a later call waits for an earlier one's check and floor.
        let arrived = Instant::now();
        let _in_order = mouth.speech.serial.lock().await;
        // **Under the lock, before anything costs time.** Two messages in one turn must not
        // both read a count of two, and a message that will not go out is not worth a check.
        if mouth.unanswered.is_full() {
            tracing::info!("not sent — the run since their last message is full");
            return Ok(Said { spoken: Spoken::Unanswered });
        }
        if !mouth.unanswered.fits(1 + hands.len() as u64) {
            tracing::info!(files = hands.len(), "not sent — the words fit the run and their files do not");
            return Ok(Said { spoken: Spoken::UnansweredWith(hands.len()) });
        }
        // **Before the floor, not after.** The check can take seconds, and whether the room
        // is free is a property of the moment the words are actually ready to go.
        if let super::legibility::check::Review::SendBack(note) =
            mouth.speech.review(&text, arrived).await
        {
            return Ok(Said { spoken: Spoken::NotSent(note) });
        }
        // A draft that is mid-pause gets a moment to settle before the gate is asked,
        // because typing is the one floor condition whose ending is not itself a
        // signal — see [`super::floor`]. Returns immediately unless they are actually
        // typing, so the ordinary reply pays nothing for it.
        mouth.floor.settle_typing().await;
        // **Asked here, not at turn start.** Length is a property of the text and can
        // be judged the moment it arrives; whether the room is free is a property of
        // *now*, and the turn that wrote these words began seconds ago.
        if let Err(busy) = mouth.floor.may_speak(Instant::now()).await {
            return Ok(Said { spoken: Spoken::NotSaid(busy) });
        }
        mouth.speech.note_sent(&text);
        mouth
            .beats
            .send(Beat::Say(text))
            .await
            .map_err(|_| anyhow::anyhow!("sequencer gone; say dropped"))?;
        // Counted where the utterance is accepted, so `TooLong` (rejected above, never
        // sent) does not read as speech.
        mouth.said.fetch_add(1, Ordering::Relaxed);
        mouth.unanswered.note_sent();
        // What it hands over follows the words on the same beat queue, nothing awaited between,
        // so the sequencer appends them in order right behind the message they belong to.
        for file in hands {
            mouth
                .beats
                .send(Beat::Hand(file))
                .await
                .map_err(|_| anyhow::anyhow!("sequencer gone; hand-over dropped"))?;
            mouth.unanswered.note_sent();
        }
        Ok(Said { spoken: Spoken::Sent })
    }

    /// Prepare for where a matter may go next (the `prepare` tool): `args` carries the matter
    /// and its whole set, which is read and checked action by action
    /// ([`super::prepared::parse`]) and replaces whatever was set for that matter. Returns the
    /// literal the tool answers with; a set that does not parse is an error, naming where, and
    /// nothing is set.
    ///
    /// Refused, with nothing set, when they have already replied since this turn began — the
    /// lines were written for a moment that has passed. Each branch's lines are then read by
    /// the pre-send check ([`super::legibility::Speech::review_prepared`]), together and off
    /// the serial lock; a branch whose line is sent back is left out, and the answer says which
    /// and why.
    pub async fn prepare(&self, args: &serde_json::Value, data_dir: &std::path::Path) -> anyhow::Result<String> {
        let mouth = self
            .mouth
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("this rung has no mouth; there is nothing to prepare"))?;
        let (matter, branches) = super::prepared::parse(args, data_dir, SAY_MAX_CHARS)
            .await
            .map_err(anyhow::Error::msg)?;
        const ALREADY: &str = "not prepared — they have already replied since this turn began; \
                               their message drives your next turn";
        if branches.is_empty() {
            mouth.prepared.set(&matter, branches, Vec::new()).await;
            return Ok(format!("cleared — nothing is prepared for \"{matter}\""));
        }
        if !super::prepared::enabled() {
            return Ok("not prepared — prepared branches are switched off on this install".into());
        }
        if !crate::body::capabilities::decision::available() {
            return Ok("not prepared — nothing is configured to read their messages against \
                       these, so none of them could run"
                .into());
        }
        if mouth.floor.unheard() {
            return Ok(ALREADY.into());
        }
        let reads = branches.iter().enumerate().flat_map(|(i, b)| {
            b.actions.iter().filter_map(move |a| match a {
                super::prepared::Action::Say { text } => Some((i, b.condition.clone(), text.clone())),
                _ => None,
            })
        });
        let reads = futures::future::join_all(reads.map(|(i, condition, text)| {
            let speech = mouth.speech.clone();
            async move { (i, speech.review_prepared(&condition, &text).await) }
        }))
        .await;
        let mut sent_back = std::collections::BTreeMap::new();
        for (i, review) in reads {
            if let super::legibility::check::Review::SendBack(note) = review {
                sent_back.entry(i).or_insert(note);
            }
        }
        // The reads take time, and they may have replied during it.
        if mouth.floor.unheard() {
            return Ok(ALREADY.into());
        }
        let notes: Vec<String> = sent_back
            .iter()
            .map(|(i, note)| format!("the line for \"{}\" was sent back ({note})", branches[*i].condition))
            .collect();
        let kept: Vec<_> = branches
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !sent_back.contains_key(i))
            .map(|(_, b)| b)
            .collect();
        let directions = kept.len();
        let evicted = mouth.prepared.set(&matter, kept, mouth.speech.said_since_their_last()).await;
        let mut ack = if directions == 0 {
            format!("not prepared — every line was sent back, so nothing is prepared for \"{matter}\"")
        } else {
            format!(
                "prepared \"{matter}\" — {directions} direction{}; it waits until they take this \
                 matter up, runs at most one, and your next turn is told what ran",
                if directions == 1 { "" } else { "s" }
            )
        };
        if !notes.is_empty() {
            ack.push_str(&format!(". {} — so that direction is left out; prepare again to change it", notes.join("; ")));
        }
        if !evicted.is_empty() {
            ack.push_str(&format!(
                ". To make room, what was prepared for {} is gone",
                evicted.iter().map(|m| format!("\"{m}\"")).collect::<Vec<_>>().join(", ")
            ));
        }
        Ok(ack)
    }

    /// Show a view (the `show` tool): queue it onto the sequencer, which
    /// paces it to the surrounding narration. `op` is `show`/`replace`/`dismiss`;
    /// `id` may be omitted (one is synthesized). `view_ref` is the ref the source was
    /// read from, carried so the restore can recompile it (`None` for an inline-source
    /// view).
    ///
    /// Unlike speech this is never gated: a view is retained state, folded and
    /// replayed to whatever connects next (and restored across restarts), so showing
    /// into an empty room costs nothing and is waiting when they arrive.
    ///
    /// **Where it lands is decided here, and said back.** A show from a turn they did not
    /// start can go into their list instead of in front of them, when they are still on a
    /// page that just went up ([`crate::foundation::server::ViewBus::claim`]). The answer
    /// says which, because the turn is still running and about to speak: "放上来了" beside
    /// a view that went into the list is the one way this can mislead them.
    pub async fn show(
        &self,
        id: Option<String>,
        op: String,
        source: String,
        view_ref: Option<String>,
    ) -> anyhow::Result<String> {
        use crate::foundation::server::view_bus::{AWAY_FOR, Claim};
        let mouth = self
            .mouth
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("this rung has no screen; there is nowhere to show it"))?;
        let claim = match (op.as_str(), mouth.floor.show_from()) {
            ("dismiss", _) | (_, None) => Claim::Takes,
            (_, Some(from)) => {
                let back_at = mouth.attachments.back_from_away(AWAY_FOR);
                mouth.views.claim(id.as_deref(), view_ref.as_deref(), from, back_at).await
            }
        };
        // Part of what the turn did, for whatever reads its words: "it's on screen" is true
        // only beside a show that took it.
        let named = view_ref.as_deref().or(id.as_deref()).unwrap_or("an inline view").to_owned();
        let (keep, ack) = match &claim {
            Claim::Takes => {
                mouth.speech.note_shown(&format!("{op} {named}"));
                (false, "shown".to_string())
            }
            Claim::Keeps { reading } => {
                mouth.speech.note_shown(&format!(
                    "{op} {named} — into their list; the screen stayed on {reading}"
                ));
                (
                    true,
                    format!(
                        "shown into their list, not in front of them: they are still on \
                         \"{reading}\", which only just went up, so the screen stayed \
                         there. It is first in their list with a mark on it, and on the task \
                         that made it; one tap opens it. Do not say it is on the screen."
                    ),
                )
            }
        };
        mouth
            .beats
            .send(Beat::Show { id, op, source, view_ref, keep })
            .await
            .map_err(|_| anyhow::anyhow!("sequencer gone; show dropped"))?;
        Ok(ack)
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
    /// A mouth wired to a live receiver, so `send` succeeds and a failure under
    /// test is never just a closed channel.
    fn mouth() -> (ToolSink, mpsc::Receiver<Beat>) {
        let (beats, rx) = mpsc::channel(8);
        let (control, _ctl) = mpsc::channel(8);
        let sink = ToolSink {
            control,
            mouth: Some(Mouth {
                beats,
                said: Arc::new(AtomicU64::new(0)),
                floor: crate::body::reaction::Floor::new(),
                speech: Arc::new(crate::body::reaction::legibility::Speech::off()),
                unanswered: Arc::new(super::super::unanswered::Unanswered::default()),
                prepared: Arc::new(super::super::prepared::Prepared::new(
                    crate::foundation::observatory::Observatory::new(None),
                    None,
                )),
                // An empty screen: a claim reads it and nothing here ever writes one.
                views: crate::foundation::server::ViewBus::load(std::path::Path::new(
                    "/nonexistent/hi-agent-tools-test",
                )),
                attachments: crate::body::attachments::Attachments::new(),
            }),
        };
        (sink, rx)
    }

    /// The count the turn loop reads to decide whether a turn spoke: an accepted
    /// utterance moves it, a rejected one does not.
    #[tokio::test]
    async fn only_an_accepted_utterance_counts_as_speech() {
        let (sink, _rx) = mouth();
        let said = sink.mouth.as_ref().unwrap().said.clone();

        assert_eq!(
            sink.say("x".repeat(SAY_MAX_CHARS + 1), Vec::new()).await.unwrap().spoken,
            Spoken::TooLong
        );
        assert_eq!(said.load(Ordering::Relaxed), 0, "a rejected call said nothing");

        sink.say("hi".into(), Vec::new()).await.unwrap();
        assert_eq!(said.load(Ordering::Relaxed), 1);
    }

    /// A refusal by the floor is not a delayed send: no beat reaches the sequencer and
    /// nothing counts as speech.
    #[tokio::test]
    async fn a_refused_utterance_says_nothing() {
        let (sink, mut rx) = mouth();
        let mouth = sink.mouth.as_ref().unwrap();
        mouth.floor.note_speech(Instant::now()).await;

        let said = sink.say("their turn".into(), Vec::new()).await.unwrap();
        assert_eq!(said.spoken, Spoken::NotSaid(crate::body::reaction::Busy::Speaking));
        assert_eq!(mouth.said.load(Ordering::Relaxed), 0, "nothing was said");
        assert!(rx.try_recv().is_err(), "no beat reached the sequencer");
        assert!(said.ack().starts_with("not said"), "{}", said.ack());
    }

    /// An accepted message has one fate, and it does not depend on who is
    /// connected. This is the assertion that keeps the gate from growing back.
    #[tokio::test]
    async fn an_accepted_message_is_simply_sent() {
        let (sink, _rx) = mouth();
        assert_eq!(sink.say("hi".into(), Vec::new()).await.unwrap().spoken, Spoken::Sent);
        assert!(Spoken::Sent.ack().starts_with("sent"), "{}", Spoken::Sent.ack());
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

    #[tokio::test]
    async fn an_overlong_say_is_rejected_without_emission() {
        let (sink, mut rx) = mouth();
        let text = "x".repeat(SAY_MAX_CHARS + 1);

        assert_eq!(sink.say(text, Vec::new()).await.unwrap().spoken, Spoken::TooLong);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn the_words_are_emitted_whatever_the_room() {
        // The gate is about voice, never about dropping the utterance: text is
        // retained and keeps, so the beat goes out even to nobody.
        let (sink, mut rx) = mouth();
        sink.say("hi".into(), Vec::new()).await.unwrap();
        assert!(matches!(rx.try_recv(), Ok(Beat::Say(t)) if t == "hi"));
    }

    #[tokio::test]
    async fn a_rung_with_no_mouth_cannot_say() {
        let (control, _ctl) = mpsc::channel(8);
        let sink = ToolSink { control, mouth: None };
        assert!(sink.say("hi".into(), Vec::new()).await.is_err());
    }

    /// **`say` carries no timer, and this is the assertion that keeps one from growing
    /// back.** It took `back_in` — the size Reaction put on a silence, armed as a wake —
    /// and that went on its own numbers: across the frame log it fired 53 times and the
    /// work it was waiting on reported within a further 1.2 minutes at the median, 5
    /// minutes in 90% of cases. It bought a line saying "still going, another five
    /// minutes" a minute before the real answer, and each one armed the next.
    #[tokio::test]
    async fn saying_something_arms_nothing() {
        let (sink, _rx) = mouth();
        let said = sink.say("give me ten minutes".into(), Vec::new()).await.unwrap();

        assert_eq!(said.spoken, Spoken::Sent);
        // The whole of the return: what happened to the utterance, and nothing about time.
        assert_eq!(said.ack(), Spoken::Sent.ack());
    }

    #[test]
    fn every_outcome_says_what_happened() {
        // The ack is read by a model, so it must be a sentence about the world —
        // and the outcomes must not read alike, or the answer carries no information.
        let acks = [
            Spoken::TooLong.ack(),
            Spoken::Sent.ack(),
            Spoken::NotSent("「公网 200」是常规检查".into()).ack(),
            Spoken::Unanswered.ack(),
        ];
        for a in &acks {
            assert!(!a.is_empty(), "an ack must state what happened: {a:?}");
        }
        assert_eq!(acks.iter().collect::<std::collections::HashSet<_>>().len(), 4);
        assert!(acks[2].starts_with("not sent — ") && acks[2].contains("公网 200"));
    }

    /// The rejection says the words were not sent, and stops there. It used to ask for
    /// "a few shorter say calls", and that instruction split one matter across several
    /// messages — the answer to too long is to say less, which is `reaction.md`'s to say.
    #[test]
    fn the_rejection_says_nothing_was_sent_and_does_not_ask_for_pieces() {
        let ack = Spoken::TooLong.ack();
        assert!(ack.contains("nothing was sent"));
        assert!(!ack.contains("shorter") && !ack.contains("a few"));
    }
    /// Past the cap nothing reaches the sequencer, however often it is asked — there is no
    /// backstop the way the floor has one — and the person's next message opens it again.
    #[tokio::test]
    async fn the_run_past_the_cap_is_refused_until_they_send_something() {
        use super::super::unanswered::MAX_UNANSWERED;
        let (sink, mut rx) = mouth();
        let mouth = sink.mouth.as_ref().unwrap();

        for _ in 0..MAX_UNANSWERED {
            assert_eq!(sink.say("progress".into(), Vec::new()).await.unwrap().spoken, Spoken::Sent);
            assert!(rx.try_recv().is_ok());
        }
        for _ in 0..5 {
            assert_eq!(sink.say("more".into(), Vec::new()).await.unwrap().spoken, Spoken::Unanswered);
        }
        assert!(rx.try_recv().is_err(), "a refused message reached the sequencer");
        assert_eq!(mouth.said.load(Ordering::Relaxed), MAX_UNANSWERED);

        mouth.unanswered.note_person();
        assert_eq!(sink.say("answer".into(), Vec::new()).await.unwrap().spoken, Spoken::Sent);
    }

    /// **What a message hands over follows its words, and each thing is a message toward the
    /// cap** — so the words and their files go together or not at all.
    #[tokio::test]
    async fn a_message_hands_over_its_files_right_behind_it_and_each_counts() {
        let (sink, mut rx) = mouth();
        let file = |id: &str| crate::types::FileRef {
            reff: format!("att:{id}"),
            mime: "image/jpeg".into(),
            name: "picture 1920×1080".into(),
            bytes: Some(1),
            peek: None,
        };
        let said = sink.say("场地线压在线上".into(), vec![file("3f9a0c11d2e4b5a6")]).await.unwrap();
        assert_eq!(said.spoken, Spoken::Sent);
        assert!(matches!(rx.try_recv(), Ok(Beat::Say(t)) if t == "场地线压在线上"));
        assert!(matches!(rx.try_recv(), Ok(Beat::Hand(f)) if f.reff == "att:3f9a0c11d2e4b5a6"));

        // Two of three are gone; one more message with a file would be four.
        let refused = sink.say("还有这张".into(), vec![file("0123456789abcdef")]).await.unwrap();
        assert_eq!(refused.spoken, Spoken::UnansweredWith(1));
        assert!(refused.ack().contains("each thing handed over is a message"), "{}", refused.ack());
        assert!(rx.try_recv().is_err(), "neither the words nor the file went out");
        assert_eq!(sink.say("就这些".into(), Vec::new()).await.unwrap().spoken, Spoken::Sent);
    }

    /// A message that was never sent does not use up the run.
    #[tokio::test]
    async fn a_rejected_message_does_not_count_toward_the_run() {
        let (sink, _rx) = mouth();
        sink.say("x".repeat(SAY_MAX_CHARS + 1), Vec::new()).await.unwrap();
        assert!(!sink.mouth.as_ref().unwrap().unanswered.is_full());
        for _ in 0..super::super::unanswered::MAX_UNANSWERED {
            assert_eq!(sink.say("hi".into(), Vec::new()).await.unwrap().spoken, Spoken::Sent);
        }
    }
}
