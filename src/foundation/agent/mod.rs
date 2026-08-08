//! Agent session layer — one `codex app-server` subprocess per session.
//!
//! Exposes each codex thread as an independent [`AgentSession`] handle. Callers
//! (the reaction) never see subprocesses or the JSON-RPC connection — those stay
//! internal to [`CodexProcess`], which the returned handle owns.
//!
//! **Granularity: one subprocess per session.** Each [`session`](AgentLayer::session)
//! call spawns its own subprocess (Chrome-style isolation taken to the session
//! level), opens that process's single thread, and hands back a handle that owns the
//! process — dropping the handle tears the process down. One session's crash or OOM
//! cannot touch another, and there is no thread-id demux. The cost is a fresh
//! subprocess spawn + `initialize` + MCP `tools/list` round-trip per session.
//!
//! Keeping a process per session also keeps the credential fresh: codex reads the
//! upstream key from its own environment, so a key minted after boot reaches the next
//! session simply by being spawned with it. One shared app-server would have frozen
//! whatever key was current when the host started.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;

use crate::foundation::codex::process::Sandbox;
use crate::foundation::codex::{AgentSession, CodexProcess, ProcessRegistry, SessionOpts, WireTap};
use crate::foundation::config::{AgentConfig, HEADER_ROLE, HEADER_SESSION_ID};

/// Which tool surface a session gets, carried as `X-HI-Role` on its MCP attach so
/// the `/mcp` server exposes the right tools (see [`crate::foundation::mcp`]). The
/// reaction is the single fast conversational voice that owns interaction: it speaks
/// via plain message text and gets a minimal `show`-only surface to put
/// cognition's artifacts on screen — the heavy work is delegated to workers. A
/// worker can only raise a question; a reflection session ("sleep") only
/// reads/writes derived memory (episodes, facets) and has no voice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionRole {
    /// The always-present **reaction** — the fast conversational voice. A turn is a
    /// single quick generation on the smart model; it speaks via its plain message
    /// text (`item/agentMessage/delta`) and may call only `show` to display a view
    /// a worker already built. Real work is delegated to [`Worker`](Self::Worker)
    /// sessions (cognition).
    #[default]
    Reaction,
    Worker,
    /// The conversation's reading and thinking rung.
    Deliberation,
    /// The shared brain.
    Cognition,
    Reflection,
}

impl SessionRole {
    fn as_str(self) -> &'static str {
        match self {
            SessionRole::Reaction => "reaction",
            SessionRole::Worker => "worker",
            SessionRole::Reflection => "reflection",
            SessionRole::Deliberation => "deliberation",
            SessionRole::Cognition => "cognition",
        }
    }

    /// How much of the machine this rung's own tools may touch.
    ///
    /// **The reaction is read-only, and that is as close as codex gets to the tools-off
    /// voice ACP gave us.** There, the reaction opened with `builtin_tools: []` and
    /// genuinely held nothing. Codex has no equivalent switch — `exec_command`,
    /// `apply_patch` and the rest are always in the schema — so the voice is held to its
    /// job by two softer things instead: a read-only sandbox, so a stray call cannot
    /// change anything, and `speaking.md` as its actual system prompt, which is a far
    /// stronger lever here than it ever was under ACP. If the voice does reach for a
    /// shell in practice, the answer is to take it off the agent process entirely (one
    /// direct Responses call, no `tools` array), not to bolt on a hard rail.
    ///
    /// Every other rung gets full access, which is what the ACP path did in effect —
    /// Claude ran unsandboxed with every permission request auto-allowed — so the wire
    /// swap changes the wire and not what a worker may do.
    fn sandbox(self) -> Sandbox {
        match self {
            SessionRole::Reaction => Sandbox::ReadOnly,
            _ => Sandbox::FullAccess,
        }
    }
}

