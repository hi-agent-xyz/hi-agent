//! Codex + view-toolchain provisioning.
//!
//! Two native binaries, for two unrelated jobs, with **no interpreter under either**.
//! **`codex`** is the agent: hi-agent drives `codex app-server` over stdio.
//! **`esbuild`** is hi-agent's own view compiler, spawned to turn agent-written
//! component source into ESM — nothing to do with the agent itself.
//!
//! Both are compiled programs (Rust and Go) that merely happen to be *distributed*
//! through the npm registry. So we fetch them the way you fetch any tarball: an HTTPS
//! GET of the platform package's `.tgz`, then `tar -x` into place. An npm package is a
//! gzipped tar whose entries all sit under a single `package/` root, so
//! `--strip-components=1` lands its contents directly in a directory of ours. There is
//! no Node, no npm, no lockfile and no `node_modules` anywhere in this path.
//!
//! That is the second interpreter to leave. First node stopped sitting *between*
//! hi-agent and the model (the ACP era's `node` → `claude-agent-acp` → `claude`, where
//! adapter/CLI version skew could wedge a turn for minutes). Now it is gone as a
//! package manager too: it was only ever the thing that unpacked two binaries we can
//! unpack ourselves, and downloading a 30 MB interpreter to run `npm ci` was the tail
//! wagging the dog.
//!
//! **We prefer what the system already offers**: a pin-matching `codex` on `PATH` is
//! used directly and we download nothing. That is also how you point hi-agent at your
//! own build for local development. A system runtime brings no esbuild with it, so the
//! view compiler is provisioned separately — see [`ensure_view_esbuild`].
//!
//! The pins live in `src/runtime/manifest.toml`, and the install dir is
//! **content-addressed** by them ([`runtime_fingerprint`]): bump a version and the next
//! run installs into a fresh subdir instead of silently reusing a stale one. That is
//! the whole auto-update story — an app update that changes a pin heals the runtime on
//! the user's next launch, with no manual cache deletion. Superseded installs are
//! pruned best-effort once the current one is ready.
//!
//! Prototype scope for the install path: macOS, Linux, and Windows on x86_64/aarch64,
//! extraction via the system `tar` (present on all three — unlike the browser's `.zip`,
//! a `.tgz` is readable by GNU tar and bsdtar alike), and no hash verification of the
//! downloads. HTTPS authenticates the registry, and a hash fetched from that same
//! registry at download time would add nothing; pinning hashes in-repo would defend
//! against a silent re-cut of a published version, which is not a threat worth six
//! per-platform constants that must be hand-updated on every bump.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use tokio::process::Command;

/// The headless browser the view render pipeline drives. Same system-first,
/// managed-fallback shape as the tools here; provisioned lazily on first render.
pub mod browser;

/// Pinned `@openai/codex` version, stamped from `src/runtime/manifest.toml`. Also used
/// to reject a *different* `codex` found on `PATH`: a stray global install silently
/// shadowing the pin is exactly the failure mode that wedged turns for minutes in the
/// ACP era, and the app-server protocol still moves fast enough that a version we did
/// not test is a real hazard (0.144 spells its sandbox modes kebab-case; the published
/// docs for a later build show camelCase).
const CODEX_VERSION: &str = env!("HI_AGENT_CODEX_VERSION");

/// Pinned `esbuild` version for the view toolchain, stamped from the same manifest.
const ESBUILD_VERSION: &str = env!("HI_AGENT_ESBUILD_VERSION");

/// Registry the platform tarballs come from. A constant rather than a literal so
/// pointing at a mirror is a one-line change.
const NPM_REGISTRY: &str = "https://registry.npmjs.org";

/// Absolute paths to the resolved runtime components.
#[derive(Debug, Clone)]
pub struct ResolvedRuntime {
    /// The `codex` executable hi-agent drives as `codex app-server --stdio`.
    pub codex_bin: PathBuf,
    /// The esbuild this runtime was installed with, when it has one. `None` for a
    /// system runtime, which found `codex` on `PATH` and so carries no tree of ours;
    /// [`ensure_view_esbuild`] fills that gap.
    pub esbuild_bin: Option<PathBuf>,
    /// Where these came from — `"system"` (found on `PATH`), `"managed"`
    /// (downloaded into the cache), or `"bundled"` (shipped in a `.app`).
    /// For logging only.
    pub origin: &'static str,
}

