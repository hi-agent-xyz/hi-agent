//! Reflection — the inward brain. One of it for the whole agent.
//!
//! **Cognition and Reflection are the same kind of thing.** Both are , both are
//! as capable as the agent gets, both dispatch workers, neither speaks. What separates
//! them is not intelligence and not machinery — it is **who the work is for**:
//!
//! - **Cognition faces outward.** Its work arrives from a person, through a conversation, and
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
//! 3. **A host for its workers.** Reflection registers its own role-specific tool
//!    sink, so work it creates returns to Reflection rather than Reaction or Cognition.
//! 4. **Two kinds of wake.** Reflection is the one rung driven by a clock *and* by mail.
//!    The clock is its own — an adaptive backoff pacing a loop inside this subsystem,
//!    which is the only shape of timing the host has (`docs/arch/host.md#glancing-up`).
//!
//! ## One long-lived session; registration per process
//!
//! The address is registered **once, for the life of the process**; the agent session is
//! opened once and **held across wakes**, replaced only when it breaks. Same split as
//! Cognition, for the same reason: an address must be stable, a session is replaceable
//! (`docs/arch/host.md` — *"continuity lives in `data/`, not in a process"*).
//!
//! **This reverses the per-wake session this rung shipped with, and the reason is a failure
//! that was observed rather than predicted.** The note here used to read *"a consolidation
//! pass and a mail turn each open one and drop it"*, and `agents.md` justified it: the pass
//! is self-contained, so there is no thread worth keeping. That was written for a rung that
//! only swept. It stopped being true the moment this rung became a dispatcher — no cursor
//! points at which workers it opened, and nothing durable may reference a session slug, so
//! every pass woke knowing nothing about what the last one had already arranged. Observed:
//! three `person-reader` sessions for the same person, opened by three consecutive passes
//! against a prompt that says *one reader per person*, none of them closed, all of them
//! reopened on the next boot.
//!
//! **Residency is half the answer; projection is the other half.** The held session keeps a
//! pass from re-deciding what the pass before it decided. The reach block now built into
//! the consolidation prompt ([`heartbeat`]) is what survives a compaction and a restart,
//! and it is the only one of the two that can show a straggler *this* session never opened.
//! Cognition has both; this rung had neither.
//!
//! The objection residency carries is answered rather than dismissed, the same way
//! [`super::cognition`] answers it: a session that dies mid-turn leaves a handle failing
//! every later prompt with nothing above it to notice. So a failed turn — and a pass that
//! comes back [`heartbeat::Pass::Failed`] — **drops the session**, and a wedged one cannot
//! outlive the wake that wedged it.
//!
//! One red line survives from the old wording and is not negotiable: reflection may
//! **archive verbatim and write pointers, never paraphrase stored bytes**.

use std::sync::Arc;

use tokio::time::Instant;

use tokio::sync::mpsc;

use crate::foundation::codex::{AgentSession, SessionOpts, SessionUpdate};
use crate::foundation::observatory::EventKind;
use crate::foundation::registry::{self, Registration, TurnOutcome};
use crate::identity::Role;
use crate::mind::memory::snapshot;

use super::tools::{LoopControl, ToolOwner, ToolSink};
use super::{LoopInput, Reaction, LOOP_QUEUE_CAPACITY, heartbeat, workers};

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
    /// The backoff clock came due: sweep the shared frontier.
    Consolidate,
    /// Mail, or a worker of ours reporting. Drives an ordinary turn.
    Turn,
}

