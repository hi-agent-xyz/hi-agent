//! Cognition config → a thread's codex config + the child's env. The LLM credential
//! (base URL, key, model) and the cognition tunables (effort, pulse, reflection cadence,
//! …) all come from the config store (Settings). The tunables are read via [`tunables`]
//! (a startup snapshot for the reaction's argless helpers) or
//! [`crate::foundation::credentials::get_setting`] directly where a data dir is in scope.
//! Only infra vars (e.g. the server base URL) remain env-driven.
//!
//! The split that matters: **everything the model wire needs rides the thread config**
//! ([`AgentConfig::thread_config`], sent on `thread/start`), and **only the secret and
//! the scratch dir ride the environment** ([`AgentConfig::child_env`] /
//! [`AgentConfig::auth_child_env`]). Codex reads a key by env-var *name*, so that split
//! falls out of the protocol rather than being a convention we impose.

use std::path::Path;

/// Default upstream base URL when the stored LLM base URL is empty.
///
/// Codex speaks the OpenAI **Responses** wire, so this is a `/v1` provider root, not a
/// host root: the client appends `/responses` to it. In managed mode the broker supplies
/// the songguo base instead (see [`crate::foundation::broker`]).
pub const DEFAULT_AI_API_BASE: &str = "https://api.openai.com/v1";

/// The provider id hi-agent registers with codex for whatever endpoint the credential
/// store names. Codex selects a provider by id, so the id and the block that defines it
/// have to agree; both are produced by [`AgentConfig::thread_config`].
const PROVIDER_ID: &str = "hi-agent-gateway";

/// Env var carrying the upstream key. Codex reads the key by *name* — a provider block
/// says `env_key = "…"` and the process env supplies the value — which is why the
/// credential never appears in the thread config we send over the wire.
pub const ENV_LLM_KEY: &str = "HI_AGENT_LLM_KEY";

// Keys under which the cognition tunables live in the config store's `app_settings`
// table. Shared by the readers (reaction, `resolve`) and the settings handler so the
// names can't drift. Each is optional; an absent key → the built-in default.
/// Reasoning effort for a thread's turns (e.g. low | medium | high), passed through as
/// codex's `model_reasoning_effort`.
pub const KEY_EFFORT: &str = "effort";
/// Idle interval between Cognition's glance-ups — how often the brain looks up from what
/// it is doing and reads down the ledger. Duration grammar (`90s`/`30m`/`1h`); `0`/`off`
/// silences the recurring arm but never the one wake shortly after boot, which is restart
/// recovery rather than a cadence; unset / unparseable → the built-in default. Reaction
/// has no cadence of its own — it is woken by input, mail, and its own check-in.
pub const KEY_PULSE: &str = "pulse";
/// How long Reaction may leave an open-ended silence standing while its own thinking
/// is still running, before the host wakes it to say where things stand. Duration
/// grammar; `0`/`off` disables the floor — leaving only the check-ins Reaction arms
/// itself through `say`'s `back_in`, never no check-ins at all; unset → the built-in
/// default (5m). The gap doubles on each consecutive host-armed check-in, up to `pulse`.
pub const KEY_CHECK_IN: &str = "check_in";
/// Master switch for the reflection ("sleep") pass; `off` disables it entirely.
pub const KEY_REFLECT: &str = "reflect";
/// Base reflection cadence — how often a conversation with fresh input consolidates.
/// Duration grammar; `0`/`off` disables; unset → the built-in default (1m).
pub const KEY_REFLECT_EVERY: &str = "reflect_every";
/// Ceiling on the idle reflection backoff. Duration grammar; unset → default (8h).
pub const KEY_REFLECT_MAX: &str = "reflect_max";
/// Consecutive terminal-turn failures before flipping to vendor-down ("mailbox")
/// mode. Each terminal failure is already 3 failed model calls, so 2 (the default)
/// = 6 failures across two turns. `0`/unparseable → default.
pub const KEY_VENDOR_DOWN_AFTER: &str = "vendor_down_after";
/// Recovery-probe cadence while in vendor-down mode. Duration grammar;
/// `off`/`0`/unset/unparseable → the 30s default.
pub const KEY_VENDOR_PROBE: &str = "vendor_probe";

