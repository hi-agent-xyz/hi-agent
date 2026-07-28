//! Headless-browser resolution for the view render pipeline.
//!
//! Same shape as the rest of the toolchain in [`super`]: **prefer what the system
//! already offers** — a Chrome, Chromium or Edge the user already has — and only
//! fall back to downloading a pinned build into the OS cache when the system has
//! none. Rendering a view is the one thing hi-agent cannot do with a shell
//! command, so this is the third managed tool after Node and esbuild.
//!
//! **Lazy on purpose.** Nothing here runs at startup; [`ensure`] is called on the
//! first render, so a user who never reviews a view never pays ~100 MB.
//!
//! The pin lives in `src/runtime/manifest.toml` beside the Node and adapter pins
//! and is stamped in by `build.rs`. The install dir is content-addressed by
//! version + host platform (like [`super::ensure_view_esbuild`]'s standalone
//! esbuild), so a bumped pin installs fresh rather than reusing a stale tree.
//!
//! We fetch **`chrome-headless-shell`**, not full Chrome: it is the purpose-built
//! headless binary (roughly a third the unpacked size of Chrome for Testing, one
//! executable rather than an `.app` with helper processes), it never wants a
//! window server, and on macOS it needs no TCC grant — so this capability is
//! exercisable over SSH, unlike screencast or input synthesis. A *system* browser
//! found on PATH is normally a full Chrome, so [`ResolvedBrowser::headless_shell`]
//! records which we got: full Chrome must be told `--headless`, the shell must
//! not be (it rejects the flag).
//!
//! **Linux caveat.** The desktop shapes are self-sufficient; a slim Debian image
//! is not. It needs `unzip` to extract the archive at all — Chrome for Testing
//! publishes only `.zip`, and GNU tar cannot read one (the bsdtar macOS and
//! Windows ship *as* `tar` can), so the Dockerfile installs it — and it needs
//! Chromium's own shared libraries (`libnss3`, `libexpat1`, `libfontconfig1`,
//! plus fonts) for the browser to actually start. Where those are missing, the
//! launch error quotes the browser's stderr; installing a system Chromium is
//! usually the simpler fix than a managed download.

use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};
use tokio::process::Command;

use super::{find_on_path, hint, is_executable};

/// Pinned Chrome-for-Testing version, stamped from `src/runtime/manifest.toml`.
pub const CHROME_VERSION: &str = env!("HI_AGENT_CHROME_VERSION");

/// A browser we can drive over the DevTools protocol.
#[derive(Debug, Clone)]
pub struct ResolvedBrowser {
    /// The executable to spawn.
    pub bin: PathBuf,
    /// Where it came from — `"system"` (already on the machine) or `"managed"`
    /// (downloaded into the OS cache). For logging only.
    pub origin: &'static str,
    /// True for a `chrome-headless-shell` build, which is headless by
    /// construction and **rejects** `--headless`. A full Chrome/Chromium/Edge
    /// needs the flag; the render vendor branches on this.
    pub headless_shell: bool,
}

/// Resolve a headless browser: an explicit override, else a system
/// Chrome/Chromium/Edge, else the pinned managed build (installed on first use
/// and reused after).
pub async fn ensure() -> anyhow::Result<ResolvedBrowser> {
    if let Some(bin) = env_override() {
        return Ok(describe(bin, "system"));
    }
    // A shipped `.app` carries its own co-signed browser; prefer it over whatever
    // Chrome the user happens to have, exactly as [`super::ensure`] prefers the
    // bundled runtime over a PATH one.
    if let Some(bin) = bundled_bin() {
        tracing::debug!(path = %bin.display(), "using the bundled headless browser");
        return Ok(describe(bin, "bundled"));
    }
    ensure_in(&browser_dir()?, resolve_system()).await
}

/// A browser that is **already** on this machine — the override, the bundle, the
/// system, or a managed install a previous run left in the cache — without
/// downloading anything.
///
/// This is the honest answer to "can we render right now, for free?". Tests use
/// it to skip rather than pull ~100 MB, and a caller that wants to avoid a
/// surprise first-run download can offer the provisioning explicitly instead.
pub fn available() -> Option<ResolvedBrowser> {
    if let Some(bin) = env_override() {
        return Some(describe(bin, "system"));
    }
    if let Some(bin) = bundled_bin() {
        return Some(describe(bin, "bundled"));
    }
    if let Some(bin) = resolve_system() {
        return Some(describe(bin, "system"));
    }
    let bin = browser_dir().ok()?.join(executable_rel().ok()?);
    is_executable(&bin).then(|| describe(bin, "managed"))
}