/// Resolve the runtime: prefer one bundled inside a packaged `.app`, else a
/// pin-matching system `codex`, else install on first run and reuse after.
pub async fn ensure() -> anyhow::Result<ResolvedRuntime> {
    // A shipped `.app` carries a complete managed runtime under its Resources;
    // use it so the packaged app runs with no download and is unaffected by
    // whatever codex happens to be on the user's PATH. Absent (dev/Docker/
    // Linux), this is `None` and we fall through to the existing tiers.
    if let Some(res) = crate::bundle::resources_dir() {
        let rt = res.join("runtime");
        if rt.join(".complete").exists() {
            tracing::debug!(path = %rt.display(), "using bundled runtime");
            return resolve_bundled(&rt);
        }
    }

    // Prefer what the system already offers — no download when the user has a
    // pin-matching codex on PATH.
    if let Some(system) = resolve_system() {
        return Ok(system);
    }

    let target = runtime_dir()?;

    // Reuse a complete install from a previous run, otherwise install now. The
    // target is fingerprinted by the current pins, so a `.complete` here means it
    // was built from *these* pins — a changed version lands on a different path.
    let runtime = if target.join(".complete").exists() {
        tracing::debug!(path = %target.display(), "runtime already installed");
        resolve(&target)?
    } else {
        install(&target).await?
    };

    // Now that a current runtime is ready, prune installs left by older pins so
    // the cache doesn't accumulate one stale tree per update.
    gc_stale_runtimes(&target);

    Ok(runtime)
}

/// Remove sibling runtime installs left behind by prior pins. Best-effort and
/// only for the default cache location — when `HI_AGENT_RUNTIME_DIR` overrides
/// the path the parent isn't ours to prune, so we leave it untouched. In-flight
/// temp dirs (`.runtime.tmp.*`, dot-prefixed) are skipped so a concurrent install
/// is never yanked out from under itself.
fn gc_stale_runtimes(current: &Path) {
    if std::env::var_os("HI_AGENT_RUNTIME_DIR").is_some() {
        return;
    }
    let (Some(root), Some(keep)) = (current.parent(), current.file_name()) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == keep || name.to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            tracing::info!(path = %path.display(), "removing stale runtime from a prior pin");
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// Resolve an esbuild native binary for the view compiler, guaranteeing one exists
/// regardless of where the runtime came from. A **managed** or **bundled** runtime was
/// installed with esbuild alongside codex, so we reuse that; a **system** runtime
/// (local dev with codex on `PATH`) has no tree of ours, so we install a standalone
/// copy once into a hi-agent-owned cache dir.
pub async fn ensure_view_esbuild(runtime: &ResolvedRuntime) -> anyhow::Result<PathBuf> {
    if let Some(adjacent) = runtime.esbuild_bin.as_deref()
        && adjacent.exists()
    {
        tracing::debug!(path = %adjacent.display(), "using esbuild installed with the runtime");
        return Ok(adjacent.to_path_buf());
    }
    ensure_standalone_esbuild().await
}

/// Install (once) a standalone esbuild into a fingerprinted cache dir and return
/// its native binary. Content-addressed by esbuild version + host target, so a
/// version bump installs fresh instead of reusing a stale binary.
async fn ensure_standalone_esbuild() -> anyhow::Result<PathBuf> {
    let (os, arch) = npm_target()?;
    let target = esbuild_dir(os, arch)?;
    let bin = target.join(esbuild_rel());
    if bin.exists() {
        return Ok(bin);
    }

    let parent = target
        .parent()
        .ok_or_else(|| anyhow!("esbuild dir {} has no parent", target.display()))?;
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;

    // Build in a sibling temp dir, then atomically rename — same publish pattern
    // as the runtime install, so a concurrent or interrupted start never sees a
    // half-installed esbuild.
    let tmp = parent.join(format!(".esbuild.tmp.{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&tmp).await;

    hint("preparing the view compiler (downloading esbuild, ~5 MB)…");
    let staged = tmp.join(esbuild_rel());
    let outcome = fetch_npm_package(&esbuild_tarball_url(os, arch), &tmp, "esbuild")
        .await
        .and_then(|()| {
            is_executable(&staged).then_some(()).ok_or_else(|| {
                anyhow!(
                    "esbuild downloaded but its native binary is missing at {} \
                     (unsupported host target {os}-{arch}?)",
                    staged.display()
                )
            })
        });
    if let Err(e) = outcome {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        return Err(e);
    }

    let _ = tokio::fs::remove_dir_all(&target).await;
    match tokio::fs::rename(&tmp, &target).await {
        Ok(()) => {}
        Err(_) if bin.exists() => {
            let _ = tokio::fs::remove_dir_all(&tmp).await;
        }
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&tmp).await;
            return Err(anyhow!("publishing esbuild to {}: {e}", target.display()));
        }
    }
    tracing::info!(path = %bin.display(), "view compiler esbuild ready");
    Ok(bin)
}

/// Cache dir for the standalone view-toolchain esbuild, keyed by version + host
/// target so a bump (or a different machine) never reuses the wrong binary.
fn esbuild_dir(os: &str, arch: &str) -> anyhow::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "human-interface", "hi-agent")
        .ok_or_else(|| anyhow!("cannot determine OS cache dir"))?;
    Ok(dirs
        .cache_dir()
        .join("view-tool")
        .join(format!("esbuild-{ESBUILD_VERSION}-{os}-{arch}")))
}

