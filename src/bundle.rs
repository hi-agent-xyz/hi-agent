//! Locating bundled dependencies inside a packaged install.
//!
//! A shipped app carries its runtime, recognition models, `ffmpeg` and the
//! headless browser in one resources directory so it runs with **no first-run
//! downloads**. The provisioners ([`crate::runtime`],
//! [`crate::foundation::models`], [`crate::foundation::vendors::ffmpeg`])
//! consult [`resources_dir`] as their *first* resolution tier, above the
//! system-PATH / download tiers they already have. A bare dev binary or a
//! Linux/Docker process simply gets `None` and falls through to the existing
//! behavior unchanged — so this is purely additive.
//!
//! Two installers, two layouts, one directory name:
//!
//! - **macOS** — `Foo.app/Contents/Resources`, derived from a binary living at
//!   `Foo.app/Contents/MacOS/<bin>`.
//! - **Windows** — `resources` beside the executable, next to `hi-agent.exe`.
//!
//! **The Windows half is the read side only.** Nothing stages that tree for a
//! Windows target yet: `--provision-into` provisions the platform it is *running
//! on*, so packaging on the Mac mini still produces a Mac tree. Until that is
//! built, a Windows install finds no bundle here and falls through to the
//! `PATH`/download tiers exactly as it does today — this rule changes nothing
//! until something writes the directory it looks for.
//!
//! Either way the directory is populated at package time by the binary
//! provisioning itself (the hidden `--provision-into` flag), so what the shipped
//! app reads is byte-for-byte what the managed install would have downloaded.

use std::path::{Path, PathBuf};

/// Point the bundle resolution at an explicit `Contents/Resources`-shaped dir,
/// bypassing the `.app` layout detection. A packaging/testing escape hatch — it
/// lets a bare binary be exercised against a staged bundle without building an
/// actual `.app`.
const ENV_BUNDLE_DIR: &str = "HI_AGENT_BUNDLE_DIR";

/// The packaged resources directory, or `None` when not running from a packaged
/// install (dev binary, Linux, Docker).
///
/// Resolution: `HI_AGENT_BUNDLE_DIR` if it names an existing dir, else derive
/// from the executable path — the macOS `.app` layout, else a `resources`
/// directory beside the binary. The exe path is canonicalized first so a
/// symlinked launch (Finder, a wrapper symlink) still resolves the real install.
/// Returns `Some` only when the derived dir actually exists.
pub fn resources_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(ENV_BUNDLE_DIR) {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    [resources_from_exe(&exe), resources_beside_exe(&exe)]
        .into_iter()
        .flatten()
        .find(|candidate| candidate.is_dir())
}

/// Pure path logic for the macOS `.app` layout, factored out so it is unit-
/// testable without a real executable or filesystem: given
/// `…/Foo.app/Contents/MacOS/<bin>`, return `…/Foo.app/Contents/Resources`. The
/// existence check is the caller's job ([`resources_dir`]).
fn resources_from_exe(exe: &Path) -> Option<PathBuf> {
    let macos_dir = exe.parent()?; // …/Contents/MacOS
    if macos_dir.file_name()? != "MacOS" {
        return None;
    }
    let contents = macos_dir.parent()?; // …/Contents
    if contents.file_name()? != "Contents" {
        return None;
    }
    Some(contents.join("Resources"))
}

/// Pure path logic for the layout the Windows installer produces: the
/// executable directory holds a `resources` subdirectory beside it
/// (`…\Hi Agent\app\hi-agent.exe` → `…\Hi Agent\app\resources`).
///
/// Returns a candidate, and unlike the `.app` rule there is no shape to match
/// on — any directory may hold a `resources` subdirectory — so existence is
/// what decides, in [`resources_dir`].
fn resources_beside_exe(exe: &Path) -> Option<PathBuf> {
    Some(exe.parent()?.join("resources"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_resources_from_app_layout() {
        let exe = Path::new("/Applications/HiAgent.app/Contents/MacOS/hi-agent");
        assert_eq!(
            resources_from_exe(exe),
            Some(PathBuf::from("/Applications/HiAgent.app/Contents/Resources"))
        );
    }

    #[test]
    fn off_bundle_paths_yield_none() {
        // A bare dev binary or a /usr/local/bin install is not in a `.app`.
        assert_eq!(resources_from_exe(Path::new("/home/me/hi-agent/target/release/hi-agent")), None);
        assert_eq!(resources_from_exe(Path::new("/usr/local/bin/hi-agent")), None);
        // The right leaf name but the wrong enclosing dir must not match.
        assert_eq!(resources_from_exe(Path::new("/opt/MacOS/hi-agent")), None);
    }

    #[test]
    fn finds_resources_beside_the_binary() {
        // The Windows install layout: `resources` sits beside the executable, not
        // one level up and to the side the way a `.app`'s does.
        let exe = Path::new("/Users/me/AppData/Local/Programs/Hi Agent/app/hi-agent.exe");
        assert_eq!(
            resources_beside_exe(exe),
            Some(PathBuf::from("/Users/me/AppData/Local/Programs/Hi Agent/app/resources"))
        );
    }
}