/// The browser staged inside a packaged `.app`
/// (`Contents/Resources/browser/…`), or `None` when not running from a bundle or
/// it isn't present. Mirrors [`crate::foundation::vendors::ffmpeg::bundled_bin`].
fn bundled_bin() -> Option<PathBuf> {
    let rel = executable_rel().ok()?;
    let p = crate::bundle::resources_dir()?.join("browser").join(rel);
    p.is_file().then_some(p)
}

/// The tier ladder with its two environment lookups hoisted out, so the
/// system-then-managed decision is exercisable without a PATH, an env var, or a
/// ~100 MB download. `preferred` is whatever the machine already offers (the
/// override or a found system browser); `managed_dir` is the content-addressed
/// cache dir this host's pin maps to.
async fn ensure_in(
    managed_dir: &Path,
    preferred: Option<PathBuf>,
) -> anyhow::Result<ResolvedBrowser> {
    if let Some(bin) = preferred {
        tracing::debug!(path = %bin.display(), "using system browser for view rendering");
        return Ok(describe(bin, "system"));
    }

    let bin = managed_dir.join(executable_rel()?);
    if bin.exists() {
        tracing::debug!(path = %bin.display(), "managed headless browser already installed");
        return Ok(describe(bin, "managed"));
    }
    let bin = install(managed_dir).await?;
    Ok(describe(bin, "managed"))
}

/// Stage a managed headless browser into `dir` — used at package time to populate
/// a `.app`'s `Contents/Resources/browser`. Unlike [`ensure`] it never
/// short-circuits to a system browser: the packaging host is a dev Mac with
/// Chrome installed, which would otherwise stage nothing.
pub async fn provision_into(dir: &Path) -> anyhow::Result<()> {
    let cache = browser_dir()?;
    let rel = executable_rel()?;
    if !cache.join(&rel).exists() {
        install(&cache).await?;
    }
    copy_tree(&cache, dir)
        .await
        .with_context(|| format!("copying the cached browser into {}", dir.display()))?;
    if !dir.join(&rel).exists() {
        bail!(
            "browser copied to {} but its executable is missing at {}",
            dir.display(),
            dir.join(&rel).display()
        );
    }
    Ok(())
}

/// Build a [`ResolvedBrowser`], classifying the binary by name.
fn describe(bin: PathBuf, origin: &'static str) -> ResolvedBrowser {
    let headless_shell = is_headless_shell(&bin);
    ResolvedBrowser { bin, origin, headless_shell }
}

/// True when the binary is a `chrome-headless-shell` build (which must not be
/// passed `--headless`). Matches on the file name only — the containing dir of a
/// managed install is also called `chrome-headless-shell-<version>-<platform>`,
/// and a full Chrome placed inside it would still be a full Chrome.
fn is_headless_shell(bin: &Path) -> bool {
    bin.file_name()
        .map(|n| n.to_string_lossy().contains("headless-shell"))
        .unwrap_or(false)
}

/// `HI_AGENT_BROWSER_BIN` — point hi-agent at a specific browser executable.
fn env_override() -> Option<PathBuf> {
    accept_override(std::env::var_os("HI_AGENT_BROWSER_BIN"))
}

/// Validate an override value. One that isn't executable is warned about and
/// ignored, so a stale override degrades to normal resolution instead of breaking
/// rendering outright. Split out from [`env_override`] so it is testable without
/// mutating the process environment.
fn accept_override(raw: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let p = PathBuf::from(raw?);
    if is_executable(&p) {
        return Some(p);
    }
    tracing::warn!(path = %p.display(), "HI_AGENT_BROWSER_BIN is not executable; ignoring");
    None
}

/// Names to look for on `PATH`, most-preferred first. Chromium-family only: the
/// render vendor speaks the DevTools protocol, which Firefox/Safari do not.
const PATH_NAMES: &[&str] = &[
    "chrome-headless-shell",
    "google-chrome-stable",
    "google-chrome",
    "chromium-browser",
    "chromium",
    "microsoft-edge-stable",
    "microsoft-edge",
    "chrome",
];

