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
//! 4. **A host for its workers.** `hi_create_worker` needs somewhere to run them; without
//!    this, a standing rung borrows an arbitrary conversation and quietly files its work under
//!    a stranger's conversation.
//!
//! ## One long-lived session; the agent bounds its own context
//!
//! The address is registered **once, for the life of the process**; the agent session is
//! opened once and **held across wakes**, replaced only when it breaks. Two different lifetimes for two different things: an address
//! must be stable (nothing durable may hold a session id, and a prompt certainly cannot),
//! while a session is *replaceable* — `docs/arch/host.md`: *"No session is a source of
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

use crate::foundation::codex::{AgentSession, SessionOpts, SessionUpdate};
use crate::identity::Role;
use crate::foundation::observatory::EventKind;
use crate::foundation::registry::{self, Registration, TurnOutcome};
use crate::mind::memory::snapshot;

use super::tools::{LoopControl, ToolOwner, ToolSink};
use super::{LoopInput, Reaction, LOOP_QUEUE_CAPACITY, workers};

/// The agent name for [`crate::mind::memory::layout::rung_seed_path`] — what
/// Cognition carries forward between wakes, at `memory/prompts/cognition.md`.
const COGNITION_AGENT: &str = "cognition";

/// How long a `turn/steer` may go unanswered before the message it carried is put back on
/// the next-turn path instead. See the call site for why this is bounded at all.
const STEER_TIMEOUT: Duration = Duration::from_secs(5);

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

    // Its workers run under its own role-specific sink rather than the voice's.
    let (control_tx, mut control_rx) = mpsc::channel::<LoopControl>(LOOP_QUEUE_CAPACITY);
    let (report_tx, mut report_rx) = mpsc::channel::<LoopInput>(LOOP_QUEUE_CAPACITY);
    reaction
        .inner
        .tools
        // `mouth: None` — no sequencer, no audio, no screen. Cognition proposes; Reaction
        // voices. That it *cannot* express is now a fact about the sink rather than an
        // agreement between the tool list and the role check at dispatch.
        .register(
            ToolOwner::Cognition,
            ToolSink { control: control_tx, mouth: None },
        )
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
    // missing. It is **not** a scheduler and never becomes one — scheduling past this
    // cadence is the agent's own, arranged with the shell it already has
    // (`docs/arch/host.md#glancing-up`).
    //
    // Two wakes, deliberately different things: the **boot** one fires once because a
    // restart happened, and the **recurring** one fires into idleness because a duty
    // can die quietly at any time. `last_turn` resets on every turn, so the second is
    // a quiet-moment glance rather than a metronome.
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
    let mut session: Option<Arc<AgentSession>> = None;
    let mut energy = crate::foundation::energy_state::subscribe();
    let mut energy_paused = crate::foundation::energy_state::is_out();

    tracing::info!(cognition = %id, "cognition up");
    if reaction.wait_for_server_ready().await {
        warm_session(&reaction, &id, &mut session).await;
    }

    loop {
        // The rung never polls the account and never predicts whether a call can run.
        // It only reacts to the process-wide 402 edge, then waits for the broker/app's
        // explicit Resume message. Pending mail and the live agent session stay in place.
        if energy_paused {
            tokio::select! {
                event = energy.recv() => {
                    match event {
                        Ok(crate::foundation::energy_state::EnergyEvent::Resume) => {
                            energy_paused = false;
                            tracing::info!(cognition = %id, "cognition resumed after energy refill");
                            warm_session(&reaction, &id, &mut session).await;
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
                    tracing::info!(cognition = %id, "cognition shutting down");
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
                    Some(ctl) => {
                        super::apply_control(&reaction, &mut workers, ctl).await;
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
                tracing::info!(cognition = %id, "cognition shutting down");
                break;
            }
        }

        // **Every report queued here joins this turn, whichever arm woke us.** The select
        // arm above cannot carry this alone: it is `biased` with mail ahead of the report
        // channel, and a stretch busy enough to fail a worker is exactly a stretch with mail
        // in it. Measured on 2026-08-18 — three workers died on a 429 inside two minutes,
        // their reports sat unread behind mail for the fourteen minutes the log covers, and
        // the only thing Cognition could see was the ledger row saying `idle`, which it
        // answered by telling each one to stop idling. A terminal report is not something
        // chatter may outrun.
        while let Ok(input) = report_rx.try_recv() {
            if let LoopInput::Worker(r) = input {
                pending.push(workers::render_report_plainly(&r));
            }
        }

        // Drain whatever accumulated. A burst is merged by the switchboard into one
        // prompt, so no settle window is needed here.
        if let Some(batch) = registry::global().take_pending(&id) {
            pending.push(registry::render(&batch));
        }
        if pending.is_empty() {
            continue;
        }

        // Timer wakes and worker reports bypass the switchboard inbox, so they
        // have no `take_pending` edge to mark this standing session busy.
        registry::global().start_turn(&id);
        workers.reap();

        // **Keep serving `control_rx` while the turn runs.** `hi_create_worker` is the one
        // control message that is always sent from *inside* a turn — the model calls the
        // tool mid-prompt — and the loop that has to honour it is this one. Awaiting the
        // turn without polling the channel meant the request could not be granted until
        // the turn that made it had finished: the tool reported `starting`, the buffered
        // send succeeded, and the subprocess appeared only once the caller stopped
        // working. Delegation therefore failed exactly when it was worth most — a long
        // turn — and `hi_session_status` truthfully answered "no live session" the whole
        // time, which reads as a dead worker rather than an undelivered one.
        //
        // The wake arm stays outside — serving it would start a second turn on top of this
        // one, and a turn boundary is what separates them. Worker *reports* stay outside
        // too: they are next-turn input by design.
        //
        // **Mail is served here, and it does not start a turn — it walks into the one that
        // is running** ([`AgentSession::steer`]). What it answers was measured: the person
        // said "what? we backup sqlite before every deploy??", the voice relayed it in
        // eight seconds, and it sat in this inbox for seven minutes because the turn it
        // contradicted was still going. It was read twenty-seven seconds after the step it
        // would have prevented. A rung holding a shell for sixteen minutes with no way in
        // is a rung with no supervisor, and the person is the only supervisor there is.
        //
        // **Only the voice steers.** What a person just said is the one input that can
        // invalidate work in flight; a worker's message is a report about work that is
        // going fine, and letting reports interrupt would turn every busy stretch into a
        // stutter. Everything not steered — and everything steering *failed* to deliver —
        // goes into `arrived` and is folded into the next turn, which is exactly what used
        // to happen to all of it. Nothing here can lose a message.
        let mut arrived: Vec<String> = Vec::new();
        let result: anyhow::Result<()> = async {
            let live = ensure_session(&reaction, id.clone(), &mut session).await?;
            let steerable = live.clone();
            let mut turn_fut = std::pin::pin!(turn(&reaction, &id, &pending, live));
            loop {
                tokio::select! {
                    done = &mut turn_fut => break done,
                    _ = mail.notified() => {
                        let Some(batch) = registry::global().take_pending(&id) else { continue };
                        let (voice, rest): (Vec<_>, Vec<_>) =
                            batch.into_iter().partition(from_the_voice);
                        if !rest.is_empty() {
                            arrived.push(registry::render(&rest));
                        }
                        if voice.is_empty() {
                            continue;
                        }
                        // The heading is a fact about *delivery* — the model is mid-turn
                        // and needs to know this reached it there rather than at the start
                        // of a fresh one. What that obliges it to do is `cognition.md`'s
                        // to say, not the host's.
                        let rendered = registry::render(&voice);
                        // **Bounded, because a request to this app-server is not
                        // guaranteed to be answered.** Probed against the pinned 0.147:
                        // a method it knows, addressed to a thread that no longer exists,
                        // produces no response at all rather than an error — so the
                        // oneshot behind `request` would never resolve. Awaited bare, that
                        // would park this arm forever and, with it, the polling of the
                        // turn future beside it: a worse version of the stall this whole
                        // path exists to end. A live thread answers in milliseconds, so
                        // five seconds is "it is never coming".
                        let steered = tokio::time::timeout(
                            STEER_TIMEOUT,
                            steerable.steer(format!(
                                "## New message — arrived while you are working\n{rendered}"
                            )),
                        )
                        .await
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("no answer in {STEER_TIMEOUT:?}")));
                        match steered {
                            Ok(true) => tracing::info!(
                                cognition = %id,
                                chars = rendered.chars().count(),
                                "steered the running turn",
                            ),
                            // No turn to steer (it ended in the gap), or codex refused —
                            // the `steer` feature off, or a turn that stopped being
                            // steerable. Either way the message is owed, so it takes the
                            // path it always took.
                            Ok(false) => arrived.push(rendered),
                            Err(err) => {
                                tracing::warn!(
                                    cognition = %id,
                                    error = %err,
                                    "could not steer; message waits for the next turn",
                                );
                                arrived.push(rendered);
                            }
                        }
                    }
                    // **All three verbs, served here, through the same body the idle arm
                    // uses** ([`super::apply_control`]). Create is the one this arm was
                    // built for, and the other two are called from mid-prompt just as
                    // often: a cancel held until the turn ended would stop a worker no
                    // earlier than doing nothing would have, and a close held that long
                    // keeps a subprocess for no purpose.
                    ctl = control_rx.recv() => match ctl {
                        Some(ctl) => super::apply_control(&reaction, &mut workers, ctl).await,
                        // The sender is gone; the turn still deserves to finish. A closed
                        // channel resolves immediately forever, so stop selecting on it.
                        None => break (&mut turn_fut).await,
                    },
                }
            }
        }
        .await;

        // Read before the match takes the error: what the switchboard needs is the same
        // fact the branch below logs, and a second `Result` would be a second chance to
        // disagree with it.
        let outcome = match &result {
            Ok(()) => TurnOutcome::Completed,
            Err(err) => TurnOutcome::Failed(err.to_string()),
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
                    cognition = %id,
                    error = %err,
                    held = energy_paused,
                    "cognition turn failed; mail held"
                );
            }
        }

        // Mail that landed mid-turn and was not steered in — a worker's report, or a
        // steer codex refused. It is owed either way, so it joins the next turn's batch,
        // *after* the clear above so a turn that succeeded does not carry it off. The
        // notify is what makes "next turn" mean now rather than at the next pulse: this
        // mail was taken out of the inbox by hand, so nothing else will wake for it.
        if !arrived.is_empty() {
            pending.append(&mut arrived);
            mail.notify_one();
        }

        // After the turn, not before it: the glance is for quiet moments, and a turn
        // that just ran means this was not one.
        last_turn = Instant::now();
        registry::global().finish_turn(&id, outcome);
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
/// Bare situational facts and nothing else — *what a quiet moment is for* is
/// `cognition.md`'s job, and it already says: read down the active tasks, check the
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
            return Some(render_pulse(
                "I couldn't read the task ledger just now — whatever is active is not in front of me",
            ));
        }
    };

    if first {
        // Cognition can send only to live session ids. Stand the voice up before
        // building this turn's window so every owed user-facing delivery has
        // somewhere to land after a restart. `ensure_voice` registers the address
        // synchronously; its warm-up may continue while mail queues behind it.
        reaction.ensure_voice().await;
    }

    let count = active.len();
    // The offer rides the boot note and only the boot note: it describes a restart, and by
    // the recurring pulse it is either taken or judged not worth taking. Repeating it every
    // half hour would make it wallpaper, which for a thing that says "unfinished work here"
    // is the one failure mode that matters.
    let offer = if first {
        offer_lost_errands(&registry::global().lost_workers())
    } else {
        String::new()
    };
    let note = wake_for(count, first, span, &offer);
    tracing::info!(
        active = count,
        first_wake = first,
        offered = !offer.is_empty(),
        waking = note.is_some(),
        "cognition timer fired"
    );
    note
}

