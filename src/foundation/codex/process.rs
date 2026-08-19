//! `codex app-server` subprocess lifecycle and its JSON-RPC connection.
//!
//! Owns one child process and the newline-delimited JSON-RPC 2.0 stream to it,
//! hosting exactly **one** codex thread. [`spawn`](CodexProcess::spawn) returns the
//! process plus the single stream of raw notifications, and
//! [`open_thread`](CodexProcess::open_thread) opens that one thread. There is no
//! thread-id demux — every notification on the connection belongs to the one thread,
//! so they flow straight to that stream.
//!
//! **The codec is hand-rolled on `serde_json::Value`, deliberately.** The obvious
//! alternative is the `codex-app-server-protocol` crate, but it is a workspace-internal
//! crate of a fast-moving repo, and we need six methods out of ninety. Speaking `Value`
//! also *is* the frame contract (`docs/arch/foundation.md#full-frames-not-modelled-events`):
//! a notification this build has never heard of still reaches the log and the inspector
//! whole, because recording does not require understanding.
//!
//! Codex also asks *us* things — approvals, MCP elicitations, permission escalations —
//! and those do not share a response shape. See [`answer_server_request`] for what each
//! gets and why anything unrecognised is refused rather than guessed at.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::foundation::codex::tap::{Dir, WireTap};

/// Allocates the per-connection id the tap uses to group one session's frames
/// (one subprocess hosts one thread). Process-global and monotonic.
static CONN_SEQ: AtomicU64 = AtomicU64::new(0);

/// What this client calls itself in the `initialize` handshake. Codex uses
/// `clientInfo.name` to identify integrations, so it should name us, not a library.
const CLIENT_NAME: &str = "hi_agent";

/// Options for opening a session's codex thread.
#[derive(Debug, Default, Clone)]
pub struct SessionOpts {
    /// The rung's prompt, which becomes the thread's `baseInstructions` — codex's
    /// full replacement for the built-in system prompt.
    ///
    /// This is the seam ACP could not offer: there, a rung's prompt was prepended to
    /// the first `session/prompt` as user content and the agent's own persona still
    /// framed it. Here it is verified on the upstream wire as the request's
    /// `instructions` field, with nothing underneath it.
    pub system_prompt: Option<String>,

    /// Working directory for the thread. Codex requires an absolute path.
    pub cwd: Option<PathBuf>,

    /// Sandbox mode for the thread's own tools, as codex spells it: `read-only`,
    /// `workspace-write`, or `danger-full-access`.
    ///
    /// **Kebab-case, and the spelling is load-bearing** — the app-server rejects
    /// `dangerFullAccess` outright even though the published protocol docs show
    /// camelCase for a later version. Pinned by [`Sandbox::as_str`].
    ///
    /// Ignored when [`permission_profile`](Self::permission_profile) names one: codex
    /// answers `permissions cannot be combined with sandbox`, because a profile carries
    /// its own sandbox. The profile's `sandbox` field is where the setting goes then.
    pub sandbox: Sandbox,

    /// The named permission profile this thread opens under, defined by the caller in
    /// [`config`](Self::config) under `permissions.<name>`.
    ///
    /// **This is a `thread/start` parameter, not a config key.** Setting the config's
    /// `default_permissions` instead — which is how a profile is selected in
    /// `config.toml` — is accepted, ignored, and never reported: 0.147 answers a
    /// `thread/read` with `activePermissionProfile: null` and hands the thread every
    /// built-in it has. That silent no-op is what left Reaction holding a shell for
    /// months, and it is why the profile now rides the parameter that errors when it
    /// cannot be honoured.
    pub permission_profile: Option<String>,

    /// Codex config overrides for this thread — which model, over which provider, and
    /// which MCP servers to attach.
    ///
    /// This is codex's *session-flags* layer, the one `codex -c key=value` writes, so it
    /// applies to this thread alone and never touches config on disk. It is how a
    /// session's MCP attach carries per-session HTTP headers, which is what makes one
    /// `/mcp` endpoint serve five rungs with five different tool surfaces.
    pub config: serde_json::Map<String, Value>,

    /// A specific codex thread this session should pick back up, overriding the per-role
    /// resume policy.
    ///
    /// **Not a codex parameter** — it is consumed by
    /// [`AgentLayer::session`](crate::foundation::agent::AgentLayer::session) and never
    /// reaches the wire, which is why it can sit here beside fields that do. It lives on
    /// `SessionOpts` rather than in `session`'s signature so the four call sites with no
    /// opinion keep saying `..Default::default()`.
    ///
    /// The one caller is a working session created with `resume` — Cognition taking up an
    /// errand the last restart killed. A rung never sets it: which thread a *rung* comes back
    /// on is the host's decision and stays in `take_resumable`, where no call site can be
    /// given a different rule by accident.
    pub resume: Option<String>,
}

