//! Cognition — the shared brain. One of it for the whole agent.
//!
//! It belongs to no scene, and every scene hands work up to it. It owns the task ledger,
//! it is the only thing that creates workers, and it tries hard to stay idle so it is
//! free the moment something arrives. It never speaks: what it wants said it sends to a
//! scene, where Reaction decides when the room is right.
//!
//! Most of what makes Cognition *Cognition* is its prompt (`identity/cognition.md`) —
//! "a new role is a new prompt, not new machinery". This module is the four things a
//! prompt cannot be:
//!
//! 1. **An address.** [`registry::Address::Rung`] resolves to the session with a role
//!    and no scene; this is what registers one.
//! 2. **A drain.** Every live agent is driven by something. Reaction has its scene loop,
//!    workers have `drive_worker`, Reflection has a one-shot pass. A registered rung with
//!    nothing reading its inbox is a mailbox that reports "delivered" and forgets.
//! 3. **A window.** Invariant 4: open tasks are *projected, not retrieved*. The ledger's
//!    own writer going to look for it is how a duty goes missing without anyone knowing.
//! 4. **A host for its workers.** `create_worker` needs somewhere to run them; without
//!    this, a sceneless rung borrows an arbitrary scene and quietly files its work under
//!    a stranger's conversation.
//!
//! ## The registration outlives the session, deliberately
//!
//! The address is registered **once, for the life of the process**, and the ACP session
//! is opened **per wake** and dropped when the turn ends. Two different lifetimes for two
//! different things: an address must be stable (nothing durable may hold a session id,
//! and a prompt certainly cannot), while a session is disposable — `docs/arch/core.md`:
//! *"Sessions are host-owned and disposable. Continuity lives in `data/`, not in a
//! process."*
//!
//! This is not a style preference. A long-lived session that dies mid-turn — subprocess
//! crash, vendor outage, transport reset — leaves a handle that fails every subsequent
//! prompt, and Cognition has nothing above it to notice. `drive_worker` has exactly this
//! shape and gets away with it because a worker's TTL closes it; the process-lifetime
//! singleton would just wedge, accepting mail and failing silently. Per-wake makes that
//! state unrepresentable. It also means context rot has nowhere to accumulate, so the
//! unbuilt session-swap (N5) is not a dependency, and it forces continuity through the
//! ledger and `cognition.md` — where the design says continuity lives — rather than
//! hiding it in a conversation nobody can read back.
//!
//! The cost, stated: a subprocess spawn per wake, and no memory of the last wake except
//! what was written down. At "minutes and beyond" the spawn is noise, and the second half
//! is the point rather than the price.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::foundation::acp::{SessionOpts, SessionUpdate};
use crate::foundation::agent::SessionRole;
use crate::foundation::observatory::{EventKind, SessionKind};
use crate::foundation::registry::{self, Registration};
use crate::mind::memory::snapshot;
use crate::types::Scene;

use super::tools::{SceneControl, ToolSink};
use super::{LoopInput, Reactor, SCENE_QUEUE_CAPACITY, workers};

/// The pseudo-scene Cognition's sessions are opened under.
///
/// It exists for the two places that require *some* scene and mean different things by
/// it: the `X-HI-Scene` header the `/mcp` dispatch routes by, and the label on a
/// subprocess in the logs. It is never a conversation and never a data path — the `*…*`
/// form marks that ([`Scene::is_pseudo`]), and it cannot collide with a real scene id.
///
/// **Nothing may journal under it.** `encode_scene("*cognition*")` is
/// `%2Acognition%2A`, which is a perfectly ordinary directory name to
/// [`crate::mind::memory::layout::is_scene_dir`] and no longer looks pseudo to anything
/// checking — so a single write into `memory/raw/` would manufacture a scene that gets a
/// full per-scene loop, subprocess and pulse at the next boot. That is the bug `c085e29`
/// fixed arriving by a different road. Cognition's durable record is the ledger, which is
/// what the ledger is for.
const COGNITION_SCENE: &str = "*cognition*";

fn cognition_scene() -> Scene {
    let s = Scene(COGNITION_SCENE.to_string());
    debug_assert!(s.is_pseudo(), "the sentinel must be recognizable as one");
    s
}

/// The agent name for [`crate::mind::memory::layout::agent_prompt_path`] — what
/// Cognition carries forward between wakes, at `memory/prompts/cognition.md`.
const COGNITION_AGENT: &str = "cognition";

/// Stand Cognition up. Called once from [`super::start`], which creates the
/// `Registration` **synchronously** before spawning this — `tokio::spawn` ordering is not
/// guaranteed, and registering inside the task would leave a window at boot where the
/// address exists in a prompt and resolves to nothing.
pub(super) fn spawn(reactor: Reactor, registration: Registration) {
    tokio::spawn(async move {
        run(reactor, registration).await;
    });
}