/// The errands the last restart killed, written for the rung that decides what to do about
/// them.
///
/// **An offer, not an instruction**, and the wording has to carry that: `agents.md` gives the
/// judgment to Cognition precisely because most dead errands are stale — their tool calls
/// already landed, and forty minutes on the world has moved. So this says what died, when, and
/// where its mind is, and stops. What it must never do is imply that resuming is the expected
/// answer.
///
/// It also states the alternative, because the failure this whole path exists to end is not
/// "the errand was not resumed" — it is the errand being **neither resumed nor reconsidered**,
/// which is what a ledger entry with nobody on it looks like from the inside. Dropping one is
/// a fine outcome; dropping one silently is the bug.
fn offer_lost_errands(lost: &[registry::index::Ended]) -> String {
    use std::fmt::Write as _;

    if lost.is_empty() {
        return String::new();
    }
    let mut s = String::from("## Errands the restart cut off\n");
    s.push_str(
        "These were mid-flight when the host went down, so nothing reported back and nothing \
         closed them. Their sessions are gone; their threads are kept.\n",
    );
    for end in lost {
        let Some(thread) = end.thread.as_deref() else { continue };
        let title = end.title.as_deref().unwrap_or("(nothing recorded)");
        let _ = write!(s, "\n- \"{}\"", tail_of(title, 200));
        // The ledger entry it belonged to, when it had one. Without this the line is all
        // there is, and matching a phrase back to a task is exactly the reading the subject
        // exists to replace — that task is on the active list two sections up, already
        // marked as having nobody on it, and this is what ties the two together.
        if let Some(subject) = end.subject.as_deref() {
            let _ = write!(s, "\n  for task `{subject}`");
        }
        if let Some(started) = end.started {
            let _ = write!(s, "\n  started {}", started.format("%Y-%m-%d %H:%MZ"));
        }
        let _ = write!(s, "\n  thread `{thread}`");
    }
    s.push_str(
        "\n\nFor each: decide. `hi_create_worker` with `resume` set to the thread picks it up \
         knowing what it knew — brief it on what has changed since, not on the job, and pass \
         the same `subject` so the task shows as worked again. Or judge it stale and let it \
         go, in which case say so in the ledger, because a task left `doing` with nobody on \
         it reads the same as one being worked on.\n",
    );
    s
}

