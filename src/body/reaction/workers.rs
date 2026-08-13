//! Working sessions — the reaction's hands.
//!
//! The reaction keeps a single voice and must never block the floor on slow
//! work, so heavy or long-running tasks are delegated here. A worker is a
//! *voice-mute capability within the conversation*: it has the full substrate — the
//! conversation's memory, tools, code execution, and its own sub-agents — but no voice
//! of its own. Those sub-agents live *inside* its session and are invisible here:
//! they get no hi-agent session id, no address, and no registry entry, which is
//! why `create_worker` stays Cognition's and Reflection's (`docs/arch/agents.md`). It never speaks and never draws on the screen: it
//! cannot emit on the reaction's expression channels (thought, audio, view). That
//! mute-ness is what preserves single-voice coherence: only the reaction expresses
//! to the person.
//!
//! It is *not*, however, channel-blind. Over hi-agent's own HTTP surface
//! (`HI_AGENT_BASE_URL` in its env) a worker may **perceive input channels**
//! (e.g. `GET /api/in/vision` for live frames) — running detection, CV, whatever
//! the task needs on the raw bytes, all *outside* the turn loop so it never
//! contends with the reaction's serialized speech. It does not write to any output
//! channel: expression (speech and views alike) stays the reaction's, so a worker
//! reports what it found and the reaction decides what to show.
//!
//! The collaboration bus is asynchronous: a worker runs to completion, then posts a
//! [`WorkerReport`] to whoever asked for the work. It never interrupts live speech —
//! the report waits its turn like any other input. Mid-flight it may reach its owner
//! with `send_message`, the one verb, which does not wait for a reply; a worker that
//! hits ambiguity notes its best assumption and keeps going (fix-forward), so the
//! floor is never held waiting on an answer.
//!
//! Progress-checking is emergent rather than wired: each session streams its output into
//! the switchboard's bounded tail, and [`render_status`] surfaces what the agent's own
//! thinking is up to into the reaction's prompt, so the mind can decide on its own social
//! timing whether to wait, nudge, or speak to what it sees.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::{Mutex, Notify, mpsc};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::foundation::codex::{AgentSession, SessionOpts, SessionUpdate, StopReason};
use crate::foundation::observatory::{EventKind, Observatory, WorkerState};
use crate::identity::{Role, WorkerType};

use super::{LoopInput, Reaction};

/// Handle for one **agent session**, unique process-wide.
///
/// It identifies a *session*, not a role: the same role has many sessions over a run,
/// and a Cognition replaced after a failure is a second session of one role. So this is
/// never a "worker id" — ownership, addressing and reporting all key on the session,
/// which is the thing that is actually singular.
///
/// Process-wide because standing owners (Cognition, Reflection) and the voice
/// all create sessions in the same address space.
///
/// Minted before the agent session is opened, because the MCP surface identifies its
/// caller by this id in a request header — so it cannot be the id the adapter assigns.
pub(super) type SessionId = u64;

use crate::foundation::registry;

#[cfg(test)]
fn mint_session_id() -> SessionId {
    registry::mint()
}

/// How long a finished working session stays warm — its agent session held open and
/// resumable via `delegate worker:<id>` — before it closes itself to free the
/// subprocess context. A refinement arriving within this window continues the same
/// session with full context; a later one falls back to a fresh worker.
const WORKER_IDLE_TTL: Duration = Duration::from_secs(15 * 60);

// A worker used to keep a private follow-up mailbox here, beside the switchboard's.
// Two mailboxes for one session is one mailbox too many: whichever the sender picked
// decided whether the message was ever read, and only one of them had a reader. The
// switchboard's inbox is now the only one — it already merges a burst, already carries
// the sender, and already knows how to shut itself when a session idles out.

/// A working session's system prompt now lives in `prompts/workers/` — one whole file
/// per [`WorkerType`], read through [`crate::identity::role_prompt`] like every other
/// role's.
///
/// It used to be a `const &str` right here: ~120 lines of prose, and the one role
/// prompt that was not a bundled `.md`, so it alone could not be retuned without a
/// rebuild. It was also five prompts wearing one coat — *"When your task is to file a
/// file…"*, *"When your task is to build a view…"* — which meant every worker paid the
/// context of every specialism and nothing could be said to one kind without the others
/// reading it too. Splitting it is what `CreateWorker(type)` is *for*
/// (`docs/arch/foundation.md`); until the type existed there was no way to hand a
/// worker a different prompt, which is why the conditionals were there.

