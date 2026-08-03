//! The bridge between the MCP tool server and each scene's reactor loop.
//!
//! The mind (and its workers) express side-effects as MCP tool calls over the
//! `/mcp` HTTP endpoint (see [`crate::foundation::mcp`]). Those calls arrive on a different
//! task than the per-scene loop, so they cannot touch the loop's private state
//! directly. Instead each scene registers a [`ToolSink`] — a control-channel
//! sender — into a shared [`ToolRegistry`] keyed by scene. The MCP handler looks
//! the sink up by the call's `X-HI-Scene` header and forwards a [`SceneControl`]
//! the loop applies on its own turn, so worker-registry and alarm state stay
//! owned by the loop with no locking.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};

use crate::types::{Geometry, Scene};

use super::sequencer::Beat;

/// One command the MCP tool server routes to a scene's reactor loop.
///
/// Once there were four: two for dispatching work and two for a worker to reach the
/// voice. The reaching ones are gone — a worker addresses its owner with the one verb
/// now, through the switchboard, which needs no per-scene channel because it is not
/// per-scene. What is left here is what genuinely belongs to *this scene's loop*
/// because the loop owns the state it touches.
#[derive(Debug)]
pub enum SceneControl {
    /// Start a working session for `task` (the `create_worker` tool), owned by the
    /// session that asked.
    ///
    /// Creating a worker is the caller's decision but the loop's bookkeeping — the
    /// live-session map is the loop's own state, so this crosses on the control
    /// channel like everything else that touches it. `owner` is who the finished work
    /// answers to; a worker belongs to the session that created it, never to the scene
    /// it happens to run in.
    CreateWorker { id: u64, task: String, kind: crate::identity::WorkerType, owner: Option<u64> },
    /// Schedule a self-wake after `delay` (e.g. `30s`, `20m`, `1h`) carrying
    /// `note` (the `alarm` tool). The delay is parsed loop-side; an unparseable
    /// one is dropped.
    Alarm { delay: String, note: String },
}

/// Per-scene handle the MCP handler dispatches to. Cheap to clone. Carries two
/// senders: `control` for loop-applied side-effects (the alarm), and
/// `mouth` for output (say/show_view) that the scene's sequencer renders directly
/// — output bypasses the turn loop so it streams while the prompt is still
/// running.
#[derive(Clone)]
pub struct ToolSink {
    pub(super) control: mpsc::Sender<SceneControl>,
    /// Where expression goes — **`None` for a rung with no mouth.**
    ///
    /// Only a scene has somewhere for speech to go. Cognition registers a sink so its
    /// workers have a home, and it has no sequencer, no audio, no screen; expressing
    /// there is not "blocked", it is undefined. Making that an `Option` states it once
    /// in the type instead of leaving it to two guards elsewhere agreeing — the tool
    /// list and the role check at dispatch — which is the kind of arrangement that
    /// holds until someone adds a third caller.
    pub(super) mouth: Option<Mouth>,
}

/// One scene's outbound half: the sequencer to emit onto, plus the presence read
/// that decides what an emission can actually reach.
///
/// Presence lives here rather than being looked up at the call site because the two
/// are the same fact — a mouth *is* a scene's channels — and because the answer is
/// needed at the instant of emission, not at the instant the sink was built.
#[derive(Clone)]
pub(super) struct Mouth {
    pub(super) beats: mpsc::Sender<Beat>,
    pub(super) presence: crate::body::presence::Presence,
    pub(super) scene: Scene,
}

/// What became of an utterance — the answer `say` hands back to Reaction.
///
/// The three cases are not degrees of success; they are different fates, and only
/// one of them is lossy. Text is buffered per scene and delivered to a reader that
/// opens later, so words keep. **Voice does not**: a TTS span synthesized with no
/// speaker attached is spent, and the person never learns it happened — the failure
/// `docs/arch/core.md#presence` exists to prevent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Spoken {
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
    /// Forward one control command to the scene loop. Returns an error only if
    /// the loop is gone (channel closed).
    pub async fn send(&self, control: SceneControl) -> anyhow::Result<()> {
        self.control
            .send(control)
            .await
            .map_err(|_| anyhow::anyhow!("scene loop gone; control dropped"))
    }

    /// Speak `text` (the `say` tool): queue it onto the scene's output sequencer,
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
        let reach = mouth.presence.reachable(&mouth.scene);
        mouth
            .beats
            .send(Beat::Say(text))
            .await
            .map_err(|_| anyhow::anyhow!("scene sequencer gone; say dropped"))?;
        Ok(match (reach.speaker, reach.window) {
            (true, _) => Spoken::Voiced,
            (false, true) => Spoken::TextOnly,
            (false, false) => Spoken::Held,
        })
    }

    /// Show a view (the `show_view` tool): queue it onto the sequencer, which
    /// paces it to the surrounding narration. `op` is `show`/`replace`/`dismiss`;
    /// `id` may be omitted (one is synthesized). `geometry` is the view's declared
    /// placement (or `None` for the host's floor layout).
    ///
    /// Unlike speech this is never gated: a view is retained scene state, folded and
    /// replayed to whatever connects next (and restored across restarts), so showing
    /// into an empty room costs nothing and is waiting when they arrive.
    pub async fn show_view(
        &self,
        id: Option<String>,
        op: String,
        source: String,
        geometry: Option<Geometry>,
    ) -> anyhow::Result<()> {
        self.mouth
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("this rung has no screen; there is nowhere to show it"))?
            .beats
            .send(Beat::Show { id, op, source, geometry })
            .await
            .map_err(|_| anyhow::anyhow!("scene sequencer gone; show_view dropped"))
    }
}