/// A system Chrome/Chromium/Edge, if one is installed: `PATH` first, then the
/// canonical per-OS install locations a GUI installer uses (which are typically
/// *not* on `PATH` on macOS or Windows).
fn resolve_system() -> Option<PathBuf> {
    for name in PATH_NAMES {
        if let Some(p) = find_on_path(name) {
            return Some(p);
        }
    }
    canonical_browser_paths().into_iter().find(|p| is_executable(p))
}

/// Standard places a GUI installer puts a Chromium-family browser.
fn canonical_browser_paths() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    #[cfg(target_os = "macos")]
    {
        const APPS: &[&str] = &[
            "Google Chrome.app/Contents/MacOS/Google Chrome",
            "Chromium.app/Contents/MacOS/Chromium",
            "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ];
        for app in APPS {
            out.push(PathBuf::from("/Applications").join(app));
            if let Some(home) = std::env::var_os("HOME") {
                out.push(PathBuf::from(&home).join("Applications").join(app));
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        for p in [
            "/usr/bin/google-chrome",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/snap/bin/chromium",
            "/usr/bin/microsoft-edge",
        ] {
            out.push(PathBuf::from(p));
        }
    }
    #[cfg(target_os = "windows")]
    {
        const RELS: &[&str] = &[
            r"Google\Chrome\Application\chrome.exe",
            r"Microsoft\Edge\Application\msedge.exe",
            r"Chromium\Application\chrome.exe",
        ];
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(root) = std::env::var_os(var) {
                for rel in RELS {
                    out.push(PathBuf::from(&root).join(rel));
                }
            }
        }
    }
    out
}

/// Chrome-for-Testing platform token for this host. `Err` on platforms the CfT
/// project doesn't publish for — the caller then has no managed tier and needs a
/// system browser.
pub(crate) fn cft_platform() -> anyhow::Result<&'static str> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "mac-arm64",
        ("macos", "x86_64") => "mac-x64",
        ("linux", "x86_64") => "linux64",
        ("windows", "x86_64") => "win64",
        (os, arch) => bail!(
            "no pinned headless browser is published for {os}-{arch}. Install \
             Chrome, Chromium or Edge (or set HI_AGENT_BROWSER_BIN) to render views."
        ),
    })
}

/// The executable's path *within* an extracted archive. The zip carries one
/// top-level dir named after the platform token.
fn executable_rel() -> anyhow::Result<PathBuf> {
    let platform = cft_platform()?;
    let dir = format!("chrome-headless-shell-{platform}");
    let exe = if cfg!(target_os = "windows") {
        "chrome-headless-shell.exe"
    } else {
        "chrome-headless-shell"
    };
    Ok(PathBuf::from(dir).join(exe))
}

/// Download URL for the pinned build on this host.
fn download_url(platform: &str) -> String {
    format!(
        "https://storage.googleapis.com/chrome-for-testing-public/\
         {CHROME_VERSION}/{platform}/chrome-headless-shell-{platform}.zip"
    )
}

