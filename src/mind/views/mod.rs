//! View compiler — turns agent-authored JSX/TSX into an ESM module the browser
//! imports same-origin.
//!
//! The agent emits a view as component source. We run it through esbuild's
//! single-file *transform* (not a bundle): JSX/TS → ESM, with every bare import
//! (`react`, `react/jsx-runtime`, `@hi/core`, `motion/react`) left untouched so
//! the page's import map resolves them to the host's shared instances. Output is a
//! disposable, content-addressed cache under `data_dir/views/_compiled/<hash>.mjs`
//! (a tool dir inside the views tree, like node_modules), served from `/views/`;
//! identical source compiles at most once. The agent-authored *source* sediments
//! separately as `views/<project>/<name>.jsx`.
//!
//! esbuild ships as a native binary in the `@esbuild/<os>-<arch>` package, whose
//! tarball the runtime downloads and unpacks alongside the codex CLI (see
//! `src/runtime`). We exec that binary directly — there is no Node anywhere in this,
//! neither as a wrapper nor as the thing that installed it.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, bail};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub mod builtin;
pub use builtin::install_builtin_views;

/// What the **review** half of the view loop needs, set once at startup.
///
/// Building a view is a conversation-side act — the reaction holds the compiler and hands
/// source down. *Reviewing* one is a tool call from a working session, and
/// `dispatch_tool` reaches neither the reaction nor the bound port: it is handed a
/// conversation registry, a data dir and the call's arguments, and rendering needs both a
/// compiler and this server's own base URL. Rather than thread two more parameters
/// through the whole tool surface for one call, the process publishes them here —
/// the same shape `registry::global()` and `foundation::run` already use.
///
/// `None` until [`set_render_context`] runs, which is what a unit test sees; the tool
/// answers with a plain error rather than panicking, because a review that cannot run
/// is a fixable condition and not a bug in the caller.
static RENDER: std::sync::OnceLock<RenderContext> = std::sync::OnceLock::new();

/// The compiler and base URL a view review runs against.
#[derive(Debug, Clone)]
pub struct RenderContext {
    pub compiler: ViewCompiler,
    /// This server's own origin, e.g. `http://127.0.0.1:12358` — the host page
    /// (`GET /render/view`) is served by us, and its import map has to be *ours* or the
    /// view's bare imports resolve against a different React.
    pub base_url: String,
}

/// Publish the render context. Called once from startup; later calls are ignored, so
/// there is no way for a second one to swap the compiler under a running review.
pub fn set_render_context(compiler: ViewCompiler, base_url: impl Into<String>) {
    let _ = RENDER.set(RenderContext { compiler, base_url: base_url.into() });
}

/// The render context, or `None` before startup published it.
pub fn render_context() -> Option<&'static RenderContext> {
    RENDER.get()
}