/// The sandbox modes codex offers, in its own spelling.
///
/// Model-driven workers use [`WorkspaceWrite`](Self::WorkspaceWrite): they can work
/// in their project tree but cannot open arbitrary network connections around the
/// host's privacy broker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sandbox {
    /// No writes, no network. The reaction's posture: it speaks, it does not act.
    ReadOnly,
    /// Project writes, no direct network. External effects use host-owned tools.
    #[default]
    WorkspaceWrite,
    /// Unrestricted host access. Retained for protocol compatibility, not used by
    /// normal model sessions.
    FullAccess,
}

impl Sandbox {
    /// Also read by the permission profile Reaction opens under, which carries its own
    /// sandbox rather than the `thread/start` param.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Sandbox::ReadOnly => "read-only",
            Sandbox::WorkspaceWrite => "workspace-write",
            Sandbox::FullAccess => "danger-full-access",
        }
    }
}

/// One child-process-hosted codex connection, hosting a single thread.
pub struct CodexProcess {
    /// Outbound queue into the writer task. Cloneable, so the reader task can answer
    /// server-initiated requests without contending for stdin.
    out: mpsc::UnboundedSender<Value>,
    /// In-flight requests by JSON-RPC id, awaiting their response.
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, Value>>>>>,
    next_id: AtomicU64,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

/// Tracks every live codex subprocess's driver task, so the host can reap them all on
/// shutdown rather than leaking orphaned children.
///
/// Each [`CodexProcess::spawn`] registers its driver here under the per-connection id;
/// the driver removes its own entry when it exits (its session handle was dropped,
/// which signals shutdown), so the map only ever holds *live* processes.
/// [`shutdown`](Self::shutdown) aborts whatever remains — dropping a driver future
/// drops the [`Child`], which was spawned `kill_on_drop`.
#[derive(Clone, Default)]
pub struct ProcessRegistry {
    inner: Arc<Mutex<HashMap<u64, JoinHandle<()>>>>,
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn insert(&self, id: u64, driver: JoinHandle<()>) {
        self.inner.lock().expect("process registry mutex").insert(id, driver);
    }

    /// Drop a driver handle from the live map. Called by the driver's own guard when it
    /// exits; removing a finished task's handle is harmless.
    fn remove(&self, id: u64) {
        let _ = self.inner.lock().expect("process registry mutex").remove(&id);
    }

    /// Reap every live codex subprocess. Aborting a driver future drops its [`Child`]
    /// (killing the process); awaiting the aborted handle confirms the kill ran before
    /// we return. The caller should bound this with a timeout so a wedged child cannot
    /// hang process exit.
    pub async fn shutdown(&self) {
        let drivers: Vec<JoinHandle<()>> = {
            let mut map = self.inner.lock().expect("process registry mutex");
            map.drain().map(|(_, driver)| driver).collect()
        };
        if drivers.is_empty() {
            tracing::info!("no live codex subprocesses to reap");
            return;
        }
        let n = drivers.len();
        tracing::info!(sessions = n, "reaping codex subprocesses");
        for driver in drivers {
            driver.abort();
            let _ = driver.await;
        }
        tracing::info!(sessions = n, "codex subprocesses reaped");
    }
}

/// Removes one driver's entry from the [`ProcessRegistry`] when the driver task ends —
/// by clean shutdown, error, or abort. Lives inside the driver's future, so it fires
/// however that future ends.
struct RegistryGuard {
    registry: ProcessRegistry,
    id: u64,
}

impl Drop for RegistryGuard {
    fn drop(&mut self) {
        self.registry.remove(self.id);
    }
}

/// How long a held stderr record waits for a continuation line before it is logged.
///
/// A record's own lines arrive back-to-back, so any gap this wide means the record is
/// complete. Without it a lone error would sit unlogged until codex happened to log
/// again — or until the process exited.
const STDERR_IDLE_FLUSH: Duration = Duration::from_millis(200);

/// The level codex stamped on one of its own log records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StderrLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

/// Reassembles codex's `tracing` records from the lines its stderr pipe delivers.
///
/// Codex logs for a terminal: ANSI-styled, `<ts> <LEVEL> <target>: <message>`, and a
/// message containing newlines arrives as several lines of which only the first carries
/// a header. Logging each line as it lands turned one error into six warnings full of
/// escape bytes — 463 of the 471 stderr lines ever recorded were continuation fragments
/// of a handful of records. A line with a header closes the record before it; anything
/// else is a continuation of the record still open.
#[derive(Default)]
struct StderrJoiner {
    pending: Option<(StderrLevel, String)>,
}

impl StderrJoiner {
    /// Takes one raw line, returning the record it completed (if any).
    fn push(&mut self, line: &str) -> Option<(StderrLevel, String)> {
        let clean = strip_ansi(line);
        match parse_record_header(&clean) {
            Some((level, message)) => self.pending.replace((level, message.to_string())),
            // Header-less output — a continuation, or spew from something in the child
            // that isn't codex's logger. The latter has no level of its own, and unlabelled
            // output from a subprocess is worth seeing, so it keeps `Warn`.
            None => match self.pending.as_mut() {
                Some((_, text)) => {
                    text.push('\n');
                    text.push_str(&clean);
                    None
                }
                None => {
                    self.pending = Some((StderrLevel::Warn, clean));
                    None
                }
            },
        }
    }

