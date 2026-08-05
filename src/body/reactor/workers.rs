//! Working sessions — the reactor's hands.
//!
//! The reactor keeps a single voice and must never block the floor on slow
//! work, so heavy or long-running tasks are delegated here. A worker is a
//! *voice-mute capability within the scene*: it has the full substrate — the
//! scene's memory, tools, code execution, and its own sub-agents — but no voice
//! of its own. Those sub-agents live *inside* its session and are invisible here:
//! they get no hi-agent session id, no address, and no registry entry, which is
//! why `create_worker` stays Cognition's and Reflection's (`docs/arch/agents.md`). It never speaks and never draws on the screen: it
//! cannot emit on the reactor's expression channels (thought, audio, view). That
//! mute-ness is what preserves single-voice coherence: only the reactor expresses
//! to the person.
//!
//! It is *not*, however, channel-blind. Over hi-agent's own HTTP surface
//! (`HI_AGENT_BASE_URL` in its env) a worker may **perceive input channels**
//! (e.g. `GET /api/in/vision` for live frames) — running detection, CV, whatever
//! the task needs on the raw bytes, all *outside* the turn loop so it never
//! contends with the reactor's serialized speech. It does not write to any output
//! channel: expression (speech and views alike) stays the reactor's, so a worker
//! reports what it found and the reactor decides what to show.
//!
//! The collaboration bus is asynchronous: a worker runs to completion, then posts a
//! [`WorkerReport`] to whoever asked for the work. It never interrupts live speech —
//! the report waits its turn like any other input. Mid-flight it may reach its owner
//! with `send_message`, the one verb, which does not wait for a reply; a worker that
//! hits ambiguity notes its best assumption and keeps going (fix-forward), so the
//! floor is never held waiting on an answer.
//!
//! Progress-checking is emergent rather than wired: each worker streams its
//! output into an inspectable transcript, and [`WorkerRegistry::render_status`]
//! surfaces a live tail of every running worker into the reactor's prompt, so
//! the mind can decide on its own social timing whether to wait, nudge, or
//! speak to what it sees.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::{Mutex, Notify, mpsc};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::foundation::acp::{AcpSession, SessionOpts, SessionUpdate};
use crate::foundation::agent::SessionRole;
use crate::foundation::observatory::{EventKind, Observatory, WorkerState};
use crate::identity::WorkerType;
use crate::mind::memory::layout;
use crate::types::Scene;

use super::{LoopInput, Reactor};

/// Handle for one **agent session**, unique process-wide.
///
/// It identifies a *session*, not a role: the same role has many sessions over a run,
/// and two scenes' Deliberations are two sessions of one role. So this is never a
/// "worker id" — ownership, addressing and reporting all key on the session, which is
/// the thing that is actually singular.
///
/// Process-wide rather than per-scene because ownership crosses scenes: a sceneless
/// owner (Cognition, Reflection) holds sessions that no scene counter could name
/// without colliding.
///
/// Minted before the ACP session is opened, because the MCP surface identifies its
/// caller by this id in a request header — so it cannot be the id the adapter assigns.
pub(super) type SessionId = u64;

use crate::foundation::registry;

fn mint_session_id() -> SessionId {
    registry::mint()
}

/// How long a finished working session stays warm — its ACP session held open and
/// resumable via `delegate worker:<id>` — before it closes itself to free the
/// subprocess context. A refinement arriving within this window continues the same
/// session with full context; a later one falls back to a fresh worker.
const WORKER_IDLE_TTL: Duration = Duration::from_secs(15 * 60);

// A worker used to keep a private follow-up mailbox here, beside the switchboard's.
// Two mailboxes for one session is one mailbox too many: whichever the sender picked
// decided whether the message was ever read, and only one of them had a reader. The
// switchboard's inbox is now the only one — it already merges a burst, already carries
// the sender, and already knows how to shut itself when a session idles out.