/// Cache dir for the managed browser, keyed by version + platform so a bump never
/// reuses the wrong binary. Override the whole path with `HI_AGENT_BROWSER_DIR`
/// (used verbatim — a dev escape hatch), mirroring `HI_AGENT_RUNTIME_DIR`.
fn browser_dir() -> anyhow::Result<PathBuf> {
    if let Ok(dir) = std::env::var("HI_AGENT_BROWSER_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let platform = cft_platform()?;
    let dirs = directories::ProjectDirs::from("dev", "human-interface", "hi-agent")
        .ok_or_else(|| anyhow!("cannot determine OS cache dir"))?;
    Ok(dirs
        .cache_dir()
        .join("browser")
        .join(format!("chrome-headless-shell-{CHROME_VERSION}-{platform}")))
}

/// Download + extract the pinned build into `target`, returning its executable.
/// Builds in a sibling temp dir and atomically renames into place, so a
/// concurrent or interrupted first render never observes a half-extracted
/// browser — the same publish pattern the runtime and esbuild installs use.
async fn install(target: &Path) -> anyhow::Result<PathBuf> {
    let platform = cft_platform()?;
    let rel = executable_rel()?;

    let parent = target
        .parent()
        .ok_or_else(|| anyhow!("browser dir {} has no parent", target.display()))?;
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;

    let tmp = parent.join(format!(".browser.tmp.{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    tokio::fs::create_dir_all(&tmp)
        .await
        .with_context(|| format!("creating {}", tmp.display()))?;

    let url = download_url(platform);
    hint(&format!(
        "preparing the view renderer (downloading chrome-headless-shell {CHROME_VERSION}, ~100 MB)…"
    ));
    tracing::debug!(%url, "downloading the headless browser");
    let client = crate::net::http_client();
    let fetched = crate::net::with_retries("chrome-headless-shell", || {
        super::fetch_url_bytes(&client, url.as_str())
    })
    .await;
    let bytes = match fetched {
        Ok(b) => b,
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&tmp).await;
            return Err(e);
        }
    };

    let archive = tmp.join("chrome-headless-shell.zip");
    tokio::fs::write(&archive, &bytes)
        .await
        .with_context(|| format!("writing {}", archive.display()))?;

    if let Err(e) = unzip(&archive, &tmp).await {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        return Err(e);
    }
    let _ = tokio::fs::remove_file(&archive).await;

    let staged = tmp.join(&rel);
    make_executable(&staged);
    if !is_executable(&staged) {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        bail!(
            "the headless browser archive extracted but `{}` is missing or not \
             executable (the published layout may have changed)",
            staged.display()
        );
    }

    let _ = tokio::fs::remove_dir_all(target).await;
    match tokio::fs::rename(&tmp, target).await {
        Ok(()) => {}
        // Another process won the race and published a complete install.
        Err(_) if target.join(&rel).exists() => {
            let _ = tokio::fs::remove_dir_all(&tmp).await;
        }
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&tmp).await;
            return Err(anyhow!("publishing the browser to {}: {e}", target.display()));
        }
    }

    let bin = target.join(&rel);
    tracing::info!(path = %bin.display(), "headless browser ready");
    hint("view renderer ready.");
    Ok(bin)
}

