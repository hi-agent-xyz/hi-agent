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
use std::time::Duration;

use tokio::sync::{Mutex, mpsc};
// The loop's clock, so a deadline set here can be handed straight to `sleep_until`
// there. `tokio::time::Instant` is also what a paused test clock advances.
use tokio::time::Instant;

use crate::types::ViewTraits;

use super::sequencer::Beat;

/// Loose guard against turning one `say` call into a paragraph-sized delivery.
/// Reaction normally speaks in much smaller natural chunks; this only catches
/// accidental dumps and leaves room for a few ordinary sentences.
pub(super) const SAY_MAX_CHARS: usize = 240;

/// One command the MCP tool server routes to the reaction loop.
///
/// Once there were four: two for dispatching work and two for a worker to reach the
/// voice. The reaching ones are gone — a worker addresses its owner with the one verb
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
    /// answers to; a worker belongs to the session that created it, never to the voice
    /// it happens to run in.
    ///
    /// `resume` is a codex thread the new session should pick back up instead of starting
    /// cold — an errand the last restart killed, named by the boot glance that offered it.
    /// It is a fresh session either way: a new id, a new registration, and the same prompt.
    /// Only where its mind starts differs.
    CreateWorker {
        id: u64,
        task: String,
        kind: crate::identity::WorkerType,
        owner: Option<u64>,
        resume: Option<String>,
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
    CancelWorker { id: u64, reply: tokio::sync::oneshot::Sender<bool> },
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
    /// Only the voice has somewhere for speech to go. Cognition registers a sink so its
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
    /// doubles its gap — and it is what a turn that typed a reply and said none of it
    /// is noticed by. A counter rather than a flag, so nothing has to reset it and two
    /// turns cannot race over whose flag it was.
    pub(super) said: Arc<AtomicU64>,
    /// When the voice next owes them a word. Written here, at the instant it makes the
    /// promise: it is a property of *this utterance*, and a turn can outlive the
    /// window that started it.
    pub(super) next_word: NextWord,
}

/// When the voice next owes the person a word.
///
/// `after` is the size that was named — kept alongside the deadline so the wake can
/// say *"you said ten minutes, and it has been ten minutes"* rather than arriving as
/// an anonymous tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Owed {
    pub(super) at: Instant,
    pub(super) after: Duration,
    /// `true` when the voice named the number itself, in `say`'s `back_in`. `false`
    /// when the host put a floor under a silence the voice left open-ended — see
    /// [`NextWord::floor`]. The two produce different notes, because only one of them
    /// is a promise the person actually heard.
    pub(super) promised: bool,
}

/// The slot holding the voice's own next check-in — the *only* thing in this host
/// that fires at a named time.
///
/// `reaction.md` has always told the voice to put a size on a silence ("give me a
/// couple of minutes, I'll tell you when it's up") and, in the same breath, that the
/// number is decorative: *"You have no timer — nothing taps you on the shoulder at the
/// minute you named."* What actually woke the voice was the work coming back, the
/// person speaking, the person returning to the window, or the pulse — half an hour by
/// default. A promise made in minutes against a wake measured in half-hours is one the
/// host cannot keep, and the person closes the gap by asking "progress?", which is the
/// failure `reaction.md` calls out in its own words: *a check-in they have to ask for
/// is already late*.
///
/// **This is not the clock [`docs/arch/host.md`] removed, and must not grow into one.**
/// It holds exactly one deadline, process-wide-per-voice; it fires nothing but the
/// reaction loop; it carries no task, no target and no payload beyond a note; and the
/// only moment it can name is one the voice just said out loud (or a floor under a
/// silence it left open). A task's `due` still fires nothing. What it adds is a second
/// deadline in a `select!` that already carries the pulse's.
///
/// Written from the `/mcp` task (`say`) and read by the loop, so it is a shared cell
/// rather than loop-private state. A `std::sync::Mutex` because every critical section
/// here is a move of one `Option` and never crosses an await.
#[derive(Clone, Default)]
pub(super) struct NextWord {
    cell: Arc<std::sync::Mutex<Option<Owed>>>,
}

impl NextWord {
    /// The voice named a number: it just told them they'd hear back within `after`.
    /// Replaces whatever was armed — the newest promise is the one they heard.
    pub(super) fn promise(&self, after: Duration) {
        self.set(Some(Owed { at: Instant::now() + after, after, promised: true }));
    }