/// Master switch for the right-⌘ attention gestures (the global key event tap).
/// Off unless explicitly enabled, because arming the tap forces the macOS
/// "Input Monitoring" grant at boot — we don't want that prompt out of the box.
/// `on`/`true`/`1`/`yes` (case-insensitive) enable it; anything else (incl. unset)
/// leaves gestures disarmed. Toggled from Settings ▸ General ▸ Attention gestures.
pub const KEY_GESTURES: &str = "gestures";

/// Interface appearance: `system` (follow the OS — the default when unset),
/// `light`, or `dark`. Applied on macOS by forcing `NSApp.appearance`, which drives
/// both the native chrome and the face web view's `prefers-color-scheme` together.
/// Set from Settings ▸ General ▸ Theme.
pub const KEY_THEME: &str = "theme";

/// The language the agent should speak/write in with the user: `system` (follow the
/// person's lead — the default when unset), or a language code from [`LANGUAGES`]
/// (e.g. `en`, `zh-Hans`). Surfaced to the mind as one soft-guidance line in the
/// system prompts (see `crate::identity::character_seed` and
/// `crate::identity::reaction_system_prompt`); applies on restart, like
/// the other cognition tunables. Set from Settings ▸ General ▸ Language.
pub const KEY_LANGUAGE: &str = "language";

/// **Who this install belongs to** — the `people/` facet subject naming the one person
/// whose agent this is (e.g. `赵力`), or unset.
///
/// This is the default sender for the *addressed* channels — `text` and `file`, the
/// things somebody deliberately sends to the agent — per
/// [`docs/arch/signal-attribution.md`]. Ambient channels (`audio`, `vision`) never use
/// it: whoever a room contains is a question for recognition, not for config.
///
/// **It is config rather than memory because it is an identity, not an instruction.**
/// `docs/arch/data.md` retires the user prompt slot on the grounds that what the person
/// *tells* the agent must land as a facet or a task and go through its judgment. Who the
/// person *is* was never in that category — it sits with the handle and the credential,
/// and an agent that has to work out whose it is will sometimes work it out wrong and
/// then be unable to tell that it guessed.
///
/// Unset is legitimate and means addressed signals are unattributed. Nothing infers it.
pub const KEY_OWNER: &str = "owner";

/// The theme options offered in Settings, as `(stored value, menu label)`. `system`
/// is first (the default). Shared by the picker and the applier so they can't drift.
pub const THEMES: &[(&str, &str)] = &[("system", "System"), ("light", "Light"), ("dark", "Dark")];

/// The language options offered in Settings, as `(stored value, menu label)`.
/// `system` (follow the person's lead) is first and is the default. The rest are
/// BCP-47-ish codes paired with their endonym; extend the list to add a language.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("system", "System (follow the person)"),
    ("en", "English"),
    ("zh-Hans", "简体中文"),
];

/// The human name of a real language choice (for the seed line), or `None` for
/// `system` / unset / an unknown code — in which case the agent is given no language
/// instruction and simply follows the person. Matches on [`LANGUAGES`].
pub fn language_name(value: Option<&str>) -> Option<&'static str> {
    let code = value.map(str::trim).filter(|c| !c.is_empty())?;
    if code.eq_ignore_ascii_case("system") {
        return None;
    }
    LANGUAGES
        .iter()
        .find(|(c, _)| c.eq_ignore_ascii_case(code))
        .map(|(_, label)| *label)
}

/// Interpret a stored on/off flag (e.g. [`KEY_GESTURES`]): `on`/`true`/`1`/`yes`
/// (case-insensitive, trimmed) → `true`; unset or anything else → `false`.
pub fn flag_on(value: Option<String>) -> bool {
    matches!(
        value
            .as_deref()
            .map(|v| v.trim().to_ascii_lowercase())
            .as_deref(),
        Some("on" | "true" | "1" | "yes")
    )
}

/// Env var (set on the cognition subprocess) carrying hi-agent's own HTTP base
/// URL, so sessions can read input channels and write the overlay over the same
/// wire the browser uses. See [`AgentConfig::child_env`]. Infra, not user config.
pub const ENV_SERVER_BASE_URL: &str = "HI_AGENT_BASE_URL";

