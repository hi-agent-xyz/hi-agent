//! Reflection — the inward brain. One of it for the whole agent.
//!
//! **Cognition and Reflection are the same kind of thing.** Both are sceneless, both are
//! as capable as the agent gets, both dispatch workers, neither speaks. What separates
//! them is not intelligence and not machinery — it is **who the work is for**:
//!
//! - **Cognition faces outward.** Its work arrives from a person, through a scene, and
//!   its answers go back that way. It owns the task ledger because a duty is something
//!   owed to someone.
//! - **Reflection faces inward.** Its work is the agent's own house: what it remembers,
//!   who it has met, what it has learnt, what it is still carrying that it should not
//!   be. Nobody asked for any of it, which is exactly why it needs a rung of its own —
//!   work nobody is waiting on is work that never happens if it has to compete with
//!   work someone is.
//!
//! That is the whole of the split. An earlier draft cast this rung as a *curator* beside
//! a *brain*, and the shape followed the words: a one-shot pass that opened a session,
//! prompted it once, and dropped it. It could dispatch a worker and then could not read
//! the report, because the session that asked was already gone. This module is that
//! correction — Reflection gets Cognition's shape, for Cognition's reasons.
//!
//! ## What a prompt could not be
//!
//! `identity/reflection.md` is still most of what makes Reflection *Reflection*. Four
//! things it cannot be, and each was a way for the rung to exist and not work:
//!
//! 1. **An address that lasts.** The registration was created *inside* the pass and
//!    dropped with it, so between passes Reflection was unreachable and during one its id
//!    was nobody's to know. It now registers once, for the life of the process.
//!
//!    **That its id is not projected to anyone is correct, not a gap.** Nothing addresses
//!    Reflection: no prompt names it as a recipient, and it wakes on its own backoff clock,
//!    not on mail. The registration exists so that a rung *has* a stable identity — it is
//!    not a promise that someone will write to it. This keeps being re-filed as "Reflection
//!    is unreachable"; it is unreachable the way a room with no door is under-furnished.
//! 2. **A drain.** Nothing read its inbox — the note this replaces said so outright. A
//!    registered rung nobody reads is a mailbox that answers "delivered" and forgets.
//! 3. **A host for its workers.** Without a sink of its own, `create_worker` fell through
//!    to `any_host()`, which lent an arbitrary live conversation: the borrowed scene
//!    became the worker's `X-HI-Scene` (so `watch` resolved to a stranger's camera), the
//!    `{scene}` in its prompt, and the scene its report was journaled under — feeding
//!    *that* scene's episodes with work it never asked for. That function is gone as of
//!    this commit, because this was its last caller.
//! 4. **Two kinds of wake.** Reflection is the one rung driven by a clock *and* by mail.
//!    The clock is its own — an adaptive backoff pacing a loop inside this subsystem,
//!    which `docs/arch/core.md` is explicit is not what the (deferred) global clock is
//!    for.
//!
//! ## Sessions per wake, registration per process
//!
//! Same rule as Cognition, for the same reason: an address must be stable, a session must
//! be disposable (`docs/arch/core.md` — *"continuity lives in `data/`, not in a
//! process"*). A consolidation pass and a mail turn each open one and drop it.
//!
//! One red line survives from the old wording and is not negotiable: reflection may
//! **archive verbatim and write pointers, never paraphrase stored bytes**.

use tokio::time::Instant;

use tokio::sync::mpsc;

use crate::foundation::registry::{self, Registration};
use crate::types::Scene;

use super::tools::{SceneControl, ToolSink};
use super::{LoopInput, Reaction, SCENE_QUEUE_CAPACITY, heartbeat, workers};

/// The agent name for [`crate::mind::memory::layout::agent_prompt_path`] — what
/// Reflection carries forward between wakes, at `memory/prompts/reflection.md`.
///
/// It has one for the same reason Cognition does: a per-wake session remembers nothing,
/// so whatever is worth carrying has to be written down. For the inward rung that is
/// mostly *where it had got to* — which stores it has swept recently, what it deferred,
/// what it noticed and could not act on yet.
const REFLECTION_AGENT: &str = "reflection";

/// Stand Reflection up. Called once from [`super::start`], which creates the
/// `Registration` **synchronously** before spawning this, for the reason
/// [`super::cognition::spawn`] gives: `tokio::spawn` promises no ordering, and an address
/// named in a prompt that resolves to nothing is worse than one that is plainly absent.
pub(super) fn spawn(reaction: Reaction, registration: Registration) {
    tokio::spawn(async move {
        run(reaction, registration).await;
    });
}