/// Shared scene→sink table. Created once in `lib.rs`, shared (cloneable handle)
/// between the HTTP front's `/mcp` handler and the reactor that registers sinks.
#[derive(Clone, Default)]
pub struct ToolRegistry {
    inner: Arc<Mutex<HashMap<Scene, ToolSink>>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register (or replace) a scene's sink. Called when the per-scene loop is
    /// created, before its session opens and can issue any tool call.
    pub async fn register(&self, scene: Scene, sink: ToolSink) {
        self.inner.lock().await.insert(scene, sink);
    }

    /// Look a scene's sink up by its `X-HI-Scene` header. `None` if no loop is
    /// registered for it (e.g. a stale or unknown scene).
    pub async fn get(&self, scene: &Scene) -> Option<ToolSink> {
        self.inner.lock().await.get(scene).cloned()
    }

    // `any_host()` used to live here: "any live scene loop, for work that must run
    // somewhere but belongs to nobody's conversation". It is **deleted**, not moved.
    //
    // It existed because a sceneless rung creating a worker had nowhere to run it, so it
    // borrowed the lowest-named live scene. Borrowing was never only hosting: the lent
    // scene became the worker's `X-HI-Scene` (so `watch`/`see` resolved to a stranger's
    // camera), the `{scene}` in its prompt, and the scene its report was journaled under
    // — which then fed *that* scene's episodes with work it never asked for. The doc
    // comment claimed the scene was not told; it was told three ways.
    //
    // Both callers have their own sink now — Cognition under `*cognition*`, Reflection
    // under `*consolidation*` — so `registry.get(scene)` succeeds for each and the
    // fallback had no remaining caller. A rung that dispatches work hosts its own
    // workers; that is the whole rule, and it needs no fallback.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::presence::{OutChannel, Presence};

    /// A scene mouth wired to a live receiver, so `send` succeeds and the outcome
    /// under test is the presence read rather than a closed channel.
    fn mouth(presence: &Presence, scene: &Scene) -> (ToolSink, mpsc::Receiver<Beat>) {
        let (beats, rx) = mpsc::channel(8);
        let (control, _ctl) = mpsc::channel(8);
        let sink = ToolSink {
            control,
            mouth: Some(Mouth { beats, presence: presence.clone(), scene: scene.clone() }),
        };
        (sink, rx)
    }

    #[tokio::test]
    async fn a_speaker_makes_it_voiced() {
        let p = Presence::new();
        let s = Scene("boss".to_owned());
        let _audio = p.connect(&s, OutChannel::Audio);
        let (sink, _rx) = mouth(&p, &s);
        assert_eq!(sink.say("hi".into()).await.unwrap(), Spoken::Voiced);
    }

    #[tokio::test]
    async fn a_window_without_a_speaker_is_text_only() {
        let p = Presence::new();
        let s = Scene("boss".to_owned());
        let _view = p.connect(&s, OutChannel::View);
        let (sink, _rx) = mouth(&p, &s);
        assert_eq!(sink.say("hi".into()).await.unwrap(), Spoken::TextOnly);
    }

    #[tokio::test]
    async fn an_empty_room_holds_it() {
        let p = Presence::new();
        let s = Scene("boss".to_owned());
        let (sink, _rx) = mouth(&p, &s);
        assert_eq!(sink.say("hi".into()).await.unwrap(), Spoken::Held);
    }

    #[tokio::test]
    async fn the_words_are_emitted_whatever_the_room() {
        // The gate is about voice, never about dropping the utterance: text is
        // buffered per scene and keeps, so the beat goes out even to nobody.
        let p = Presence::new();
        let s = Scene("boss".to_owned());
        let (sink, mut rx) = mouth(&p, &s);
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
        // and the three must not read alike, or the answer carries no information.
        let acks =
            [Spoken::Voiced.ack(), Spoken::TextOnly.ack(), Spoken::Held.ack()];
        for a in acks {
            assert!(a.len() > 10, "an ack must state what happened: {a:?}");
        }
        assert_eq!(acks.iter().collect::<std::collections::HashSet<_>>().len(), 3);
    }
}