/// The directory the managed runtime installs into. Override with
/// `HI_AGENT_RUNTIME_DIR` (used verbatim — a dev escape hatch); otherwise a
/// fingerprinted subdir under the OS cache dir, so a changed pin installs fresh
/// instead of reusing a stale tree.
fn runtime_dir() -> anyhow::Result<PathBuf> {
    if let Ok(dir) = std::env::var("HI_AGENT_RUNTIME_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let dirs = directories::ProjectDirs::from("dev", "human-interface", "hi-agent")
        .ok_or_else(|| anyhow!("cannot determine OS cache dir"))?;
    Ok(dirs.cache_dir().join("runtime").join(runtime_fingerprint()))
}

/// A short, stable fingerprint of everything that determines the installed tree: the
/// two pinned versions. FNV-1a, not a crypto hash — this is a cache key for de-duping
/// installs, not a security boundary — but deterministic across builds and platforms so
/// the same pins always resolve to the same dir.
///
/// (Under npm this had to hash the whole lockfile, because a transitive dep could
/// change the installed tree without either direct version moving. Fetching two exact
/// tarballs leaves no transitive deps to capture.)
fn runtime_fingerprint() -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    // NUL between the parts so a byte can't migrate across the boundary and
    // collide a different (codex, esbuild) split.
    for part in [CODEX_VERSION.as_bytes(), b"\0", ESBUILD_VERSION.as_bytes()] {
        for &b in part {
            h ^= b as u64;
            h = h.wrapping_mul(PRIME);
        }
    }
    format!("{h:016x}")
}

/// Provision a complete managed runtime into `dir` — used at package time to
/// populate a `.app`'s `Contents/Resources/runtime`. Reuses the same fingerprinted
/// cache as [`ensure`]'s auto-install tier, so a repeat `make dmg` (or a prior
/// `make dev`) downloads nothing; only a cold cache pays the download. Unlike
/// [`ensure`] it never short-circuits to a system runtime on `PATH` — the packaging
/// host (a dev Mac with codex installed) would otherwise stage nothing. The bundle
/// gets a *copy* of the cache, not a link: `make dmg` codesigns the bundle's Mach-O in
/// place, which must never reach back into the shared cache.
pub async fn provision_into(dir: &Path) -> anyhow::Result<()> {
    let cache = runtime_dir()?;
    if cache.join(".complete").exists() {
        tracing::debug!(path = %cache.display(), "reusing cached runtime for the bundle");
    } else {
        install(&cache).await?;
    }
    copy_tree(&cache, dir)
        .await
        .with_context(|| format!("copying the cached runtime into {}", dir.display()))?;
    if !dir.join(".complete").exists() {
        bail!(
            "runtime copied to {} but its .complete marker is missing",
            dir.display()
        );
    }
    Ok(())
}

/// Recursively copy `src` to `dst` via the system `cp -Rp`, preserving symlinks and
/// the execute bits on the binaries. `dst` is removed first so `cp` creates it as an
/// exact copy rather than nesting `src` inside an existing dir. Used to stamp a
/// bundle's runtime from the shared cache.
async fn copy_tree(src: &Path, dst: &Path) -> anyhow::Result<()> {
    let _ = tokio::fs::remove_dir_all(dst).await;
    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let status = Command::new("cp")
        .arg("-Rp")
        .arg(src)
        .arg(dst)
        .status()
        .await
        .context("running `cp` to copy the cached runtime (is `cp` present?)")?;
    if !status.success() {
        bail!("`cp -Rp {} {}` failed", src.display(), dst.display());
    }
    Ok(())
}

