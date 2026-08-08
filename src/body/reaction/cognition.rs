//! Cognition — the shared brain. One of it for the whole agent.
//!
//! It belongs to no conversation, and the conversation hands work up to it. It owns the task ledger,
//! it is the only thing that creates workers, and it tries hard to stay idle so it is
//! free the moment something arrives. It never speaks: what it wants said it sends to a
//! conversation, where Reaction decides when the room is right.
//!
//! Most of what makes Cognition *Cognition* is its prompt (`identity/cognition.md`) —
//! "a new role is a new prompt, not new machinery". This module is the four things a
//! prompt cannot be:
//!
//! 1. **An address.** An address is a session id, and its own is minted fresh each boot —
//!    so it registers once, for the life of the process, and the host projects that id
//!    into the window of everyone allowed to reach it ([`registry::Registry::reachable`]).
//! 2. **A drain.** Every live agent is driven by something. Reaction has its reaction loop,
//!    workers have `drive_worker`, Reflection has a one-shot pass. A registered rung with
//!    nothing reading its inbox is a mailbox that reports "delivered" and forgets.
//! 3. **A window.** Invariant 4: active tasks are *projected, not retrieved*. The ledger's
//!    own writer going to look for it is how a duty goes missing without anyone knowing.
//! 4. **A host for its workers.** `create_worker` needs somewhere to run them; without
//!    this, a standing rung borrows an arbitrary conversation and quietly files its work under
//!    a stranger's conversation.
//!
//! ## One long-lived session; the agent bounds its own context
//!
//! The address is registered **once, for the life of the process**; the ACP session is
//! opened once and **held across wakes**, replaced only when it breaks. Two different lifetimes for two different things: an address
//! must be stable (nothing durable may hold a session id, and a prompt certainly cannot),
//! while a session is *replaceable* — `docs/arch/core.md`: *"No session is a source of
//! truth — continuity lives in `data/`."*
//!
//! **This reverses the per-wake session this rung shipped with, and the reason is a
//! failure that was observed rather than predicted.** Reopening every wake meant Cognition
//! could not remember that it had already done something, and the ledger cannot hand that
//! back: the ledger records what is **owed**, not what has been arranged, tried, or ruled
//! out. Live, it armed a recurring check, forgot it had, woke to its own ledger entry
//! warning that the check was fragile, and deleted it as redundant — every step correct
//! given what it could see. Continuity of *work* is not a fact projection can supply, so
//! the session has to carry it.
//!
//! The original objection stands and is answered rather than dismissed: a long-lived
//! session that dies mid-turn leaves a handle failing every later prompt with nothing
//! above it to notice. So a failed turn **drops the session** (see the turn's error arm),
//! and a wedged one cannot outlive a single turn. Wedged-and-silent stays unrepresentable;
//! the difference is that recovery costs one cold open instead of being the steady state.
//!
//! **Nothing here bounds the session by size**, and that is deliberate: the underlying
//! agent compacts its own context in place. See [`super::heartbeat`]'s module doc for why
//! the character-counting swap that briefly lived here was deleted rather than retuned —
//! in short, it could not see most of the context it claimed to measure, and it threw away
//! exactly the working thread this rung is long-lived in order to keep.
//!
//! The cost, stated: a resident subprocess. What it buys is a rung that knows what it was
//! in the middle of.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};

use crate::foundation::acp::{AcpSession, SessionOpts, SessionUpdate};
use crate::foundation::agent::SessionRole;
use crate::foundation::observatory::{EventKind, SessionKind};
use crate::foundation::registry::{self, Registration};
use crate::mind::memory::snapshot;

use super::tools::{LoopControl, ToolSink};
use super::{LoopInput, Reaction, LOOP_QUEUE_CAPACITY, workers};