/// The cognition tunables loaded once from the config store at startup into a
/// process global, so the reaction's argless helpers can read them without threading
/// a data dir. Changes apply on restart — like every other setting.
pub mod tunables {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::OnceLock;

    static TUNABLES: OnceLock<HashMap<String, String>> = OnceLock::new();

    /// Snapshot the config store's `app_settings` into the global. Idempotent (first
    /// wins); the composition root calls this once before the reaction spawns.
    pub fn init(data_dir: &Path) {
        let _ = TUNABLES.set(crate::foundation::credentials::all_settings(data_dir));
    }

    /// A stored tunable (trimmed, non-empty), or `None` when unset / before
    /// [`init`] — callers then apply their built-in default.
    pub fn get(key: &str) -> Option<String> {
        let map = TUNABLES.get()?;
        let v = map.get(key)?.trim();
        (!v.is_empty()).then(|| v.to_string())
    }

    /// The declared owner's `people/` subject, or `None` when this install has none.
    /// The one read behind every addressed-channel attribution — see
    /// [`super::KEY_OWNER`].
    pub fn owner() -> Option<String> {
        get(super::KEY_OWNER)
    }
}

/// HTTP headers a session's MCP attach carries on every tool call. Set when the
/// session is opened (see `agent::AgentLayer::session`) and read by the MCP
/// handler (see `crate::foundation::mcp`). The role selects the tool surface and
/// owning loop; the session id names the caller.
pub const HEADER_ROLE: &str = "X-HI-Role";
pub const HEADER_SESSION_ID: &str = "X-HI-Session-Id";

/// Cognition parameters, resolved from the credential store. The upstream credential
/// never lives in git and never rides the thread config — only the env var that names it.
#[derive(Clone)]
pub struct AgentConfig {
    pub upstream_base_url: String,
    pub model: Option<String>,
    /// Companion model for cheap background work; `None` → reuse `model`.
    pub small: Option<String>,
    pub effort: Option<String>,
    pub upstream_key: String,
}

// Hand-written so the upstream credential never lands in logs (`Config` derives
// Debug and is traced at startup). The key is reduced to a redaction marker.
impl std::fmt::Debug for AgentConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentConfig")
            .field("upstream_base_url", &self.upstream_base_url)
            .field("model", &self.model)
            .field("small", &self.small)
            .field("effort", &self.effort)
            .field("upstream_key", &"<redacted>")
            .finish()
    }
}