/// Install the pinned codex + esbuild into `target`. Builds in a sibling temp dir
/// and atomically renames into place, so concurrent or interrupted starts never
/// observe a half-installed runtime.
async fn install(target: &Path) -> anyhow::Result<ResolvedRuntime> {
    let (os, arch) = npm_target()?;
    let parent = target
        .parent()
        .ok_or_else(|| anyhow!("runtime dir {} has no parent", target.display()))?;
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;

    let tmp = parent.join(format!(".runtime.tmp.{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    tokio::fs::create_dir_all(&tmp)
        .await
        .with_context(|| format!("creating {}", tmp.display()))?;

    // 1. codex — the agent binary, plus the helper tree it expects beside itself.
    hint(&format!(
        "first run — downloading codex {CODEX_VERSION} (~130 MB)…"
    ));
    fetch_npm_package(&codex_tarball_url(os, arch), &tmp.join("codex"), "codex")
        .await
        .context("installing the codex runtime")?;
    // 2. esbuild — hi-agent's own view compiler.
    hint("first run — downloading the view compiler (esbuild, ~5 MB)…");
    fetch_npm_package(
        &esbuild_tarball_url(os, arch),
        &tmp.join("esbuild"),
        "esbuild",
    )
    .await
    .context("installing the view compiler")?;

    // Fail loudly if the install didn't produce the paths we expect, before we publish
    // anything (a corrupt cache dir is worse than a clear error). `resolve` already
    // errors when the codex binary can't be located under the extracted tree at all.
    let staged = resolve(&tmp)?;
    for (label, path) in [
        ("codex", Some(staged.codex_bin.as_path())),
        ("esbuild", staged.esbuild_bin.as_deref()),
    ] {
        match path {
            Some(p) if is_executable(p) => {}
            Some(p) => bail!(
                "runtime installed but the {label} entry is missing or not executable at {} \
                 (the pinned package layout may have changed)",
                p.display()
            ),
            None => bail!("runtime installed but no {label} path was resolved"),
        }
    }

    tokio::fs::write(tmp.join(".complete"), b"")
        .await
        .context("writing the completion marker")?;

    // Clear any leftover partial install at the fixed path, then atomically
    // rename ours into place. If another process won the race (a complete
    // install already sits there), drop ours and reuse it.
    let _ = tokio::fs::remove_dir_all(target).await;
    match tokio::fs::rename(&tmp, target).await {
        Ok(()) => {}
        Err(_) if target.join(".complete").exists() => {
            let _ = tokio::fs::remove_dir_all(&tmp).await;
        }
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&tmp).await;
            return Err(anyhow!("publishing runtime to {}: {e}", target.display()));
        }
    }

    hint("runtime ready.");
    resolve(target)
}

/// Build absolute paths from an installed (or reused) target dir. Errors when the
/// codex binary can't be found under it, so a corrupt or layout-changed tree reports
/// itself here rather than as a spawn failure much later.
fn resolve(target: &Path) -> anyhow::Result<ResolvedRuntime> {
    let codex_dir = target.join("codex");
    let codex_bin = codex_bin_in(&codex_dir).ok_or_else(|| {
        anyhow!(
            "no codex binary under {} (expected vendor/<target>/bin/codex — \
             the pinned package layout may have changed)",
            codex_dir.display()
        )
    })?;
    Ok(ResolvedRuntime {
        codex_bin,
        esbuild_bin: Some(target.join("esbuild").join(esbuild_rel())),
        origin: "managed",
    })
}