/// A working session's system prompt now lives in `prompts/workers/` — `common.md`
/// plus one file per [`WorkerType`] — and is assembled by
/// [`crate::identity::worker_prompt`].
///
/// It used to be a `const &str` right here: ~120 lines of prose, and the one role
/// prompt that was not a bundled `.md`, so it alone could not be retuned without a
/// rebuild. It was also five prompts wearing one coat — *"When your task is to file a
/// file…"*, *"When your task is to build a view…"* — which meant every worker paid the
/// context of every specialism and nothing could be said to one kind without the others
/// reading it too. Splitting it is what `CreateWorker(type)` is *for*
/// (`docs/arch/foundation.md`); until the type existed there was no way to hand a
/// worker a different prompt, which is why the conditionals were there.

/// A report a worker posts back to the reactor's per-scene loop. It enters the
/// queue as a `LoopInput::Worker`, so it waits its turn and never interrupts
/// live speech.
pub(super) struct WorkerReport {
    pub(super) id: SessionId,
    pub(super) task: String,
    pub(super) kind: WorkerReportKind,
    /// Whether this is the scene's **Deliberation** (vs. an ordinary task worker).
    /// Deliberation is the scene's own reading and thinking, so its result is not one
    /// signal among many the voice may note or drop — it is the substance of a reply
    /// Reaction asked for, and [`render_report`] frames it as a must-relay instruction.
    /// A plain worker's report stays an observation the voice surfaces on its own
    /// social timing.
    pub(super) is_deliberation: bool,
    /// The session this report is *for*. `None` means the scene loop — the report
    /// becomes a signal the voice may speak to. `Some` means it travels up to the
    /// session that asked for the work, and the scene never sees it.
    pub(super) owner: Option<SessionId>,
}

pub(super) enum WorkerReportKind {
    /// The task finished; the string is the worker's self-contained summary.
    Done(String),
    /// The task errored out (session open failed, prompt failed, etc.).
    Failed(String),
}

/// One live working session. The registry holds it to inspect its transcript, to
/// resume it with follow-up tasks, and to know when its drive task has finally
/// exited; the drive task owns the session itself and closes it once it goes idle
/// past the TTL (or is told to stop).
struct Worker {
    /// The current (or most recent) task — updated on each follow-up — for status
    /// lines and the reports posted back.
    task: String,
    /// The worker's accumulated (channel-stripped) output, grown by its drive
    /// task and read by [`WorkerRegistry::render_status`].
    transcript: Arc<Mutex<String>>,
    /// Whether the drive loop is mid-prompt right now, vs. idle and resumable.
    busy: Arc<AtomicBool>,
    // A copy of the owner used to live here, read by `render_status` to print
    // "[under session {o}]" in the roster of other people's workers. That roster is
    // gone, and the switchboard holds the authoritative owner (`registry::global()`),
    // so a second copy could only ever disagree with it — which is the argument
    // `WorkerRegistry`'s own TODO makes about this whole map.
    drive: JoinHandle<()>,
}

