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
//! Keeping a process per session also isolates local rollout state. The child receives
//! only a per-boot credential for the loopback model proxy; the trusted host resolves
//! the current upstream credential when each request crosses that proxy.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;

use crate::foundation::codex::process::Sandbox;
use crate::foundation::codex::{AgentSession, CodexProcess, ProcessRegistry, SessionOpts, WireTap};
use crate::foundation::config::{AgentConfig, HEADER_ROLE, HEADER_SESSION_ID};
use crate::identity::Role;

const REACTION_PERMISSION_PROFILE: &str = "hi-agent-reaction";

/// The sandbox half of what a [`Role`] decides, kept here beside the codex process it
/// configures rather than in [`crate::identity`], which has no business knowing what a
/// sandbox is.
///
/// **The role itself now comes from `identity`, with the prompt it selects.** This module
/// used to define a private `SessionRole` carrying the same five variants, so the type
/// that picked a tool surface and the type that picked a prompt were separate things that
/// could not disagree out loud — and neither could say which *kind* of worker it was.
/// `Role::as_str` is the `X-HI-Role` value the `/mcp` server reads (see
/// [`crate::foundation::mcp::tools_for_role`]); all five worker types answer `worker`
/// there, because a type picks a prompt and never a surface.
impl Role {
    /// How much of the machine this role's own tools may touch.
    ///
    /// **The reaction is read-only, and it is also tools-off** — the second half is
    /// [`AgentLayer::thread_config`], not this, and not the permission profile either.
    /// Codex has no single switch for its own toolset: `builtin_tools: []` was ACP's,
    /// the profile's `default_tools_enabled` reads like the replacement and removes
    /// nothing, and each tool is off at its own key. Two comments here have now claimed
    /// otherwise while Reaction went on holding a shell — and Reaction holding a shell
    /// answers in prose instead of calling `say`, which reaches nobody.
    ///
    /// For Reaction this value rides *inside* the profile rather than as the
    /// `thread/start` sandbox param, which codex refuses alongside one. It still earns
    /// its place next to the list: it bounds whatever arrives by a route the list did
    /// not anticipate, and it is a mechanism rather than an enumeration.
    ///
    /// Every other rung gets full access, including ordinary files in drive and the
    /// network needed to run CLIs. Sensitive values are projected at the model egress
    /// boundary, not hidden from Hi Agent's local execution environment.
    fn sandbox(self) -> Sandbox {
        match self {
            Role::Reaction => Sandbox::ReadOnly,
            _ => Sandbox::FullAccess,
        }
    }
}