/// The first `n` characters of a task line, flattened — a brief can run to a paragraph and the
/// offer is a list, not the briefs themselves.
fn tail_of(s: &str, n: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= n {
        return flat;
    }
    flat.chars().take(n).collect::<String>() + "…"
}

/// Whether this timer fire becomes a turn, and what it carries — the pure half of
/// [`glance_note`], so what wakes this rung can be pinned without standing up a `Reaction`.
///
/// **Two independent reasons to wake, and the offer is not the weaker one.** `note_for`
/// declines to spend a subprocess on an empty ledger, which is right when nothing is owed. But
/// an errand the restart cut off is something owed that the ledger *may never have been told
/// about* — Cognition can dispatch before it writes, and a crash in that window leaves the
/// session row as the only surviving trace. Making the offer ride on the ledger being non-empty
/// would drop it in exactly the case where nothing else will ever mention that work again.
fn wake_for(active: usize, first: bool, span: Duration, offer: &str) -> Option<String> {
    match (note_for(active, first, span), offer.is_empty()) {
        (_, false) => Some(format!("{}\n\n{offer}", pulse_line(first, span))),
        (note, true) => note,
    }
}

/// The pure half of [`glance_note`] — split out so the two things worth pinning can be
/// tested without standing up a `Reaction`: that an empty ledger produces **no wake at
/// all**, and that the boot note says a restart happened.
fn note_for(active: usize, first: bool, span: Duration) -> Option<String> {
    if active == 0 {
        return None;
    }
    Some(pulse_line(first, span))
}