/// How to spawn one codex subprocess. Cloned per session: the pinned runtime, args,
/// and **static** env (codex home, server URL, PATH — resolved once at startup).
/// The volatile upstream credential is NOT frozen here — it is re-resolved from the
/// credential store at each [`session`](AgentLayer::session) spawn and merged onto
/// this env, so a fresh child never carries a stale key.
#[derive(Debug, Clone)]
pub struct SpawnConfig {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// The per-session subprocess spawner. Cloneable handle; clones share one config.
#[derive(Clone)]
pub struct AgentLayer {
    inner: Arc<Inner>,
}

struct Inner {
    spawn: SpawnConfig,
    /// Data dir, so each spawn can re-resolve the upstream credential from the
    /// store ([`AgentConfig::resolve`]) rather than freeze a boot-time key. Cheap
    /// SQLite read, dwarfed by the subprocess spawn + `initialize` it precedes.
    data_dir: PathBuf,
    /// hi-agent's own HTTP base URL (e.g. `http://127.0.0.1:12358`), used to build
    /// each session's MCP attach URL (`<base>/mcp`). The same value the child gets
    /// as `HI_AGENT_BASE_URL`.
    server_base_url: String,
    /// Raw JSON-RPC wire tap — every session's subprocess records its frames here
    /// for the raw session inspector. Handed to each [`CodexProcess`] at spawn.
    tap: WireTap,
    /// Every spawned subprocess registers its driver here, so the host can reap
    /// them all on shutdown instead of leaking orphaned children. See
    /// [`AgentLayer::shutdown`].
    registry: ProcessRegistry,
}

impl AgentLayer {
    pub fn new(
        spawn: SpawnConfig,
        data_dir: PathBuf,
        tap: WireTap,
        server_base_url: String,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                spawn,
                data_dir,
                server_base_url,
                tap,
                registry: ProcessRegistry::new(),
            }),
        }
    }

    /// Spawn a dedicated subprocess and open its single thread.
    ///
    /// `role` selects the tool surface the session gets; `session_id` (workers and
    /// anything else the host has minted an id for) names which session a tool call
    /// comes from. The thread attaches hi-agent's `/mcp` endpoint tagged with both via
    /// HTTP headers, so the server can route its tool calls. The returned handle owns
    /// the subprocess — the caller drives turns on it, and dropping it tears the process
    /// down.
    pub async fn session(
        &self,
        role: SessionRole,
        session_id: Option<u64>,
        opts: SessionOpts,
    ) -> anyhow::Result<AgentSession> {
        let SessionOpts { system_prompt, cwd, .. } = opts;

        // Never let a session root at the process cwd. An unset cwd falls through to
        // `std::env::current_dir()`, which for a Finder-launched `.app` is `/` and in dev
        // is often `~`. The agent reads its project tree on startup, so rooting it there
        // walks into `~/Pictures`, `~/Music`, `~/Documents`, … and fires a burst of TCC
        // "wants to access your Photos/Music/…" prompts at first launch. Default instead
        // to the data dir (under Application Support — not a TCC-gated location), the
        // agent's own world. Workers still override with `views_dir`.
        let cwd = cwd.or_else(|| Some(self.inner.data_dir.clone()));

        // Merge the current upstream credential onto the static env at spawn time, so this
        // child always carries the freshest key from the store (broker re-mint, Settings
        // edit, mode switch) — never a stale boot-time snapshot.
        let cfg = AgentConfig::resolve(&self.inner.data_dir);
        let spawn = &self.inner.spawn;
        let mut env = spawn.env.clone();
        env.extend(cfg.auth_child_env());

        tracing::info!(role = role.as_str(), cwd = ?cwd, "spawning codex subprocess for session");
        let (process, rx) = CodexProcess::spawn(
            spawn.program.clone(),
            spawn.args.clone(),
            env,
            self.inner.tap.clone(),
            role.as_str().to_string(),
            // hi-agent's own session id, not the protocol's: it exists before the
            // subprocess does, which is exactly what a durable per-session record needs.
            session_id,
            &self.inner.registry,
        )
        .await?;

        let id = process
            .open_thread(SessionOpts {
                system_prompt,
                cwd,
                sandbox: role.sandbox(),
                config: self.thread_config(&cfg, role, session_id),
            })
            .await?;

        Ok(AgentSession::new(id, process, rx, self.inner.data_dir.clone()))
    }

    /// The codex config a thread opens with: the model/provider from the credential
    /// store, plus hi-agent's own tool surface attached over MCP.
    ///
    /// Every role attaches the same endpoint and is told apart **server-side by the
    /// `X-HI-Role` header** (see [`crate::foundation::mcp::tools_for_role`]) — one place
    /// decides which rung holds which tool. Codex would also accept an `enabled_tools`
    /// allow-list here, and it is deliberately not used: two filters for one decision is
    /// how they drift apart, and the header already covers the case the allow-list
    /// cannot (the same tool behaving differently per rung).
    fn thread_config(
        &self,
        cfg: &AgentConfig,
        role: SessionRole,
        session_id: Option<u64>,
    ) -> serde_json::Map<String, serde_json::Value> {
        let mut headers = serde_json::Map::new();
        headers.insert(HEADER_ROLE.to_string(), json!(role.as_str()));
        if let Some(id) = session_id {
            headers.insert(HEADER_SESSION_ID.to_string(), json!(id.to_string()));
        }

        let mut config = cfg.thread_config();
        config.insert(
            "mcp_servers".into(),
            json!({
                "hi-agent": {
                    "url": format!("{}/mcp", self.inner.server_base_url),
                    "http_headers": headers,
                    // These are hi-agent's *own* tools, served by this very process —
                    // there is nobody to ask about them. Without this codex gates every
                    // call behind an approval, which showed up live as `say` failing
                    // with "user rejected MCP tool call" and the voice going silent.
                    "default_tools_approval_mode": "auto",
                }
            }),
        );
        config
    }

    /// Reap every live codex subprocess this layer has spawned (reaction, worker and
    /// reflection sessions all flow through [`session`](Self::session)). Used on host
    /// shutdown so no `codex` children are orphaned. Bound the call with a timeout — a
    /// wedged child should not hang process exit.
    pub async fn shutdown(&self) {
        self.inner.registry.shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer() -> AgentLayer {
        AgentLayer::new(
            SpawnConfig {
                program: PathBuf::from("/bin/false"),
                args: Vec::new(),
                env: Vec::new(),
            },
            PathBuf::from("/tmp/hi-agent-test"),
            WireTap::new(),
            "http://127.0.0.1:12358".to_string(),
        )
    }

    fn config() -> AgentConfig {
        AgentConfig::new(
            Some("gpt-5.1-codex".to_string()),
            None,
            None,
            "https://gateway.example/v1".to_string(),
            "sk-secret".to_string(),
        )
    }

    #[test]
    fn a_worker_thread_attaches_mcp_with_its_routing_headers() {
        let config = layer().thread_config(&config(), SessionRole::Worker, Some(42));
        let server = &config["mcp_servers"]["hi-agent"];
        assert_eq!(server["url"], "http://127.0.0.1:12358/mcp");
        assert_eq!(server["http_headers"][HEADER_ROLE], "worker");
        // Stringified: HTTP header values are text, and codex forwards them verbatim.
        assert_eq!(server["http_headers"][HEADER_SESSION_ID], "42");
    }

    #[test]
    fn a_session_without_an_id_still_names_its_role() {
        let config = layer().thread_config(&config(), SessionRole::Reaction, None);
        let headers = &config["mcp_servers"]["hi-agent"]["http_headers"];
        assert_eq!(headers[HEADER_ROLE], "reaction");
        assert!(
            headers.get(HEADER_SESSION_ID).is_none(),
            "an absent id must not become the string \"None\""
        );
    }

    #[test]
    fn the_model_wire_rides_the_same_thread_config() {
        // One object carries both halves, so a thread cannot come up attached to our
        // tools but pointed at the wrong endpoint.
        let config = layer().thread_config(&config(), SessionRole::Cognition, Some(1));
        assert_eq!(config["model"], "gpt-5.1-codex");
        assert!(config.contains_key("model_providers"));
        assert!(config.contains_key("mcp_servers"));
    }

    #[test]
    fn only_the_voice_is_sandboxed() {
        assert_eq!(SessionRole::Reaction.sandbox(), Sandbox::ReadOnly);
        for role in [
            SessionRole::Worker,
            SessionRole::Deliberation,
            SessionRole::Cognition,
            SessionRole::Reflection,
        ] {
            assert_eq!(role.sandbox(), Sandbox::FullAccess, "{role:?} does real work");
        }
    }
}
