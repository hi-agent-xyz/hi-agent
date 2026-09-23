//! The model catalog hi-agent hands codex at spawn — the one place the broker's
//! facts about a model are translated into codex's own metadata schema.
//!
//! **Why this exists.** Codex ships a metadata table covering OpenAI's own models and
//! nothing else. Ask it to run any other slug and it logs
//! `Model metadata for '<slug>' not found. Defaulting to fallback metadata` and
//! proceeds on a *guess* — most consequentially [`FALLBACK_CONTEXT_WINDOW`] tokens.
//! Every model the broker puts on the menu holds far more than that (deepseek-v4 and
//! Claude Opus hold 1M, gpt-5.5 1.05M), and codex auto-compacts at ~90% of whatever it
//! believes the window is. So an uncorrected agent summarises and discards its context
//! at a quarter of the window it is paying for, on every session, silently.
//!
//! **The facts come from the broker** ([`ModelFacts`]), because the menu is served
//! from hi-agent.xyz and changes without a client release — a table compiled in here
//! would be wrong about exactly the models that were added after it shipped. BYOK has
//! no broker and so no facts: that path writes no catalog and keeps codex's fallback,
//! which is what it does today.
//!
//! **The path in, and why it is not the thread config.** Everything else hi-agent
//! overrides rides `thread/start`'s config map ([`super::AgentConfig::thread_config`]).
//! This one cannot: codex's `model_catalog_json` is read once when the process builds
//! its models manager, and its own `ConfigToml` doc says per-thread overrides of it
//! "are accepted but do not reapply this (no-ops)". So the catalog is a file written
//! before the spawn and named by a `-c` argument on the command line — the same
//! startup layer, reached the only way a host can reach it.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::foundation::credentials::ModelFacts;

/// The context window codex assumes for a model it has never heard of. Not a number
/// we chose — it is `models-manager`'s fallback record, quoted here so the comparison
/// below has something to compare against.
const FALLBACK_CONTEXT_WINDOW: i64 = 272_000;

/// The catalog file's name inside `CODEX_HOME`. One file, overwritten per spawn: the
/// model can change under a running host (Settings edit, broker re-mint, mode switch)
/// exactly as the key can, and a spawn reads both fresh for the same reason.
const CATALOG_FILE: &str = "hi-agent-models.json";

/// The codex config key naming the file. Startup-layer only — see the module docs.
pub const CONFIG_KEY: &str = "model_catalog_json";

/// Render the catalog for one model, or `None` when there is nothing truer than
/// codex's own fallback to say.
///
/// Two cases return `None`, and both mean *stay out of the way*: a model nobody named,
/// and facts nobody published. Writing a catalog anyway would replace codex's guess
/// with our own, which is not an improvement — it is the same guess with our name on
/// it. The warning in the log is then correct and worth leaving there.
pub fn render(model: Option<&str>, facts: ModelFacts) -> Option<String> {
    let slug = model.map(str::trim).filter(|m| !m.is_empty())?;
    if facts.is_unknown() {
        return None;
    }

    // Reasoning levels, or none at all. Codex keeps a requested effort only if the
    // model's own list contains it, so an empty list on a reasoning model silently
    // drops the `model_reasoning_effort=high` we set beside it; a populated list on a
    // model that cannot reason would ask a vendor for a parameter it rejects. The
    // levels themselves are codex's vocabulary, not the broker's — the broker says
    // only whether the model reasons.
    let levels: Vec<serde_json::Value> = if facts.reasoning {
        ["low", "medium", "high"]
            .iter()
            .map(|e| serde_json::json!({ "effort": e, "description": "" }))
            .collect()
    } else {
        Vec::new()
    };

    // Everything below the two facts is deliberately codex's own fallback value rather
    // than a better guess: the shell type, the truncation limit, parallel tool calls,
    // apply-patch and verbosity are all things the broker never told us, and the point
    // of this file is to stop *guessing* about models. Only the fields we actually
    // know are answered differently.
    //
    // `visibility: list` and `priority: 0` are the exception, and they are not claims
    // about the model — they make the entry the one codex finds when it resolves the
    // requested slug against the catalog, instead of a model it might substitute.
    let model_info = serde_json::json!({
        "slug": slug,
        "display_name": slug,
        "description": null,
        // **Required, and empty on purpose.** Codex's catalog deserializer rejects any
        // entry carrying neither `base_instructions` nor
        // `model_messages.instructions_template` — the whole file then fails to parse,
        // which is a `Config::load` error and so no agent at all, not a degraded one.
        //
        // Empty rather than a copy of codex's own 18KB default prompt, because in this
        // host a model has no built-in prompt: every thread is opened with
        // `baseInstructions`, which is the rung's prompt and the reason codex was
        // chosen over ACP in the first place. Codex folds that over whatever the
        // catalog says (`with_config_overrides`), so this string is never what reaches
        // a model. It is a slot codex requires us to fill, and "" is the honest filler.
        //
        // The one thing that would make it visible: a session opened with no
        // `baseInstructions` at all would get an empty system prompt where it used to
        // get codex's default. All four call sites pass one today.
        "base_instructions": "",
        "supported_reasoning_levels": levels,
        "shell_type": "default",
        "visibility": "list",
        "supported_in_api": true,
        "priority": 0,
        "availability_nux": null,
        "upgrade": null,
        "support_verbosity": false,
        "default_verbosity": null,
        "apply_patch_tool_type": null,
        "web_search_tool_type": "text",
        "truncation_policy": { "mode": "bytes", "limit": 10_000 },
        "supports_parallel_tool_calls": false,
        "experimental_supported_tools": [],
        // The fact this whole module exists for. `max_context_window` is the ceiling
        // codex clamps a `model_context_window` override to, so it has to be the real
        // window too or the override could never reach it.
        "context_window": facts.context,
        "max_context_window": facts.context,
    });

    serde_json::to_string_pretty(&serde_json::json!({ "models": [model_info] })).ok()
}