/// A view ref is a relative path under the views tree, naming the view's source file
/// minus the `.jsx` — e.g. `badminton-top10/leader` → `views/badminton-top10/
/// leader.jsx`. Each `/`-separated segment is a slug (letters, digits, `-`, `_`) —
/// no dots, no empty segments — so the ref stays inside the views tree and can't
/// traverse out. The build sub-agent writes `<ref>.jsx` with its own file tools (no
/// MCP tool needed); this reads it back server-side, so the JSX never enters the
/// mind's context.
pub fn valid_ref(view_ref: &str) -> bool {
    !view_ref.is_empty()
        && view_ref.len() <= 128
        && view_ref.split('/').all(|seg| {
            !seg.is_empty()
                && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

/// Read a view ref's source and what it declared about itself.
///
/// This is the one place a ref becomes source, because two callers now need it and
/// they are on opposite sides of the view's life: `show` resolves a ref the agent
/// named, and the appearance restore re-resolves the ref a *past* show recorded.
/// Both must read the same file by the same rules, or a view would come back as
/// something the tool would refuse to put up.
///
/// Views are full-bleed, so `owns_conversation` is the only thing left to declare and a
/// missing or unparseable sidecar is not an error — it just means host-owned captions.
pub async fn resolve_ref(
    data_dir: &Path,
    view_ref: &str,
) -> Result<(String, Option<crate::types::ViewTraits>), String> {
    let view_ref = view_ref.trim();
    if !valid_ref(view_ref) {
        return Err(format!("invalid ref `{view_ref}` (names and `/` only, no dots)"));
    }
    let views = data_dir.join("views");
    let source = tokio::fs::read_to_string(views.join(format!("{view_ref}.jsx")))
        .await
        .map_err(|e| format!("no such view ({e})"))?;
    let traits = match tokio::fs::read(views.join(format!("{view_ref}.geom.json"))).await {
        Ok(bytes) => serde_json::from_slice::<crate::types::ViewTraits>(&bytes).ok(),
        Err(_) => None,
    };
    Ok((source, traits))
}

/// Compiles agent view source to a served ESM module URL. Cheap to clone.
#[derive(Debug, Clone)]
pub struct ViewCompiler {
    /// The esbuild native binary (`@esbuild/<os>-<arch>/bin/esbuild`).
    esbuild_bin: PathBuf,
    /// Where compiled modules are written (`data_dir/views/_compiled`).
    generated_dir: PathBuf,
}

impl ViewCompiler {
    /// Build from a resolved esbuild binary (see [`runtime::ensure_view_esbuild`],
    /// which guarantees one regardless of where the runtime came from) and a
    /// `data_dir` under which compiled modules are written.
    pub fn new(esbuild_bin: PathBuf, data_dir: &Path) -> Self {
        // Compiled modules are a disposable, content-addressed cache living under the
        // views tree (a tool dir like node_modules), served at /views/_compiled.
        Self::with_paths(esbuild_bin, data_dir.join("views").join("_compiled"))
    }

    fn with_paths(esbuild_bin: PathBuf, generated_dir: PathBuf) -> Self {
        Self { esbuild_bin, generated_dir }
    }

    /// Compile `source` to an ESM module and return its served URL
    /// (`/views/_compiled/<hash>.mjs`). Content-addressed: identical source
    /// yields the same URL and is compiled at most once (a cache hit never
    /// spawns esbuild).
    pub async fn compile(&self, source: &str) -> anyhow::Result<String> {
        let (hash, url) = module_ref(source);
        let out_path = self.generated_dir.join(format!("{hash}.mjs"));
        if out_path.exists() {
            return Ok(url);
        }
        if !self.esbuild_bin.exists() {
            bail!(
                "esbuild not found at {} — `runtime::ensure_view_esbuild` should have \
                 downloaded it; check that dir and the startup log",
                self.esbuild_bin.display()
            );
        }

        let js = self.transform(source).await?;

        tokio::fs::create_dir_all(&self.generated_dir)
            .await
            .with_context(|| format!("creating {}", self.generated_dir.display()))?;
        // Atomic publish: write a temp then rename, so a concurrent import never
        // observes a half-written module.
        let tmp = self
            .generated_dir
            .join(format!("{hash}.mjs.tmp.{}", std::process::id()));
        tokio::fs::write(&tmp, js.as_bytes())
            .await
            .with_context(|| format!("writing {}", tmp.display()))?;
        tokio::fs::rename(&tmp, &out_path)
            .await
            .with_context(|| format!("publishing {}", out_path.display()))?;
        Ok(url)
    }

    /// Run esbuild as a single-file transform: source on stdin, ESM on stdout.
    /// No `--bundle`, so bare imports survive for the import map to resolve.
    async fn transform(&self, source: &str) -> anyhow::Result<String> {
        let mut child = Command::new(&self.esbuild_bin)
            .arg("--loader=tsx")
            .arg("--format=esm")
            .arg("--jsx=automatic")
            .arg("--jsx-import-source=react")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawning esbuild at {}", self.esbuild_bin.display()))?;

        // Write stdin from a task so a large stdout can't deadlock us against a
        // full pipe while we're still feeding the source. Dropping the handle
        // closes stdin, signalling EOF.
        let mut stdin = child.stdin.take().expect("stdin is piped");
        let src = source.as_bytes().to_vec();
        let writer = tokio::spawn(async move {
            let _ = stdin.write_all(&src).await;
        });

        let out = child.wait_with_output().await.context("waiting for esbuild")?;
        let _ = writer.await;

        if !out.status.success() {
            bail!(
                "esbuild rejected the view source:\n{}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        String::from_utf8(out.stdout).context("esbuild output was not UTF-8")
    }
}

/// Deterministic content hash + served URL for `source`. A cache key, not a
/// security boundary: a 64-bit hash is ample for de-duping a few authored views.
///
/// `pub(crate)` so a test can seed the compiled cache for a known source and
/// exercise a caller of [`ViewCompiler::compile`] on the cache-hit path, which is
/// the one path through the compiler that never spawns esbuild.
pub(crate) fn module_ref(source: &str) -> (String, String) {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut h);
    let hash = format!("{:016x}", h.finish());
    let url = format!("/views/_compiled/{hash}.mjs");
    (hash, url)
}

#[cfg(test)]
mod view_ref_tests {
    use super::*;

    #[test]
    fn ref_validation_allows_nested_slugs_blocks_traversal() {
        assert!(valid_ref("badminton-top10"));
        assert!(valid_ref("badminton-top10/leader"));
        assert!(valid_ref("a/b/c_2"));
        assert!(!valid_ref(""), "empty");
        assert!(!valid_ref("../etc/passwd"), "dots blocked");
        assert!(!valid_ref("a//b"), "empty segment");
        assert!(!valid_ref("dot.name"), "dot blocked");
        assert!(!valid_ref("/abs"), "leading slash → empty segment");
        assert!(!valid_ref(&"x".repeat(129)), "too long");
    }

    #[tokio::test]
    async fn resolve_reads_views_source() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("views").join("deck");
        tokio::fs::create_dir_all(&proj).await.unwrap();
        tokio::fs::write(proj.join("leader.jsx"), "export default () => 1").await.unwrap();
        let (source, traits) = resolve_ref(dir.path(), "deck/leader").await.unwrap();
        assert_eq!(source, "export default () => 1");
        // No sidecar written → host-owned captions.
        assert!(traits.is_none());
    }

    #[tokio::test]
    async fn resolve_reads_traits_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("views").join("deck");
        tokio::fs::create_dir_all(&proj).await.unwrap();
        tokio::fs::write(proj.join("leader.jsx"), "export default () => 1").await.unwrap();
        tokio::fs::write(proj.join("leader.geom.json"), r#"{"owns_conversation":true}"#)
            .await
            .unwrap();
        let (_, traits) = resolve_ref(dir.path(), "deck/leader").await.unwrap();
        assert!(traits.expect("sidecar traits").owns_conversation);
    }

    /// Sidecars written under the old placement schema are still on disk in every
    /// existing workshop. They must degrade to the default rather than failing the
    /// show: the unknown `region`/`size` keys are ignored and `owns_conversation` — the
    /// one field that survived — is read straight through.
    #[tokio::test]
    async fn resolve_ignores_retired_placement_keys() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("views").join("deck");
        tokio::fs::create_dir_all(&proj).await.unwrap();
        tokio::fs::write(proj.join("leader.jsx"), "export default () => 1").await.unwrap();
        tokio::fs::write(
            proj.join("leader.geom.json"),
            r#"{"region":"right","size":"wide","owns_conversation":true}"#,
        )
        .await
        .unwrap();
        let (_, traits) = resolve_ref(dir.path(), "deck/leader").await.unwrap();
        assert!(traits.expect("sidecar traits").owns_conversation);
    }

    #[tokio::test]
    async fn resolve_rejects_bad_refs() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_ref(dir.path(), "../secret").await.is_err(), "traversal");
        assert!(resolve_ref(dir.path(), "missing/view").await.is_err(), "no file");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_ref_is_deterministic_and_hex() {
        let (h1, u1) = module_ref("export default () => null");
        let (h2, u2) = module_ref("export default () => null");
        assert_eq!(h1, h2);
        assert_eq!(u1, u2);
        assert_eq!(u1, format!("/views/_compiled/{h1}.mjs"));
        assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
        let (h3, _) = module_ref("a different view");
        assert_ne!(h1, h3, "different source must hash differently");
    }

    /// Locate an esbuild native binary if one is provisioned on this host — either the
    /// standalone view-tool install or the copy that ships with a managed runtime.
    /// Returns `None` to skip the spawning tests where esbuild isn't provisioned.
    ///
    /// `pub(crate)` so [`super::builtin`]'s tests can compile the bundled views through the
    /// real compiler rather than keeping a second copy of this walk.
    pub(crate) fn esbuild_probe() -> Option<PathBuf> {
        let (os, arch) = crate::runtime::npm_target().ok()?;
        let rel = crate::runtime::esbuild_rel();
        let cache = directories::ProjectDirs::from("dev", "human-interface", "hi-agent")?
            .cache_dir()
            .to_path_buf();

        // Standalone view-tool install (what `ensure_view_esbuild` provisions when the
        // runtime came from PATH).
        let standalone = cache
            .join("view-tool")
            .join(format!(
                "esbuild-{}-{os}-{arch}",
                env!("HI_AGENT_ESBUILD_VERSION")
            ))
            .join(&rel);
        if standalone.exists() {
            return Some(standalone);
        }

        // Managed runtime under a fingerprinted dir: any `runtime/*/esbuild`.
        let entries = std::fs::read_dir(cache.join("runtime")).ok()?;
        for entry in entries.flatten() {
            let bin = entry.path().join("esbuild").join(&rel);
            if bin.exists() {
                return Some(bin);
            }
        }
        None
    }

    #[tokio::test]
    async fn compiles_jsx_to_esm_and_preserves_bare_imports() {
        let Some(esbuild_bin) = esbuild_probe() else {
            eprintln!("skipping: esbuild not provisioned on this host");
            return;
        };
        let tmp = std::env::temp_dir().join(format!("hi-views-test-{}", std::process::id()));
        let compiler = ViewCompiler::with_paths(esbuild_bin, tmp.clone());

        let source = r#"
            import { motion } from "motion/react";
            import { useSpeech } from "@hi/core";
            export default function V() {
              const s = useSpeech();
              return <motion.div layoutId="x">{s.length}</motion.div>;
            }
        "#;
        let url = compiler.compile(source).await.expect("compile succeeds");
        assert!(url.starts_with("/views/_compiled/") && url.ends_with(".mjs"));

        let file = tmp.join(url.rsplit('/').next().unwrap());
        let js = std::fs::read_to_string(&file).expect("module written");
        assert!(js.contains(r#"from "react/jsx-runtime""#), "jsx runtime import emitted");
        assert!(js.contains(r#"from "motion/react""#), "bare motion import preserved");
        assert!(js.contains(r#"from "@hi/core""#), "bare @hi/core import preserved");
        assert!(!js.contains("<motion.div"), "JSX transformed away");

        // Second compile of identical source is a cache hit (same URL).
        let url2 = compiler.compile(source).await.expect("cache hit");
        assert_eq!(url, url2);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