/// The native `codex` inside an extracted platform package.
///
/// The executable lives at `vendor/<rust-target>/bin/codex` — and the rest of that
/// `vendor/<rust-target>/` directory is not incidental: codex ships `rg`, a `zsh`, and
/// (on Linux) `bwrap` beside the binary and finds them by relative path. That is why we
/// extract the whole package rather than cherry-picking one file out of the tarball.
///
/// The vendor directory is named with a Rust target triple (`aarch64-apple-darwin`)
/// while the package it came from is named the npm way (`darwin-arm64`), so the binary
/// is found by search rather than by a mapping table we would have to keep correct for
/// six platforms. The tarball is this host's by construction, so the search has exactly
/// one candidate.
fn codex_bin_in(codex_dir: &Path) -> Option<PathBuf> {
    let exe = if cfg!(target_os = "windows") {
        "codex.exe"
    } else {
        "codex"
    };
    let targets = std::fs::read_dir(codex_dir.join("vendor")).ok()?;
    for target in targets.flatten() {
        let candidate = target.path().join("bin").join(exe);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Same as [`resolve`] but stamped `origin = "bundled"` — the layout of a
/// provisioned `.app` runtime is identical to a managed-cache one (it was built
/// by the same [`install`]), so only the origin label differs (for logging).
fn resolve_bundled(target: &Path) -> anyhow::Result<ResolvedRuntime> {
    let mut r = resolve(target)?;
    r.origin = "bundled";
    Ok(r)
}

/// Registry URL for the `codex` platform tarball.
///
/// The per-platform packages are npm *aliases*: `@openai/codex-darwin-arm64` resolves
/// to `npm:@openai/codex@<version>-darwin-arm64` — the same package name under a
/// platform-suffixed *version*, not a package of its own. So the host tokens belong in
/// the version, and there is one package to know about rather than six.
fn codex_tarball_url(os: &str, arch: &str) -> String {
    format!("{NPM_REGISTRY}/@openai/codex/-/codex-{CODEX_VERSION}-{os}-{arch}.tgz")
}

/// Registry URL for the `esbuild` platform tarball. Unlike codex's, these are genuinely
/// separate scoped packages (`@esbuild/darwin-arm64`) at the plain version — and a
/// scoped package's tarball path drops the scope.
fn esbuild_tarball_url(os: &str, arch: &str) -> String {
    format!("{NPM_REGISTRY}/@esbuild/{os}-{arch}/-/{os}-{arch}-{ESBUILD_VERSION}.tgz")
}

/// Download an npm package tarball and extract it into `dir`.
///
/// An npm tarball is a gzipped tar whose every entry sits under a single `package/`
/// root, so `--strip-components=1` lands the package's own contents directly in `dir`.
/// Extraction goes through the system `tar`, which handles symlinks, hardlinks and the
/// execute bits correctly and exists on all three supported platforms (`-xf` without
/// `z` lets it auto-detect the gzip, matching how the browser install invokes it).
async fn fetch_npm_package(url: &str, dir: &Path, label: &str) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(dir)
        .await
        .with_context(|| format!("creating {}", dir.display()))?;

    tracing::debug!(%url, %label, "downloading npm package tarball");
    let client = crate::net::http_client();
    let bytes = with_heartbeat(
        crate::net::with_retries(label, || fetch_url_bytes(&client, url)),
        &format!("…still downloading {label}"),
    )
    .await?;

    // Beside the destination rather than inside it, so a failed extraction can't leave
    // the archive behind in a tree we go on to publish. Built by *appending* `.tgz`
    // rather than `with_extension`, which would treat the pid in `.esbuild.tmp.<pid>`
    // as an extension and replace it — collapsing two concurrent installs onto one
    // archive path.
    let archive = {
        let name = dir
            .file_name()
            .ok_or_else(|| anyhow!("{} has no file name to derive an archive from", dir.display()))?;
        let mut name = name.to_os_string();
        name.push(".tgz");
        dir.with_file_name(name)
    };
    tokio::fs::write(&archive, &bytes)
        .await
        .with_context(|| format!("writing {}", archive.display()))?;

    let status = Command::new("tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(dir)
        .arg("--strip-components=1")
        .status()
        .await
        .with_context(|| format!("running `tar` to extract {label} (is `tar` installed?)"))?;
    let _ = tokio::fs::remove_file(&archive).await;
    if !status.success() {
        bail!("`tar` failed to extract the {label} package from {url}");
    }
    Ok(())
}

/// GET `url` and buffer the whole body through the shared timeout client, so a
/// connect failure or a mid-stream stall surfaces as an error (lettable
/// [`crate::net::with_retries`] re-attempt) instead of hanging.
pub(crate) async fn fetch_url_bytes(
    client: &reqwest::Client,
    url: &str,
) -> anyhow::Result<bytes::Bytes> {
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("requesting {url}"))?
        .error_for_status()
        .with_context(|| format!("downloading from {url}"))?;
    resp.bytes()
        .await
        .with_context(|| format!("reading the download body from {url}"))
}

/// Await `fut`, printing `slow_hint` every 15s so a long download doesn't look hung.
async fn with_heartbeat<T>(fut: impl Future<Output = T>, slow_hint: &str) -> T {
    tokio::pin!(fut);
    let mut ticker = tokio::time::interval(Duration::from_secs(15));
    ticker.tick().await; // consume the immediate first tick
    loop {
        tokio::select! {
            res = &mut fut => return res,
            _ = ticker.tick() => hint(slow_hint),
        }
    }
}

/// Map the host to the **npm platform token** pair used by the packages we fetch —
/// `@esbuild/<os>-<arch>` and codex's `<version>-<os>-<arch>`. `Err` on platforms we
/// don't auto-install.
pub(crate) fn npm_target() -> anyhow::Result<(&'static str, &'static str)> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        "windows" => "win32",
        other => bail!(
            "runtime auto-install supports macOS, Linux, and Windows only (OS `{other}`). \
             Install codex on your PATH to use the system runtime instead."
        ),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => bail!(
            "runtime auto-install supports x86_64 and aarch64 only (arch `{other}`). \
             Install codex on your PATH to use the system runtime instead."
        ),
    };
    Ok((os, arch))
}