/// A report a worker posts back to the reaction's per-reaction loop. It enters the
/// queue as a `LoopInput::Worker`, so it waits its turn and never interrupts
/// live speech.
pub(super) struct WorkerReport {
    pub(super) id: SessionId,
    pub(super) task: String,
    pub(super) kind: WorkerReportKind,
    /// The session this report is *for*. `None` means the reaction loop — the report
    /// becomes a signal the voice may speak to. `Some` means it travels up to the
    /// session that asked for the work, and the conversation never sees it.
    pub(super) owner: Option<SessionId>,
}

pub(super) enum WorkerReportKind {
    /// The task finished; the string is the worker's self-contained summary.
    Done(String),
    /// The task errored out (session open failed, prompt failed, etc.).
    Failed(String),
    /// The turn was cut short by [`WorkerRegistry::interrupt`]; the string is whatever
    /// the worker had produced by then, which may be nothing.
    ///
    /// Its own variant because the other two would both lie about it. `Done` is what
    /// made this bug expensive rather than merely wrong: `run.wait()` returns `Ok` on an
    /// interrupt, so a cancelled worker would report its half-finished output as a
    /// finished result — and the rung reading that closes the task. `Failed` is not true
    /// either; nothing went wrong, someone changed their mind. What the owner needs to
    /// know is that the work stopped **because it was told to**, so it can leave the
    /// ledger alone rather than closing it as delivered.
    Interrupted(String),
}

/// One live working session — **two handles, and nothing else**.
///
/// It used to also carry `task`, `transcript` and `busy`, each a second copy of
/// something the switchboard already holds. Their last reader was the voice's status
/// line, which read them for Deliberation; that line now asks `registry::global()` about
/// Cognition, and the copies went with it. What is left is the two things the switchboard
/// genuinely cannot do — cancel a running turn, and abort a task — because both are
/// handles to a live tokio object rather than facts about a session. The same argument
/// the map's own TODO makes, applied one level down.
struct Worker {
    /// The agent session itself, held so the turn it is running can be **interrupted**
    /// — the one thing a mailbox cannot do.
    ///
    /// Everything else that reaches a worker is a message, and a message is read
    /// between turns: the inbox is appended to and [`drive_worker`] picks it up only
    /// once `run_worker` has returned. So "stop" sent that way arrives after the thing
    /// it was meant to stop. A retraction has to reach *inside* the running turn, and
    /// [`AgentSession::cancel`] (`turn/interrupt`) is the only thing that does. The drive
    /// task owns the session; this is a second handle to the same `Arc`, which is what
    /// makes the cancel available to the registry without disturbing the loop.
    session: Arc<AgentSession>,
    drive: JoinHandle<()>,
}

/// The conversation's live working sessions. Owned by the per-reaction loop, so a plain
/// map suffices — no locking. Survives a reaction-session cold reopen: workers are
/// independent of the mind's own lifecycle within a conversation.
///
/// **TODO — this map is an optimization that currently optimizes nothing, and the
/// switchboard already is the process-wide session pool. Do not "move the pool";
/// delete it, whenever the code next comes through here.**
///
/// There is no pool to move: `registry::global()` is already keyed by `SessionId`,
/// process-wide, and holds `task`, `busy`, `owner` and a bounded output tail.
/// Everything in `Worker` except the `JoinHandle` is a second copy of that, and
/// the copies can disagree.
///
/// What the map is actually for, checked one by one:
///
/// - **Warm reuse** — gone with Deliberation. `follow_up` had exactly one caller,
///   `deliberate`, and a worker made by `create_worker` is never followed up: it runs
///   once and ends. So the whole warm-idle path served one client, and that client was
///   the rung this refactor retired into Cognition.
/// - **Reaping** ([`WorkerRegistry::reap`]) — bookkeeping, and the only reader of
///   `Worker::drive`. A session that has finished is the thing that knows it finished;
///   self-removal at the end of `drive_worker` replaces both, and works for a worker
///   with no reaction loop to reap it.
/// - **Status** — no longer a reader of this map at all. What the voice needs is whether
///   *Cognition* is working on the conversation's behalf, and that is the switchboard's
///   to answer ([`registry::status`] plus the tail `record_output` keeps).
///
/// So on-demand creation is fine: spawn, register, self-remove. What a spawn needs is
/// *dependencies* (memory, agent layer, observatory, views dir), not a home.
///
/// Worker ownership is now explicit: Cognition and Reflection have distinct tool
/// sinks, registration precedes session open, and no user-supplied routing key is
/// interpolated into worker prompts or data paths.
pub(super) struct WorkerRegistry {
    /// A clone of the conversation's queue sender, handed to each worker's drive task so
    /// its reports land back in the same loop.
    inbound: mpsc::Sender<LoopInput>,
    workers: HashMap<SessionId, Worker>,
}