/// The situational fact a wake carries, with no opinion about whether to wake.
///
/// Split from [`note_for`] because there are now two reasons to wake — something owed, and an
/// errand the restart cut off — and they must not each spell this sentence. The `(pulse)`
/// marker and the "just come back up" phrasing are both load-bearing: `cognition.md` keys on
/// them to tell a restart from an ordinary quiet moment.
fn pulse_line(first: bool, span: Duration) -> String {
    let m = span.as_secs() / 60;
    render_pulse(&if first {
        format!("you've just come back up (host process started {m}m ago)")
    } else {
        format!("nothing new for {m}m")
    })
}

/// The marker a quiet moment arrives under, and **this rung's alone**. It lived in
/// [`super`] while the conversation loop had a pulse of its own; that loop no longer wakes
/// itself, so the word now names exactly one thing — the brain glancing up — and lives
/// with it. `cognition.md` keys on it.
fn render_pulse(note: &str) -> String {
    format!("(pulse) {note}")
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

    fn lost(title: &str, thread: &str) -> registry::index::Ended {
        registry::index::Ended {
            run: "run-prev".into(),
            session: 5.into(),
            role: "worker".into(),
            worker_type: Some("general".into()),
            title: Some(title.into()),
            subject: Some("chase-harbor".into()),
            owner: Some(3.into()),
            started: Some(chrono::Utc::now()),
            ended: None,
            how: registry::index::EndedHow::Restart,
            turns: None,
            thread: Some(thread.into()),
        }
    }

    /// The offer has to carry the two things Cognition cannot act without: what the errand
    /// *was*, and the handle that picks it back up. A list of thread ids with no tasks is
    /// unreadable; a list of tasks with no threads cannot be taken.
    #[test]
    fn the_offer_names_the_errand_and_the_thread_that_holds_it() {
        let text = offer_lost_errands(&[lost("Finish the KT8-056 placeholder work", "th-42")]);
        assert!(text.contains("Finish the KT8-056 placeholder work"), "{text}");
        assert!(text.contains("th-42"), "{text}");
        assert!(text.contains("resume"), "it has to say how to take it: {text}");
    }

    /// **It stays an offer.** Most dead errands are stale — their tool calls already landed and
    /// the world has moved — so `agents.md` gives the judgment to Cognition. Wording that reads
    /// as an instruction would spend a subprocess on every errand any restart ever interrupted.
    ///
    /// And the alternative is stated, because the failure being fixed is not "the errand was
    /// not resumed" but the errand being neither resumed nor reconsidered: a task left `doing`
    /// with nobody on it looks exactly like one being worked on.
    #[test]
    fn the_offer_leaves_dropping_it_open_and_says_to_write_that_down() {
        let text = offer_lost_errands(&[lost("Rebuild the deck", "th-7")]);
        assert!(text.contains("stale"), "dropping it must be a named option: {text}");
        assert!(text.contains("ledger"), "and it must be written down: {text}");
    }

    /// Nothing lost, nothing said. An empty section in a boot prompt is a heading that trains
    /// the reader to skip the heading.
    #[test]
    fn nothing_lost_is_no_section_at_all() {
        assert!(offer_lost_errands(&[]).is_empty());
    }

    /// **The offer wakes this rung by itself.** Cognition can dispatch an errand before it
    /// writes the ledger entry, so a crash in that window leaves a mid-flight worker and
    /// nothing owed — and if the offer only rode along with a ledger wake, that is the one
    /// case where it would be dropped, and nothing would ever mention the work again.
    #[test]
    fn an_errand_cut_off_wakes_even_when_the_ledger_is_empty() {
        let offer = offer_lost_errands(&[lost("chase the deploy", "th-9")]);
        let note = wake_for(0, true, Duration::from_secs(60), &offer)
            .expect("a lost errand is worth waking for on its own");

        assert!(note.starts_with("(pulse) "), "{note}");
        assert!(note.contains("just come back up"), "{note}");
        assert!(note.contains("chase the deploy"), "{note}");
    }

    /// And with nothing lost, the empty-ledger rule is untouched — the offer must not become
    /// a reason to wake for nothing.
    #[test]
    fn an_empty_ledger_with_no_offer_still_does_not_wake() {
        assert_eq!(wake_for(0, true, Duration::from_secs(60), ""), None);
    }

    /// A brief can run to a paragraph — the one this was built from was 1,400 characters. The
    /// offer is an index of what died, not a reprint of the briefs.
    #[test]
    fn a_long_brief_is_clipped_into_one_line() {
        let text = offer_lost_errands(&[lost(&format!("start {}", "x ".repeat(400)), "th-1")]);
        assert!(text.contains("start"), "{text}");
        assert!(text.contains('…'), "the clip announces itself: {text}");
        assert!(text.lines().count() < 12, "still a list: {text}");
    }

    /// Both notes are bare situational facts under the `(pulse)` marker — `cognition.md`
    /// keys on that word, and on "the first pulse after the host process starts" to know a
    /// restart is what it is looking at.
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
    id: &registry::SessionId,
    held: &mut Option<Arc<AgentSession>>,
) {
    if held.is_some() {
        return;
    }
    if crate::foundation::energy_state::is_out() {
        tracing::info!(cognition = %id, "cognition warm-up held while out of energy");
        return;
    }

    let session = match open_session(reaction, id.clone()).await {
        Ok(session) => session,
        Err(err) => {
            tracing::warn!(
                cognition = %id,
                error = %err,
                "cognition warm-up could not open a session; first turn will cold-start"
            );
            return;
        }
    };

    // Opening the thread *is* the warm-up: `baseInstructions` carries the prompt on
    // `thread/start`, so by the time we hold this session the soul is already in place.
    // The ACP path had to spend a whole turn pre-sending it, because there was nowhere
    // else to put a system prompt.
    tracing::info!(cognition = %id, "cognition session warmed");
    *held = Some(session);
}

