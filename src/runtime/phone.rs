//! `adb` resolution for the `phone` tool.
//!
//! Same ladder as [`super::browser`] — **prefer what the machine already has**, fall
//! back to a pinned download into the OS cache — and for the same reason: a note that
//! says `phone` must mean *whatever this machine turned out to have*, and only the
//! machine knows. A dev box has adb on the PATH already; a fresh Linux server has
//! nothing, and 9 MB is a cheaper answer than telling someone to install an SDK.
//!
//! **Lazy, like the browser.** Nothing here runs at boot. The `bin/phone` shim asks
//! `--resolve-phone` on every invocation, so an install that never touches a phone
//! never downloads one.
//!
//! # Why adb and not a wrapper of our own
//!
//! `adb` is already a command line, already documents itself, and Google has kept its
//! surface stable for fifteen years. The reason [`super::browser`] exists as a *shim*
//! is that CDP is a raw WebSocket protocol with no CLI — a gap somebody has to bridge.
//! There is no such gap here, so this module resolves a binary and gets out of the way:
//! `phone --help` is adb's own help, and the argv prefix is one element.
//!
//! # What this deliberately does not reach
//!
//! Android only. Every iOS path that exists today costs the person a tethered device
//! plus either host root (`pymobiledevice3`'s tunnel) or an Apple developer team to
//! sign WebDriverAgent — and neither reaches the phone in someone's pocket. The
//! shipped iOS answer runs the other way: the Action Button hands a screenshot in
//! (`docs/user-journeys/36-show-your-screen-from-a-button.md`). The seeded note says so
//! in prose rather than leaving the command's name to imply otherwise.

use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};

use super::browser::{make_executable, unzip};
use super::{find_on_path, hint, is_executable};

/// Pinned platform-tools release, stamped from `src/runtime/manifest.toml`.
pub const PLATFORM_TOOLS_VERSION: &str = env!("HI_AGENT_PLATFORM_TOOLS_VERSION");

/// An `adb` we can drive a handset with.
#[derive(Debug, Clone)]
pub struct ResolvedAdb {
    /// The executable to spawn.
    pub bin: PathBuf,
    /// Where it came from — `"system"` (already here) or `"managed"` (downloaded into
    /// the OS cache). For logging only.
    pub origin: &'static str,
}

/// The argv prefix `bin/phone` puts in front of whatever the caller passed.
///
/// One element, and that is the point: adb needs no flag normalised away the way a
/// full Chrome needs `--headless` and a headless shell rejects it. This function
/// exists so the shim's contract is *a list of argv elements, one per line* rather
/// than *a path*, which is what lets a future prefix element arrive without rewriting
/// a shim already on disk in every install.
pub fn argv_prefix(adb: &ResolvedAdb) -> Vec<String> {
    vec![adb.bin.display().to_string()]
}

/// Resolve an adb: an explicit override, else one this machine already has, else the
/// pinned managed download (installed on first use and reused after).
pub async fn ensure() -> anyhow::Result<ResolvedAdb> {
    if let Some(bin) = env_override() {
        return Ok(ResolvedAdb { bin, origin: "system" });
    }
    ensure_in(&adb_dir()?, resolve_system()).await
}

/// An adb that is **already** on this machine, without downloading anything.
///
/// The honest answer to "can we reach a phone right now, for free?" — tests use it to
/// skip rather than pull the archive.
pub fn available() -> Option<ResolvedAdb> {
    if let Some(bin) = env_override() {
        return Some(ResolvedAdb { bin, origin: "system" });
    }
    if let Some(bin) = resolve_system() {
        return Some(ResolvedAdb { bin, origin: "system" });
    }
    let bin = adb_dir().ok()?.join(executable_rel().ok()?);
    is_executable(&bin).then(|| ResolvedAdb { bin, origin: "managed" })
}

/// The tier ladder with its environment lookups hoisted out, so system-then-managed is
/// exercisable without a PATH, an env var, or a download. Mirrors the browser's
/// `ensure_in` beside it.
async fn ensure_in(managed_dir: &Path, preferred: Option<PathBuf>) -> anyhow::Result<ResolvedAdb> {
    if let Some(bin) = preferred {
        tracing::debug!(path = %bin.display(), "using system adb");
        return Ok(ResolvedAdb { bin, origin: "system" });
    }
    let bin = managed_dir.join(executable_rel()?);
    if bin.exists() {
        tracing::debug!(path = %bin.display(), "managed adb already installed");
        return Ok(ResolvedAdb { bin, origin: "managed" });
    }
    let bin = install(managed_dir).await?;
    Ok(ResolvedAdb { bin, origin: "managed" })
}