/// The scene's live working sessions. Owned by the per-scene loop, so a plain
/// map suffices — no locking. Survives a reactor-session cold reopen: workers are
/// independent of the mind's own lifecycle within a scene.
///
/// **TODO — this map is an optimization that currently optimizes nothing, and the
/// switchboard already is the process-wide session pool. Do not "move the pool";
/// delete it, whenever the code next comes through here.**
///
/// It looked like the pool was per-scene and had to be re-homed, because a worker
/// belongs to Cognition or Reflection and neither has a scene. But there is no pool to
/// move — `registry::global()` is already keyed by `SessionId`, process-wide, and holds
/// `task`, `busy`, `owner`, `scene` and a bounded output tail. Everything in `Worker`
/// except the `JoinHandle` is a second copy of that, and the copies can disagree.
///
/// What the map is actually for, checked one by one:
///
/// - **Warm reuse** ([`WorkerRegistry::follow_up`]) — the thing a pool exists for. Its
///   only caller is [`WorkerRegistry::deliberate`]. A worker made by `create_worker` is
///   never followed up; it runs once and ends. So the whole `WORKER_IDLE_TTL` warm-idle
///   machinery serves exactly one client, and that client is **Deliberation** — one
///   long-lived session per scene, which the scene already tracks in the single
///   `deliberation` field below. A map is not needed to hold one id.
/// - **Reaping** ([`WorkerRegistry::reap`]) — bookkeeping, and the only reader of
///   `Worker::drive`. A session that has finished is the thing that knows it finished;
///   self-removal at the end of `drive_worker` replaces both, and works for a worker
///   with no scene loop to reap it.
/// - **Status** ([`WorkerRegistry::render_status`]) — the only genuine reader, and it
///   wants metadata the switchboard already has (`registry::status`), plus a transcript
///   tail the switchboard also keeps (`record_output`).
///
/// So on-demand creation is fine: spawn, register, self-remove. What a spawn needs is
/// *dependencies* (memory, agent layer, observatory, views dir), not a home.
///
/// **The three provenance bugs this used to list are all fixed**, and the way the first
/// one went is worth keeping, because it is the pattern:
///
/// 1. ~~`any_host()` lends an **arbitrary** scene to host a sceneless-owned worker,~~
///    which then became the worker's `X-HI-Scene` (so `watch`/`see` resolved to a
///    stranger's camera), the `{scene}` in its prompt, and the scene its report was
///    journaled under — hosting used as provenance. **Fixed by deletion.** Both rungs
///    that dispatch now register a sink under their own sentinel scene, so the lookup
///    succeeds and there is nothing to borrow. A rung that dispatches work hosts its own
///    workers.
/// 2. ~~`create_worker` answers "brief it with `send_message`" before `register` runs.~~
///    **Fixed** — registration now precedes the session open, with `unregister` on the
///    spawn-failure path.
/// 3. ~~The `{scene}` in `memory/raw/{scene}/file/` is interpolated raw where the
///    directory is percent-encoded.~~ **Fixed** — `{scene_dir}` is a separate
///    substitution, because the word meant two different things in the two places.
pub(super) struct WorkerRegistry {
    scene: Scene,
    /// A clone of the scene's queue sender, handed to each worker's drive task so
    /// its reports land back in the same loop.
    inbound: mpsc::Sender<LoopInput>,
    workers: HashMap<SessionId, Worker>,
    /// The scene's persistent **Deliberation**, if spawned — the rung that reads a
    /// little, checks the file, looks at the photo, and works out what was actually
    /// asked, per scene, so no scene ever waits on another. Reaction follows up with it
    /// every turn; followed up rather than respawned, so it keeps full context.
    /// `None` until the first turn that needs it. See [`WorkerRegistry::deliberate`].
    deliberation: Option<SessionId>,
}