async fn open_session(
    reaction: &Reaction,
    id: registry::SessionId,
) -> anyhow::Result<Arc<AgentSession>> {
    let data_dir = reaction.inner.memory.data_dir();
    let system_prompt = crate::identity::cognition_prompt(data_dir).await;
    let opened = Arc::new(
        reaction
            .inner
            .agent
            .session(
                Role::Cognition,
                Some(id),
                SessionOpts {
                    system_prompt: Some(system_prompt),
                    // The data dir: the ledger it writes lives under it, and it has no
                    // view workshop to work in — it delegates the making of things.
                    cwd: Some(data_dir.to_path_buf()),
                    // Left at the adapter's defaults so Cognition can read and write its
                    // ledger. Delegation remains prompt guidance rather than a tool rail.
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
                kind: Role::Cognition,
                id: opened.id().to_string(),
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
/// Was this message put here by the voice?
///
/// The one sender whose word can invalidate work already in flight, because it is the only
/// one carrying what a *person* just said. Host-posted mail (`from: None`) is not a
/// colleague and does not steer; neither does a worker, whose messages are reports about
/// work that is going to plan. A sender that has since gone (no registry entry) reads as
/// not-the-voice, which is the safe way round: the cost is a message waiting for the turn
/// boundary it would have waited for anyway.
fn from_the_voice(m: &registry::Message) -> bool {
    m.from
        .as_ref()
        .and_then(|from| registry::global().status(from))
        .is_some_and(|st| st.role == Role::Reaction)
}

/// Reuse the held session, or open one — first turn after start, or after a failure or
/// wedge dropped it.
///
/// **Split out of [`turn`] so the caller holds the handle while the turn runs.** A turn
/// that opens its own session owns it for the duration, and a rung nothing can reach for
/// sixteen minutes is what that costs; steering needs a handle from outside the future.
async fn ensure_session(
    reaction: &Reaction,
    id: registry::SessionId,
    held: &mut Option<Arc<AgentSession>>,
) -> anyhow::Result<Arc<AgentSession>> {
    if let Some(existing) = held.as_ref() {
        return Ok(existing.clone());
    }
    let opened = open_session(reaction, id).await?;
    *held = Some(opened.clone());
    Ok(opened)
}

async fn turn(
    reaction: &Reaction,
    id: &registry::SessionId,
    pending: &[String],
    session: Arc<AgentSession>,
) -> anyhow::Result<()> {
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
    tracing::info!(cognition = %id, prompt_chars = prompt.chars().count(), "cognition turn start");

    // What is owed *before* the turn, so a duty this turn retires can be recognized
    // afterwards. Subjects only — the check is "did this leave the active set", and
    // reading the titles back out of a closed record is the WARN's job, not this one's.
    let owed_before = active_subjects(reaction).await;

    let mut run = session.prompt(prompt).await?;
    let mut full = String::new();
    let mut sent = 0usize;
    while let Some(update) = run.next_update().await {
        // What it is *doing*, kept apart from what it has *said* below. Cognition can spend
        // a long turn dispatching and reading with nothing to show on the tail, so without
        // this the roster reports the outward brain as silent while it works.
        if let Some(what) = update.activity() {
            registry::global().record_activity(id, &what);
        }
        match update {
            SessionUpdate::Text(text) => {
                full.push_str(&text);
                // Mirror it into the switchboard's bounded tail, or `hi_session_messages`
                // answers "nothing yet" forever — and the voice's `## Still looking into`
                // line has no other way to see what this rung is making of the question.
                registry::global().record_output(id, &text);
            }
            // Counted from the raw frame rather than the tool dispatch, because what is
            // being checked is the model's behaviour, not the host's: this has to stay
            // true of a `hi_send_message` that the host went on to refuse.
            SessionUpdate::Frame(frame) if is_send_message_call(&frame) => sent += 1,
            _ => {}
        }
    }
    run.wait().await?;

    // **A turn's reply text is not a channel.** It goes to the bounded tail above, which is
    // only ever *pulled* (`hi_session_messages`), so a timer-woken turn's conclusion has no
    // reader at all — `cognition.md` says so ("Nothing you write reaches anyone directly")
    // and the only way anything leaves this rung is `hi_send_message`.
    //
    // Silence is legitimate: deciding a finding is not worth raising is Cognition's own
    // gate (`docs/arch/agents.md`), and the host must not overrule it. So `sent` is
    // reported plainly and the WARN is kept for the one case the host can judge without
    // guessing — **a duty left the ledger and nobody was told**, which is the shape of
    // `done` meaning "finished *and delivered*". Observed live 2026-08-10: a restart sweep
    // closed a task, wrote a 402-char report as its final answer, and sent nothing.
    let closed = owed_before.difference(&active_subjects(reaction).await).count();
    if closed > 0 && sent == 0 {
        tracing::warn!(
            cognition = %id,
            closed,
            typed_chars = full.chars().count(),
            "cognition closed a task and sent nothing; the report reached no one",
        );
    }

    tracing::info!(
        cognition = %id,
        typed_chars = full.chars().count(),
        sent,
        "cognition turn done"
    );
    Ok(())
}

/// The subjects of every `todo`/`doing` task, for the before/after comparison above.
/// An unreadable ledger yields an empty set, which can only *suppress* the warning —
/// the same direction every other failure here takes, and the opposite of inventing a
/// close that did not happen.
async fn active_subjects(reaction: &Reaction) -> std::collections::HashSet<String> {
    crate::mind::memory::tasks::active_tasks(reaction.inner.memory.data_dir())
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|task| task.subject)
        .collect()
}

/// Whether a raw codex frame is Cognition reaching another part of itself — an
/// `item/started` for the `hi_send_message` MCP tool.
///
/// `item/started` and not `item/completed`: the question is whether it *tried*. A call the
/// host rejected (a dead address, a malformed id) is a delivery that failed, which is a
/// different fault from never having addressed anyone, and only the first is this
/// function's business.
fn is_send_message_call(frame: &serde_json::Value) -> bool {
    if frame.get("method").and_then(serde_json::Value::as_str) != Some("item/started") {
        return false;
    }
    let Some(item) = frame.get("params").and_then(|p| p.get("item")) else {
        return false;
    };
    item.get("type").and_then(serde_json::Value::as_str) == Some("mcpToolCall")
        && item.get("tool").and_then(serde_json::Value::as_str) == Some("hi_send_message")
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_send_message_call_is_recognized_from_the_raw_frame() {
        assert!(is_send_message_call(&json!({
            "method": "item/started",
            "params": { "item": {
                "type": "mcpToolCall", "id": "exec-1", "server": "hi-agent",
                "tool": "hi_send_message", "status": "inProgress",
                "arguments": { "to": "2", "message": "the build failed on the auth tests" },
            }},
        })));
    }

    /// The failure this exists to catch looks exactly like a normal turn on the wire:
    /// shell work, file edits, and prose. None of it addresses anyone.
    #[test]
    fn nothing_else_in_a_turn_counts_as_reaching_someone() {
        for frame in [
            json!({ "method": "item/started", "params": { "item": {
                "type": "commandExecution", "command": "/bin/zsh -lc 'sed -n 1,40p facet.md'",
            }}}),
            json!({ "method": "item/started", "params": { "item": {
                "type": "fileChange", "changes": [{ "path": "memory/facets/tasks/x/facet.md" }],
            }}}),
            json!({ "method": "item/completed", "params": { "item": {
                "type": "agentMessage", "phase": "final_answer", "text": "Restart sweep complete.",
            }}}),
            json!({ "method": "item/started", "params": { "item": {
                "type": "mcpToolCall", "server": "hi-agent", "tool": "hi_create_worker",
            }}}),
            // The completion of a send is not a second send.
            json!({ "method": "item/completed", "params": { "item": {
                "type": "mcpToolCall", "server": "hi-agent", "tool": "hi_send_message",
            }}}),
        ] {
            assert!(!is_send_message_call(&frame), "{frame}");
        }
    }
}