    /// The host's floor under an open-ended silence, armed only when nothing is
    /// already armed (a promise the voice made outranks it).
    pub(super) fn floor(&self, after: Duration) {
        let mut cell = self.cell.lock().expect("next-word slot poisoned");
        if cell.is_none() {
            *cell = Some(Owed { at: Instant::now() + after, after, promised: false });
        }
    }

    /// When the next check-in is due, for the loop's deadline set. `None` = nothing armed.
    pub(super) fn due_at(&self) -> Option<Instant> {
        self.cell.lock().expect("next-word slot poisoned").map(|owed| owed.at)
    }

    /// Take the check-in if it has come due, clearing the slot. One-shot by
    /// construction: a wake that fires is a wake that disarms, so a voice that stays
    /// quiet is not re-woken every loop iteration for the same overdue promise.
    pub(super) fn take_due(&self, now: Instant) -> Option<Owed> {
        let mut cell = self.cell.lock().expect("next-word slot poisoned");
        match *cell {
            Some(owed) if owed.at <= now => cell.take(),
            _ => None,
        }
    }

    /// Drop whatever is armed. Used when the moment passes into an empty room — the
    /// return wake is the better carrier then — and when a turn has just spoken to
    /// the work anyway.
    pub(super) fn clear(&self) {
        self.set(None);
    }

    /// What is armed right now, without taking it.
    pub(super) fn peek(&self) -> Option<Owed> {
        *self.cell.lock().expect("next-word slot poisoned")
    }

    fn set(&self, owed: Option<Owed>) {
        *self.cell.lock().expect("next-word slot poisoned") = owed;
    }
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
/// **Two cases, and only one of them is about the room's contents: neither.** This
/// used to distinguish *voiced* / *on screen only* / *waiting for them to come back*,
/// so Reaction could read the answer and go quiet on an empty room. That whole axis
/// is gone: an accepted message is appended to the conversation, where it keeps and
/// is read whenever they look. Whether a speaker happened to be attached decides
/// only whether frames were synthesized, which is the host's business and not a
/// thing to reason about.
///
/// What remains is the one fact the caller can act on, because only the caller can
/// fix it: the message was too long to be a message.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Spoken {
    /// Rejected: longer than a message. Nothing was sent.
    TooLong,
    /// Appended to the conversation.
    Sent,
}

impl Spoken {
    /// The literal `say` returns. Written as a plain statement of what happened,
    /// because it is read by a model deciding what to do next — not a status code.
    pub fn ack(self) -> &'static str {
        match self {
            Spoken::TooLong => {
                "too long for one message — send it as a few shorter say calls instead"
            }
            Spoken::Sent => "sent",
        }
    }
}

/// What one `say` call did: where the words landed, and whether it also put the voice
/// back on the hook for a named time.
///
/// Two facts rather than one enum arm each, because they are independent — an
/// utterance held for an empty room can still carry a promise, and most utterances
/// carry none.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Said {
    pub spoken: Spoken,
    /// The check-in this call armed, when `back_in` named a usable size. `None` means
    /// nothing was armed — either none was asked for, or what was asked for was
    /// unreadable (which the ack says outright, since only the caller can fix it).
    pub armed: Option<Duration>,
    /// A `back_in` arrived and could not be read as a duration.
    pub unreadable_back_in: bool,
}

impl Said {
    /// The literal `say` returns — a plain statement of what happened, read by a model
    /// deciding what to do next.
    pub fn ack(self) -> String {
        let mut s = self.spoken.ack().to_string();
        if let Some(after) = self.armed {
            s.push_str(&format!(
                ". You're on the hook for a word within {} — I'll wake you when it's up \
                 if nothing else has by then",
                render_gap(after)
            ));
        } else if self.unreadable_back_in {
            s.push_str(
                ". I couldn't read your `back_in`, so nothing will wake you — say it as \
                 `90s`, `10m` or `1h`",
            );
        }
        s
    }
}