impl WorkerRegistry {
    pub(super) fn new(inbound: mpsc::Sender<LoopInput>) -> Self {
        Self {
            inbound,
            workers: HashMap::new(),
        }
    }

    /// Start a channel-mute working session for `task`, under an id the **caller
    /// already holds**.
    ///
    /// `create_worker` answers with the id before the session is open, because a caller
    /// that cannot name what it made cannot brief it, ask after it, or read it. So the
    /// id is minted at the tool surface and carried here; minting a second one would
    /// hand back an address that names nothing. There is no id-minting variant, so that
    /// cannot drift back.
    ///
    /// Returns once the session is open and its drive task is running; the work proceeds
    /// in the background and its report goes to `owner`.
    ///
    /// `resume` names a codex thread this session should pick back up instead of opening
    /// cold — an errand the last restart killed, offered to Cognition by the boot glance. It
    /// changes only where the session's *mind* starts; the id, the owner, the prompt and the
    /// registration are the same as any other spawn, because to everything downstream this
    /// **is** a new session. Resuming is not un-ending the old one.
    pub(super) async fn spawn_with_id(
        &mut self,
        reaction: &Reaction,
        id: SessionId,
        title: String,
        task: String,
        kind: WorkerType,
        owner: Option<SessionId>,
        resume: Option<String>,
        subject: Option<String>,
    ) -> anyhow::Result<SessionId> {
        self.spawn_inner(reaction, id, title, task, kind, owner, resume, subject).await
    }

    /// The one construction path for a working session: open it, record it, drive it.
    ///
    /// It used to carry an `is_deliberation` flag threaded down through `open_working_session`,
    /// `drive_worker` and every `WorkerReport`, whose whole job was to mark one session as
    /// the conversation's own thinking rather than an errand. That rung is gone, so every
    /// session reaching here is an errand and the flag had exactly one value.
    /// `title` is the one line the errand shows up as everywhere a person or another rung
    /// reads it; `task` is the brief, and the brief's only reader is the worker — it goes
    /// out as the session's first prompt and nowhere else. See
    /// [`registry::Status::title`](crate::foundation::registry::Status::title).
    async fn spawn_inner(
        &mut self,
        reaction: &Reaction,
        id: SessionId,
        title: String,
        task: String,
        kind: WorkerType,
        owner: Option<SessionId>,
        resume: Option<String>,
        subject: Option<String>,
    ) -> anyhow::Result<SessionId> {
        let resumed = resume.is_some();
        let (session, mail) = self
            .open_working_session(reaction, id, &title, kind, owner, resume, subject)
            .await?;

        let observatory = reaction.inner.observatory.clone();
        observatory
            // A pseudo-conversation is a routing tag, never a mirror key: Cognition hosts its
            // workers under `*cognition*`, and passing that through would put a room
            // nobody is in on the dashboard's conversation list. The event still records — it
            // just describes no conversation, which is the truth about it.
            // The title, not the brief: this record is read on a dashboard, and a
            // paragraph in that slot is a paragraph nobody scrolls.
            .record(EventKind::WorkerSpawned { id, title: title.clone() })
            .await;

        let transcript = Arc::new(Mutex::new(String::new()));
        let busy = Arc::new(AtomicBool::new(true));
        // Cloned before the drive task takes it: the registry needs a handle to cancel
        // through, and the drive task needs the session to run turns on. Same `Arc`.
        let handle = Arc::clone(&session);
        let drive = tokio::spawn(drive_worker(
            id,
            Some(task.clone()),
            session,
            transcript.clone(),
            self.inbound.clone(),
            observatory,
            mail.clone(),
            busy.clone(),
            owner,
        ));


        self.workers.insert(
            id,
            Worker { session: handle, drive },
        );
        tracing::info!(
            session = id,
            owner = owner.map(|o| o.to_string()).unwrap_or_else(|| "conversation-loop".into()),
            resumed,
            "spawned working session"
        );
        Ok(id)
    }