impl WorkerRegistry {
    pub(super) fn new(scene: Scene, inbound: mpsc::Sender<LoopInput>) -> Self {
        Self {
            scene,
            inbound,
            workers: HashMap::new(),
            deliberation: None,
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
    pub(super) async fn spawn_with_id(
        &mut self,
        reactor: &Reactor,
        id: SessionId,
        task: String,
        kind: WorkerType,
        owner: Option<SessionId>,
    ) -> anyhow::Result<SessionId> {
        self.spawn_inner(reactor, id, task, kind, false, owner).await
    }

    /// `spawn`, plus the `is_deliberation` flag that tags every report this worker posts
    /// so the voice can tell the scene's own thinking (must-relay) from a task worker's
    /// observation. Deliberation goes through [`deliberate`](Self::deliberate);
    /// everything else through [`spawn`](Self::spawn) with the flag false.
    async fn spawn_inner(
        &mut self,
        reactor: &Reactor,
        id: SessionId,
        task: String,
        kind: WorkerType,
        is_deliberation: bool,
        owner: Option<SessionId>,
    ) -> anyhow::Result<SessionId> {

        // Deliberation gets **one file**, like every other rung. It used to be three
        // layers — the seed it Read its character from, the worker capability guidance,
        // then its role — which is why it kept coming out shaped like a worker with a
        // flag. `deliberation.md` is self-contained now and carries only what this rung
        // can actually do: it has `send_message` and its built-ins, no `look`, no `act`,
        // no `create_worker`.
        let system_prompt = if is_deliberation {
            let data_dir = reactor.inner.memory.data_dir();
            // The agent is told to create this itself, but a directory that already
            // exists is one less thing between it and the write.
            if let Some(parent) = layout::scene_prompt_path(data_dir, &self.scene).parent() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    tracing::warn!(scene = %self.scene, error = %e, "could not pre-create the scene prompt dir");
                }
            }
            crate::identity::deliberation_prompt(data_dir, &self.scene).await
        } else {
            crate::identity::worker_prompt(reactor.inner.memory.data_dir(), &self.scene, kind).await
        };

        // Announce it to the switchboard **before opening the session**, not merely
        // before the drive task starts. `create_worker` hands the id back and tells its
        // caller to "brief it with send_message" — and opening the session is a
        // subprocess spawn, so registering after it left a window, measured in seconds,
        // where an obedient model did exactly as instructed and got `Delivery::Unknown`.
        // The tool's own reply was walking callers into a race.
        //
        // The order was presumably the other way round to avoid leaking an entry when
        // the spawn fails; that is handled below instead, which is the cheaper half of
        // the trade.
        let mail = registry::global().register(
            id,
            if is_deliberation { registry::Role::Deliberation } else { registry::Role::Worker },
            Some(self.scene.clone()),
            owner,
            task.clone(),
        );

        let opened = reactor
            .inner
            .agent
            .session(
                &self.scene,
                // The role the session is *opened* as is what its `X-HI-Role` header
                // says, which is what picks its tool surface. Deliberation was opened as
                // `SessionRole::Worker` — so the registry called it Deliberation while
                // the tool surface called it a worker, and it got `look`/`act`/`watch`
                // it has no business with. `SessionRole::Deliberation` existed the whole
                // time and was never constructed.
                if is_deliberation { SessionRole::Deliberation } else { SessionRole::Worker },
                Some(id),
                SessionOpts {
                    system_prompt: Some(system_prompt),
                    // The worker's cwd is the agent's view workshop, so a
                    // build sub-agent works in a real project dir (ls/write).
                    cwd: Some(reactor.inner.views_dir.clone()), builtin_tools: None,
                },
            )
            .await;
        let session = match opened {
            Ok(s) => Arc::new(s),
            Err(err) => {
                // Take the address back, or a failed spawn leaves a live-looking entry
                // that accepts mail nothing will ever read.
                registry::global().unregister(id);
                return Err(err);
            }
        };

        let observatory = reactor.inner.observatory.clone();
        observatory
            // A pseudo-scene is a routing tag, never a mirror key: Cognition hosts its
            // workers under `*cognition*`, and passing that through would put a room
            // nobody is in on the dashboard's scene list. The event still records — it
            // just describes no conversation, which is the truth about it.
            .record(self.mirror_scene(), EventKind::WorkerSpawned { id, task: task.clone() })
            .await;

        let transcript = Arc::new(Mutex::new(String::new()));
        let busy = Arc::new(AtomicBool::new(true));
        let drive = tokio::spawn(drive_worker(
            id,
            task.clone(),
            session,
            transcript.clone(),
            self.inbound.clone(),
            observatory,
            self.scene.clone(),
            mail.clone(),
            busy.clone(),
            is_deliberation,
            owner,
        ));


        self.workers.insert(
            id,
            Worker {
                task,
                transcript,
                busy,
                drive,
            },
        );
        tracing::info!(
            scene = %self.scene,
            session = id,
            owner = owner.map(|o| o.to_string()).unwrap_or_else(|| "scene-loop".into()),
            "spawned working session"
        );
        Ok(id)
    }

