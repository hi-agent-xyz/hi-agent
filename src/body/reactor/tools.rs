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
    CreateWorker { id: u64, task: String, owner: Option<u64> },
    /// Schedule a self-wake after `delay` (e.g. `30s`, `20m`, `1h`) carrying
    /// `note` (the `alarm` tool). The delay is parsed loop-side; an unparseable
    /// one is dropped.
    Alarm { delay: String, note: String },
}

/// Per-scene handle the MCP handler dispatches to. Cheap to clone. Carries two
/// senders: `control` for loop-applied side-effects (the alarm), and
/// `beats` for output (say/show_view) that the scene's sequencer renders directly
/// — output bypasses the turn loop so it streams while the prompt is still
/// running.
#[derive(Clone)]
pub struct ToolSink {
    pub(super) control: mpsc::Sender<SceneControl>,
    pub(super) beats: mpsc::Sender<Beat>,
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
    pub async fn say(&self, text: String) -> anyhow::Result<()> {
        self.beats
            .send(Beat::Say(text))
            .await
            .map_err(|_| anyhow::anyhow!("scene sequencer gone; say dropped"))
    }

    /// Show a view (the `show_view` tool): queue it onto the sequencer, which
    /// paces it to the surrounding narration. `op` is `show`/`replace`/`dismiss`;
    /// `id` may be omitted (one is synthesized). `geometry` is the view's declared
    /// placement (or `None` for the host's floor layout).
    pub async fn show_view(
        &self,
        id: Option<String>,
        op: String,
        source: String,
        geometry: Option<Geometry>,
    ) -> anyhow::Result<()> {
        self.beats
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

    /// Any live scene loop, for work that must *run* somewhere but belongs to nobody's
    /// conversation.
    ///
    /// A worker belongs to the session that created it, and a sceneless rung —
    /// Reflection now, Cognition next — creates workers with no conversation to put
    /// them in. Its worker still needs a host: a loop to hold its handle and reap it.
    /// So one is borrowed. This is **hosting, not ownership**: the report goes to the
    /// owner, never into the borrowed scene, and the scene is not told.
    ///
    /// Chosen deterministically (lowest scene name) rather than arbitrarily, so a run
    /// is reproducible and a log names the same host twice.
    pub async fn any_host(&self) -> Option<(Scene, ToolSink)> {
        let map = self.inner.lock().await;
        let scene = map.keys().min().cloned()?;
        let sink = map.get(&scene).cloned()?;
        Some((scene, sink))
    }
}