    /// Releases the open record — on EOF, or once the pipe has gone quiet.
    fn flush(&mut self) -> Option<(StderrLevel, String)> {
        self.pending.take()
    }
}

/// Splits codex's `<timestamp> <LEVEL> <rest>` header off an ANSI-stripped line.
///
/// `None` means the line carries no header of its own: a continuation line, or output
/// that didn't come from codex's logger at all.
fn parse_record_header(line: &str) -> Option<(StderrLevel, &str)> {
    let (timestamp, rest) = line.trim_start().split_once(char::is_whitespace)?;
    // RFC 3339 UTC, which is what codex's logger emits. Checked so an ordinary line that
    // happens to start with two words isn't mistaken for a new record.
    if !(timestamp.contains('T') && timestamp.ends_with('Z')) {
        return None;
    }
    let (level, message) = rest.trim_start().split_once(char::is_whitespace)?;
    let level = match level {
        "ERROR" => StderrLevel::Error,
        "WARN" => StderrLevel::Warn,
        "INFO" => StderrLevel::Info,
        "DEBUG" => StderrLevel::Debug,
        "TRACE" => StderrLevel::Trace,
        _ => return None,
    };
    Some((level, message.trim_start()))
}

/// Drops ANSI escape sequences. Codex styles its log lines for a terminal; the raw bytes
/// reach the tap verbatim (a raw frame is raw), but in our log they are noise in a file
/// and a second layer of styling on a console.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' || chars.peek() != Some(&'[') {
            out.push(c);
            continue;
        }
        chars.next();
        // CSI runs to its final byte, `@`..=`~`.
        for c in chars.by_ref() {
            if ('\u{40}'..='\u{7e}').contains(&c) {
                break;
            }
        }
    }
    out
}

fn log_stderr_record(level: StderrLevel, text: &str) {
    match level {
        StderrLevel::Error => tracing::error!(target: "codex::stderr", "{text}"),
        StderrLevel::Warn => tracing::warn!(target: "codex::stderr", "{text}"),
        StderrLevel::Info => tracing::info!(target: "codex::stderr", "{text}"),
        StderrLevel::Debug => tracing::debug!(target: "codex::stderr", "{text}"),
        StderrLevel::Trace => tracing::trace!(target: "codex::stderr", "{text}"),
    }
}

/// Aborts the reader/writer/stderr tasks when the driver future is dropped, so an
/// aborted driver takes its whole connection down rather than leaving three tasks
/// pumping a dead pipe.
struct TaskGuard(Vec<JoinHandle<()>>);

impl Drop for TaskGuard {
    fn drop(&mut self) {
        for handle in &self.0 {
            handle.abort();
        }
    }
}

/// The thread id out of a `thread/start` or `thread/resume` response.
///
/// One reader for both, because both answer with the same `{ thread: { id } }` shape and a
/// resume that came back without one would be exactly as broken as an open that did.
fn thread_id_of(result: &Value, method: &str) -> anyhow::Result<String> {
    result
        .get("thread")
        .and_then(|t| t.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("{method} returned no thread id: {result}"))
}