    async fn open_working_session(
        &self,
        reaction: &Reaction,
        id: SessionId,
        title: &str,
        kind: WorkerType,
        owner: Option<SessionId>,
        resume: Option<String>,
        subject: Option<String>,
    ) -> anyhow::Result<(Arc<AgentSession>, Arc<Notify>)> {
        // **One role drives all three things this function decides**: which prompt the
        // session opens with, what the switchboard records, and which tool surface it
        // attaches. Those used to be three separate expressions over three separate types
        // — `identity::WorkerType` for the prompt, `registry::Role` for the record,
        // `agent::SessionRole` for the surface — and only the first carried the
        // specialism. That is precisely how a `view-builder` came to be registered as an
        // anonymous `worker`, with nothing downstream able to say otherwise.
        let role = Role::Worker(kind);

        let system_prompt = crate::identity::role_prompt(reaction.inner.memory.data_dir(), role).await;

        // The address exists before subprocess startup so mail can queue during both
        // eager warm-up and an ordinary create_worker spawn.
        let mail = registry::global().register(id, role, owner, title.to_string(), subject);

        let opened = reaction
            .inner
            .agent
            .session(
                role,
                Some(id),
                SessionOpts {
                    system_prompt: Some(system_prompt),
                    cwd: Some(reaction.inner.views_dir.clone()),
                    // The only place in the codebase that sets this. A failed resume falls
                    // back to a cold open inside the agent layer, so an offer that has gone
                    // stale on disk costs the errand its context and never its existence.
                    resume,
                    ..Default::default()
                },
            )
            .await;
        match opened {
            Ok(session) => Ok((Arc::new(session), mail)),
            Err(err) => {
                registry::global().unregister(id);
                Err(err)
            }
        }
    }

    /// Stop the turn `id` is running, because the person no longer wants the work.
    ///
    /// This is the half of dispatch that was missing. Handing work out was one call;
    /// taking it back was nothing at all — a retraction could only be *posted*, and a
    /// posted retraction is read after the turn it was meant to stop, which is to say
    /// never in time. The failure that earned this: an idea was floated, withdrawn
    /// twenty-five seconds later, acknowledged with "I've stopped the build work", and
    /// built anyway six minutes after that, because no code path existed behind the
    /// sentence.
    ///
    /// Interrupting is not killing. The turn unwinds through
    /// [`StopReason::Interrupted`], the worker reports what it had got to, and the
    /// session **stays warm and addressable** — so "stop, and instead…" is one cancel
    /// plus one follow-up on a session that still remembers everything. Aborting the
    /// drive task would take the context with it, which is why [`Drop`] does that and
    /// this does not.
    ///
    /// Returns whether a turn was actually interrupted. `false` is ordinary — the worker
    /// is gone, or it finished on its own in the moment between deciding to stop it and
    /// getting here — and the caller must say *that* rather than report a stop that
    /// never happened. A session sitting idle takes the cancel silently and reports
    /// nothing back, so a caller told "stopping it" would wait for a report that is never
    /// coming.
    pub(super) async fn interrupt(&self, id: SessionId) -> bool {
        let Some(w) = self.workers.get(&id) else {
            tracing::info!(worker = id, "nothing to interrupt; worker is gone");
            return false;
        };
        match w.session.cancel().await {
            Ok(true) => {
                tracing::info!(worker = id, "working session interrupted");
                true
            }
            Ok(false) => {
                tracing::info!(worker = id, "nothing to interrupt; worker was idle");
                false
            }
            Err(err) => {
                tracing::warn!(worker = id, error = %err, "interrupt failed");
                false
            }
        }
    }

    /// Forget workers whose drive task has finished, so the map doesn't grow.
    /// Their result already rode back as a report; this just drops the handle.
    /// This registry's conversation as a *mirror* key — `None` when it is a pseudo-conversation.
    ///
    /// A worker pool is hosted under a conversation, and since Cognition that conversation is sometimes
    /// `*cognition*`: a value the `/mcp` dispatch routes by and the logs label with, but
    /// which names no conversation. The observatory's mirror is keyed by conversation and its
    /// entry is created on sight, so handing it one is enough to invent a room.
    pub(super) fn reap(&mut self) {
        self.workers.retain(|id, w| {
            let alive = !w.drive.is_finished();
            if !alive {
                registry::global().unregister(*id);
            }
            alive
        });
    }