/// Path of the esbuild native binary within an extracted `@esbuild/<plat>` package. On
/// Windows the package ships `esbuild.exe` at its root; on unix the binary lives under
/// `bin/esbuild`.
pub(crate) fn esbuild_rel() -> PathBuf {
    if cfg!(target_os = "windows") {
        PathBuf::from("esbuild.exe")
    } else {
        PathBuf::from("bin").join("esbuild")
    }
}

/// True if `p` is a regular file that can be spawned. On unix that means a file
/// with any execute bit set; Windows has no execute bit, so existence as a regular
/// file is the test.
#[cfg(unix)]
pub(crate) fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(p) {
        Ok(m) => m.is_file() && (m.permissions().mode() & 0o111 != 0),
        Err(_) => false,
    }
}

#[cfg(not(unix))]
pub(crate) fn is_executable(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_file()).unwrap_or(false)
}

/// Use the system's `codex` when it offers a pin-matching one.
///
/// This used to require node *and* codex, all-or-nothing, so a system tool was never
/// paired with a managed one. With node gone there is only one tool left to find, and
/// esbuild is provisioned independently either way.
fn resolve_system() -> Option<ResolvedRuntime> {
    let codex_bin = resolve_codex_bin()?;

    // A `codex` on `PATH` whose version differs from our pin is a trap: the app-server
    // protocol is still moving (0.144 rejects the camelCase sandbox spelling that a
    // later build's docs show), so an unpinned global install can fail in ways that look
    // like our bug. On a definite mismatch, skip the system runtime and fall through to
    // the managed install of the pinned version. A version we can't read is accepted —
    // don't break unusual-but-valid setups.
    if let Some(found) = codex_version_on_path(&codex_bin)
        && found != CODEX_VERSION
    {
        tracing::warn!(
            found = %found,
            pinned = %CODEX_VERSION,
            codex = %codex_bin.display(),
            "ignoring PATH codex: version != pinned; using the managed runtime instead",
        );
        return None;
    }

    tracing::debug!(codex = %codex_bin.display(), "using system runtime from PATH");
    Some(ResolvedRuntime {
        codex_bin,
        esbuild_bin: None,
        origin: "system",
    })
}

/// Best-effort `codex --version`, reduced to the bare version string.
///
/// The CLI prints `codex-cli 0.144.1`. Returns `None` if it can't be run or parsed, so
/// the caller treats "unknown" as "don't reject". Blocking on purpose — this runs once
/// during startup resolution, before the async runtime has any work to do.
fn codex_version_on_path(codex_bin: &Path) -> Option<String> {
    let out = std::process::Command::new(codex_bin)
        .arg("--version")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace().last().map(str::to_string)
}