/// The pseudo-conversation Cognition's sessions are opened under.
///
/// It exists for the two places that require *some* conversation and mean different things by
/// it: the `X-HI-Conversation` header the `/mcp` dispatch routes by, and the label on a
/// subprocess in the logs. It is never a conversation and never a data path — the `*…*`
/// form marks that ([`Conversation::is_pseudo`]), and it cannot collide with a real conversation id.
///
/// **Nothing may journal under it.** The old percent-encoding of a sentinel id is
/// `%2Acognition%2A`, which is a perfectly ordinary directory name to
/// [`crate::mind::memory::layout::is_signal_dir`] and no longer looks pseudo to anything
/// checking — so a single write into `memory/raw/` would manufacture a conversation that gets a
/// full per-reaction loop, subprocess and pulse at the next boot. That is the bug `c085e29`
/// fixed arriving by a different road. Cognition's durable record is the ledger, which is
/// The agent name for [`crate::mind::memory::layout::agent_prompt_path`] — what
/// Cognition carries forward between wakes, at `memory/prompts/cognition.md`.
const COGNITION_AGENT: &str = "cognition";

/// Cognition's restart-recovery wake is immediate once the reaction exists.
///
/// Runtime provisioning and broker refresh finish before `reaction::start`, and
/// [`glance_note`] explicitly stands every task-owned report conversation up before the
/// recovery prompt is built. Readiness is therefore structural rather than a
/// sleep-and-hope delay.
const BOOT_WAKE_AFTER: Duration = Duration::ZERO;

/// Stand Cognition up. Called once from [`super::start`], which creates the
/// `Registration` **synchronously** before spawning this — `tokio::spawn` ordering is not
/// guaranteed, and registering inside the task would leave a window at boot where the
/// address exists in a prompt and resolves to nothing.
pub(super) fn spawn(reaction: Reaction, registration: Registration) {
    tokio::spawn(async move {
        run(reaction, registration).await;
    });
}