    /// Hand `text` to session `id` as if it were a follow-up, returning whether the
    /// session was still there to take it.
    ///
    /// This is how work travels **up**: a worker reports to the session that asked,
    /// which reads it on its next prompt and decides what, if anything, is worth
    /// passing further up. A worker never reaches a conversation, because a conversation is where a
    /// person is spoken to and only Reaction speaks there.
    ///
    /// Anything but `Delivered` means the owner is gone. The caller must fall back to
    /// the reaction loop rather than drop the report — losing finished work because its
    /// requester shut down is worse than surfacing it one rung too high.
    ///
    /// Returns the outcome rather than a bool so the caller can *record* the edge; a
    /// `Delivery` collapsed to `false` loses which way it failed.
    pub(super) fn deliver_to(&mut self, id: SessionId, text: String) -> registry::Delivery {
        registry::global().post(id, text)
    }

}

/// Whether the agent's own thinking is running right now.
///
/// The cheap half of [`render_status`]'s question, for the check-in floor: that one
/// builds a tail nobody needs when the question is only *is anything in flight*.
///
/// **Cognition, and nothing else.** A worker belongs to whoever created it, and this
/// loop creates none — so the roster of other people's work is not the voice's to
/// describe, and their substance comes back down the report path, which drives a turn of
/// its own.
pub(super) fn thinking() -> bool {
    registry::global()
        .session_of_role(Role::Cognition)
        .is_some_and(|status| status.busy)
}

/// One line for Reaction's turn: whether the agent is still working on what was just
/// asked, so the voice can say "still on it" with a straight face instead of guessing.
/// Empty string when there is nothing to say.
///
/// **It reads the switchboard, not a local map.** The block this replaced listed every
/// live session under `## Working sessions (delegated)`, which was wrong twice over:
/// Reaction owns none of them, and the idle rows advertised a `delegate` tool retired
/// with the old channel. What survives is the one fact the voice needs mid-conversation,
/// and since the hand-down now goes to Cognition, Cognition is the session it is about.
pub(super) fn render_status() -> String {
    let Some(status) = registry::global().session_of_role(Role::Cognition) else {
        return String::new();
    };
    if !status.busy {
        // Idle means it finished, and finishing sends the voice a message it has already
        // seen. A second mention here would read as work still in flight.
        return String::new();
    }
    let mut s = String::from("## Still looking into\n");
    let _ = write!(s, "- \"{}\"", status.title);
    if status.queued {
        s.push_str(" (with a follow-up queued behind it)");
    }
    // What it is *doing* rather than what it has said: a rung mid-shell-command has an
    // empty output tail, and a blank line there is the "silence read as health" failure
    // the `doing` field exists to end.
    if let Some(doing) = status.doing.as_deref().filter(|d| !d.trim().is_empty()) {
        let _ = write!(s, "\n    latest: {}", tail_chars(doing, 240));
    }
    s.push('\n');
    s
}

impl Drop for WorkerRegistry {
    fn drop(&mut self) {
        // JoinHandle::drop detaches. A registry owns these tasks, so leaving its scope
        // must instead stop them and remove their switchboard addresses.
        for (id, worker) in self.workers.drain() {
            worker.drive.abort();
            registry::global().unregister(id);
        }
    }
}