/// A duration in the words the voice would use for it. Rounded, never precise: it is
/// read back into a sentence about a promise, and "10m" is what was said out loud.
pub(super) fn render_gap(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s => format!("{}h", s / 3600),
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
    /// `back_in` is the size the voice just put on a silence — "give me ten minutes" →
    /// `10m`. It arms [`NextWord`], and that is the whole of the timing the host owns:
    /// a promise is only a promise once it has been *said*, so the only way to set one
    /// is as part of saying it. An utterance rejected for length arms nothing, because
    /// nothing was said.
    pub async fn say(&self, text: String, back_in: Option<&str>) -> anyhow::Result<Said> {
        let mouth = self
            .mouth
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("this rung has no voice; there is nowhere to say it"))?;
        if text.chars().count() > SAY_MAX_CHARS {
            return Ok(Said {
                spoken: Spoken::TooLong,
                armed: None,
                unreadable_back_in: false,
            });
        }
        mouth
            .beats
            .send(Beat::Say(text))
            .await
            .map_err(|_| anyhow::anyhow!("sequencer gone; say dropped"))?;
        // Counted where the utterance is accepted, so `TooLong` (rejected above, never
        // sent) does not read as speech.
        mouth.said.fetch_add(1, Ordering::Relaxed);
        let asked = back_in.map(str::trim).filter(|s| !s.is_empty());
        let armed = asked.and_then(super::parse_delay).filter(|d| !d.is_zero());
        if let Some(after) = armed {
            mouth.next_word.promise(after);
        }
        Ok(Said {
            spoken: Spoken::Sent,
            armed,
            unreadable_back_in: asked.is_some() && armed.is_none(),
        })
    }

    /// Show a view (the `show` tool): queue it onto the sequencer, which
    /// paces it to the surrounding narration. `op` is `show`/`replace`/`dismiss`;
    /// `id` may be omitted (one is synthesized). `traits` is what the view declared
    /// about itself (or `None` — host-owned captions). `view_ref` is the ref the
    /// source was read from, carried so the restore can recompile it (`None` for an
    /// inline-source view).
    ///
    /// Unlike speech this is never gated: a view is retained state, folded and
    /// replayed to whatever connects next (and restored across restarts), so showing
    /// into an empty room costs nothing and is waiting when they arrive.
    pub async fn show(
        &self,
        id: Option<String>,
        op: String,
        source: String,
        traits: Option<ViewTraits>,
        view_ref: Option<String>,
    ) -> anyhow::Result<()> {
        self.mouth
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("this rung has no screen; there is nowhere to show it"))?
            .beats
            .send(Beat::Show { id, op, source, traits, view_ref })
            .await
            .map_err(|_| anyhow::anyhow!("sequencer gone; show dropped"))
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
                next_word: NextWord::default(),
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
            sink.say("x".repeat(SAY_MAX_CHARS + 1), None).await.unwrap().spoken,
            Spoken::TooLong
        );
        assert_eq!(said.load(Ordering::Relaxed), 0, "a rejected call said nothing");

        sink.say("hi".into(), None).await.unwrap();
        assert_eq!(said.load(Ordering::Relaxed), 1);
    }

    /// An accepted message has one fate, and it does not depend on who is
    /// connected. This is the assertion that keeps the gate from growing back.
    #[tokio::test]
    async fn an_accepted_message_is_simply_sent() {
        let (sink, _rx) = mouth();
        assert_eq!(sink.say("hi".into(), None).await.unwrap().spoken, Spoken::Sent);
        assert_eq!(Spoken::Sent.ack(), "sent");
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
            (ToolOwner::Reaction, 1),
            (ToolOwner::Cognition, 2),
            (ToolOwner::Reflection, 3),
        ] {
            registry
                .get(owner)
                .await
                .unwrap()
                .send(LoopControl::CreateWorker {
                    id,
                    task: format!("task-{id}"),
                    kind: crate::identity::WorkerType::default(),
                    owner: None,
                    resume: None,
                })
                .await
                .unwrap();
        }

        assert!(matches!(
            reaction_rx.recv().await,
            Some(LoopControl::CreateWorker { id: 1, .. })
        ));
        assert!(matches!(
            cognition_rx.recv().await,
            Some(LoopControl::CreateWorker { id: 2, .. })
        ));
        assert!(matches!(
            reflection_rx.recv().await,
            Some(LoopControl::CreateWorker { id: 3, .. })
        ));
    }

    #[tokio::test]
    async fn an_overlong_say_is_rejected_without_emission() {
        let (sink, mut rx) = mouth();
        let text = "x".repeat(SAY_MAX_CHARS + 1);

        assert_eq!(sink.say(text, None).await.unwrap().spoken, Spoken::TooLong);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn the_words_are_emitted_whatever_the_room() {
        // The gate is about voice, never about dropping the utterance: text is
        // retained and keeps, so the beat goes out even to nobody.
        let (sink, mut rx) = mouth();
        sink.say("hi".into(), None).await.unwrap();
        assert!(matches!(rx.try_recv(), Ok(Beat::Say(t)) if t == "hi"));
    }

    #[tokio::test]
    async fn a_rung_with_no_mouth_cannot_say() {
        let (control, _ctl) = mpsc::channel(8);
        let sink = ToolSink { control, mouth: None };
        assert!(sink.say("hi".into(), None).await.is_err());
    }

    /// The number the voice names in `say` is the whole mechanism: without it the
    /// promise is words only — the state `reaction.md` used to describe outright
    /// ("you have no timer"), and the reason a person ends up asking "progress?".
    #[tokio::test]
    async fn a_named_size_arms_the_check_in() {
        let (sink, _rx) = mouth();
        let said = sink.say("give me ten minutes".into(), Some("10m")).await.unwrap();

        assert_eq!(said.armed, Some(Duration::from_secs(600)));
        let owed = sink.mouth.as_ref().unwrap().next_word.peek().expect("armed");
        assert!(owed.promised, "the voice named this one itself");
        assert_eq!(owed.after, Duration::from_secs(600));
        assert!(said.ack().contains("10m"), "the ack must confirm the number: {}", said.ack());
    }

    /// Nothing said, nothing promised. An overlong call never reaches the person, so
    /// arming a wake for it would put the host on the hook for words they never heard.
    #[tokio::test]
    async fn a_rejected_utterance_promises_nothing() {
        let (sink, _rx) = mouth();
        let said = sink.say("x".repeat(SAY_MAX_CHARS + 1), Some("10m")).await.unwrap();

        assert_eq!(said.spoken, Spoken::TooLong);
        assert_eq!(said.armed, None);
        assert!(sink.mouth.as_ref().unwrap().next_word.peek().is_none());
    }

    /// An unreadable size arms nothing and *says so*. Swallowing it would leave the
    /// voice believing it is covered, which is the one state worse than no timer.
    #[tokio::test]
    async fn an_unreadable_size_is_reported_back() {
        let (sink, _rx) = mouth();
        let said = sink.say("a moment".into(), Some("soonish")).await.unwrap();

        assert_eq!(said.armed, None);
        assert!(said.unreadable_back_in);
        assert!(sink.mouth.as_ref().unwrap().next_word.peek().is_none());
        assert!(said.ack().contains("back_in"), "{}", said.ack());
    }

    /// The newest number is the one they heard, so it replaces the older promise
    /// rather than queueing behind it.
    #[tokio::test]
    async fn a_new_number_replaces_the_old_one() {
        let (sink, _rx) = mouth();
        sink.say("ten minutes".into(), Some("10m")).await.unwrap();
        sink.say("make that two".into(), Some("2m")).await.unwrap();

        let owed = sink.mouth.as_ref().unwrap().next_word.peek().expect("armed");
        assert_eq!(owed.after, Duration::from_secs(120));
    }

    /// The host's floor never overwrites a promise the person actually heard.
    #[tokio::test]
    async fn a_promise_outranks_the_floor() {
        let next = NextWord::default();
        next.promise(Duration::from_secs(600));
        next.floor(Duration::from_secs(300));

        let owed = next.peek().expect("armed");
        assert!(owed.promised);
        assert_eq!(owed.after, Duration::from_secs(600));
    }

    /// One-shot: firing disarms. A voice that reads the room and stays quiet must not
    /// be re-woken for the same overdue promise on every pass round the loop.
    #[tokio::test]
    async fn taking_a_due_check_in_disarms_it() {
        let next = NextWord::default();
        next.floor(Duration::from_secs(300));
        let due = next.due_at().expect("armed");

        assert!(next.take_due(due - Duration::from_secs(1)).is_none(), "not due yet");
        assert!(next.take_due(due).is_some());
        assert!(next.take_due(due).is_none(), "and it is gone");
        assert!(next.due_at().is_none());
    }

    #[test]
    fn every_outcome_says_what_happened() {
        // The ack is read by a model, so it must be a sentence about the world —
        // and the outcomes must not read alike, or the answer carries no information.
        let acks = [Spoken::TooLong.ack(), Spoken::Sent.ack()];
        for a in acks {
            assert!(!a.is_empty(), "an ack must state what happened: {a:?}");
        }
        assert_eq!(acks.iter().collect::<std::collections::HashSet<_>>().len(), 2);
    }

    /// The rejection has to tell the caller what to do, because the caller is the
    /// only one who can: a message that is too long is split, not truncated.
    #[test]
    fn the_rejection_asks_for_shorter_messages() {
        assert!(Spoken::TooLong.ack().contains("shorter"));
    }
}
