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
/// `Alarm` went with [the clock we declined](../../../docs/arch/core.md);
/// scheduling is the agent's own, built with the shell it already has. What is left
/// is one variant, and it is here because the loop owns the state it touches.
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
    CreateWorker { id: u64, task: String, kind: crate::identity::WorkerType, owner: Option<u64> },
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

/// The outbound half: the sequencer to emit onto, plus the presence read
/// that decides what an emission can actually reach.
///
/// Presence lives here rather than being looked up at the call site because the two
/// are the same fact — a mouth *is* the channels — and because the answer is
/// needed at the instant of emission, not at the instant the sink was built.
#[derive(Clone)]
pub(super) struct Mouth {
    pub(super) beats: mpsc::Sender<Beat>,
    pub(super) presence: crate::body::presence::Presence,
}

/// The standing loop that owns a tool sink.
///
/// Deliberation and workers do not own loop state reached through MCP. Keeping
/// this narrower than the session-role enum makes an accidental registration for
/// either impossible.
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
/// The accepted cases are not degrees of success; they are different fates, and
/// only one of them is lossy. Text is retained and delivered to every reader
/// that opens later, so words keep. **Voice does not**: a TTS span synthesized with
/// no speaker attached is spent, and the person never learns it happened — the
/// failure `docs/arch/core.md#presence` exists to prevent. `TooLong` is rejected
/// before it reaches any output channel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Spoken {
    /// The call was rejected because it exceeded the loose per-call limit.
    TooLong,
    /// Heard aloud and on screen: a speaker is attached.
    Voiced,
    /// On screen only — no speaker, so nothing was synthesized. Not a failure: the
    /// words landed in the channel they're actually on.
    TextOnly,
    /// Nothing is attached at all. The words are buffered and will land whenever
    /// they next open a window; no voice was spent on the empty room.
    Held,
}

impl Spoken {
    /// The literal `say` returns. Written as a plain statement of what happened,
    /// because it is read by a model deciding what to do next — not a status code.
    pub fn ack(self) -> &'static str {
        match self {
            Spoken::TooLong => "too_long — split this into shorter say calls",
            Spoken::Voiced => "said aloud, and on their screen",
            Spoken::TextOnly => {
                "on their screen — not said aloud, because no speaker is attached right now"
            }
            Spoken::Held => {
                "nobody is connected — nothing was said aloud, and the words are waiting for \
                 them and will show the moment they open a window"
            }
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

    /// Speak `text` (the `say` tool): queue it onto the output sequencer,
    /// which paces it to TTS. Acks immediately — never waits on synthesis.
    ///
    /// The returned [`Spoken`] is read against presence *here*, at emission, rather
    /// than left to the turn's rendered snapshot: a turn can outlive the window that
    /// started it, and the whole point of speech being a call is that the answer is
    /// true at the moment it is given.
    pub async fn say(&self, text: String) -> anyhow::Result<Spoken> {
        let mouth = self
            .mouth
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("this rung has no voice; there is nowhere to say it"))?;
        if text.chars().count() > SAY_MAX_CHARS {
            return Ok(Spoken::TooLong);
        }
        let reach = mouth.presence.reachable();
        mouth
            .beats
            .send(Beat::Say(text))
            .await
            .map_err(|_| anyhow::anyhow!("sequencer gone; say dropped"))?;
        Ok(match (reach.speaker, reach.window) {
            (true, _) => Spoken::Voiced,
            (false, true) => Spoken::TextOnly,
            (false, false) => Spoken::Held,
        })
    }

    /// Show a view (the `show` tool): queue it onto the sequencer, which
    /// paces it to the surrounding narration. `op` is `show`/`replace`/`dismiss`;
    /// `id` may be omitted (one is synthesized). `traits` is what the view declared
    /// about itself (or `None` — host-owned captions).
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
    ) -> anyhow::Result<()> {
        self.mouth
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("this rung has no screen; there is nowhere to show it"))?
            .beats
            .send(Beat::Show { id, op, source, traits })
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
    use crate::body::presence::{OutChannel, Presence};

    /// A mouth wired to a live receiver, so `send` succeeds and the outcome
    /// under test is the presence read rather than a closed channel.
    fn mouth(presence: &Presence) -> (ToolSink, mpsc::Receiver<Beat>) {
        let (beats, rx) = mpsc::channel(8);
        let (control, _ctl) = mpsc::channel(8);
        let sink = ToolSink {
            control,
            mouth: Some(Mouth { beats, presence: presence.clone() }),
        };
        (sink, rx)
    }

    #[tokio::test]
    async fn a_speaker_makes_it_voiced() {
        let p = Presence::new();
        let _audio = p.connect(OutChannel::Audio);
        let (sink, _rx) = mouth(&p);
        assert_eq!(sink.say("hi".into()).await.unwrap(), Spoken::Voiced);
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
    async fn a_window_without_a_speaker_is_text_only() {
        let p = Presence::new();
        let _view = p.connect(OutChannel::View);
        let (sink, _rx) = mouth(&p);
        assert_eq!(sink.say("hi".into()).await.unwrap(), Spoken::TextOnly);
    }

    #[tokio::test]
    async fn an_empty_room_holds_it() {
        let p = Presence::new();
        let (sink, _rx) = mouth(&p);
        assert_eq!(sink.say("hi".into()).await.unwrap(), Spoken::Held);
    }

    #[tokio::test]
    async fn an_overlong_say_is_rejected_without_emission() {
        let p = Presence::new();
        let (sink, mut rx) = mouth(&p);
        let text = "x".repeat(SAY_MAX_CHARS + 1);

        assert_eq!(sink.say(text).await.unwrap(), Spoken::TooLong);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn the_words_are_emitted_whatever_the_room() {
        // The gate is about voice, never about dropping the utterance: text is
        // retained and keeps, so the beat goes out even to nobody.
        let p = Presence::new();
        let (sink, mut rx) = mouth(&p);
        sink.say("hi".into()).await.unwrap();
        assert!(matches!(rx.try_recv(), Ok(Beat::Say(t)) if t == "hi"));
    }

    #[tokio::test]
    async fn a_rung_with_no_mouth_cannot_say() {
        let (control, _ctl) = mpsc::channel(8);
        let sink = ToolSink { control, mouth: None };
        assert!(sink.say("hi".into()).await.is_err());
    }

    #[test]
    fn every_outcome_says_what_happened() {
        // The ack is read by a model, so it must be a sentence about the world —
        // and the outcomes must not read alike, or the answer carries no information.
        let acks = [
            Spoken::TooLong.ack(),
            Spoken::Voiced.ack(),
            Spoken::TextOnly.ack(),
            Spoken::Held.ack(),
        ];
        for a in acks {
            assert!(a.len() > 10, "an ack must state what happened: {a:?}");
        }
        assert_eq!(acks.iter().collect::<std::collections::HashSet<_>>().len(), 4);
    }
}