async fn run(reactor: Reactor, registration: Registration) {
    let id = registration.id();
    let mail = registration.mail.clone();
    let scene = cognition_scene();

    // Its workers run under it. `create_worker` looks the caller's scene up in the tool
    // registry, and the caller's header scene *is* `*cognition*` — so registering a sink
    // here means the lookup succeeds and `any_host()`, which would have lent an arbitrary
    // live conversation, is never reached. The per-scene worker map is untouched: this is
    // not that map moving, it is Cognition having its own.
    let (control_tx, mut control_rx) = mpsc::channel::<SceneControl>(SCENE_QUEUE_CAPACITY);
    let (report_tx, mut report_rx) = mpsc::channel::<LoopInput>(SCENE_QUEUE_CAPACITY);
    reactor
        .inner
        .tools
        // `beats: None` — no sequencer, no audio, no screen. Cognition proposes; Reaction
        // voices. That it *cannot* express is now a fact about the sink rather than an
        // agreement between the tool list and the role check at dispatch.
        .register(scene.clone(), ToolSink { control: control_tx, beats: None })
        .await;

    let mut workers = workers::WorkerRegistry::new(scene.clone(), report_tx);

    // Held across turns, cleared only after one succeeds — a turn that fails must not
    // swallow the mail it was carrying. (The scene loop keeps its batch for the same
    // reason; this is that property without the vendor-gate machinery around it, which
    // Cognition does not need: it has no floor to hold and nobody waiting on a reply.)
    let mut pending: Vec<String> = Vec::new();

    tracing::info!(cognition = id, "cognition up");

    loop {
        tokio::select! {
            _ = mail.notified() => {}
            ctl = control_rx.recv() => {
                match ctl {
                    Some(SceneControl::CreateWorker { id: worker, task, owner }) => {
                        if let Err(err) =
                            workers.spawn_with_id(&reactor, worker, task, owner).await
                        {
                            tracing::warn!(error = %err, "cognition failed to create a worker");
                        }
                        continue;
                    }
                    // Alarms are the clock's, and the clock does not exist (N4). A rung
                    // that cannot schedule one is not offered the tool, so this arm is
                    // unreachable rather than merely unused — say so rather than let it
                    // read as handled.
                    Some(SceneControl::Alarm { note, .. }) => {
                        tracing::warn!(note = %note, "cognition has no clock yet; alarm dropped");
                        continue;
                    }
                    None => break,
                }
            }
            report = report_rx.recv() => {
                match report {
                    // A worker Cognition created has finished. Its owner is Cognition, so
                    // there is nowhere further up for this to go: it becomes the next
                    // turn's input directly.
                    Some(LoopInput::Worker(r)) => {
                        pending.push(workers::render_report_plainly(&r));
                    }
                    Some(_) => continue,
                    None => break,
                }
            }
            _ = reactor.inner.shutdown.cancelled() => {
                tracing::info!(cognition = id, "cognition shutting down");
                break;
            }
        }

        // Drain whatever accumulated. A burst is merged by the switchboard into one
        // prompt, so no settle window is needed here.
        if let Some(batch) = registry::global().take_pending(id) {
            for m in batch {
                pending.push(match m.from {
                    Some(from) => format!("(from session {from}) {}", m.text),
                    None => m.text,
                });
            }
        }
        if pending.is_empty() {
            continue;
        }

        workers.reap();

        match turn(&reactor, id, &scene, &pending).await {
            Ok(()) => pending.clear(),
            Err(err) => {
                // Keep `pending` — the mail is still owed. Nothing retries on a timer
                // yet; the next message to arrive carries it along.
                tracing::warn!(cognition = id, error = %err, "cognition turn failed; mail held");
            }
        }
        registry::global().finish_turn(id);
    }
}

/// One wake: open a session, prompt it once, drop it.
///
/// The window goes in as part of the prompt rather than the system prompt because the
/// session is per-wake — there is no long-lived context for it to go stale in, and
/// building it fresh each time is what makes "projected, not retrieved" true rather than
/// true-at-open.
async fn turn(
    reactor: &Reactor,
    id: registry::SessionId,
    scene: &Scene,
    pending: &[String],
) -> anyhow::Result<()> {
    let data_dir = reactor.inner.memory.data_dir();
    let system_prompt = format!(
        "{}\n\n{}",
        crate::identity::character_seed(data_dir),
        crate::identity::cognition_prompt(data_dir).await
    );

    let session = Arc::new(
        reactor
            .inner
            .agent
            .session(
                scene,
                SessionRole::Cognition,
                Some(id),
                SessionOpts {
                    system_prompt: Some(system_prompt),
                    // The data dir: the ledger it writes lives under it, and it has no
                    // view workshop to work in — it delegates the making of things.
                    cwd: Some(data_dir.to_path_buf()),
                    // Left at the adapter's defaults so it can read and write the ledger,
                    // which is a plain facet on disk and needs no tool of its own.
                    //
                    // Worth naming rather than leaving to read as considered-and-fine:
                    // this also hands it a shell, and the whole point of the rung is that
                    // it delegates rather than does. That is guidance in `cognition.md`,
                    // not a rail — `docs/arch/foundation.md` is explicit that tool
                    // surfaces are sized for context, not to fence anyone out.
                    builtin_tools: None,
                },
            )
            .await?,
    );

    reactor
        .inner
        .observatory
        .record(
            // Sceneless: `*cognition*` is a routing tag, not a conversation, and the
            // mirror keys on scene. Passing it would put a room nobody is in on the
            // dashboard.
            None,
            EventKind::SessionOpened {
                kind: SessionKind::Cognition,
                id: session.id().0.to_string(),
            },
        )
        .await;

    let window = snapshot::agent_window(&reactor.inner.memory, COGNITION_AGENT).await;
    let messages = pending.join("\n\n");
    let prompt = if window.trim().is_empty() {
        format!("## New messages\n{messages}")
    } else {
        format!("{}\n\n## New messages\n{messages}", window.trim())
    };

    let mut run = session.prompt(prompt).await?;
    let mut full = String::new();
    while let Some(update) = run.next_update().await {
        if let SessionUpdate::Text(text) = update {
            full.push_str(&text);
            // Mirror it into the switchboard's bounded tail, or `session_messages`
            // answers "nothing yet" forever — and a Deliberation that handed work up has
            // no other way to see what became of it.
            registry::global().record_output(id, &text);
        }
    }
    run.wait().await?;

    tracing::info!(cognition = id, reply_chars = full.chars().count(), "cognition turn done");
    Ok(())
}