/// Extract `archive` into `dir`. Chrome for Testing publishes `.zip` on **every**
/// platform, and unlike the Node tarball there is no single extractor present
/// everywhere: GNU `tar` (Linux) cannot read zip, while macOS/Windows ship bsdtar
/// as `tar` and *can*. So try the extractors in order and report all of them if
/// none is available.
async fn unzip(archive: &Path, dir: &Path) -> anyhow::Result<()> {
    let attempts: [(&str, Vec<&std::ffi::OsStr>); 3] = [
        ("unzip", vec!["-q".as_ref(), "-o".as_ref(), archive.as_ref(), "-d".as_ref(), dir.as_ref()]),
        ("bsdtar", vec!["-xf".as_ref(), archive.as_ref(), "-C".as_ref(), dir.as_ref()]),
        ("tar", vec!["-xf".as_ref(), archive.as_ref(), "-C".as_ref(), dir.as_ref()]),
    ];
    let mut tried = Vec::new();
    for (program, args) in attempts {
        match Command::new(program).args(&args).output().await {
            Ok(out) if out.status.success() => return Ok(()),
            Ok(out) => tried.push(format!(
                "`{program}` failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )),
            Err(e) => tried.push(format!("`{program}` unavailable: {e}")),
        }
    }
    bail!(
        "could not extract {} — no working zip extractor ({})",
        archive.display(),
        tried.join("; ")
    )
}

/// Ensure the extracted binary carries an execute bit. `unzip` normally preserves
/// the mode from the archive, but a restrictive umask or an extractor that drops
/// permissions would otherwise leave a browser we can't spawn.
#[cfg(unix)]
fn make_executable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(p) {
        let mut perms = meta.permissions();
        perms.set_mode(perms.mode() | 0o755);
        let _ = std::fs::set_permissions(p, perms);
    }
}

#[cfg(not(unix))]
fn make_executable(_p: &Path) {}

/// Recursively copy `src` to `dst` via the system `cp -Rp`, preserving symlinks
/// and execute bits — the same helper shape [`super::provision_into`] uses to
/// stamp a bundle's runtime from the shared cache.
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
        .context("running `cp` to copy the cached browser (is `cp` present?)")?;
    if !status.success() {
        bail!("`cp -Rp {} {}` failed", src.display(), dst.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_version_looks_like_a_chrome_version() {
        // Four dotted numbers, e.g. 151.0.7922.47 — the build.rs stamp reaching
        // the binary at all is what this really guards.
        let parts: Vec<&str> = CHROME_VERSION.split('.').collect();
        assert_eq!(parts.len(), 4, "unexpected pin `{CHROME_VERSION}`");
        assert!(parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit())));
    }

    #[test]
    fn platform_token_matches_the_host() {
        let platform = cft_platform().expect("test host should be a published platform");
        assert!(matches!(platform, "mac-arm64" | "mac-x64" | "linux64" | "win64"));
    }

    #[test]
    fn download_url_is_the_headless_shell_archive() {
        let url = download_url("mac-arm64");
        assert!(url.starts_with("https://storage.googleapis.com/chrome-for-testing-public/"));
        assert!(url.contains(CHROME_VERSION));
        assert!(url.ends_with("/mac-arm64/chrome-headless-shell-mac-arm64.zip"));
    }

    #[test]
    fn executable_rel_nests_under_the_platform_dir() {
        let rel = executable_rel().expect("supported host");
        let s = rel.to_string_lossy().replace('\\', "/");
        assert!(s.starts_with("chrome-headless-shell-"), "{s}");
        #[cfg(not(target_os = "windows"))]
        assert!(s.ends_with("/chrome-headless-shell"), "{s}");
        #[cfg(target_os = "windows")]
        assert!(s.ends_with("/chrome-headless-shell.exe"), "{s}");
    }

    #[test]
    fn cache_dir_is_keyed_by_version_and_platform() {
        // Not the env-override path: assert the derived shape.
        if std::env::var_os("HI_AGENT_BROWSER_DIR").is_some() {
            return;
        }
        let dir = browser_dir().expect("supported host");
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.contains(CHROME_VERSION), "{name}");
        assert!(name.contains(cft_platform().unwrap()), "{name}");
        assert!(dir.parent().unwrap().ends_with("browser"));
    }

    #[test]
    fn headless_shell_is_told_apart_from_full_chrome() {
        assert!(is_headless_shell(Path::new(
            "/cache/chrome-headless-shell-151-mac-arm64/chrome-headless-shell"
        )));
        assert!(is_headless_shell(Path::new("/usr/bin/chrome-headless-shell.exe")));
        assert!(!is_headless_shell(Path::new(
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
        )));
        // A full Chrome that happens to sit inside a managed dir is still full
        // Chrome — only the file name decides.
        assert!(!is_headless_shell(Path::new(
            "/cache/chrome-headless-shell-151-linux64/chrome"
        )));
    }

    #[test]
    fn path_candidates_are_chromium_family_only() {
        // The render vendor speaks CDP; a Firefox/Safari entry here would resolve
        // to a browser we cannot drive.
        for name in PATH_NAMES {
            assert!(
                name.contains("chrom") || name.contains("edge"),
                "non-Chromium candidate `{name}`"
            );
        }
    }

    #[test]
    fn an_override_is_accepted_only_when_executable() {
        let exe = find_on_path("tar").expect("`tar` is required on every supported host");
        assert_eq!(accept_override(Some(exe.clone().into())), Some(exe));
        assert!(
            accept_override(Some("/definitely/not/here/chrome".into())).is_none(),
            "a stale override must fall through, not break rendering"
        );
        assert!(accept_override(None).is_none());
    }

    /// A system browser short-circuits the managed tier: no download, no cache
    /// dir consulted, even though the managed dir here is empty.
    #[tokio::test]
    async fn system_browser_wins_over_the_managed_install() {
        let exe = find_on_path("tar").expect("`tar` is required on every supported host");
        let empty = std::env::temp_dir().join(format!("hi-browser-empty-{}", std::process::id()));
        let resolved = ensure_in(&empty, Some(exe.clone()))
            .await
            .expect("a system browser resolves with no download");
        assert_eq!(resolved.bin, exe);
        assert_eq!(resolved.origin, "system");
        assert!(!empty.exists(), "the managed dir must not be touched");
    }

    /// With nothing on the system, an already-extracted managed install is reused
    /// as-is — the tier below, and still no download.
    #[tokio::test]
    async fn managed_install_is_reused_when_the_system_offers_nothing() {
        let dir = std::env::temp_dir().join(format!("hi-browser-managed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rel = executable_rel().expect("supported host");
        let bin = dir.join(&rel);
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, b"#!/bin/sh\n").unwrap();
        make_executable(&bin);

        let resolved = ensure_in(&dir, None).await.expect("the cached install is reused");
        assert_eq!(resolved.bin, bin);
        assert_eq!(resolved.origin, "managed");
        assert!(resolved.headless_shell, "the managed build is a headless shell");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