/// `HI_AGENT_ADB_BIN` — point hi-agent at a specific adb.
fn env_override() -> Option<PathBuf> {
    accept_override(std::env::var_os("HI_AGENT_ADB_BIN"))
}

/// Validate an override. One that isn't executable is warned about and ignored, so a
/// stale value degrades to normal resolution instead of breaking the tool outright.
/// Split out so it is testable without mutating the process environment.
fn accept_override(raw: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let p = PathBuf::from(raw?);
    if is_executable(&p) {
        return Some(p);
    }
    tracing::warn!(path = %p.display(), "HI_AGENT_ADB_BIN is not executable; ignoring");
    None
}

/// An adb this machine already has: `PATH` first, then an SDK the environment points
/// at, then the canonical per-OS install locations.
///
/// `ANDROID_HOME`/`ANDROID_SDK_ROOT` are checked because that is how a machine with
/// Android Studio is usually set up *without* adb reaching the PATH — the same export
/// the Mac mini needs before `make android` will run.
fn resolve_system() -> Option<PathBuf> {
    if let Some(p) = find_on_path(adb_exe_name()) {
        return Some(p);
    }
    canonical_adb_paths().into_iter().find(|p| is_executable(p))
}

/// The executable's bare name on this host.
fn adb_exe_name() -> &'static str {
    if cfg!(target_os = "windows") { "adb.exe" } else { "adb" }
}

/// Standard places an SDK install puts adb.
fn canonical_adb_paths() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let exe = adb_exe_name();

    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(root) = std::env::var_os(var) {
            out.push(PathBuf::from(&root).join("platform-tools").join(exe));
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            out.push(PathBuf::from(&home).join("Library/Android/sdk/platform-tools").join(exe));
        }
        for p in ["/opt/homebrew/bin/adb", "/usr/local/bin/adb"] {
            out.push(PathBuf::from(p));
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            out.push(PathBuf::from(&home).join("Android/Sdk/platform-tools").join(exe));
        }
        for p in ["/usr/lib/android-sdk/platform-tools/adb", "/usr/bin/adb", "/usr/local/bin/adb"] {
            out.push(PathBuf::from(p));
        }
    }
    #[cfg(target_os = "windows")]
    {
        for var in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(root) = std::env::var_os(var) {
                out.push(PathBuf::from(&root).join(r"Android\Sdk\platform-tools").join(exe));
            }
        }
    }
    out
}

/// The platform token in Google's archive names for this host.
///
/// **`win`, not `windows`** — the other two are spelled in full and this one is not. A
/// reasonable guess 404s, so the token is verified rather than inferred.
///
/// `Err` on hosts Google publishes no build for, and **`linux-aarch64` is the real one**:
/// there is no arm64 Linux platform-tools at all, so an arm64 server has no managed tier
/// and the message has to say what to do instead of leaving a download to fail.
pub(crate) fn platform_token() -> anyhow::Result<&'static str> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", _) => "darwin",
        ("linux", "x86_64") => "linux",
        ("windows", "x86_64") => "win",
        (os, arch) => bail!(
            "Google publishes no platform-tools build for {os}-{arch}. Install adb from \
             this machine's package manager (`apt install android-tools-adb`, `pacman -S \
             android-tools`) or point HI_AGENT_ADB_BIN at one."
        ),
    })
}

/// The executable's path *within* an extracted archive. The zip carries one top-level
/// `platform-tools/` directory on every platform.
fn executable_rel() -> anyhow::Result<PathBuf> {
    platform_token()?;
    Ok(PathBuf::from("platform-tools").join(adb_exe_name()))
}

/// Download URL for the pinned release on this host.
///
/// **Versioned, never `platform-tools-latest-<os>.zip`.** The manifest's own rule is
/// that the version in it is the whole pin; a `-latest-` URL would make the content of
/// an install depend on the day it first ran, which is the one thing a pin exists to
/// stop.
fn download_url(platform: &str) -> String {
    format!(
        "https://dl.google.com/android/repository/\
         platform-tools_r{PLATFORM_TOOLS_VERSION}-{platform}.zip"
    )
}