    /// Resume an existing warm worker with a follow-up `task`, so a refinement
    /// continues the SAME session — full context, no clobbering — instead of a cold
    /// fresh one. The task is *merged* into the worker's mailbox: if it's still
    /// mid-prompt the task is concatenated onto whatever else is pending and the
    /// whole lot is picked up in one go when it next goes free; if it's idle-waiting,
    /// this wakes it. When the target is gone (its idle session already closed, or it
    /// shut down between our lookup and the merge), falls back to spawning a fresh
    /// worker so the request is never silently lost.
    pub(super) async fn follow_up(
        &mut self,
        reactor: &Reactor,
        id: SessionId,
        task: String,
        is_deliberation: bool,
        owner: Option<SessionId>,
    ) -> anyhow::Result<SessionId> {
        if let Some(w) = self.workers.get_mut(&id) {
            // Into the switchboard inbox, which decides the race for us: a worker
            // that closed itself between our lookup and this send reports `Unknown`
            // rather than swallowing the task.
            let delivery = registry::global().post(id, task.clone());
            // A host-posted edge: `from: None`, because the host is not an agent. Same
            // event as `send_message`, so the inspector shows every crossing on one
            // timeline rather than only the ones an agent initiated.
            reactor
                .inner
                .observatory
                .record(
                    Some(&self.scene).filter(|s| !s.is_pseudo()),
                    EventKind::MessageSent {
                        from: None,
                        to: id,
                        delivery,
                        message: task.clone(),
                    },
                )
                .await;
            if matches!(delivery, registry::Delivery::Delivered) {
                w.task = task.clone();
                reactor
                    .inner
                    .observatory
                    .record(self.mirror_scene(), EventKind::WorkerResumed { id, task })
                    .await;
                tracing::info!(scene = %self.scene, worker = id, "merged follow-up into working session");
                return Ok(id);
            }
            // The worker closed itself (idle past TTL) before we got the lock; drop
            // the stale handle and fall through to a fresh spawn.
            self.workers.remove(&id);
            registry::global().unregister(id);
        }
        tracing::info!(scene = %self.scene, worker = id, "follow-up target gone; spawning fresh worker");
        // `WorkerType::General` rather than a threaded parameter: this method has
        // exactly one caller, `deliberate`, and Deliberation's base layer *is* the
        // general worker's — it is a working session with a role layer on top, not a
        // specialism. A parameter with one possible value is a parameter that lies.
        self.spawn_inner(reactor, mint_session_id(), task, WorkerType::General, is_deliberation, owner)
            .await
    }

    /// Ensure the scene's persistent **Deliberation** is working on `task`: resume the
    /// warm one if it exists (full context, no clobbering), else spawn it. Reaction
    /// follows up with it each turn that carries a human request, so the scene keeps
    /// reading and thinking off the floor while the voice speaks; its output rides back
    /// as an ordinary [`WorkerReport`] Reaction articulates.
    /// [`follow_up`](Self::follow_up) already falls back to a fresh spawn if the tracked
    /// worker has gone, so the id is re-stored from whatever it returns.
    ///
    /// Anything heavy — a real task, a standing duty, a long errand — belongs *up* at
    /// Cognition rather than here. Deliberation stays light on purpose: it exists so a
    /// scene can think without leaving the scene.
    pub(super) async fn deliberate(&mut self, reactor: &Reactor, task: String) -> anyhow::Result<()> {
        let id = match self.deliberation {
            Some(id) => self.follow_up(reactor, id, task, true, None).await?,
            None => {
                self.spawn_inner(reactor, mint_session_id(), task, WorkerType::General, true, None)
                    .await?
            }
        };
        self.deliberation = Some(id);
        Ok(())
    }

    /// Forget workers whose drive task has finished, so the map doesn't grow.
    /// Their result already rode back as a report; this just drops the handle.
    /// This registry's scene as a *mirror* key — `None` when it is a pseudo-scene.
    ///
    /// A worker pool is hosted under a scene, and since Cognition that scene is sometimes
    /// `*cognition*`: a value the `/mcp` dispatch routes by and the logs label with, but
    /// which names no conversation. The observatory's mirror is keyed by scene and its
    /// entry is created on sight, so handing it one is enough to invent a room.
    fn mirror_scene(&self) -> Option<&Scene> {
        Some(&self.scene).filter(|s| !s.is_pseudo())
    }

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
    /// passing further up. A worker never reaches a scene, because a scene is where a
    /// person is spoken to and only Reaction speaks there.
    ///
    /// Anything but `Delivered` means the owner is gone. The caller must fall back to
    /// the scene loop rather than drop the report — losing finished work because its
    /// requester shut down is worse than surfacing it one rung too high.
    ///
    /// Returns the outcome rather than a bool so the caller can *record* the edge; a
    /// `Delivery` collapsed to `false` loses which way it failed.
    pub(super) fn deliver_to(&mut self, id: SessionId, text: String) -> registry::Delivery {
        registry::global().post(id, text)
    }