async fn run(reaction: Reaction, registration: Registration) {
    let id = registration.id();
    let mail = registration.mail.clone();

    // Its workers run under it — see the module note's point 3. `beats: None`: Reflection
    // has no floor, no sequencer, no screen. It never speaks, and that is now a fact
    // about its sink rather than an agreement between a tool list and a role check.
    let (control_tx, mut control_rx) = mpsc::channel::<LoopControl>(LOOP_QUEUE_CAPACITY);
    let (report_tx, mut report_rx) = mpsc::channel::<LoopInput>(LOOP_QUEUE_CAPACITY);
    reaction
        .inner
        .tools
        .register(
            ToolOwner::Reflection,
            ToolSink { control: control_tx, mouth: None },
        )
        .await;

    let mut workers = workers::WorkerRegistry::new(report_tx);
    let mut pending: Vec<String> = Vec::new();
    // **One session, held across wakes** (`docs/arch/agents.md#session-lifetime-per-rung`).
    // Opened lazily rather than warmed at startup the way Cognition's is: nobody is waiting
    // on this rung's first word, and the first pass is a cadence away, so holding a
    // subprocess from boot buys nothing. `None` after a failure means the next wake
    // cold-opens.
    let mut session: Option<Arc<AgentSession>> = None;

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
        tracing::info!(reflection = %id, "consolidation cadence disabled; rung stays addressable");
    }
    let loop_started = Instant::now();
    let mut last_reflection: Option<Instant> = None;
    let mut backoff_gap = reflect_base.unwrap_or(super::DEFAULT_REFLECT_EVERY);
    // Consecutive passes that had new input on the log and still found nothing to
    // consolidate — see [`note_pass`].
    let mut stalled: u32 = 0;
    let mut energy = crate::foundation::energy_state::subscribe();
    let mut energy_paused = crate::foundation::energy_state::is_out();

    tracing::info!(reflection = %id, "reflection up");

    loop {
        // Reflection does not preflight the account. A managed 402 from any agent
        // session flips the shared state; this rung simply parks until the positive
        // balance sends Resume, keeping its pending mail intact.
        if energy_paused {
            tokio::select! {
                event = energy.recv() => {
                    match event {
                        Ok(crate::foundation::energy_state::EnergyEvent::Resume) => {
                            energy_paused = false;
                            tracing::info!(reflection = %id, "reflection resumed after energy refill");
                        }
                        Ok(crate::foundation::energy_state::EnergyEvent::Pause) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            energy_paused = crate::foundation::energy_state::is_out();
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = reaction.inner.shutdown.cancelled() => {
                    tracing::info!(reflection = %id, "reflection shutting down");
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
                    Some(ctl) => {
                        super::apply_control(&reaction, &mut workers, ctl).await;
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
                tracing::info!(reflection = %id, "reflection shutting down");
                break;
            }
        };

        if reaction.inner.shutdown.is_triggered() {
            tracing::info!(reflection = %id, "shutdown requested; ending reflection loop");
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
                // Read before the sweep, so it cannot be set by something that arrived
                // during one: the question is whether input was already on the log when
                // the frontier was gathered.
                let fresh_input = last_activity > anchor;
                backoff_gap = if fresh_input {
                    reflect_base.unwrap_or(super::DEFAULT_REFLECT_EVERY)
                } else {
                    backoff_gap.checked_mul(2).unwrap_or(reflect_max).min(reflect_max)
                };
                last_reflection = Some(now);

                workers.reap();
                registry::global().start_turn(&id);
                // A settling pass is the longest thing this rung does and the place it
                // dispatches most — one `person-reader` per person present in the stretch.
                // Every one of those calls is made from inside this await.
                let pass = match ensure_session(&reaction, id.clone(), &mut session).await {
                    Ok(live) => {
                        serving_control(
                            &reaction,
                            &mut workers,
                            &mut control_rx,
                            heartbeat::consolidate(&reaction, &id, live),
                        )
                        .await
                    }
                    Err(err) => {
                        tracing::warn!(
                            reflection = %id,
                            error = %format!("{err:#}"),
                            "reflection could not open a session; pass skipped",
                        );
                        heartbeat::Pass::Skipped
                    }
                };
                // A handle that broke must not outlive the wake that broke it — see the
                // module note. Energy is not breakage and comes back `Skipped`.
                if pass == heartbeat::Pass::Failed {
                    session = None;
                }
                // A settling pass has no `Result` to read: `consolidate` handles its own
                // failures internally and the pass is over either way. What it reports is
                // whether it swept — the half this loop judges cadence on — and, since the
                // session became the loop's, whether the failure was the session's. Stall
                // accounting reads a failed pass as one that swept nothing, which it is.
                stalled = note_pass(stalled, fresh_input, pass);
                registry::global().finish_turn(&id, TurnOutcome::Completed);
            }
            Wake::Turn => {}
        }

        // Reports too, and for the reason spelled out in [`super::cognition`]: the arm above
        // sits behind mail in a `biased` select, and a report it does not reach is a
        // finished errand nobody is told about. A consolidation pass makes it worse here —
        // it is the longest thing this rung does, and every report that lands during one
        // arrives while the channel is unread.
        while let Ok(input) = report_rx.try_recv() {
            if let LoopInput::Worker(r) = input {
                pending.push(workers::render_report_plainly(&r));
            }
        }

        // Drain whatever accumulated, whichever way we woke — a consolidation pass is
        // long, and mail that arrived during it is owed a turn immediately after.
        if let Some(batch) = registry::global().take_pending(&id) {
            pending.push(registry::render(&batch));
        }
        if pending.is_empty() {
            continue;
        }

        // Worker reports bypass the switchboard inbox; mailbox turns are already
        // busy here, and `start_turn` is deliberately idempotent.
        registry::global().start_turn(&id);
        workers.reap();
        let turned = match ensure_session(&reaction, id.clone(), &mut session).await {
            Ok(live) => {
                serving_control(
                    &reaction,
                    &mut workers,
                    &mut control_rx,
                    turn(&reaction, id.clone(), &pending, live),
                )
                .await
            }
            Err(err) => Err(err),
        };
        let outcome = match &turned {
            Ok(()) => TurnOutcome::Completed,
            Err(err) => TurnOutcome::Failed(err.to_string()),
        };
        match turned {
            Ok(()) => pending.clear(),
            Err(err) => {
                // Keep `pending` — the mail is still owed, and the next wake carries it.
                //
                // **Drop the session too, unless it was energy.** A managed 402 left the
                // handle good — `SessionRun::wait` restored the prompt receiver — so the
                // same session retries this batch after Resume. Anything else is the
                // session, and holding it would make this rung quietly deaf.
                if crate::foundation::energy_state::is_402_error(&err)
                    && crate::foundation::energy_state::is_out()
                {
                    energy_paused = true;
                } else {
                    session = None;
                }
                tracing::warn!(reflection = %id, error = %format!("{err:#}"), "reflection turn failed; mail held");
            }
        }
        super::compact_if_full(&id, session.as_deref()).await;
        registry::global().finish_turn(&id, outcome);
    }
}

/// Open Reflection's session — the one it holds across wakes.
///
/// **One cwd, and it is the data dir.** The two wakes used to open their own and disagree
/// about this: a mail turn passed the data dir, a consolidation pass passed nothing. Absent
/// is not none — [`crate::foundation::codex::CodexProcess::open_thread`] falls back to
/// `std::env::current_dir()`, so the longest-running half of this rung's work ran with its
/// file tools pointed at whatever directory the binary was launched from, while two
/// comments in [`heartbeat`] explained the prompt's shape by saying it had no cwd at all.
async fn open_session(
    reaction: &Reaction,
    id: registry::SessionSlug,
) -> anyhow::Result<Arc<AgentSession>> {
    let data_dir = reaction.inner.memory.data_dir();
    // One file, whole — see `cognition.rs` for why the seed went.
    let system_prompt = crate::identity::reflection_prompt(data_dir).await;

    let opened = Arc::new(
        reaction
            .inner
            .agent
            .session(
                Role::Reflection,
                Some(id),
                SessionOpts {
                    system_prompt: Some(system_prompt),
                    // Its whole job is the tree under it.
                    cwd: Some(data_dir.to_path_buf()),
                    ..Default::default()
                },
            )
            .await?,
    );

    reaction
        .inner
        .observatory
        .record(
            EventKind::SessionOpened {
                kind: Role::Reflection,
                id: opened.id().to_string(),
            },
        )
        .await;

    Ok(opened)
}

/// Reuse the held session, or open one — first wake after start, or after a failure or
/// wedge dropped it.
///
/// **Split out of the wakes so the loop holds the handle while one runs**, which is
/// [`super::cognition::ensure_session`]'s reason and applies harder here: a settling pass
/// is the longest thing this rung does, and a session owned by the future running it is
/// unreachable for as long as that takes.
async fn ensure_session(
    reaction: &Reaction,
    id: registry::SessionSlug,
    held: &mut Option<Arc<AgentSession>>,
) -> anyhow::Result<Arc<AgentSession>> {
    if let Some(existing) = held.as_ref() {
        return Ok(existing.clone());
    }
    let opened = open_session(reaction, id).await?;
    *held = Some(opened.clone());
    Ok(opened)
}

/// Await `work` **while going on serving `control_rx`** — the shape Cognition already
/// has, applied to the other rung that dispatches.
///
/// Every dispatch verb is called from *inside* a turn: the model reaches
/// `hi_create_worker` mid-prompt, and the loop that has to honour it is the one running
/// that prompt. Reflection awaited its turn outside its `select!`, so its own tool calls
/// queued behind the turn that made them and were applied only once it ended. What that
/// looked like from the caller's side, on 2026-08-17: a settling pass created a worker at
/// 08:52:42, and for the next three minutes `hi_session_status`, `hi_send_message` and
/// `hi_session_messages` all answered — truthfully — that no such session existed. The
/// pass concluded the worker had never started, did the facet update itself, and closed a
/// session that was not there yet. Eleven seconds later the turn ended, the queued create
/// was applied, and the worker ran the same update a second time. It has been idle and
/// unclosable ever since: its owner was told it was gone, so it will never ask again.
///
/// So this is not a latency improvement. A control message a loop cannot answer until it
/// stops working is a message that gets a *wrong answer* in the meantime, and the wrong
/// answer is the confident one.
///
/// The mail and clock arms stay outside, for Cognition's reason: serving them here would
/// start a second turn on top of this one, and a turn boundary is what separates them.
/// Consecutive stalled passes before the loop says so. Three separate ticks that each had
/// new input on the log and each gathered an empty frontier.
///
/// Three rather than one because a single tick can race: [`Reaction::deliver`] stamps
/// `last_signal_at` just before the entry is journaled, so one pass can legitimately see
/// "input arrived" and gather the log a moment too early. Three cannot — a signal that has
/// not landed after three cadences is not late, it is unreachable.
const STALL_WARN: u32 = 3;

/// How many further stalled passes between repeats of the warning, so a stall that lasts
/// hours stays visible without filling the log. At the base cadence this is roughly
/// half-hourly; the failure that motivated it ran for thirty-five hours.
const STALL_REWARN: u32 = 30;

/// Fold one consolidation tick into the stall count, warning when a caught-up store stops
/// being a plausible explanation.
///
/// **A skipped pass is ordinary; a skipped pass with new input on the log is a
/// contradiction** (`docs/arch/host.md#the-same-rule-turned-inward`). The frontier is
/// everything after the consolidation cursor, and the raw log is written before anything
/// reacts to it, so a signal the loop has already seen delivered must be *on* that
/// frontier. When it is not, the cursor cannot advance — and the two facts needed to notice
/// were already being computed one line apart for the backoff, and never compared.
///
/// A quiet tick neither counts nor clears: it is the one case that proves nothing, and
/// clearing on it is what would let a real stall oscillate below the threshold forever in a
/// conversation that is busy but not busy every cadence. Only an actual sweep clears.
fn note_pass(stalled: u32, fresh_input: bool, pass: heartbeat::Pass) -> u32 {
    match pass {
        heartbeat::Pass::Swept => {
            if stalled >= STALL_WARN {
                tracing::info!(
                    after = stalled,
                    "consolidation is sweeping again; the frontier advanced",
                );
            }
            0
        }
        heartbeat::Pass::Skipped | heartbeat::Pass::Failed if !fresh_input => stalled,
        heartbeat::Pass::Skipped | heartbeat::Pass::Failed => {
            let stalled = stalled.saturating_add(1);
            if stalled == STALL_WARN
                || (stalled > STALL_WARN && (stalled - STALL_WARN) % STALL_REWARN == 0)
            {
                tracing::warn!(
                    passes = stalled,
                    "consolidation keeps finding an empty frontier while new signals arrive; \
                     the cursor is not advancing and nothing is being remembered",
                );
            }
            stalled
        }
    }
}

async fn serving_control<T>(
    reaction: &Reaction,
    workers: &mut workers::WorkerRegistry,
    control_rx: &mut mpsc::Receiver<LoopControl>,
    work: impl std::future::Future<Output = T>,
) -> T {
    let mut work = std::pin::pin!(work);
    loop {
        tokio::select! {
            done = &mut work => return done,
            ctl = control_rx.recv() => match ctl {
                Some(ctl) => super::apply_control(reaction, workers, ctl).await,
                // The senders are gone; the work still deserves to finish. A closed channel
                // resolves immediately forever, so stop selecting on it.
                None => return (&mut work).await,
            },
        }
    }
}

/// One mail wake: open a session, prompt it once, drop it.
///
/// Distinct from a consolidation pass, which has a whole prompt of its own built from
/// the conversation's frontier ([`heartbeat::consolidate`]). This is the ordinary path — a
/// worker reported, or another rung sent something — and it carries the projected window
/// rather than a frontier.
async fn turn(
    reaction: &Reaction,
    id: registry::SessionSlug,
    pending: &[String],
    session: Arc<AgentSession>,
) -> anyhow::Result<()> {
    let window = snapshot::agent_window(&reaction.inner.memory, None, &id).await;
    let messages = pending.join("\n\n");
    let prompt = if window.trim().is_empty() {
        format!("## New messages\n{messages}")
    } else {
        format!("{}\n\n## New messages\n{messages}", window.trim())
    };

    let mut run = session.prompt(prompt).await?;
    let mut full = String::new();
    while let Some(update) = run.next_update().await {
        if let Some(what) = update.activity() {
            registry::global().record_activity(&id, &what);
        }
        if let SessionUpdate::Text(text) = update {
            full.push_str(&text);
            registry::global().record_output(&id, &text);
        }
    }
    run.wait().await?;

    tracing::info!(reflection = %id, typed_chars = full.chars().count(), "reflection turn done");
    Ok(())
}

#[cfg(test)]
mod stall_tests {
    use super::*;
    use heartbeat::Pass;

    /// The healthy shape: passes that sweep never accumulate a stall, however busy.
    #[test]
    fn sweeping_keeps_the_count_at_zero() {
        let mut stalled = 0;
        for _ in 0..10 {
            stalled = note_pass(stalled, true, Pass::Swept);
        }
        assert_eq!(stalled, 0);
    }

    /// A caught-up store skipping is ordinary and must never be read as a stall, no matter
    /// how long it goes on — this is the case the whole check has to stay quiet about.
    #[test]
    fn a_quiet_store_never_stalls() {
        let mut stalled = 0;
        for _ in 0..1_000 {
            stalled = note_pass(stalled, false, Pass::Skipped);
        }
        assert_eq!(stalled, 0);
    }

    /// The broken-cursor shape: input keeps arriving, the frontier keeps coming back
    /// empty. It reaches the threshold rather than sitting below it forever.
    #[test]
    fn fresh_input_with_an_empty_frontier_accumulates() {
        let mut stalled = 0;
        for _ in 0..STALL_WARN {
            stalled = note_pass(stalled, true, Pass::Skipped);
        }
        assert_eq!(stalled, STALL_WARN);
    }

    /// The oscillation this design exists to avoid: a conversation busy enough to break
    /// but not busy every single cadence. Quiet ticks in between must not clear the count,
    /// or a real stall never reaches the threshold.
    #[test]
    fn a_quiet_tick_between_stalls_does_not_clear_it() {
        let mut stalled = 0;
        for _ in 0..STALL_WARN {
            stalled = note_pass(stalled, true, Pass::Skipped);
            stalled = note_pass(stalled, false, Pass::Skipped);
        }
        assert_eq!(stalled, STALL_WARN);
    }

    /// Only an actual sweep clears it — that is what "recovered" means here.
    #[test]
    fn one_sweep_clears_a_long_stall() {
        let mut stalled = 0;
        for _ in 0..(STALL_WARN + STALL_REWARN * 2) {
            stalled = note_pass(stalled, true, Pass::Skipped);
        }
        assert!(stalled > STALL_WARN);
        assert_eq!(note_pass(stalled, true, Pass::Swept), 0);
    }
}