/// Find an executable named `name` on `PATH`, returning the first match. On
/// Windows, where executables carry an extension, each `PATHEXT` suffix is also
/// tried, so a bare `codex` finds `codex.cmd`.
pub(crate) fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let bare = dir.join(name);
        if is_executable(&bare) {
            return Some(bare);
        }
        #[cfg(target_os = "windows")]
        {
            let exts =
                std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string());
            for ext in exts.split(';').map(str::trim).filter(|e| !e.is_empty()) {
                let cand = dir.join(format!("{name}{ext}"));
                if is_executable(&cand) {
                    return Some(cand);
                }
            }
        }
    }
    None
}

/// Locate a *usable* `codex` CLI, resisting launcher shims.
///
/// Priority: `HI_AGENT_CODEX_BIN` override → first non-shim `codex` on PATH →
/// canonical install locations (in case the launcher's PATH omits them). A "shim" is
/// any candidate inside a macOS `.app` bundle — the pattern used by GUI launchers like
/// cmux, whose binary only works in their own auth sandbox. Returns `None` when only a
/// shim exists, so the caller falls back to the managed runtime instead of a `codex`
/// that will fail at prompt time.
///
/// (This resistance was learnt the hard way with `claude` under ACP, and it transfers
/// unchanged: the trap is the launcher, not the vendor.)
fn resolve_codex_bin() -> Option<PathBuf> {
    if let Some(raw) = std::env::var_os("HI_AGENT_CODEX_BIN") {
        let p = PathBuf::from(raw);
        if is_executable(&p) {
            return Some(p);
        }
        tracing::warn!(path = %p.display(), "HI_AGENT_CODEX_BIN is not executable; ignoring");
    }

    let mut shim: Option<PathBuf> = None;
    if let Some(path) = std::env::var_os("PATH") {
        for cand in std::env::split_paths(&path).map(|dir| dir.join("codex")) {
            if !is_executable(&cand) {
                continue;
            }
            if is_app_bundle_path(&cand) {
                shim.get_or_insert(cand); // remember, but keep looking for a real one
                continue;
            }
            return Some(cand);
        }
    }

    // PATH yielded only a shim (or nothing). Try canonical install locations the
    // launcher's PATH may have dropped, before giving up.
    for cand in canonical_codex_paths() {
        if is_executable(&cand) && !is_app_bundle_path(&cand) {
            return Some(cand);
        }
    }

    if let Some(shim) = &shim {
        tracing::warn!(
            path = %shim.display(),
            "the only `codex` on PATH is an app-bundle shim (e.g. a GUI launcher's); \
             falling back to the managed runtime — set HI_AGENT_CODEX_BIN to override",
        );
    }
    None
}

/// Standard places the official installer / package managers put `codex`.
fn canonical_codex_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        out.push(PathBuf::from(&home).join(".local/bin/codex"));
    }
    out.push(PathBuf::from("/opt/homebrew/bin/codex"));
    out.push(PathBuf::from("/usr/local/bin/codex"));
    out
}

/// True if any path component is a macOS application bundle (`*.app`).
fn is_app_bundle_path(p: &Path) -> bool {
    p.components()
        .any(|c| c.as_os_str().to_string_lossy().ends_with(".app"))
}