/// Render one report for the `## New signals` section the reaction sees.
///
/// **Every report reaching here is now a task worker's**, and it stays an observation the
/// reaction voices on its own social timing — it may already have spoken to it, or choose
/// to show a view instead of narrating.
///
/// This function used to have a second, must-relay shape for Deliberation's reports,
/// because the conversation's own thinking came back on this path and a mute-by-default
/// voice was entitled to drop it. The reading rung is Cognition now and it answers by
/// mail, so that framing moved to where the answer actually travels
/// ([`super::LoopInput::Mail::owed`]). The failure it guards against is unchanged; only
/// the path is.
pub(super) fn render_report(report: &WorkerReport) -> String {
    match &report.kind {
        WorkerReportKind::Done(answer) => format!(
            "working session {} finished — task was \"{}\":\n{}",
            report.id,
            report.task,
            answer.trim()
        ),
        WorkerReportKind::Failed(err) => format!(
            "working session {} FAILED — task was \"{}\": {}",
            report.id,
            report.task,
            err.trim()
        ),
        WorkerReportKind::Interrupted(partial) if partial.trim().is_empty() => format!(
            "working session {} was stopped before it produced anything — task was \"{}\"",
            report.id, report.task
        ),
        WorkerReportKind::Interrupted(partial) => format!(
            "working session {} was stopped part-way — task was \"{}\". What it had got to:\n{}",
            report.id,
            report.task,
            partial.trim()
        ),
    }
}

/// The same report with the prompt framing stripped: who reported, what happened,
/// and what came back. [`render_report`] speaks *to* the voice ("this is the answer
/// you owe them, relay it now") because it is building a turn; the durable log wants
/// the signal itself, so a later reader sees what the worker actually returned
/// rather than an instruction addressed to a mind that has since restarted.
pub(super) fn render_report_plainly(report: &WorkerReport) -> String {
    let (verb, body) = match &report.kind {
        WorkerReportKind::Done(answer) => ("finished", answer.trim()),
        WorkerReportKind::Failed(err) => ("failed", err.trim()),
        WorkerReportKind::Interrupted(partial) => ("was stopped", partial.trim()),
    };
    format!("worker {} {verb} — task \"{}\": {body}", report.id, report.task)
}

/// Drive one worker across one or more tasks, posting a terminal report after each
/// and staying warm in between so a follow-up can resume the same session with full
/// context. Runs as its own task so the reaction stays free; the session is closed
/// (this returns) once the worker sits idle past [`WORKER_IDLE_TTL`].
async fn drive_worker(
    id: SessionId,
    initial_task: Option<String>,
    session: Arc<AgentSession>,
    transcript: Arc<Mutex<String>>,
    inbound: mpsc::Sender<LoopInput>,
    observatory: Observatory,
    mail: Arc<Notify>,
    busy: Arc<AtomicBool>,
    owner: Option<SessionId>,
) {
    let mut next_task = initial_task;
    let mut energy = crate::foundation::energy_state::subscribe();
    let mut energy_paused = crate::foundation::energy_state::is_out();
    // A worker made by `create_worker` runs its errand and ends. It is not bounded by
    // size from here — the underlying agent compacts its own context (see
    // [`crate::body::reaction::heartbeat`]).
    let session = session;
    loop {
        let task = match next_task.take() {
            Some(task) => task,
            None => match wait_for_mail(id, &mail).await {
                Some(task) => task,
                None => {
                    tracing::info!(worker = id, "working session idle past ttl; closing");
                    return;
                }
            },
        };

        // Workers follow the same reactive contract as every other rung: no balance
        // preflight, hold the current task after a managed 402, then rerun that exact
        // task when the balance broadcasts Resume. The agent session stays alive.
        if !wait_for_energy_resume(&mut energy, &mut energy_paused).await {
            return;
        }
        busy.store(true, Ordering::Relaxed);
        registry::global().start_turn(id);
        // Paired with the "turn done" line below. Without both, a reader of the log
        // cannot tell a working session that is still building from one that finished
        // and went quiet — the two look identical, which is exactly the ambiguity the
        // completion event exists to remove.
        tracing::info!(
            worker = id,
            task_chars = task.chars().count(),
            "working session turn start"
        );
        let kind = match run_worker(id, &task, &session, &transcript).await {
            Ok((partial, StopReason::Interrupted)) => WorkerReportKind::Interrupted(partial),
            Ok((answer, _)) => WorkerReportKind::Done(answer),
            Err(err)
                if crate::foundation::energy_state::is_402_error(&err)
                    && crate::foundation::energy_state::is_out() =>
            {
                tracing::warn!(
                    worker = id,
                    "working session paused on 402; task and session held"
                );
                busy.store(false, Ordering::Relaxed);
                registry::global().finish_turn(id);
                energy_paused = true;
                next_task = Some(task);
                continue;
            }
            Err(err) => WorkerReportKind::Failed(err.to_string()),
        };
        busy.store(false, Ordering::Relaxed);
        // The turn is over — say so, or `session_status` reports every session as
        // permanently mid-turn once it has taken any mail at all.
        registry::global().finish_turn(id);
        let (state, summary_chars) = match &kind {
            WorkerReportKind::Done(answer) => (WorkerState::Done, answer.chars().count()),
            WorkerReportKind::Failed(err) => (WorkerState::Failed, err.chars().count()),
            WorkerReportKind::Interrupted(partial) => {
                (WorkerState::Interrupted, partial.chars().count())
            }
        };
        observatory
            .record(
                                EventKind::WorkerFinished { id, state, summary_chars },
            )
            .await;
        tracing::info!(
            worker = id,
            state = ?state,
            summary_chars,
            "working session turn done"
        );
        let report = WorkerReport { id, task: task.clone(), kind, owner };
        if inbound.send(LoopInput::Worker(report)).await.is_err() {
            tracing::warn!(worker = id, "worker report dropped; reaction loop gone");
            return;
        }


        // Stay warm for a follow-up; pick up everything that accumulated in the
        // inbox as one prompt. Close (return, dropping the session) once idle past
        // the TTL.
        next_task = None;
    }
}