/// Cache dir for the managed adb, keyed by version + platform so a bump never reuses
/// the wrong binary. `HI_AGENT_ADB_DIR` overrides the whole path (a dev escape hatch),
/// mirroring `HI_AGENT_BROWSER_DIR`.
fn adb_dir() -> anyhow::Result<PathBuf> {
    if let Ok(dir) = std::env::var("HI_AGENT_ADB_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let platform = platform_token()?;
    let dirs = directories::ProjectDirs::from("dev", "human-interface", "hi-agent")
        .ok_or_else(|| anyhow!("cannot determine OS cache dir"))?;
    Ok(dirs
        .cache_dir()
        .join("phone")
        .join(format!("platform-tools-{PLATFORM_TOOLS_VERSION}-{platform}")))
}

/// Download + extract the pinned release into `target`, returning its adb. Builds in a
/// sibling temp dir and atomically renames into place, so a concurrent or interrupted
/// first call never observes a half-extracted tree — the same publish pattern the
/// runtime, esbuild and browser installs use.
async fn install(target: &Path) -> anyhow::Result<PathBuf> {
    let platform = platform_token()?;
    let rel = executable_rel()?;

    let parent =
        target.parent().ok_or_else(|| anyhow!("adb dir {} has no parent", target.display()))?;
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;

    let tmp = parent.join(format!(".phone.tmp.{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    tokio::fs::create_dir_all(&tmp).await.with_context(|| format!("creating {}", tmp.display()))?;

    let url = download_url(platform);
    hint(&format!(
        "preparing the phone tool (downloading Android platform-tools \
         {PLATFORM_TOOLS_VERSION}, ~9 MB)…"
    ));
    tracing::debug!(%url, "downloading platform-tools");
    let client = crate::net::http_client();
    let fetched = crate::net::with_retries("platform-tools", || {
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

    let archive = tmp.join("platform-tools.zip");
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
            "the platform-tools archive extracted but `{}` is missing or not executable \
             (the published layout may have changed)",
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
            return Err(anyhow!("publishing adb to {}: {e}", target.display()));
        }
    }

    let bin = target.join(&rel);
    tracing::info!(path = %bin.display(), "adb ready");
    hint("phone tool ready.");
    Ok(bin)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one thing about the archive names that cannot be guessed: Windows is `win`
    /// while the other two are spelled in full. Probed against dl.google.com — the
    /// `-windows.zip` spelling returns an HTML 404 page.
    #[test]
    fn the_windows_token_is_win_not_windows() {
        let url = download_url("win");
        assert!(url.ends_with("-win.zip"), "{url}");
        assert!(!url.contains("-windows.zip"), "{url}");
    }

    /// A pin that resolved to `-latest-` would make two installs of the same build
    /// disagree about what adb they have.
    #[test]
    fn the_download_url_is_versioned_not_latest() {
        let url = download_url("linux");
        assert!(url.contains(PLATFORM_TOOLS_VERSION), "{url}");
        assert!(!url.contains("latest"), "{url}");
    }

    /// The archive's single top-level directory, verified against the real download.
    #[test]
    fn the_executable_sits_under_platform_tools() {
        let Ok(rel) = executable_rel() else { return }; // no managed tier on this host
        assert!(rel.starts_with("platform-tools"), "{}", rel.display());
        assert!(rel.file_name().is_some_and(|n| n.to_string_lossy().starts_with("adb")));
    }

    /// A stale override degrades to normal resolution rather than breaking the tool.
    #[test]
    fn a_non_executable_override_is_ignored() {
        assert!(accept_override(Some("/definitely/not/here/adb".into())).is_none());
        assert!(accept_override(None).is_none());
    }

    /// The prefix is the binary and nothing else — the shim adds no interface.
    #[test]
    fn the_prefix_is_just_the_binary() {
        let adb = ResolvedAdb { bin: PathBuf::from("/x/adb"), origin: "system" };
        assert_eq!(argv_prefix(&adb), vec!["/x/adb".to_string()]);
    }

    /// System beats managed, and a managed dir is only consulted when nothing is here.
    #[tokio::test]
    async fn a_system_adb_wins_and_nothing_is_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        let resolved = ensure_in(dir.path(), Some(PathBuf::from("/usr/bin/adb"))).await.unwrap();
        assert_eq!(resolved.origin, "system");
        assert_eq!(resolved.bin, PathBuf::from("/usr/bin/adb"));
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none(), "nothing written");
    }
}