/// Why this loop woke. Two sources, and they do different work — which is the one real
/// difference from Cognition's loop, where every wake is a turn.
enum Wake {
    /// The backoff clock came due: sweep the recently-active scenes.
    Consolidate,
    /// Mail, or a worker of ours reporting. Drives an ordinary turn.
    Turn,
}

async fn run(reaction: Reaction, registration: Registration) {
    let id = registration.id();
    let mail = registration.mail.clone();
    let scene = heartbeat::consolidation_scene();

    // Its workers run under it — see the module note's point 3. `beats: None`: Reflection
    // has no floor, no sequencer, no screen. It never speaks, and that is now a fact
    // about its sink rather than an agreement between a tool list and a role check.
    let (control_tx, mut control_rx) = mpsc::channel::<SceneControl>(SCENE_QUEUE_CAPACITY);
    let (report_tx, mut report_rx) = mpsc::channel::<LoopInput>(SCENE_QUEUE_CAPACITY);
    reaction
        .inner
        .tools
        .register(scene.clone(), ToolSink { control: control_tx, mouth: None })
        .await;

    let mut workers = workers::WorkerRegistry::new(scene.clone(), report_tx);
    let mut pending: Vec<String> = Vec::new();

    // The clock. Unchanged in behaviour from the free-standing loop this replaces:
    // anchored on the last completed pass, base cadence while anything saw fresh input,
    // doubling toward the cap while the whole system is quiet.
    let reflect_base = super::reflect_interval();
    let reflect_max = super::reflect_max_interval();
    let clock_on = reflect_base.is_some();
    if !clock_on {
        // Disabled by `reflect=off`. The loop still runs: the rung stays addressable and
        // its workers still report, which is the half that has nothing to do with
        // consolidation cadence. Previously this returned outright, taking the address
        // down with the clock.
        tracing::info!(reflection = id, "consolidation cadence disabled; rung stays addressable");
    }
    let loop_started = Instant::now();
    let mut last_reflection: Option<Instant> = None;
    let mut backoff_gap = reflect_base.unwrap_or(super::DEFAULT_REFLECT_EVERY);
    let mut energy = crate::foundation::energy_state::subscribe();
    let mut energy_paused = crate::foundation::energy_state::is_out();

    tracing::info!(reflection = id, "reflection up");

    loop {
        // Reflection does not preflight the account. A managed 402 from any ACP
        // session flips the shared state; this rung simply parks until the positive
        // balance sends Resume, keeping its pending mail intact.
        if energy_paused {
            tokio::select! {
                event = energy.recv() => {
                    match event {
                        Ok(crate::foundation::energy_state::EnergyEvent::Resume) => {
                            energy_paused = false;
                            tracing::info!(reflection = id, "reflection resumed after energy refill");
                        }
                        Ok(crate::foundation::energy_state::EnergyEvent::Pause) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            energy_paused = crate::foundation::energy_state::is_out();
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = reaction.inner.shutdown.cancelled() => {
                    tracing::info!(reflection = id, "reflection shutting down");
                    break;
                }
            }
            continue;
        }

        // When the clock is off, park the timer arm forever rather than branching the
        // whole `select!` — a never-completing sleep is the cheapest "this arm is not in
        // play" there is.
        let last_activity = *reaction.inner.last_signal_at.lock().unwrap();
        let due = if clock_on {
            super::next_reflection_at(
                loop_started,
                last_activity,
                last_reflection,
                reflect_base,
                backoff_gap,
            )
        } else {
            None
        };
        // An hour rather than `Duration::MAX` when the clock is off: tokio's timer has a
        // finite horizon, and a bounded re-loop costs one wake an hour to stay obviously
        // within it. The arm does nothing when it fires — `due` is still `None`.
        let sleep_for = due
            .map(|at| at.saturating_duration_since(Instant::now()))
            .unwrap_or(std::time::Duration::from_secs(3600));

        let wake = tokio::select! {
            biased;
            event = energy.recv() => {
                match event {
                    Ok(crate::foundation::energy_state::EnergyEvent::Pause) => {
                        energy_paused = true;
                    }
                    Ok(crate::foundation::energy_state::EnergyEvent::Resume) => {
                        energy_paused = false;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        energy_paused = crate::foundation::energy_state::is_out();
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
                continue;
            }
            _ = tokio::time::sleep(sleep_for) => Wake::Consolidate,
            // Fresh input landed: re-derive the deadline rather than sit out a long
            // backoff. Not a wake in itself.
            _ = reaction.inner.reflect_wake.notified() => continue,
            _ = mail.notified() => Wake::Turn,
            ctl = control_rx.recv() => {
                match ctl {
                    Some(SceneControl::CreateWorker { id: worker, task, kind, owner }) => {
                        if let Err(err) =
                            workers.spawn_with_id(&reaction, worker, task, kind, owner).await
                        {
                            tracing::warn!(error = %err, "reflection failed to create a worker");
                        }
                        continue;
                    }
                    None => break,
                }
            }
            report = report_rx.recv() => {
                match report {
                    // A worker Reflection created has finished. Reflection is its owner,
                    // so there is nowhere further up: the report becomes the next turn's
                    // input. This is the edge that did not exist before — the pass that
                    // dispatched was already gone by the time anything came back.
                    Some(LoopInput::Worker(r)) => {
                        pending.push(workers::render_report_plainly(&r));
                        Wake::Turn
                    }
                    Some(_) => continue,
                    None => break,
                }
            }
            _ = reaction.inner.shutdown.cancelled() => {
                tracing::info!(reflection = id, "reflection shutting down");
                break;
            }
        };

        if reaction.inner.shutdown.is_triggered() {
            tracing::info!(reflection = id, "shutdown requested; ending reflection loop");
            break;
        }

        match wake {
            Wake::Consolidate => {
                // Adapt the backoff against the *old* anchor before re-anchoring: fresh
                // input since the last pass snaps the gap back to base; a quiet pass
                // doubles it toward the cap. Re-anchor whether or not anything actually
                // consolidates, so a no-op tick cannot hot-spin the clock.
                let now = Instant::now();
                let last_activity = *reaction.inner.last_signal_at.lock().unwrap();
                let anchor = last_reflection.unwrap_or(loop_started);
                backoff_gap = if last_activity > anchor {
                    reflect_base.unwrap_or(super::DEFAULT_REFLECT_EVERY)
                } else {
                    backoff_gap.checked_mul(2).unwrap_or(reflect_max).min(reflect_max)
                };
                last_reflection = Some(now);

                workers.reap();
                let scenes =
                    super::recent_scenes(reaction.inner.memory.data_dir(), super::REWARM_WINDOW);
                heartbeat::consolidate(&reaction, &scenes, id).await;
            }
            Wake::Turn => {}
        }

        // Drain whatever accumulated, whichever way we woke — a consolidation pass is
        // long, and mail that arrived during it is owed a turn immediately after.
        if let Some(batch) = registry::global().take_pending(id) {
            pending.push(registry::render(&batch));
        }
        if pending.is_empty() {
            continue;
        }

        workers.reap();
        match turn(&reaction, id, &scene, &pending).await {
            Ok(()) => pending.clear(),
            Err(err) => {
                // Keep `pending` — the mail is still owed, and the next wake carries it.
                if crate::foundation::energy_state::is_402_error(&err)
                    && crate::foundation::energy_state::is_out()
                {
                    energy_paused = true;
                }
                tracing::warn!(reflection = id, error = %err, "reflection turn failed; mail held");
            }
        }
        registry::global().finish_turn(id);
    }
}

/// One mail wake: open a session, prompt it once, drop it.
///
/// Distinct from a consolidation pass, which has a whole prompt of its own built from
/// every scene's frontier ([`heartbeat::consolidate`]). This is the ordinary path — a
/// worker reported, or another rung sent something — and it carries the projected window
/// rather than a frontier.
async fn turn(
    reaction: &Reaction,
    id: registry::SessionId,
    scene: &Scene,
    pending: &[String],
) -> anyhow::Result<()> {
    use std::sync::Arc;

    use crate::foundation::acp::{SessionOpts, SessionUpdate};
    use crate::foundation::agent::SessionRole;
    use crate::foundation::observatory::{EventKind, SessionKind};
    use crate::mind::memory::snapshot;

    let data_dir = reaction.inner.memory.data_dir();
    // One file, whole — see `cognition.rs` for why the seed went.
    let system_prompt = crate::identity::reflection_prompt(data_dir).await;

    let session = Arc::new(
        reaction
            .inner
            .agent
            .session(
                scene,
                SessionRole::Reflection,
                Some(id),
                SessionOpts {
                    system_prompt: Some(system_prompt),
                    // The data dir: its whole job is the tree under it.
                    cwd: Some(data_dir.to_path_buf()),
                    builtin_tools: None,
                },
            )
            .await?,
    );

    reaction
        .inner
        .observatory
        .record(
            // Sceneless: the sentinel is a routing tag, not a conversation. Passing it
            // would put a room nobody is in on the dashboard — `15391e9`.
            None,
            EventKind::SessionOpened {
                kind: SessionKind::Reflection,
                id: session.id().0.to_string(),
            },
        )
        .await;

    let window = snapshot::agent_window(&reaction.inner.memory, REFLECTION_AGENT, id).await;
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
            registry::global().record_output(id, &text);
        }
    }
    run.wait().await?;

    tracing::info!(reflection = id, reply_chars = full.chars().count(), "reflection turn done");
    Ok(())
}