impl CodexProcess {
    /// Spawn `codex app-server --stdio` and complete the `initialize` handshake.
    ///
    /// Returns the process plus the stream of raw notifications for its one thread.
    /// Notifications can arrive *during* the handshake (`remoteControl/status/changed`
    /// is emitted before the `initialize` response), which is why the stream exists
    /// before the handshake runs rather than after the thread is opened.
    pub async fn spawn(
        program: PathBuf,
        args: Vec<String>,
        env: Vec<(String, String)>,
        tap: WireTap,
        role: String,
        // hi-agent's minted session id, when the caller has one. The tap files this
        // connection's frames under it; `None` means they reach the inspector but are
        // not kept, because there is nothing durable to file them as.
        session_id: Option<crate::foundation::registry::SessionId>,
        registry: &ProcessRegistry,
    ) -> anyhow::Result<(Self, mpsc::UnboundedReceiver<Value>)> {
        let mut command = Command::new(&program);
        command
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (key, value) in &env {
            command.env(key, value);
        }
        let mut child: Child = command
            .spawn()
            .with_context(|| format!("spawning {}", program.display()))?;

        let stdin = child.stdin.take().ok_or_else(|| anyhow!("codex child has no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("codex child has no stdout"))?;
        let stderr = child.stderr.take().ok_or_else(|| anyhow!("codex child has no stderr"))?;

        // One id per subprocess (= per thread), so the tap can group this connection's
        // frames — including its pre-`threadId` handshake.
        let conn = CONN_SEQ.fetch_add(1, Ordering::Relaxed);

        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
        let (note_tx, note_rx) = mpsc::unbounded_channel::<Value>();
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, Value>>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Writer: the only thing that touches stdin, so requests and auto-approvals
        // never interleave mid-line.
        let writer = {
            let tap = tap.clone();
            let role = role.clone();
            let mut stdin = stdin;
            let session_id = session_id.clone();
            tokio::spawn(async move {
                while let Some(msg) = out_rx.recv().await {
                    let line = match serde_json::to_string(&msg) {
                        Ok(line) => line,
                        Err(err) => {
                            tracing::error!(error = %err, "outbound frame would not serialize");
                            continue;
                        }
                    };
                    tap.record(conn, session_id.as_ref(), &role, Dir::Send, &line);
                    tracing::trace!(target: "codex::send", "{line}");
                    if stdin.write_all(line.as_bytes()).await.is_err()
                        || stdin.write_all(b"\n").await.is_err()
                        || stdin.flush().await.is_err()
                    {
                        tracing::debug!("codex stdin closed; writer stopping");
                        break;
                    }
                }
            })
        };

        // Reader: responses resolve their waiter, server-initiated requests are
        // auto-accepted, everything else is a notification for the session's stream.
        let reader = {
            let tap = tap.clone();
            let role = role.clone();
            let pending = Arc::clone(&pending);
            let out_tx = out_tx.clone();
            let session_id = session_id.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                loop {
                    let line = match lines.next_line().await {
                        Ok(Some(line)) => line,
                        Ok(None) => {
                            tracing::info!("codex stdout closed");
                            break;
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "codex stdout read failed");
                            break;
                        }
                    };
                    if line.trim().is_empty() {
                        continue;
                    }
                    tap.record(conn, session_id.as_ref(), &role, Dir::Recv, &line);
                    tracing::trace!(target: "codex::recv", "{line}");

                    let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                        tracing::warn!("codex emitted a non-JSON line on stdout: {line}");
                        continue;
                    };
                    dispatch(msg, &pending, &out_tx, &note_tx);
                }
                // The connection is gone; fail every waiter rather than leaving a turn
                // hanging on a response that can never arrive.
                let waiters: Vec<_> = {
                    let mut map = pending.lock().expect("pending mutex");
                    map.drain().map(|(_, tx)| tx).collect()
                };
                for tx in waiters {
                    let _ = tx.send(Err(json!({ "message": "codex connection closed" })));
                }
            })
        };

        let stderr_task = {
            let tap = tap.clone();
            let role = role.clone();
            let session_id = session_id.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                let mut joiner = StderrJoiner::default();
                loop {
                    match tokio::time::timeout(STDERR_IDLE_FLUSH, lines.next_line()).await {
                        Ok(Ok(Some(line))) => {
                            if line.trim().is_empty() {
                                continue;
                            }
                            // The tap keeps the line exactly as it crossed the pipe; only
                            // the log gets the reassembled, de-styled record.
                            tap.record(conn, session_id.as_ref(), &role, Dir::Stderr, &line);
                            if let Some((level, text)) = joiner.push(&line) {
                                log_stderr_record(level, &text);
                            }
                        }
                        // Idle: whatever is still held is complete.
                        Err(_elapsed) => {
                            if let Some((level, text)) = joiner.flush() {
                                log_stderr_record(level, &text);
                            }
                        }
                        // EOF, or the pipe broke — either way there is no more of this record.
                        Ok(_) => {
                            if let Some((level, text)) = joiner.flush() {
                                log_stderr_record(level, &text);
                            }
                            break;
                        }
                    }
                }
            })
        };

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let registry_guard = RegistryGuard { registry: registry.clone(), id: conn };
        let task_guard = TaskGuard(vec![reader, writer, stderr_task]);
        let driver: JoinHandle<()> = tokio::spawn(async move {
            let _registry_guard = registry_guard;
            let _task_guard = task_guard;
            // Owning the child here is what makes reaping work: aborting this future
            // drops it, and it was spawned `kill_on_drop`.
            let _child = child;
            let _ = shutdown_rx.await;
            tracing::info!("codex driver received shutdown signal");
        });
        registry.insert(conn, driver);

        let process = Self {
            out: out_tx,
            pending,
            next_id: AtomicU64::new(1),
            shutdown_tx: Some(shutdown_tx),
        };

        let init = process
            .request(
                "initialize",
                json!({
                    "clientInfo": {
                        "name": CLIENT_NAME,
                        "title": "hi-agent",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    // Gates `thread/start.permissions`, the only spelling that actually
                    // pins a permission profile to a thread (see
                    // [`SessionOpts::permission_profile`]). Without it that parameter is
                    // refused outright with `requires experimentalApi capability`.
                    "capabilities": { "experimentalApi": true },
                }),
            )
            .await
            .context("codex initialize failed")?;
        tracing::info!(
            user_agent = init.get("userAgent").and_then(|v| v.as_str()),
            "codex connection initialised"
        );
        process.notify("initialized", Value::Null)?;

        Ok((process, note_rx))
    }

    /// Open this process's single thread and return its id. The id addresses
    /// `turn/start` and `turn/interrupt`; inbound notifications are not routed by it —
    /// they all flow to the stream returned by [`spawn`](Self::spawn).
    pub async fn open_thread(&self, opts: SessionOpts) -> anyhow::Result<String> {
        let cwd = match opts.cwd {
            Some(path) => path,
            None => std::env::current_dir().context("reading current dir for a new thread")?,
        };

        let mut params = json!({
            "cwd": cwd.to_string_lossy(),
            // Approvals are a policy choice made here, not a prompt shown to the person.
            "approvalPolicy": "never",
            // Durable, so the thread outlives the process that opened it. This was
            // `true` — inherited from the ACP path, on the reasoning that a rung opens a
            // fresh thread per boot and an ephemeral one leaves no rollout to
            // garbage-collect. That was a housekeeping argument answering a continuity
            // question: a rung that reopens cannot remember what it was in the middle of,
            // which `agents.md` already calls the failure worth preventing, and every
            // restart reproduced it. Retention is the real cost and it is a real one — a
            // single trivial turn writes ~45KB — but it is bounded work, and losing the
            // thread was not.
            "ephemeral": false,
        });
        // A profile carries its own sandbox, and codex rejects the pair outright, so the
        // two spellings are exclusive rather than merged.
        match &opts.permission_profile {
            Some(profile) => params["permissions"] = json!(profile),
            None => params["sandbox"] = json!(opts.sandbox.as_str()),
        }
        if let Some(prompt) = opts.system_prompt {
            params["baseInstructions"] = json!(prompt);
        }
        if !opts.config.is_empty() {
            params["config"] = Value::Object(opts.config);
        }

        let result = self.request("thread/start", params).await?;
        let id = thread_id_of(&result, "thread/start")?;
        tracing::info!(thread_id = %id, "codex thread opened");
        Ok(id)
    }

    /// Pick `thread_id` back up in this process, carrying `opts` exactly as
    /// [`open_thread`](Self::open_thread) would.
    ///
    /// **`baseInstructions` rides the resume, and that is the load-bearing half.** Prompts
    /// are reinstalled from the bundle every boot and a binary upgrade is the most common
    /// reason to restart, so a thread resumed without them would run on the prompt of
    /// whichever release first opened it — the oldest threads carrying the stalest
    /// instructions. Verified against the 0.147 pin rather than the docs: a thread started
    /// under one prompt and resumed under another answers as the second.
    ///
    /// Errors are the caller's to absorb, and one is entirely ordinary: a thread that never
    /// took a turn has no rollout, and codex answers `no rollout found for thread id`. That
    /// is a boot where nothing had happened yet, not a fault.
    pub async fn resume_thread(&self, thread_id: &str, opts: SessionOpts) -> anyhow::Result<String> {
        let mut params = json!({
            "threadId": thread_id,
            "approvalPolicy": "never",
        });
        if let Some(cwd) = opts.cwd {
            params["cwd"] = json!(cwd.to_string_lossy());
        }
        match &opts.permission_profile {
            Some(profile) => params["permissions"] = json!(profile),
            None => params["sandbox"] = json!(opts.sandbox.as_str()),
        }
        if let Some(prompt) = opts.system_prompt {
            params["baseInstructions"] = json!(prompt);
        }
        if !opts.config.is_empty() {
            params["config"] = Value::Object(opts.config);
        }

        let result = self.request("thread/resume", params).await?;
        // Resume appends to the thread's original rollout, so the id that comes back is the
        // id that went in — read it from the response anyway rather than echoing the
        // argument, so the session records what codex says it is on.
        let id = thread_id_of(&result, "thread/resume")?;
        tracing::info!(thread_id = %id, "codex thread resumed");
        Ok(id)
    }

    /// Issue a JSON-RPC request and await its response.
    pub(crate) async fn request(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending mutex").insert(id, tx);

        let mut msg = json!({ "id": id, "method": method });
        if !params.is_null() {
            msg["params"] = params;
        }
        if self.out.send(msg).is_err() {
            self.pending.lock().expect("pending mutex").remove(&id);
            bail!("codex connection is closed ({method})");
        }

        match rx.await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(err)) => Err(anyhow!("{method} failed: {}", error_text(&err))),
            Err(_) => Err(anyhow!("{method} never answered; codex connection ended")),
        }
    }

    /// Send a JSON-RPC notification (no response expected).
    pub(crate) fn notify(&self, method: &str, params: Value) -> anyhow::Result<()> {
        let mut msg = json!({ "method": method });
        if !params.is_null() {
            msg["params"] = params;
        }
        self.out
            .send(msg)
            .map_err(|_| anyhow!("codex connection is closed ({method})"))
    }

    fn signal_shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for CodexProcess {
    fn drop(&mut self) {
        self.signal_shutdown();
    }
}