impl AgentConfig {
    /// Resolve the upstream LLM credential + adapter tunables for startup from the
    /// config store — the user's BYOK key, or (xiaoyuanzhu) the broker-minted bundle,
    /// plus the stored `effort` / `permission_mode`. There is no `.env` fallback: a
    /// fresh install works out of the box because xiaoyuanzhu auto-bootstraps a device
    /// account and the broker mints the key. Never errors — with no key the agent
    /// boots **unconfigured** (see [`is_configured`](Self::is_configured)), the server
    /// + Settings UI come up, and prompts fail clearly until a key is set.
    pub fn resolve(data_dir: &Path) -> Self {
        let store = crate::foundation::credentials::Credentials::load(data_dir);
        let llm = store.effective().map(|e| e.llm.clone()).unwrap_or_default();
        let model = llm
            .model
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty());
        let small = llm
            .small
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty());
        use crate::foundation::credentials::get_setting;
        Self::new(
            model,
            small,
            get_setting(data_dir, KEY_EFFORT),
            llm.base_url,
            llm.api_key,
        )
    }

    /// Whether an upstream key is configured. When false the agent is inert: it
    /// boots so the user can set a key in Settings, but prompts will fail until then.
    pub fn is_configured(&self) -> bool {
        !self.upstream_key.trim().is_empty()
    }

    /// Assemble from explicit parts. The base URL falls back to
    /// [`DEFAULT_AI_API_BASE`] when unset; an empty key is allowed (the
    /// **unconfigured** state — BYOK before the user has pasted a key).
    pub fn new(
        model: Option<String>,
        small: Option<String>,
        effort: Option<String>,
        upstream_base_url: String,
        upstream_key: String,
    ) -> Self {
        let upstream_base_url = if upstream_base_url.trim().is_empty() {
            DEFAULT_AI_API_BASE.to_string()
        } else {
            upstream_base_url
        };
        Self {
            upstream_base_url,
            model,
            small,
            effort,
            upstream_key,
        }
    }

    /// The codex config overrides a thread should open with: which model, over which
    /// provider, at what reasoning effort.
    ///
    /// These ride `thread/start`'s `config` map — codex's session-flags layer, the same
    /// one `codex -c key=value` writes — so they apply per thread and leave the user's
    /// own `config.toml` alone. Nothing is written to disk, and **the key is not in
    /// here**: the provider block names an env var ([`ENV_LLM_KEY`]) and the value
    /// arrives on the child's environment, so a thread config can be logged verbatim.
    ///
    /// Verified against `codex app-server` 0.144: a thread opened with these overrides
    /// and an otherwise empty `CODEX_HOME` reaches the configured endpoint. Re-checked
    /// on 0.147 at the bump — the thread still opens with this block; the endpoint leg
    /// was not re-exercised.
    pub fn thread_config(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut config = serde_json::Map::new();
        if let Some(model) = &self.model {
            config.insert("model".into(), serde_json::json!(model));
        }
        if let Some(effort) = &self.effort {
            config.insert("model_reasoning_effort".into(), serde_json::json!(effort));
        }
        config.insert("model_provider".into(), serde_json::json!(PROVIDER_ID));
        config.insert(
            "model_providers".into(),
            serde_json::json!({
                PROVIDER_ID: {
                    "name": "hi-agent gateway",
                    "base_url": self.upstream_base_url,
                    "env_key": ENV_LLM_KEY,
                    "wire_api": "responses",
                }
            }),
        );
        config
    }

    /// The **volatile** env vars — the upstream key — that the child sends to the LLM
    /// gateway. Split out from [`child_env`](Self::child_env) because this is the only
    /// var sourced from the credential store, and the store changes under a running app
    /// (broker re-mint, Settings edit, mode switch). Callers re-resolve it at each
    /// session spawn (see [`crate::foundation::agent`]) so a fresh child never carries a
    /// stale key, rather than freezing it at boot.
    ///
    /// The key rides [`ENV_LLM_KEY`] and *only* there: the model provider we render into
    /// the thread config names it via `env_key`, so codex reads the secret out of the
    /// child's environment itself. That is the whole reason this is an env var rather
    /// than a config field — a credential in the thread config would be a credential on
    /// the wire, and in the frame log the wire tap keeps. The test at the bottom of this
    /// module asserts the key never appears in that config.
    pub fn auth_child_env(&self) -> Vec<(String, String)> {
        vec![(ENV_LLM_KEY.to_string(), self.upstream_key.clone())]
    }

    /// The model the **reaction** (what reaches the person) should run: the **main,
    /// smart** model, same as cognition. The reaction's core skill is judging the edge
    /// of what it already holds — "can I answer from my prepared context, or must I
    /// hand this to cognition?" — which is a smart-model job, not a small-model one.
    /// Its speed comes from a *single bounded generation* over a prepared context (no
    /// fetch, no tool loop), not from a lighter model. So it takes `model`, falling back
    /// to `small` only when no main model is configured.
    ///
    /// (Historically this pinned the reaction to the *small* slot — that was from a spell
    /// when the reaction had accidentally inherited Opus *and* rode a hang-zone ACP
    /// adapter, and the small model was a workaround for a ~7-min turn. Both the adapter
    /// and the protocol are gone; the workaround is retired in favour of the smart model
    /// the contract calls for. See docs/arch/agents.md.)
    pub fn reaction_model(&self) -> Option<String> {
        self.model.as_ref().or(self.small.as_ref()).cloned()
    }

    /// Build the **static** env var pairs for the codex child process — everything fixed
    /// for the process lifetime (the server URL, codex's home). The volatile upstream
    /// credential comes from [`auth_child_env`](Self::auth_child_env), re-resolved per
    /// spawn and merged in by the agent layer, so a fresh child never carries a stale
    /// key.
    ///
    /// `server_port` is hi-agent's own HTTP port (handed to the child as
    /// `HI_AGENT_BASE_URL` so a session can reach the channels); `codex_home` is the
    /// scratch dir codex keeps its own state in.
    ///
    /// PATH is not among these: the child inherits ours untouched. It used to be
    /// rewritten to prepend the managed `node` dir, back when the runtime downloaded a
    /// Node to run `npm` with — a worker writing a throwaway script got that node for
    /// free. Now that nothing here installs an interpreter, handing the agent a node
    /// the user never installed would be inventing a capability rather than reflecting
    /// one, so a worker sees exactly the toolchain the host actually has.
    ///
    /// Nothing about the model or provider is here any more — that all rides
    /// [`thread_config`](Self::thread_config) on `thread/start`. The env carries exactly
    /// two things the wire cannot: the secret, and where codex may scribble.
    pub fn child_env(&self, server_port: u16, codex_home: &Path) -> Vec<(String, String)> {
        vec![
            (
                ENV_SERVER_BASE_URL.to_string(),
                format!("http://127.0.0.1:{server_port}"),
            ),
            (
                "CODEX_HOME".to_string(),
                codex_home.to_string_lossy().into_owned(),
            ),
            // Nothing here may open a browser: an agent process that pops Safari during
            // an OAuth path would be a surprise on someone's desktop.
            ("NO_BROWSER".to_string(), "1".to_string()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_name_maps_only_real_languages() {
        // A real code returns its endonym for the seed line.
        assert_eq!(language_name(Some("en")), Some("English"));
        assert_eq!(language_name(Some("zh-Hans")), Some("简体中文"));
        // Case-insensitive on the code.
        assert_eq!(language_name(Some("ZH-HANS")), Some("简体中文"));
        // `system`, unset, blank, and unknown all mean "no instruction — follow the person".
        assert_eq!(language_name(Some("system")), None);
        assert_eq!(language_name(None), None);
        assert_eq!(language_name(Some("  ")), None);
        assert_eq!(language_name(Some("kling-on")), None);
    }

    #[test]
    fn theme_and_language_options_lead_with_system() {
        // `system` is the default and must be the first option in each picker.
        assert_eq!(THEMES.first().map(|(v, _)| *v), Some("system"));
        assert_eq!(LANGUAGES.first().map(|(v, _)| *v), Some("system"));
    }


    #[test]
    fn takes_all_parts_from_args() {
        let cfg = AgentConfig::new(
            Some("gpt-5.1-codex".to_string()),
            None,
            Some("high".to_string()),
            "https://upstream.example/v1".to_string(),
            "secret-key".to_string(),
        );
        assert_eq!(cfg.upstream_base_url, "https://upstream.example/v1");
        assert_eq!(cfg.model.as_deref(), Some("gpt-5.1-codex"));
        assert_eq!(cfg.effort.as_deref(), Some("high"));
        assert_eq!(cfg.upstream_key, "secret-key");
    }

    #[test]
    fn empty_base_url_falls_back_to_default() {
        let cfg = AgentConfig::new(None, None, None, String::new(), "k".to_string());
        assert_eq!(cfg.upstream_base_url, DEFAULT_AI_API_BASE);
    }

    #[test]
    fn debug_redacts_the_upstream_key() {
        let cfg = AgentConfig::new(
            None,
            None,
            None,
            "https://x/v1".to_string(),
            "super-secret-key".to_string(),
        );
        let rendered = format!("{cfg:?}");
        assert!(!rendered.contains("super-secret-key"), "key leaked: {rendered}");
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn empty_key_means_unconfigured() {
        let cfg = AgentConfig::new(None, None, None, "https://x/v1".to_string(), String::new());
        assert!(!cfg.is_configured());
        let cfg = AgentConfig::new(None, None, None, "https://x/v1".to_string(), "k".to_string());
        assert!(cfg.is_configured());
    }

    #[test]
    fn unset_optionals_default_to_none() {
        let cfg = AgentConfig::new(None, None, None, "https://x/v1".to_string(), "k".to_string());
        assert!(cfg.model.is_none());
        assert!(cfg.effort.is_none());
    }

    #[test]
    fn thread_config_points_codex_at_the_gateway() {
        let cfg = AgentConfig::new(
            Some("gpt-5.1-codex".to_string()),
            None,
            Some("high".to_string()),
            "https://gateway.example/v1".to_string(),
            "sk-secret".to_string(),
        );
        let config = serde_json::Value::Object(cfg.thread_config());
        assert_eq!(config["model"], "gpt-5.1-codex");
        assert_eq!(config["model_reasoning_effort"], "high");
        assert_eq!(config["model_provider"], PROVIDER_ID);
        let provider = &config["model_providers"][PROVIDER_ID];
        assert_eq!(provider["base_url"], "https://gateway.example/v1");
        assert_eq!(provider["wire_api"], "responses");
        // The provider names the key's env var; the key itself must never be in here,
        // because a thread config is logged and tapped verbatim.
        assert_eq!(provider["env_key"], ENV_LLM_KEY);
        assert!(
            !config.to_string().contains("sk-secret"),
            "the credential leaked into the thread config: {config}"
        );
    }

    #[test]
    fn thread_config_omits_what_is_unset() {
        let cfg = AgentConfig::new(None, None, None, "https://x/v1".to_string(), "k".to_string());
        let config = cfg.thread_config();
        assert!(!config.contains_key("model"), "no model → let codex choose");
        assert!(!config.contains_key("model_reasoning_effort"));
        // The provider is not optional: without it codex would talk to OpenAI directly.
        assert!(config.contains_key("model_provider"));
    }

    #[test]
    fn child_env_sets_static_vars_only() {
        let cfg = AgentConfig::new(
            Some("gpt-5.1-codex".to_string()),
            None,
            None,
            "https://x/v1".to_string(),
            "k".to_string(),
        );
        let env = cfg.child_env(8080, std::path::Path::new("/data/codex-home"));
        let map: std::collections::HashMap<_, _> = env.into_iter().collect();
        assert_eq!(map["HI_AGENT_BASE_URL"], "http://127.0.0.1:8080");
        assert_eq!(map["CODEX_HOME"], "/data/codex-home");
        assert_eq!(map["NO_BROWSER"], "1");
        // PATH is inherited, not rewritten — we no longer install an interpreter to
        // splice in front of the user's own toolchain.
        assert!(!map.contains_key("PATH"));
        // The volatile credential is NOT frozen into the static env — it comes from
        // `auth_child_env`, re-resolved per session spawn.
        assert!(!map.contains_key(ENV_LLM_KEY));
    }

    #[test]
    fn reaction_runs_the_smart_model_and_falls_back_to_small() {
        let cfg = AgentConfig::new(
            Some("gpt-5.1-codex".to_string()),
            Some("gpt-5-mini".to_string()),
            None,
            "https://x/v1".to_string(),
            "k".to_string(),
        );
        assert_eq!(cfg.reaction_model().as_deref(), Some("gpt-5.1-codex"));

        let cfg = AgentConfig::new(
            None,
            Some("gpt-5-mini".to_string()),
            None,
            "https://x/v1".to_string(),
            "k".to_string(),
        );
        assert_eq!(cfg.reaction_model().as_deref(), Some("gpt-5-mini"));
    }

    #[test]
    fn resolve_reflects_the_current_stored_key() {
        // The whole point of re-resolving per spawn: a key change written to the
        // store (broker re-mint, Settings edit) is visible on the next resolve,
        // without freezing anything at boot. Uses BYOK so the stored key is the
        // effective one directly.
        use crate::foundation::credentials::{Credentials, LlmCredentials, Mode};
        let dir = tempfile::tempdir().unwrap();

        let mut store = Credentials {
            mode: Mode::Byok,
            llm: LlmCredentials {
                api_key: "key-A".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        store.save(dir.path()).unwrap();
        let a: std::collections::HashMap<_, _> = AgentConfig::resolve(dir.path())
            .auth_child_env()
            .into_iter()
            .collect();
        assert_eq!(a[ENV_LLM_KEY], "key-A");

        // Rotate the stored key; a fresh resolve must carry the new one.
        store.llm.api_key = "key-B".into();
        store.save(dir.path()).unwrap();
        let b: std::collections::HashMap<_, _> = AgentConfig::resolve(dir.path())
            .auth_child_env()
            .into_iter()
            .collect();
        assert_eq!(b[ENV_LLM_KEY], "key-B");
    }
}