async fn wait_for_energy_resume(
    energy: &mut tokio::sync::broadcast::Receiver<crate::foundation::energy_state::EnergyEvent>,
    paused: &mut bool,
) -> bool {
    // Apply lifecycle messages already queued for this session before starting work.
    // This is event consumption, not an account/state preflight before the provider
    // call. The process flag is consulted only if this receiver actually lagged.
    loop {
        match energy.try_recv() {
            Ok(crate::foundation::energy_state::EnergyEvent::Pause) => *paused = true,
            Ok(crate::foundation::energy_state::EnergyEvent::Resume) => *paused = false,
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                *paused = crate::foundation::energy_state::is_out();
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Closed) => return false,
        }
    }
    while *paused {
        match energy.recv().await {
            Ok(crate::foundation::energy_state::EnergyEvent::Resume) => *paused = false,
            Ok(crate::foundation::energy_state::EnergyEvent::Pause) => *paused = true,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                *paused = crate::foundation::energy_state::is_out();
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return false,
        }
    }
    true
}

/// Block until this session has mail to act on, returning it as one prompt — or
/// `None` if it sat idle past [`WORKER_IDLE_TTL`], in which case the inbox is now
/// closed, so a racing sender is told `Unknown` and starts a fresh session rather
/// than posting into a dead one.
///
/// Everything waiting is taken together and rendered with its sender, because a
/// worker may only answer *whoever asked*, and it cannot answer an address it was
/// never given.
async fn wait_for_mail(id: SessionId, mail: &Notify) -> Option<String> {
    loop {
        if let Some(batch) = registry::global().take_pending(id) {
            return Some(registry::render(&batch));
        }
        // Nothing pending — wait for a nudge or the idle TTL. `Notify` holds a permit
        // if `notify_one` raced ahead of this `notified()`, so no wakeup is lost
        // between the take above and the wait here.
        match timeout(WORKER_IDLE_TTL, mail.notified()).await {
            Ok(()) => continue, // nudged — loop back and take it
            Err(_) => {
                // Idle past the TTL. Taking and closing are one act under one lock, so
                // a follow-up that landed in the meantime still wins.
                return registry::global()
                    .take_pending_or_close(id)
                    .map(|batch| registry::render(&batch));
            }
        }
    }
}


/// Prompt the worker session with the task, streaming its output into the
/// transcript, and return the full reply as the task's result. Anything the worker
/// wants to raise mid-flight it sends to its owner with the one verb, out of band.
async fn run_worker(
    id: SessionId,
    task: &str,
    session: &AgentSession,
    transcript: &Arc<Mutex<String>>,
) -> anyhow::Result<(String, StopReason)> {
    let mut run = session.prompt(task.to_string()).await?;
    let mut full = String::new();

    loop {
        match run.next_update().await {
            Some(SessionUpdate::Text(text)) => {
                full.push_str(&text);
                transcript.lock().await.push_str(&text);
                // The switchboard keeps a bounded tail of the same stream, so an owner
                // can ask `session_messages` what this one has found without waiting
                // for it to finish. Without this the tool answers "nothing yet" for
                // the whole life of every session.
                //
                // This is also the *only* live-progress mirror now: the observatory's
                // The voice-specific worker tail is gone, because a worker's
                // progress belongs to its owner. Whoever asked for the work can read it
                // here, keyed by the session id they were handed.
                registry::global().record_output(id, &text);
            }
            // Thoughts, tool calls, and unmodelled events don't enter the
            // transcript — only the worker's text output does. They do feed the
            // switchboard's "doing" line, which is the other question a reader asks: a
            // worker four minutes into a shell command has said nothing, and its silence
            // used to be indistinguishable from being dead.
            Some(update) => {
                if let Some(what) = update.activity() {
                    registry::global().record_activity(id, &what);
                }
            }
            None => break,
        }
    }

    // The stop reason is carried out rather than dropped: `wait` returns `Ok` for an
    // interrupted turn just as it does for a completed one, so discarding it is what
    // made a cancelled worker indistinguishable from a finished one.
    let result = run.wait().await?;
    Ok((full.trim().to_string(), result.stop_reason))
}