/// Route one inbound message: response, server-initiated request, or notification.
fn dispatch(
    msg: Value,
    pending: &Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, Value>>>>>,
    out: &mpsc::UnboundedSender<Value>,
    notifications: &mpsc::UnboundedSender<Value>,
) {
    let has_method = msg.get("method").is_some();
    let id = msg.get("id").and_then(Value::as_u64);

    // A response carries an id and no method.
    if let Some(id) = id
        && !has_method
    {
        let waiter = pending.lock().expect("pending mutex").remove(&id);
        let Some(waiter) = waiter else {
            tracing::warn!(id, "codex answered a request nobody is waiting for");
            return;
        };
        let outcome = match msg.get("error") {
            Some(err) => Err(err.clone()),
            None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
        };
        let _ = waiter.send(outcome);
        return;
    }

    // An id *and* a method is codex asking us something.
    if let Some(id) = msg.get("id")
        && has_method
    {
        answer_server_request(
            msg.get("method").and_then(Value::as_str).unwrap_or(""),
            id.clone(),
            msg.get("params"),
            out,
        );
        return;
    }

    if notifications.send(msg).is_err() {
        tracing::debug!("session receiver dropped while a notification arrived");
    }
}

/// Answer a server-initiated request, **in that request's own response shape**.
///
/// Codex asks the client several different things, and they do not share a reply type:
/// an approval wants `{decision}`, an MCP elicitation wants `{action, content}`, a
/// permissions escalation wants a whole granted profile. Answering them all with one
/// shape is not a shortcut, it is a bug — a blanket `{"decision":"accept"}` drew
/// `failed to deserialize McpServerElicitationRequestResponse: missing field 'action'`
/// from a live run, which left the tool call hanging with nothing in our own log to
/// explain it.
///
/// So: answer what we have a policy for, and **return a JSON-RPC error for everything
/// else**. An error is honest and surfaces at the turn; a wrong-shaped result is a
/// silent corruption of codex's state. Anything on the unhandled list that turns out to
/// matter should get a real decision here, not a plausible-looking reply.
fn answer_server_request(
    method: &str,
    id: Value,
    params: Option<&Value>,
    out: &mpsc::UnboundedSender<Value>,
) {
    let result = match method {
        // Approvals. The rungs run `approvalPolicy: "never"`, so these should not arrive
        // at all; we accept rather than hang, because the sensitive-action policy lives
        // in the sandbox mode and the prompts, not in a modal nobody is there to answer.
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            Some(json!({ "decision": "accept" }))
        }
        // The pre-v2 spellings of the same thing, in case an older codex is on PATH.
        "execCommandApproval" | "applyPatchApproval" => Some(json!({ "decision": "approved" })),
        // Elicitation is two different things wearing one method name, and the
        // difference decides whether the agent can act at all.
        //
        // With `_meta.codex_approval_kind`, it is codex gating a tool call — including
        // *our own* MCP tools, which it gates even under `approvalPolicy: "never"`.
        // Declining those is declining the agent's own hands: live, it turned a `say`
        // into "user rejected MCP tool call" and Reaction went silent while the turn
        // reported success. Accept.
        //
        // Without it, an MCP server is asking the *person* for structured input. There
        // is no modal to show and inventing an answer is worse than saying no — the
        // agent can ask through its own `send_message` instead. Decline.
        "mcpServer/elicitation/request" => {
            let is_approval = params
                .and_then(|p| p.get("_meta"))
                .and_then(|m| m.get("codex_approval_kind"))
                .is_some();
            Some(if is_approval {
                json!({ "action": "accept", "content": {} })
            } else {
                json!({ "action": "decline", "content": null })
            })
        }
        "item/tool/requestUserInput" => Some(json!({ "action": "decline", "content": null })),
        _ => None,
    };

    match result {
        Some(result) => {
            tracing::info!(method, "answering a codex server request");
            let _ = out.send(json!({ "id": id, "result": result }));
        }
        None => {
            tracing::warn!(method, "codex asked something we have no policy for; refusing");
            let _ = out.send(json!({
                "id": id,
                "error": { "code": -32601, "message": format!("hi-agent does not handle {method}") }
            }));
        }
    }
}