/// Write the catalog into `codex_home` and return the path to name on the command
/// line, or `None` when there is no catalog to write (see [`render`]) or the write
/// failed.
///
/// **A failure here is not a failed spawn.** Everything this file carries is an
/// improvement on a default that already works, so a read-only `CODEX_HOME` costs a
/// log line and the session starts with codex's fallback — the behaviour of every
/// release before this one. The opposite arrangement would let a disk problem take
/// the agent down over a context-window number.
///
/// Written to a temporary file and renamed, because sessions spawn concurrently and
/// each one writes this same path. Rename is atomic, so a codex process reading the
/// file either sees the whole previous catalog or the whole new one, never a half of
/// each. (Concurrent spawns write identical bytes unless the model changed between
/// them, so this guards the one case that is rare rather than the common one.)
pub fn install(codex_home: &Path, model: Option<&str>, facts: ModelFacts) -> Option<PathBuf> {
    let json = render(model, facts)?;
    let path = codex_home.join(CATALOG_FILE);
    match write_atomically(&path, &json) {
        Ok(()) => {
            tracing::debug!(
                model = model.unwrap_or_default(),
                context = facts.context,
                path = %path.display(),
                "wrote codex model catalog"
            );
            Some(path)
        }
        Err(e) => {
            tracing::warn!(
                error = %format!("{e:#}"),
                path = %path.display(),
                falls_back_to = FALLBACK_CONTEXT_WINDOW,
                "could not write the codex model catalog; this session runs on codex's \
                 built-in fallback model metadata"
            );
            None
        }
    }
}

fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Same directory as the target, so the rename stays within one filesystem. The pid
    // keeps two concurrent spawns from writing the same temporary file.
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> ModelFacts {
        ModelFacts { context: 1_000_000, reasoning: true }
    }

    /// The whole point: the rendered entry claims the model's real window, and it is
    /// not the one codex would have assumed.
    #[test]
    fn carries_the_real_context_window() {
        let json = render(Some("deepseek-v4-flash"), known()).expect("catalog");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        let m = &v["models"][0];
        assert_eq!(m["slug"], "deepseek-v4-flash");
        assert_eq!(m["context_window"], 1_000_000);
        assert_eq!(m["max_context_window"], 1_000_000);
        assert_ne!(m["context_window"], FALLBACK_CONTEXT_WINDOW);
    }

    /// **Codex refuses a catalog entry with no instructions template**, and the refusal
    /// is a parse error on the whole file — which reaches us as a failed `Config::load`
    /// and a codex that will not start at all. Checked here because the field is
    /// otherwise invisible: hi-agent's own `baseInstructions` overwrites it on every
    /// thread, so nothing downstream would ever notice it was missing.
    #[test]
    fn every_entry_carries_an_instructions_slot() {
        let json = render(Some("deepseek-v4-flash"), known()).expect("catalog");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        let m = &v["models"][0];
        assert!(
            m.get("base_instructions").is_some()
                || m.pointer("/model_messages/instructions_template").is_some(),
            "codex rejects an entry with neither; entry was {m}"
        );
    }

    /// Codex drops a requested effort that the model's own level list does not
    /// contain, so a reasoning model must publish the levels we ask for.
    #[test]
    fn reasoning_models_publish_the_efforts_we_set() {
        let json = render(Some("deepseek-v4-pro"), known()).expect("catalog");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        let efforts: Vec<String> = v["models"][0]["supported_reasoning_levels"]
            .as_array()
            .expect("levels")
            .iter()
            .map(|l| l["effort"].as_str().unwrap_or_default().to_string())
            .collect();
        assert_eq!(efforts, ["low", "medium", "high"]);

        let plain = render(Some("plain"), ModelFacts { context: 32_768, reasoning: false })
            .expect("catalog");
        let v: serde_json::Value = serde_json::from_str(&plain).expect("valid json");
        assert!(
            v["models"][0]["supported_reasoning_levels"]
                .as_array()
                .expect("levels")
                .is_empty()
        );
    }

    /// Nothing published, nothing written — BYOK keeps codex's fallback rather than
    /// getting ours.
    #[test]
    fn says_nothing_when_it_knows_nothing() {
        assert!(render(Some("some-byok-model"), ModelFacts::default()).is_none());
        assert!(render(None, known()).is_none());
        assert!(render(Some("   "), known()).is_none());
    }

    /// The file lands where the `-c` argument will point, and a second spawn over the
    /// top of it leaves one whole catalog rather than two spliced.
    #[test]
    fn install_writes_one_whole_file() {
        let dir = tempfile::tempdir().expect("tmp");
        let first = install(dir.path(), Some("deepseek-v4-flash"), known()).expect("path");
        assert_eq!(first, dir.path().join(CATALOG_FILE));

        let second = install(dir.path(), Some("gpt-5.5"), ModelFacts { context: 1_050_000, reasoning: true })
            .expect("path");
        assert_eq!(second, first);
        let text = std::fs::read_to_string(&second).expect("read back");
        let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(v["models"].as_array().expect("models").len(), 1);
        assert_eq!(v["models"][0]["slug"], "gpt-5.5");

        // No temporary left behind for codex to trip over.
        let strays: Vec<_> = std::fs::read_dir(dir.path())
            .expect("readdir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != CATALOG_FILE)
            .collect();
        assert!(strays.is_empty(), "left behind {strays:?}");
    }
}