/// Last `n` characters of `s`, flattened to a single line for a status tail.
fn tail_chars(s: &str, n: usize) -> String {
    let trimmed = s.trim();
    let start = trimmed.chars().count().saturating_sub(n);
    let tail: String = trimmed.chars().skip(start).collect();
    tail.replace('\n', " ").trim().to_string()
}

#[cfg(test)]
mod ownership_tests {
    use super::*;

    fn registry() -> WorkerRegistry {
        let (tx, _rx) = mpsc::channel(8);
        WorkerRegistry::new(tx)
    }

    /// The fallback that keeps finished work from vanishing. An owner can shut down
    /// while the worker it asked for is still running; when that happens `deliver_to`
    /// must *say so* rather than quietly accept the report, so the caller can surface
    /// it to the conversation instead. A silent success here would lose completed work in the
    /// one case nobody would think to test by hand.
    ///
    /// Asserted as `Unknown` specifically, not merely "not delivered": the caller now
    /// records the outcome, and *why* it failed is the difference between "the owner
    /// finished" and "a worker addressed someone it shouldn't".
    #[test]
    fn delivering_to_a_vanished_owner_reports_failure_rather_than_swallowing_it() {
        let mut reg = registry();
        assert_eq!(
            reg.deliver_to(4242, "the errand is done".into()),
            registry::Delivery::Unknown
        );
    }


    fn report(kind: WorkerReportKind) -> WorkerReport {
        WorkerReport {
            id: 7,
            task: "build the 1-pager skill".into(),
            kind,
            owner: Some(2),
        }
    }

    /// The bug this whole change exists for, at the point it did its damage.
    ///
    /// `SessionRun::wait` returns `Ok` for an interrupted turn exactly as it does for a
    /// completed one, so a cancelled worker's half-finished output used to travel up as
    /// `Done` — and the rung reading a `Done` report closes the task as delivered. The
    /// person cancels; the ledger records success; nobody can tell from the record that
    /// anything was stopped. So the report must *say* it was stopped, in both renderings:
    /// the one the mind reads and the one written down for whoever reads it next.
    #[test]
    fn an_interrupted_worker_never_reports_as_finished() {
        let stopped = report(WorkerReportKind::Interrupted("got as far as the outline".into()));
        for text in [render_report(&stopped), render_report_plainly(&stopped)] {
            assert!(text.contains("stopped"), "should say it was stopped: {text}");
            assert!(!text.contains("finished"), "must not read as finished: {text}");
        }

        // And the partial work still travels — a stop is not a reason to throw away what
        // was already learned, since the usual next move is to redirect the same session.
        assert!(render_report(&stopped).contains("got as far as the outline"));
    }

    /// A cancel that lands before the worker has said anything is the *common* case —
    /// it is what "stop it now" is supposed to achieve — so it needs a sentence of its
    /// own rather than the part-way one trailing an empty section where the work goes.
    #[test]
    fn a_worker_stopped_before_it_produced_anything_says_so() {
        let text = render_report(&report(WorkerReportKind::Interrupted(String::new())));
        assert!(text.contains("before it produced anything"), "{text}");
    }

    /// Session ids are process-wide, not per registry, so ownership is
    /// unambiguous across every role.
    #[test]
    fn session_ids_are_unique_across_registries() {
        let (a, b, c) = (mint_session_id(), mint_session_id(), mint_session_id());
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
    }
}