    /// Whether this scene's own thinking is still running, for injection into
    /// Reaction's turn. Empty string when there is nothing to say.
    ///
    /// **One line, about Deliberation, and nothing else.** The block used to list every
    /// live session in the map under `## Working sessions (delegated)`, which was wrong
    /// twice over. Reaction *owns none of them* — a worker belongs to the session that
    /// created it, and Reaction creates none — so it was reading a roster of other
    /// people's work it could neither steer nor report on. And the idle rows told it to
    /// `delegate with worker:<id> to continue it`, naming a tool retired with the old
    /// channel: the voice's own window was the last place still advertising it.
    ///
    /// What survives is the thing the block was actually for. Reaction hands the
    /// question down to its Deliberation and keeps talking; the one fact it needs back
    /// mid-conversation is *am I still looking into this* — so it can say "still on it"
    /// with a straight face instead of guessing. Anything a worker produces reaches the
    /// voice as a report, on the report path, which is where it belongs.
    pub(super) async fn render_status(&self) -> String {
        let Some(id) = self.deliberation else {
            return String::new();
        };
        let Some(w) = self.workers.get(&id) else {
            // Tracked but gone — it closed itself past the TTL. Nothing to say rather
            // than a line about a session that is not there.
            return String::new();
        };
        if !w.busy.load(Ordering::Relaxed) {
            // Idle means it finished, and finishing posts a report the voice has
            // already seen. A second mention here would read as work still in flight.
            return String::new();
        }
        let mut s = String::from("## Still looking into
");
        let _ = write!(s, "- \"{}\"", w.task);
        if registry::global().status(id).is_some_and(|st| st.queued) {
            s.push_str(" (with a follow-up queued behind it)");
        }
        let tail = {
            let t = w.transcript.lock().await;
            tail_chars(&t, 240)
        };
        if !tail.is_empty() {
            let _ = write!(s, "\n    latest: {tail}");
        }
        s.push('\n');
        s
    }
}

/// Render one report for the `## New signals` section the reactor sees.
///
/// A **Deliberation** report is the scene's own thinking coming back — the answer to
/// something it told the person it would look into — so it is framed as a *must-relay
/// instruction*, not a passive "a worker finished" line the voice might note in passing
/// and drop. This is the fix for that substance never reaching the person: the
/// result was structurally optional, one signal among many a mute-by-default voice
/// discarded. A plain **task worker** report stays an observation the reactor voices on
/// its own social timing (it may already have spoken to it, or choose to show a view
/// instead of narrating). Both still pass through the reactor's judgment — must-relay
/// means "this is a reply you owe them," not "dump it verbatim": the reactor still says
/// it in its own plain words, reconciling with what it already said.
pub(super) fn render_report(report: &WorkerReport) -> String {
    match &report.kind {
        WorkerReportKind::Done(answer) if report.is_deliberation => format!(
            "Your thinking on \"{}\" is back — this is the answer you owe the person, so \
relay what matters here in your own plain words now (don't leave them waiting, and \
don't just acknowledge it — tell them what you found):\n{}",
            report.task,
            answer.trim()
        ),
        WorkerReportKind::Done(answer) => format!(
            "working session {} finished — task was \"{}\":\n{}",
            report.id,
            report.task,
            answer.trim()
        ),
        WorkerReportKind::Failed(err) if report.is_deliberation => format!(
            "Your thinking on \"{}\" hit a wall: {} — let the person know you couldn't get \
there (plainly, no jargon), rather than going silent.",
            report.task,
            err.trim()
        ),
        WorkerReportKind::Failed(err) => format!(
            "working session {} FAILED — task was \"{}\": {}",
            report.id,
            report.task,
            err.trim()
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
    };
    let who = if report.is_deliberation {
        "deliberation".to_string()
    } else {
        format!("worker {}", report.id)
    };
    format!("{who} {verb} — task \"{}\": {body}", report.task)
}

/// Drive one worker across one or more tasks, posting a terminal report after each
/// and staying warm in between so a follow-up can resume the same session with full
/// context. Runs as its own task so the reactor stays free; the session is closed
/// (this returns) once the worker sits idle past [`WORKER_IDLE_TTL`].
async fn drive_worker(
    id: SessionId,
    initial_task: String,
    session: Arc<AcpSession>,
    transcript: Arc<Mutex<String>>,
    inbound: mpsc::Sender<LoopInput>,
    observatory: Observatory,
    scene: Scene,
    mail: Arc<Notify>,
    busy: Arc<AtomicBool>,
    is_deliberation: bool,
    owner: Option<SessionId>,
) {
    let mut task = initial_task;
    // Deliberation is long-lived per scene; a worker made by `create_worker` runs its
    // errand and ends. Neither is bounded by size from here — the underlying agent
    // compacts its own context (see [`crate::body::reactor::heartbeat`]).
    let session = session;
    loop {
        busy.store(true, Ordering::Relaxed);
        // Paired with the "turn done" line below. Without both, a reader of the log
        // cannot tell a working session that is still building from one that finished
        // and went quiet — the two look identical, which is exactly the ambiguity the
        // completion event exists to remove.
        tracing::info!(
            scene = %scene,
            worker = id,
            task_chars = task.chars().count(),
            "working session turn start"
        );
        let kind = match run_worker(id, &task, &session, &transcript).await {
            Ok(answer) => WorkerReportKind::Done(answer),
            Err(err) => WorkerReportKind::Failed(err.to_string()),
        };
        busy.store(false, Ordering::Relaxed);
        // The turn is over — say so, or `session_status` reports every session as
        // permanently mid-turn once it has taken any mail at all.
        registry::global().finish_turn(id);
        let (state, summary_chars) = match &kind {
            WorkerReportKind::Done(answer) => (WorkerState::Done, answer.chars().count()),
            WorkerReportKind::Failed(err) => (WorkerState::Failed, err.chars().count()),
        };
        observatory
            .record(
                Some(&scene).filter(|s| !s.is_pseudo()),
                EventKind::WorkerFinished { id, state, summary_chars },
            )
            .await;
        tracing::info!(
            scene = %scene,
            worker = id,
            state = ?state,
            summary_chars,
            "working session turn done"
        );
        let report = WorkerReport { id, task: task.clone(), kind, is_deliberation, owner };
        if inbound.send(LoopInput::Worker(report)).await.is_err() {
            tracing::warn!(worker = id, "worker report dropped; scene loop gone");
            return;
        }


        // Stay warm for a follow-up; pick up everything that accumulated in the
        // inbox as one prompt. Close (return, dropping the session) once idle past
        // the TTL.
        match wait_for_mail(id, &mail).await {
            Some(next) => task = next,
            None => {
                tracing::info!(scene = %scene, worker = id, "working session idle past ttl; closing");
                return;
            }
        }
    }
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
    session: &AcpSession,
    transcript: &Arc<Mutex<String>>,
) -> anyhow::Result<String> {
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
                // per-scene worker tail is gone, because a worker's progress is not a
                // fact about a conversation. Whoever asked for the work can read it
                // here, keyed by the session id they were handed.
                registry::global().record_output(id, &text);
            }
            // Thoughts, tool calls, and unmodelled events don't enter the
            // transcript — only the worker's text output does.
            Some(_) => {}
            None => break,
        }
    }

    run.wait().await?;
    Ok(full.trim().to_string())
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
        WorkerRegistry::new(Scene("alice@phone".into()), tx)
    }

    /// The fallback that keeps finished work from vanishing. An owner can shut down
    /// while the worker it asked for is still running; when that happens `deliver_to`
    /// must *say so* rather than quietly accept the report, so the caller can surface
    /// it to the scene instead. A silent success here would lose completed work in the
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


    /// Session ids are process-wide, not per registry — two scenes must never mint
    /// the same id, or a report would be delivered into the wrong conversation.
    #[test]
    fn session_ids_are_unique_across_registries() {
        let (a, b, c) = (mint_session_id(), mint_session_id(), mint_session_id());
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
    }
}