/// The human-readable half of a JSON-RPC error object, for an `anyhow` chain.
///
/// Kept as text on purpose: `energy_state::is_402_text` classifies the managed-energy
/// edge by matching the upstream status in the message, and codex forwards the gateway's
/// status into it (`codexErrorInfo.httpStatusCode`). Structured classification is a
/// strict improvement to make later; dropping the text would break the gate today.
fn error_text(err: &Value) -> String {
    match err.get("message").and_then(Value::as_str) {
        Some(message) => match err.get("data") {
            Some(data) if !data.is_null() => format!("{message} ({data})"),
            _ => message.to_string(),
        },
        None => err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The record that produced this test, verbatim off the wire on 2026-08-10.
    const STYLED_ERROR: &str = "\u{1b}[2m2026-08-10T10:26:00.471756Z\u{1b}[0m \u{1b}[31mERROR\u{1b}[0m \u{1b}[2mcodex_core::tools::router\u{1b}[0m\u{1b}[2m:\u{1b}[0m \u{1b}[3merror\u{1b}[0m\u{1b}[2m=\u{1b}[0mexec_command failed for `/bin/zsh -lc 'if [ ! -d /Users/iloahz/projects/KTV/.git ]; then";

    #[test]
    fn ansi_styling_is_dropped_and_text_kept() {
        let clean = strip_ansi(STYLED_ERROR);
        assert!(!clean.contains('\u{1b}'), "no escape bytes survive: {clean}");
        assert!(clean.starts_with("2026-08-10T10:26:00.471756Z ERROR codex_core::tools::router: error="));
        assert_eq!(strip_ansi("plain"), "plain");
    }

    #[test]
    fn a_header_yields_its_level_and_message() {
        let clean = strip_ansi(STYLED_ERROR);
        let (level, message) = parse_record_header(&clean).expect("header");
        assert_eq!(level, StderrLevel::Error);
        assert!(message.starts_with("codex_core::tools::router: error=exec_command failed"));
    }

    #[test]
    fn the_padded_unstyled_form_parses_too() {
        // An unstyled build pads the level to five columns.
        let (level, message) =
            parse_record_header("2026-08-10T10:26:00.471756Z  WARN codex_core: slow").expect("header");
        assert_eq!(level, StderrLevel::Warn);
        assert_eq!(message, "codex_core: slow");
    }

    #[test]
    fn a_line_without_a_header_is_not_read_as_one() {
        assert!(parse_record_header("git status --short").is_none());
        assert!(parse_record_header("Cloning into '/Users/iloahz/projects/KTV'...").is_none());
        // Two words, but the first is no timestamp.
        assert!(parse_record_header("ERROR something happened").is_none());
    }

    #[test]
    fn continuation_lines_join_the_record_above_them() {
        let mut joiner = StderrJoiner::default();
        assert!(joiner.push(STYLED_ERROR).is_none(), "a record is held open, not logged per line");
        assert!(joiner.push("git status --short").is_none());
        assert!(joiner.push("git log -1 --oneline").is_none());

        let (level, text) = joiner.flush().expect("the open record flushes");
        assert_eq!(level, StderrLevel::Error);
        assert_eq!(text.lines().count(), 3, "one record, not three: {text}");
        assert!(text.ends_with("git log -1 --oneline"));
        assert!(joiner.flush().is_none(), "nothing is held twice");
    }

    #[test]
    fn a_new_header_closes_the_record_before_it() {
        let mut joiner = StderrJoiner::default();
        joiner.push("2026-08-10T10:26:00.471756Z INFO codex_core: first");
        let (level, text) = joiner
            .push("2026-08-10T10:26:01.000000Z ERROR codex_core: second")
            .expect("the first record completes");
        assert_eq!(level, StderrLevel::Info);
        assert_eq!(text, "codex_core: first");
        assert_eq!(joiner.flush().expect("the second is still open").0, StderrLevel::Error);
    }

    #[test]
    fn header_less_spew_stays_a_warning() {
        let mut joiner = StderrJoiner::default();
        joiner.push("dyld: symbol not found");
        assert_eq!(joiner.flush().expect("held"), (StderrLevel::Warn, "dyld: symbol not found".into()));
    }

    fn channels() -> (
        Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, Value>>>>>,
        mpsc::UnboundedSender<Value>,
        mpsc::UnboundedReceiver<Value>,
        mpsc::UnboundedSender<Value>,
        mpsc::UnboundedReceiver<Value>,
    ) {
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        let (note_tx, note_rx) = mpsc::unbounded_channel();
        (Arc::new(Mutex::new(HashMap::new())), out_tx, out_rx, note_tx, note_rx)
    }

    #[tokio::test]
    async fn a_response_resolves_its_waiter() {
        let (pending, out_tx, _out_rx, note_tx, _note_rx) = channels();
        let (tx, rx) = oneshot::channel();
        pending.lock().unwrap().insert(7, tx);

        dispatch(json!({ "id": 7, "result": { "thread": { "id": "t1" } } }), &pending, &out_tx, &note_tx);

        let got = rx.await.unwrap().unwrap();
        assert_eq!(got["thread"]["id"], "t1");
        assert!(pending.lock().unwrap().is_empty(), "the waiter is consumed, not leaked");
    }

    #[tokio::test]
    async fn an_error_response_reaches_the_waiter_as_an_error() {
        let (pending, out_tx, _out_rx, note_tx, _note_rx) = channels();
        let (tx, rx) = oneshot::channel();
        pending.lock().unwrap().insert(1, tx);

        dispatch(
            json!({ "id": 1, "error": { "code": -32600, "message": "Invalid request" } }),
            &pending,
            &out_tx,
            &note_tx,
        );

        let err = rx.await.unwrap().unwrap_err();
        assert_eq!(error_text(&err), "Invalid request");
    }

    #[tokio::test]
    async fn an_approval_request_is_accepted_on_the_writer() {
        let (pending, out_tx, mut out_rx, note_tx, _note_rx) = channels();

        dispatch(
            json!({ "id": 3, "method": "item/commandExecution/requestApproval", "params": {} }),
            &pending,
            &out_tx,
            &note_tx,
        );

        let reply = out_rx.recv().await.unwrap();
        assert_eq!(reply["id"], 3);
        assert_eq!(reply["result"]["decision"], "accept");
    }

    /// An elicitation is not an approval and does not share its response type. Answering
    /// it with `{decision}` is what codex rejected live, so the shapes are pinned apart.
    #[tokio::test]
    async fn an_elicitation_is_declined_in_its_own_shape() {
        let (pending, out_tx, mut out_rx, note_tx, _note_rx) = channels();

        dispatch(
            json!({ "id": 4, "method": "mcpServer/elicitation/request",
                    "params": { "serverName": "hi-agent", "mode": "form" } }),
            &pending,
            &out_tx,
            &note_tx,
        );

        let reply = out_rx.recv().await.unwrap();
        assert_eq!(reply["result"]["action"], "decline");
        assert!(reply["result"].get("decision").is_none());
    }

    /// ...but the *same method* carrying `codex_approval_kind` is codex gating a tool
    /// call, not a question for the person. Declining that one rejects the agent's own
    /// hands — live, it turned a `say` into "user rejected MCP tool call".
    #[tokio::test]
    async fn an_elicitation_that_is_really_a_tool_gate_is_accepted() {
        let (pending, out_tx, mut out_rx, note_tx, _note_rx) = channels();

        dispatch(
            json!({ "id": 6, "method": "mcpServer/elicitation/request", "params": {
                "serverName": "hi-agent",
                "mode": "form",
                "_meta": { "codex_approval_kind": "mcp_tool_call" }
            }}),
            &pending,
            &out_tx,
            &note_tx,
        );

        let reply = out_rx.recv().await.unwrap();
        assert_eq!(reply["result"]["action"], "accept");
    }

    /// A request we have no policy for gets an error, not a plausible-looking result: a
    /// wrong shape corrupts codex's state silently, an error surfaces at the turn.
    #[tokio::test]
    async fn an_unknown_server_request_is_refused_rather_than_guessed() {
        let (pending, out_tx, mut out_rx, note_tx, _note_rx) = channels();

        dispatch(
            json!({ "id": 5, "method": "attestation/generate", "params": {} }),
            &pending,
            &out_tx,
            &note_tx,
        );

        let reply = out_rx.recv().await.unwrap();
        assert!(reply.get("result").is_none());
        assert_eq!(reply["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn a_notification_goes_to_the_session_stream() {
        let (pending, out_tx, _out_rx, note_tx, mut note_rx) = channels();

        // Codex emits this one *before* the initialize response, so the stream has to
        // exist and accept traffic during the handshake.
        dispatch(
            json!({ "method": "remoteControl/status/changed", "params": { "status": "disabled" } }),
            &pending,
            &out_tx,
            &note_tx,
        );

        let note = note_rx.recv().await.unwrap();
        assert_eq!(note["method"], "remoteControl/status/changed");
    }

    #[test]
    fn sandbox_modes_use_codex_spelling() {
        // The app-server rejects camelCase outright ("unknown variant `dangerFullAccess`"),
        // so this is pinned rather than left to a formatter's taste.
        assert_eq!(Sandbox::FullAccess.as_str(), "danger-full-access");
        assert_eq!(Sandbox::WorkspaceWrite.as_str(), "workspace-write");
        assert_eq!(Sandbox::ReadOnly.as_str(), "read-only");
    }
}