async fn run(reaction: Reaction, registration: Registration) {
    let id = registration.id();
    let mail = registration.mail.clone();

    // Its workers run under it. `create_worker` reaches the tool registry, so
    // registering a sink here means the lookup succeeds and `any_host()`, which would
    // have lent an arbitrary
    // live conversation, is never reached. The per-conversation worker map is untouched: this is
    // not that map moving, it is Cognition having its own.
    let (control_tx, mut control_rx) = mpsc::channel::<LoopControl>(LOOP_QUEUE_CAPACITY);
    let (report_tx, mut report_rx) = mpsc::channel::<LoopInput>(LOOP_QUEUE_CAPACITY);
    reaction
        .inner
        .tools
        // `mouth: None` — no sequencer, no audio, no screen. Cognition proposes; Reaction
        // voices. That it *cannot* express is now a fact about the sink rather than an
        // agreement between the tool list and the role check at dispatch.
        .register(ToolSink { control: control_tx, mouth: None })
        .await;

    let mut workers = workers::WorkerRegistry::new(report_tx);

    // Held across turns, cleared only after one succeeds — a turn that fails must not
    // swallow the mail it was carrying. (The reaction loop keeps its batch for the same
    // reason; this is that property without the vendor-gate machinery around it, which
    // Cognition does not need: it has no floor to hold and nobody waiting on a reply.)
    let mut pending: Vec<String> = Vec::new();

    // Cognition's own wake. Until this existed the rung that **owns the ledger** was
    // woken only by mail, while the pulse woke the rungs that cannot read it — so a
    // standing duty survived a restart on disk and nothing ever picked it back up.
    // `docs/arch/agents.md` has always specified the recovery sequence ("the glance-up
    // fires → Cognition wakes → reads active tasks → …"); this is the half of it that was
    // missing. It is **not** a scheduler and never becomes one: a clock module was
    // designed, deferred, and then declined outright — scheduling past this cadence is
    // the agent's own, built with the shell it already has
    // (`docs/arch/core.md#glancing-up--and-why-there-is-no-clock`).
    //
    // Two wakes, deliberately different things: the **boot** one fires once because a
    // restart happened, and the **recurring** one fires into idleness because a duty
    // can die quietly at any time. `last_turn` resets on every turn, so the second is
    // a quiet-moment glance rather than a metronome — the conversation pulse's shape.
    let started = Instant::now();
    let mut last_turn = started;
    let mut woke_at_boot = false;

    // **One session, held across wakes** (`docs/arch/agents.md#session-lifetime-per-rung`).
    // It used to be opened per wake and dropped, which made this rung unable to remember
    // that it had already done something — and the ledger cannot hand that back, because
    // the ledger records what is *owed*, not what has been arranged, tried, or ruled out.
    // The observed failure: it armed a timer, forgot it had, woke to a ledger entry saying
    // that timer was fragile, and deleted it as redundant.
    //
    // Startup opens and primes this eagerly once `/mcp` is live. `None` remains the
    // cold-open fallback when warming failed or a later turn discarded the session.
    let mut session: Option<Arc<AcpSession>> = None;
    let mut energy = crate::foundation::energy_state::subscribe();
    let mut energy_paused = crate::foundation::energy_state::is_out();

    tracing::info!(cognition = id, "cognition up");
    if reaction.wait_for_server_ready().await {
        warm_session(&reaction, id, &mut session).await;
    }

    loop {
        // The rung never polls the account and never predicts whether a call can run.
        // It only reacts to the process-wide 402 edge, then waits for the broker/app's
        // explicit Resume message. Pending mail and the live ACP session stay in place.
        if energy_paused {
            tokio::select! {
                event = energy.recv() => {
                    match event {
                        Ok(crate::foundation::energy_state::EnergyEvent::Resume) => {
                            energy_paused = false;
                            tracing::info!(cognition = id, "cognition resumed after energy refill");
                            warm_session(&reaction, id, &mut session).await;
                            // A failed turn may already be waiting in `pending`. Retain a
                            // notify permit so the next loop iteration drives it now rather
                            // than waiting for the recurring pulse or fresh mail.
                            if !pending.is_empty() {
                                mail.notify_one();
                            }
                        }
                        Ok(crate::foundation::energy_state::EnergyEvent::Pause) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            energy_paused = crate::foundation::energy_state::is_out();
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = reaction.inner.shutdown.cancelled() => {
                    tracing::info!(cognition = id, "cognition shutting down");
                    break;
                }
            }
            continue;
        }

        // Resolved per iteration so a mid-session change to the `pulse` tunable takes
        // effect at the next wake, the way it does for a conversation.
        //
        // **`pulse: off` silences the recurring arm and not the boot one.** They are
        // different mechanisms wearing one knob: the cadence is a preference, while
        // restart recovery is the thing whose absence loses a promise. An operator
        // turning pulses down should get a quieter agent, never one that forgets what
        // it owes across a restart.
        let wake_at = if woke_at_boot {
            super::pulse_interval().map(|every| last_turn + every)
        } else {
            Some(started + BOOT_WAKE_AFTER)
        };

        tokio::select! {
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
            _ = mail.notified() => {}
            _ = sleep_until_opt(wake_at) => {
                let first = !woke_at_boot;
                woke_at_boot = true;
                // The boot wake reports **uptime** and the recurring one reports
                // **idleness**. Usually the same span, but not always: mail can drive a
                // turn before the boot timer arm wins, and then the next timer wake still
                // needs startup uptime rather than time since that intervening turn.
                let span = if first { started.elapsed() } else { last_turn.elapsed() };
                last_turn = Instant::now();
                match glance_note(&reaction, first, span).await {
                    Some(note) => pending.push(note),
                    // Nothing is owed, so there is nothing to glance at. Skipping
                    // costs one directory scan; waking would cost a subprocess and a
                    // model turn to reach the same conclusion.
                    //
                    // Gating the *wake* on the ledger being non-empty is safe in a way
                    // gating a *loop* on it would not be (N3's objection): the boot arm
                    // is one-shot and the recurring arm is paced by a timer, so a
                    // permanently-open serving task re-arms the timer, never re-enters
                    // it. Nothing here can feed itself.
                    None => continue,
                }
            }
            ctl = control_rx.recv() => {
                match ctl {
                    Some(LoopControl::CreateWorker { id: worker, task, kind, owner }) => {
                        if let Err(err) =
                            workers.spawn_with_id(&reaction, worker, task, kind, owner).await
                        {
                            tracing::warn!(error = %err, "cognition failed to create a worker");
                        }
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
            _ = reaction.inner.shutdown.cancelled() => {
                tracing::info!(cognition = id, "cognition shutting down");
                break;
            }
        }

        // Drain whatever accumulated. A burst is merged by the switchboard into one
        // prompt, so no settle window is needed here.
        if let Some(batch) = registry::global().take_pending(id) {
            pending.push(registry::render(&batch));
        }
        if pending.is_empty() {
            continue;
        }

        workers.reap();

        // **Keep serving `control_rx` while the turn runs.** `create_worker` is the one
        // control message that is always sent from *inside* a turn — the model calls the
        // tool mid-prompt — and the loop that has to honour it is this one. Awaiting the
        // turn without polling the channel meant the request could not be granted until
        // the turn that made it had finished: the tool reported `starting`, the buffered
        // send succeeded, and the subprocess appeared only once the caller stopped
        // working. Delegation therefore failed exactly when it was worth most — a long
        // turn — and `session_status` truthfully answered "no live session" the whole
        // time, which reads as a dead worker rather than an undelivered one.
        //
        // Only the control arm belongs here. Serving the wake or mail arms would start a
        // second turn on top of this one; those stay outside, where a turn boundary
        // separates them. Worker *reports* stay outside too — they need `&mut pending`,
        // which this turn has borrowed, and they are next-turn input by design.
        let result = {
            let mut turn_fut = std::pin::pin!(turn(&reaction, id, &pending, &mut session));
            loop {
                tokio::select! {
                    done = &mut turn_fut => break done,
                    ctl = control_rx.recv() => match ctl {
                        Some(LoopControl::CreateWorker { id: worker, task, kind, owner }) => {
                            if let Err(err) =
                                workers.spawn_with_id(&reaction, worker, task, kind, owner).await
                            {
                                tracing::warn!(error = %err, "cognition failed to create a worker");
                            }
                        }
                        // The sender is gone; the turn still deserves to finish. A closed
                        // channel resolves immediately forever, so stop selecting on it.
                        None => break (&mut turn_fut).await,
                    },
                }
            }
        };

        match result {
            Ok(()) => pending.clear(),
            Err(err) => {
                // Keep `pending` — the mail is still owed. The recurring wake above is
                // now what carries it: a failed turn used to wait for the next message
                // to arrive, which for a standing rung could be never.
                //
                // **Drop the session too.** A handle that failed once will usually fail
                // every later prompt, and nothing above would notice a rung that has gone
                // quietly deaf — the exact failure mode the per-wake session used to make
                // unrepresentable. Dropping it means the retry cold-opens, which costs one
                // subprocess and loses only the working thread; the ledger and the carried
                // notes are re-projected either way.
                if crate::foundation::energy_state::is_402_error(&err)
                    && crate::foundation::energy_state::is_out()
                {
                    // The prompt receiver was restored by `SessionRun::wait`; keep the
                    // session and retry this same pending batch after Resume.
                    energy_paused = true;
                } else {
                    session = None;
                }
                tracing::warn!(
                    cognition = id,
                    error = %err,
                    held = energy_paused,
                    "cognition turn failed; mail held"
                );
            }
        }

        // After the turn, not before it: the glance is for quiet moments, and a turn
        // that just ran means this was not one.
        last_turn = Instant::now();
        registry::global().finish_turn(id);
    }
}

/// `sleep_until`, or never. Keeps the `select!` above written once instead of twice —
/// a disabled cadence is an arm that never completes, not a second copy of the loop.
async fn sleep_until_opt(at: Option<Instant>) {
    match at {
        Some(at) => sleep_until(at).await,
        None => std::future::pending::<()>().await,
    }
}

/// What a timer wake carries into the turn, or `None` when nothing is owed.
///
/// Bare situational facts, exactly like the conversation pulse — *what a quiet moment is for*
/// is `cognition.md`'s job, and it already says: read down the active tasks, check the
/// things we own are actually alive, and read each check's real output because a probe
/// that returns nothing means **down**, not fine. That guidance has been in the prompt
/// since before anything could deliver a pulse to this rung.
async fn glance_note(reaction: &Reaction, first: bool, span: Duration) -> Option<String> {
    let data_dir = reaction.inner.memory.data_dir();
    let active = match crate::mind::memory::tasks::active_tasks(data_dir).await {
        Ok(active) => active,
        // **Unreadable is not empty.** A ledger that cannot be read is a reason to wake
        // the one rung that can do something about it, not a reason to stay quiet — the
        // opposite reading is the whole failure this arm exists to fix, one level up.
        Err(err) => {
            tracing::warn!(error = %err, "cognition could not read the ledger; waking anyway");
            return Some(super::render_pulse(
                "I couldn't read the task ledger just now — whatever is active is not in front of me",
            ));
        }
    };

    if first {
        // Cognition can send only to live session ids. Stand the voice up before
        // building this turn's window so every owed user-facing delivery has
        // somewhere to land after a restart. `ensure_voice` registers the address
        // synchronously; its ACP warm-up may continue while mail queues behind it.
        reaction.ensure_voice().await;
    }

    let count = active.len();
    let note = note_for(count, first, span);
    tracing::info!(
        active = count,
        first_wake = first,
        waking = note.is_some(),
        "cognition timer fired"
    );
    note
}

/// The pure half of [`glance_note`] — split out so the two things worth pinning can be
/// tested without standing up a `Reaction`: that an empty ledger produces **no wake at
/// all**, and that the boot note says a restart happened.
fn note_for(active: usize, first: bool, span: Duration) -> Option<String> {
    if active == 0 {
        return None;
    }
    let m = span.as_secs() / 60;
    Some(super::render_pulse(&if first {
        format!("you've just come back up (host process started {m}m ago)")
    } else {
        format!("nothing new for {m}m")
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cadence must not burn a subprocess and a model turn to conclude that an
    /// empty ledger is empty. Nothing owed, nothing to glance at.
    #[test]
    fn an_empty_ledger_is_not_worth_waking_for() {
        assert_eq!(note_for(0, true, Duration::from_secs(0)), None);
        assert_eq!(note_for(0, false, Duration::from_secs(9_999)), None);
    }

    /// Both notes are bare situational facts under the same `(pulse)` marker the conversation
    /// loop uses — `cognition.md` keys on that word, and on "the first pulse after the
    /// host process starts" to know a restart is what it is looking at.
    #[test]
    fn the_boot_note_says_a_restart_happened_and_the_others_do_not() {
        let boot = note_for(3, true, Duration::from_secs(120)).unwrap();
        assert!(boot.starts_with("(pulse) "), "{boot}");
        assert!(boot.contains("just come back up"), "{boot}");
        assert!(boot.contains("2m ago"), "{boot}");

        let later = note_for(3, false, Duration::from_secs(1_800)).unwrap();
        assert!(later.starts_with("(pulse) "), "{later}");
        assert!(!later.contains("come back up"), "{later}");
        assert!(later.contains("nothing new for 30m"), "{later}");
    }
}

/// Open and prime Cognition's one process-lifetime session at startup.
///
/// The warm prompt contains only `cognition.md`; the first recovery/mail turn adds the
/// current projected ledger and its real messages. Failure is best-effort: the normal
/// turn path cold-opens later.
async fn warm_session(
    reaction: &Reaction,
    id: registry::SessionId,
    held: &mut Option<Arc<AcpSession>>,
) {
    if held.is_some() {
        return;
    }
    if crate::foundation::energy_state::is_out() {
        tracing::info!(cognition = id, "cognition warm-up held while out of energy");
        return;
    }

    let session = match open_session(reaction, id).await {
        Ok(session) => session,
        Err(err) => {
            tracing::warn!(
                cognition = id,
                error = %err,
                "cognition warm-up could not open a session; first turn will cold-start"
            );
            return;
        }
    };

    let warmed = match session.warm().await {
        Ok(Some(run)) => run.wait().await.map(|_| ()),
        Ok(None) => Ok(()),
        Err(err) => Err(err),
    };
    match warmed {
        Ok(()) => {
            tracing::info!(cognition = id, "cognition session warmed");
            *held = Some(session);
        }
        Err(err) => {
            tracing::warn!(
                cognition = id,
                error = %err,
                "cognition warm-up failed; first turn will cold-start"
            );
            reaction
                .inner
                .observatory
                .record(
                    EventKind::SessionClosed {
                        kind: SessionKind::Cognition,
                        id: session.id().0.to_string(),
                    },
                )
                .await;
        }
    }
}

async fn open_session(
    reaction: &Reaction,
    id: registry::SessionId,
) -> anyhow::Result<Arc<AcpSession>> {
    let data_dir = reaction.inner.memory.data_dir();
    let system_prompt = crate::identity::cognition_prompt(data_dir).await;
    let opened = Arc::new(
        reaction
            .inner
            .agent
            .session(
                SessionRole::Cognition,
                Some(id),
                SessionOpts {
                    system_prompt: Some(system_prompt),
                    // The data dir: the ledger it writes lives under it, and it has no
                    // view workshop to work in — it delegates the making of things.
                    cwd: Some(data_dir.to_path_buf()),
                    // Left at the adapter's defaults so Cognition can read and write its
                    // ledger. Delegation remains prompt guidance rather than a tool rail.
                    builtin_tools: None,
                },
            )
            .await?,
    );

    reaction
        .inner
        .observatory
        .record(
            EventKind::SessionOpened {
                kind: SessionKind::Cognition,
                id: opened.id().0.to_string(),
            },
        )
        .await;

    Ok(opened)
}

/// One wake: prompt the held session, opening one first if there isn't one.
///
/// The window goes in as part of **every** prompt rather than the system prompt, and now
/// that the session is long-lived that is load-bearing rather than incidental: a task
/// opened, a duty closed, or a note written since this session started is exactly what it
/// cannot have remembered, because it did not exist yet. Injecting per turn is what makes
/// "projected, not retrieved" true rather than true-at-open — the same correction the
/// reaction loop's prompt builder carries.
async fn turn(
    reaction: &Reaction,
    id: registry::SessionId,
    pending: &[String],
    held: &mut Option<Arc<AcpSession>>,
) -> anyhow::Result<()> {
    // Reuse the held session; open one only when there isn't one — first turn after
    // start, or after a failure/wedge dropped it.
    let session = if let Some(existing) = held.as_ref() {
        existing.clone()
    } else {
        let opened = open_session(reaction, id).await?;
        *held = Some(opened.clone());
        opened
    };

    let window = snapshot::agent_window(&reaction.inner.memory, COGNITION_AGENT, id).await;
    let messages = pending.join("\n\n");
    let prompt = if window.trim().is_empty() {
        format!("## New messages\n{messages}")
    } else {
        format!("{}\n\n## New messages\n{messages}", window.trim())
    };

    // Paired with "cognition turn done". Cognition has no conversation of its own, so it never reaches
    // the observatory mirror and the log is the only place it is visible at all; with
    // only a done line, "thinking" and "parked with nothing to do" read the same.
    tracing::info!(cognition = id, prompt_chars = prompt.chars().count(), "cognition turn start");

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