/// First-run user-facing hint. Goes straight to stderr (not `tracing`) so it is
/// visible regardless of `RUST_LOG`.
pub(crate) fn hint(msg: &str) {
    eprintln!("hi-agent: {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npm_target_maps_known_hosts() {
        // Whatever host runs the test must be a supported target.
        let (os, arch) = npm_target().expect("test host should be a supported target");
        assert!(matches!(os, "darwin" | "linux" | "win32"));
        assert!(matches!(arch, "x64" | "arm64"));
    }

    /// codex's platform packages are npm *aliases* onto a platform-suffixed version of
    /// the single `@openai/codex` package, so the host tokens land in the version, not
    /// in the package name; esbuild's are ordinary scoped packages, so they land in the
    /// name. Confusing those two shapes is the whole risk of fetching tarballs
    /// ourselves, so pin both spellings.
    #[test]
    fn tarball_urls_match_the_registry_layout() {
        assert_eq!(
            codex_tarball_url("darwin", "arm64"),
            format!(
                "https://registry.npmjs.org/@openai/codex/-/codex-{CODEX_VERSION}-darwin-arm64.tgz"
            )
        );
        assert_eq!(
            esbuild_tarball_url("linux", "x64"),
            format!(
                "https://registry.npmjs.org/@esbuild/linux-x64/-/linux-x64-{ESBUILD_VERSION}.tgz"
            )
        );
    }

    #[test]
    fn esbuild_rel_uses_host_binary_layout() {
        // esbuild.exe at the package root on Windows, bin/esbuild on unix.
        #[cfg(not(target_os = "windows"))]
        assert!(esbuild_rel().ends_with("bin/esbuild"));
        #[cfg(target_os = "windows")]
        assert!(esbuild_rel().ends_with("esbuild.exe"));
    }

    /// The binary sits under a directory named with a Rust target triple, inside a
    /// package named the npm way — which is exactly why it is found by search instead
    /// of a per-platform mapping table.
    #[test]
    fn codex_path_finds_the_native_binary_under_vendor() {
        let dir = tempfile::tempdir().unwrap();
        let codex_dir = dir.path().join("codex");
        let exe = if cfg!(target_os = "windows") {
            "codex.exe"
        } else {
            "codex"
        };
        let vendor = codex_dir.join("vendor/aarch64-apple-darwin/bin");
        std::fs::create_dir_all(&vendor).unwrap();
        std::fs::write(vendor.join(exe), b"").unwrap();

        assert_eq!(codex_bin_in(&codex_dir), Some(vendor.join(exe)));
    }

    /// With no extracted tree to search there is no plausible-looking path to invent,
    /// so the answer is `None` and `resolve` turns it into a message naming the dir.
    #[test]
    fn codex_path_is_none_when_the_tree_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(codex_bin_in(&dir.path().join("codex")), None);
        let err = resolve(dir.path()).unwrap_err().to_string();
        assert!(err.contains("no codex binary under"), "{err}");
    }

    #[test]
    fn resolve_builds_expected_paths() {
        let dir = tempfile::tempdir().unwrap();
        let exe = if cfg!(target_os = "windows") {
            "codex.exe"
        } else {
            "codex"
        };
        let vendor = dir.path().join("codex/vendor/x86_64-unknown-linux-musl/bin");
        std::fs::create_dir_all(&vendor).unwrap();
        std::fs::write(vendor.join(exe), b"").unwrap();

        let r = resolve(dir.path()).unwrap();
        assert_eq!(r.origin, "managed");
        assert_eq!(r.codex_bin, vendor.join(exe));
        assert_eq!(
            r.esbuild_bin,
            Some(dir.path().join("esbuild").join(esbuild_rel()))
        );
    }

    #[test]
    fn runtime_fingerprint_is_stable_and_hex() {
        let fp = runtime_fingerprint();
        assert_eq!(fp.len(), 16);
        assert!(fp.bytes().all(|b| b.is_ascii_hexdigit()));
        // Deterministic: same pins, same fingerprint within a run.
        assert_eq!(fp, runtime_fingerprint());
    }

    #[test]
    fn fingerprint_reacts_to_version_changes() {
        // Same hash construction as `runtime_fingerprint`, exercised over two
        // inputs to prove a changed pin yields a different dir.
        fn fp(codex: &str, esbuild: &str) -> String {
            const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
            const PRIME: u64 = 0x0000_0100_0000_01b3;
            let mut h = OFFSET;
            for part in [codex.as_bytes(), b"\0", esbuild.as_bytes()] {
                for &b in part {
                    h ^= b as u64;
                    h = h.wrapping_mul(PRIME);
                }
            }
            format!("{h:016x}")
        }
        assert_ne!(fp("0.144.1", "0.28.1"), fp("0.146.0", "0.28.1"));
        assert_ne!(fp("0.144.1", "0.28.1"), fp("0.144.1", "0.29.0"));
        // The NUL boundary makes the split unambiguous.
        assert_ne!(fp("ab", "c"), fp("a", "bc"));
    }

    #[test]
    fn app_bundle_paths_are_recognized_as_shims() {
        assert!(is_app_bundle_path(Path::new(
            "/Applications/cmux.app/Contents/Resources/bin/codex"
        )));
        assert!(!is_app_bundle_path(Path::new("/Users/me/.local/bin/codex")));
        assert!(!is_app_bundle_path(Path::new("/opt/homebrew/bin/codex")));
    }

    #[test]
    fn find_on_path_locates_a_known_executable() {
        // `tar` is required by the install path, so it's a safe always-present
        // executable to probe for on any supported host.
        assert!(find_on_path("tar").is_some());
        assert!(find_on_path("definitely-not-a-real-binary-xyz").is_none());
    }
}