/// How to spawn one codex subprocess. Cloned per session: the pinned runtime, args,
/// and **static** env (codex home, server URL, PATH — resolved once at startup).
/// The upstream credential never enters this environment.
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
    /// Data dir used as the default session root and for resolving non-secret model
    /// configuration such as the selected model.
    data_dir: PathBuf,
    /// hi-agent's own HTTP base URL (e.g. `http://127.0.0.1:12358`), used to build
    /// each session's MCP attach URL (`<base>/mcp`). The same value the child gets
    /// as `HI_AGENT_BASE_URL`.
    server_base_url: String,
    /// Per-boot model proxy token plus the private-data projector/broker. Only the
    /// token enters the child; the upstream LLM credential stays in the host.
    privacy: crate::foundation::privacy::PrivacyBoundary,
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
        privacy: crate::foundation::privacy::PrivacyBoundary,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                spawn,
                data_dir,
                server_base_url,
                privacy,
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
        role: Role,
        session_id: Option<crate::foundation::registry::SessionId>,
        opts: SessionOpts,
    ) -> anyhow::Result<AgentSession> {
        let SessionOpts { system_prompt, cwd, resume, .. } = opts;

        // Never let a session root at the process cwd. An unset cwd falls through to
        // `std::env::current_dir()`, which for a Finder-launched `.app` is `/` and in dev
        // is often `~`. The agent reads its project tree on startup, so rooting it there
        // walks into `~/Pictures`, `~/Music`, `~/Documents`, … and fires a burst of TCC
        // "wants to access your Photos/Music/…" prompts at first launch. Default instead
        // to the data dir (under Application Support — not a TCC-gated location), the
        // agent's own world. Workers still override with `views_dir`.
        let cwd = cwd.or_else(|| Some(self.inner.data_dir.clone()));

        // The child authenticates only to the loopback privacy proxy. The upstream
        // credential is resolved by the host proxy on each request and never enters
        // this process environment.
        let cfg = AgentConfig::resolve(&self.inner.data_dir);
        let spawn = &self.inner.spawn;
        let mut env = spawn.env.clone();
        env.extend(self.inner.privacy.child_env());

        tracing::info!(role = role.as_str(), cwd = ?cwd, "spawning codex subprocess for session");
        let (process, rx) = CodexProcess::spawn(
            spawn.program.clone(),
            spawn.args.clone(),
            env,
            self.inner.tap.clone(),
            role.as_str().to_string(),
            // hi-agent's own session id, not the protocol's: it exists before the
            // subprocess does, which is exactly what a durable per-session record needs.
            session_id.clone(),
            &self.inner.registry,
        )
        .await?;

        let opts = SessionOpts {
            system_prompt,
            cwd,
            sandbox: role.sandbox(),
            permission_profile: permission_profile(role),
            config: self.thread_config(&cfg, role, session_id.clone()),
            // Consumed above; it is this function's parameter, not the wire's.
            resume: None,
        };

        // **The resume policy is these two lookups, and they answer different questions.**
        //
        // `take_resumable` is the *host's* rule: which rungs come back after a restart,
        // decided by what `attach_index` seeded — the resident ones, per `agents.md` — so no
        // rung can be given a different rule by accident. A worker is never in that map.
        //
        // `resume` is the *caller's*, and a worker is the only session that ever carries one:
        // Cognition taking up an errand the last restart killed, naming the thread the boot
        // glance offered it. That is the "picking a dead errand back up is Cognition's call"
        // half of `agents.md`, which until now had the thread recorded and no way to ask for
        // it. It is checked first because an explicit request outranks a policy, and the two
        // cannot collide in practice: nothing puts a worker in the map.
        //
        // Taking rather than reading is what makes a bad thread survivable: the slot is
        // empty for every later open in this run, so the session that replaces a wedged one
        // is always cold. An offered worker thread needs no such guard — the offer is a
        // snapshot of the previous run and this run never adds to it.
        let id = match resume.or_else(|| crate::foundation::registry::global().take_resumable(role))
        {
            Some(thread) => self.resume_or_open(&process, &thread, role, opts).await?,
            None => process.open_thread(opts).await?,
        };
        if let Some(session_id) = session_id.as_ref() {
            crate::foundation::registry::global().note_thread(session_id, &id);
        }

        Ok(AgentSession::new(id, process, rx, self.inner.data_dir.clone()))
    }

    /// Resume `thread`, falling back to a fresh one if it will not come back.
    ///
    /// **A failed resume is a cold open, never a failed session.** The reasons it can fail
    /// are all ordinary: the thread never took a turn so has no rollout (a boot where
    /// nothing had happened yet), the rollout was pruned, `CODEX_HOME` moved, or the pin
    /// changed under it. None of those is a reason for the rung not to exist — losing the
    /// thread costs what it was in the middle of, and the turn's own projection carries
    /// what it owes regardless.
    ///
    /// `opts` is cloned because the fallback needs it whole; it is a prompt and a small map,
    /// paid once per session open.
    async fn resume_or_open(
        &self,
        process: &CodexProcess,
        thread: &str,
        role: Role,
        opts: SessionOpts,
    ) -> anyhow::Result<String> {
        match process.resume_thread(thread, opts.clone()).await {
            Ok(id) => {
                tracing::info!(role = role.as_str(), thread_id = %id, "resumed the previous run's thread");
                Ok(id)
            }
            Err(err) => {
                tracing::info!(
                    role = role.as_str(),
                    thread_id = %thread,
                    error = %format!("{err:#}"),
                    "could not resume the previous thread; opening a fresh one"
                );
                process.open_thread(opts).await
            }
        }
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
    ///
    /// That header decides *our* surface. It says nothing about the agent's **own**
    /// built-ins, and Reaction is the one rung where those have to be gone too —
    /// [`agents.md`](../../../docs/arch/agents.md#reaction--one-generation): "no reads,
    /// no fetches, no working directory, and no built-ins at all… Restricting our own
    /// tool surface is not sufficient on its own; the session's underlying toolset has
    /// to be restricted too, or 'cannot' means 'was asked not to'." It was asked not to,
    /// and it did anyway — a reaction turn on 2026-08-10 ran
    /// `nl -ba views/people/voice-roster.jsx | sed -n …` mid-sentence, and turns that
    /// held a shell wrote file-and-line code reviews as message text instead of calling
    /// `say`, which reaches nobody. [`reaction_permissions`] is the enforcement.
    fn thread_config(
        &self,
        cfg: &AgentConfig,
        role: Role,
        session_id: Option<crate::foundation::registry::SessionId>,
    ) -> serde_json::Map<String, serde_json::Value> {
        let mut headers = serde_json::Map::new();
        headers.insert(HEADER_ROLE.to_string(), json!(role.as_str()));
        if let Some(id) = session_id {
            headers.insert(HEADER_SESSION_ID.to_string(), json!(id.to_string()));
        }

        let mut config = cfg.thread_config(
            &format!(
                "{}{}",
                self.inner.server_base_url,
                crate::foundation::privacy::proxy::MODEL_PROXY_BASE_PATH
            ),
            crate::foundation::privacy::ENV_MODEL_PROXY_KEY,
        );
        config.insert(
            "mcp_servers".into(),
            json!({
                "hi-agent": {
                    "url": format!("{}/mcp", self.inner.server_base_url),
                    "http_headers": headers,
                    // These are hi-agent's *own* tools, served by this very process —
                    // there is nobody to ask about them. Without this codex gates every
                    // call behind an approval, which showed up live as `say` failing
                    // with "user rejected MCP tool call" and Reaction going silent.
                    "default_tools_approval_mode": "auto",
                }
            }),
        );
        // **Steering is asked for on every thread, and it is the host that needs it.**
        // `turn/steer` is what lets a message reach a rung that is already working
        // ([`AgentSession::steer`](crate::foundation::codex::AgentSession::steer)); without
        // the feature, codex refuses the call and the caller falls back to the next turn
        // boundary — which is the wait that let a correction sit for seven minutes while
        // the work it contradicted went ahead. Set for every role rather than for Cognition
        // alone: the key names a *capability of the thread*, and a rung that cannot be
        // reached mid-turn should be that way because nothing steers it, not because its
        // thread was opened unable to hear.
        let mut features = serde_json::Map::new();
        features.insert("steer".into(), json!(true));
        // The provider reads its loopback proxy credential from the parent process
        // environment. Model-authored commands must not inherit that token, or any
        // other key/secret/token the host happened to launch with.
        config.insert(
            "shell_environment_policy".into(),
            json!({
                "ignore_default_excludes": false,
                "filters": {
                    crate::foundation::privacy::ENV_MODEL_PROXY_KEY: "exclude",
                },
            }),
        );
        if role == Role::Reaction {
            let (name, profile) = reaction_permissions();
            config.insert("permissions".into(), json!({ name: profile }));
            // The profile *selects*; these two *remove*. A profile cannot take a built-in
            // away (see [`reaction_permissions`]), so the tools themselves are switched
            // off at their own knobs — verified against 0.147 by reading the tool list on
            // the upstream request: with these, `exec_command`, `write_stdin` and
            // `view_image` are gone and `mcp__hi_agent__say` remains.
            //
            // `update_plan` is left alone deliberately: its key takes a struct, not a
            // bool (`expected struct UpdatePlanToolConfig`), and a planning scratchpad
            // nobody reads is not what makes Reaction slow.
            features.insert("shell_tool".into(), json!(false));
            features.insert("view_image".into(), json!(false));
            config.insert("tools".into(), json!({ "web_search": false }));
        }
        config.insert("features".into(), serde_json::Value::Object(features));
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

/// The codex permission profile Reaction's thread opens under, and its name.
///
/// The profile carries Reaction's **sandbox**, because a thread opened with
/// `permissions` may not also pass `sandbox` — codex answers `permissions cannot be
/// combined with sandbox`, the profile being the thing that owns that setting.
///
/// It does **not** carry the toolset, though it reads as if it should.
/// `default_tools_enabled = false` was taken to be codex's switch for its own tools —
/// shell, apply-patch, web search — and it is not: with the profile provably in force
/// (`thread/read` reporting `activePermissionProfile: hi-agent-reaction`), the upstream
/// request still offered `exec_command`, `write_stdin` and `apply_patch`. It is kept
/// because it is the profile's own statement of intent and costs nothing; the tools are
/// actually removed one knob at a time in [`AgentLayer::thread_config`].
///
/// The shape is codex's, not ours, and it is picky: `permissions` is a **map of named
/// profiles**, and the name is passed to `thread/start` as a parameter (see
/// [`SessionOpts::permission_profile`]) — writing `default_permissions` into the config
/// map, as `config.toml` would, is accepted and silently ignored. The flatter spellings
/// do not exist either: 0.144.1 under `--strict-config` answers
/// `unknown configuration field tools.default_tools_enabled`, and
/// `permissions.default_tools_enabled` with `expected struct PermissionProfileToml`.
/// Which named permission profile a role's thread opens under.
///
/// Only Reaction has one, and it is the same role that must *not* pass `sandbox` —
/// codex refuses a `thread/start` carrying both, the profile being what owns that
/// setting. Every other rung keeps the plain param, and keeps its full-access
/// sandbox so local commands can consume ordinary drive files.
fn permission_profile(role: Role) -> Option<String> {
    (role == Role::Reaction).then(|| REACTION_PERMISSION_PROFILE.to_string())
}

fn reaction_permissions() -> (&'static str, serde_json::Value) {
    (
        REACTION_PERMISSION_PROFILE,
        json!({ "default_tools_enabled": false, "sandbox": Sandbox::ReadOnly.as_str() }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::WorkerType;

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
            crate::foundation::privacy::PrivacyBoundary::open(&PathBuf::from(format!(
                "/tmp/hi-agent-privacy-test-{}",
                uuid::Uuid::new_v4()
            )))
            .unwrap(),
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
        let config = layer().thread_config(&config(), Role::Worker(WorkerType::General), Some(42.into()));
        let server = &config["mcp_servers"]["hi-agent"];
        assert_eq!(server["url"], "http://127.0.0.1:12358/mcp");
        assert_eq!(server["http_headers"][HEADER_ROLE], "worker");
        // Stringified: HTTP header values are text, and codex forwards them verbatim.
        assert_eq!(server["http_headers"][HEADER_SESSION_ID], "42");
    }

    #[test]
    fn a_session_without_an_id_still_names_its_role() {
        let config = layer().thread_config(&config(), Role::Reaction, None);
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
        let config = layer().thread_config(&config(), Role::Cognition, Some(1.into()));
        assert_eq!(config["model"], "gpt-5.1-codex");
        assert!(config.contains_key("model_providers"));
        assert!(config.contains_key("mcp_servers"));
    }

    /// Swept over every role rather than a hand-written list of four, so a worker type
    /// added later cannot quietly arrive sandboxed or unsandboxed unnoticed.
    #[test]
    fn only_the_voice_is_read_only() {
        assert_eq!(Role::Reaction.sandbox(), Sandbox::ReadOnly);
        for role in Role::ALL.iter().filter(|r| **r != Role::Reaction) {
            assert_eq!(
                role.sandbox(),
                Sandbox::FullAccess,
                "{role:?} must be able to use drive files from local commands"
            );
        }
    }

    /// Swept the same way as the sandbox, and for the same reason: "no built-ins at all"
    /// is Reaction's alone, and a rung that quietly arrived without a shell would be a
    /// rung that cannot do its job.
    ///
    /// Asserts the *switches*, not the profile: naming a profile is what 0.147 ignored
    /// for months, and even honoured it leaves the shell in place. `shell_tool` is the
    /// line that actually takes it away.
    #[test]
    fn only_the_voice_opens_with_the_agents_own_tools_off() {
        let (name, _) = reaction_permissions();
        let reaction = layer().thread_config(&config(), Role::Reaction, Some(1.into()));
        assert_eq!(reaction["features"]["shell_tool"], false);
        assert_eq!(reaction["tools"]["web_search"], false);
        assert_eq!(reaction["permissions"][name]["default_tools_enabled"], false);
        // `default_permissions` is the config.toml spelling and does nothing here; the
        // name travels as a `thread/start` parameter instead. Asserted absent so nobody
        // re-adds it and reads a passing test as enforcement.
        assert!(!reaction.contains_key("default_permissions"));
        // The restriction is codex's own toolset; ours arrives over MCP and must survive
        // it, or the profile that was meant to leave Reaction holding `say` takes it.
        assert!(reaction.contains_key("mcp_servers"));

        for role in Role::ALL.iter().filter(|r| **r != Role::Reaction) {
            let config = layer().thread_config(&config(), *role, Some(1.into()));
            assert!(
                !config.contains_key("permissions"),
                "{role:?} works under its full-access sandbox"
            );
            // `features` is no longer Reaction's alone — `steer` is asked for on every
            // thread — so the sweep is over the switches that *remove* a built-in rather
            // than over the key that carries them.
            assert!(
                config["features"].get("shell_tool").is_none()
                    && config["features"].get("view_image").is_none(),
                "{role:?} works, and working needs tools"
            );
        }
    }

    /// Every rung opens able to be reached mid-turn, whether or not anything reaches it
    /// yet. A thread that cannot be steered is one where a correction has to wait for the
    /// turn to end, and the turn it most needs to reach is the long one.
    #[test]
    fn every_rung_opens_steerable() {
        for role in Role::ALL {
            let config = layer().thread_config(&config(), *role, Some(1.into()));
            assert_eq!(config["features"]["steer"], true, "{role:?} must be reachable mid-turn");
        }
    }

    /// The two spellings are mutually exclusive on the wire — codex rejects a
    /// `thread/start` carrying both — so Reaction must be the one role that names a
    /// profile, and every other role the one that names a sandbox.
    #[test]
    fn only_the_voice_opens_under_a_named_permission_profile() {
        let (name, profile) = reaction_permissions();
        assert_eq!(permission_profile(Role::Reaction).as_deref(), Some(name));
        assert_eq!(
            profile["sandbox"],
            Sandbox::ReadOnly.as_str(),
            "a profile carries its own sandbox; the param is refused alongside it"
        );
        assert_eq!(
            profile["sandbox"],
            Role::Reaction.sandbox().as_str(),
            "Reaction's posture must not depend on which of the two spellings is read"
        );
        for role in Role::ALL.iter().filter(|r| **r != Role::Reaction) {
            assert!(permission_profile(*role).is_none());
        }
    }

    /// All five worker types attach the same surface. This is the property that lets the
    /// type stay a prompt selector: if a specialism ever needed its own tools it would be
    /// a rung, not a type.
    #[test]
    fn every_worker_type_attaches_the_one_worker_surface() {
        for t in WorkerType::ALL {
            let config = layer().thread_config(&config(), Role::Worker(*t), Some(7.into()));
            assert_eq!(
                config["mcp_servers"]["hi-agent"]["http_headers"][HEADER_ROLE],
                "worker",
                "{} asked for a surface of its own",
                t.as_str()
            );
        }
    }

    #[test]
    fn model_auth_tokens_are_not_inherited_by_shell_commands() {
        for role in Role::ALL {
            let config = layer().thread_config(&config(), *role, Some(1.into()));
            assert_eq!(
                config["shell_environment_policy"]["ignore_default_excludes"],
                false
            );
            assert_eq!(
                config["shell_environment_policy"]["filters"]
                    [crate::foundation::privacy::ENV_MODEL_PROXY_KEY],
                "exclude"
            );
        }
    }
}
