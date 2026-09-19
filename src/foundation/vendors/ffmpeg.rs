//! Static `ffmpeg` binary provisioning for a packaged install.
//!
//! Unlike the recognition models, `ffmpeg` has never been *managed*: the still-
//! frame/clip helpers ([`super::ffmpeg_frame`]) shell out to whatever `ffmpeg` is
//! on `PATH` (or `FFMPEG_BIN`). That is fine on a dev box or in Docker (apt has
//! it), but a shipped app cannot assume the user installed ffmpeg. So for the
//! hermetic bundle we ship a pinned static `ffmpeg` under `<resources>/ffmpeg/`,
//! provisioned at package time and resolved first at runtime ([`bundled_bin`]).
//!
//! A pin is one immutable per-tag release asset from `eugeneware/ffmpeg-static`,
//! verified by SHA-256 + size exactly like a
//! [`super::super::models::ModelSpec`]. Two hosts have one — the macOS `.app` and
//! the Windows install; every other target keeps using `PATH`. We only ever
//! *decode* (H.264 / HEVC / VP8 / VP9 are native ffmpeg decoders) and encode
//! `mjpeg` stills + `pcm_s16le` clips — all built in — so the stock build covers
//! our use.
//!
//! **Licensing:** these are GPL builds. hi-agent invokes `ffmpeg` as a *separate
//! process* (no linking), so the GPL does not reach our Rust code; to honor the
//! source-availability obligation when *distributing* the binary we carry its
//! upstream `LICENSE` next to it (fetched best-effort by [`provision_into`]) and
//! pin the exact upstream tag below. The two assets carry different license
//! texts — ffmpeg's own `LICENSE` for the Apple build, the GPLv3 text for the
//! Windows one — because two different builders produced them; both are carried
//! verbatim.

use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

/// A pinned static ffmpeg build for one host target: where to fetch it and what a
/// correct copy is (same shape as a model pin — SHA-256 is integrity + identity).
struct FfmpegPin {
    url: &'static str,
    sha256: &'static str,
    size: u64,
    /// Upstream release asset for the build's license text, carried beside the
    /// binary for distribution compliance. Not integrity-pinned (informational).
    license_url: &'static str,
}

/// Upstream release tag we pin (eugeneware/ffmpeg-static). Bump deliberately and
/// re-verify the SHA below.
const RELEASE_TAG: &str = "b6.1.1";

/// The macOS `.app`'s build.
const MACOS_ARM64: FfmpegPin = FfmpegPin {
    url: "https://github.com/eugeneware/ffmpeg-static/releases/download/b6.1.1/ffmpeg-darwin-arm64",
    sha256: "a90e3db6a3fd35f6074b013f948b1aa45b31c6375489d39e572bea3f18336584",
    size: 45_568_216,
    license_url: "https://github.com/eugeneware/ffmpeg-static/releases/download/b6.1.1/darwin-arm64.LICENSE",
};

/// The Windows install's build. `win32-x64` is upstream's spelling and the asset
/// is a bare binary, so there is no `.exe` in the URL to match — the name it
/// takes in the bundle is [`bundled_name`]'s business, not the pin's.
const WINDOWS_X64: FfmpegPin = FfmpegPin {
    url: "https://github.com/eugeneware/ffmpeg-static/releases/download/b6.1.1/ffmpeg-win32-x64",
    sha256: "04e1307997530f9cf2fe35cba2ca7e8875ca91da02f89d6c7243df819c94ad00",
    size: 82_797_568,
    license_url: "https://github.com/eugeneware/ffmpeg-static/releases/download/b6.1.1/win32-x64.LICENSE",
};

/// The pin for the current host, or `None` on a target we ship no static build
/// for — Linux, and Windows on arm64, where upstream publishes no asset at all.
/// Those keep using `PATH`/`FFMPEG_BIN` ffmpeg.
fn pin() -> Option<&'static FfmpegPin> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some(&MACOS_ARM64),
        ("windows", "x86_64") => Some(&WINDOWS_X64),
        _ => None,
    }
}

/// What the static binary is called once provisioned. Upstream ships bare
/// binaries with no extension, so the suffix is ours to add: a Windows install's
/// copy is `ffmpeg.exe` like every other executable on that machine. Read and
/// written through this one function so the provisioner and [`bundled_bin`]
/// cannot disagree about the name.
fn bundled_name() -> &'static str {
    if cfg!(target_os = "windows") { "ffmpeg.exe" } else { "ffmpeg" }
}

/// The bundled static ffmpeg inside a packaged install, or `None` when not
/// running from one or it isn't present. `<resources>/ffmpeg/<bundled_name>`.
pub fn bundled_bin() -> Option<PathBuf> {
    let p = crate::bundle::resources_dir()?.join("ffmpeg").join(bundled_name());
    p.is_file().then_some(p)
}

/// Provision the pinned static ffmpeg into `<dir>/ffmpeg/` (made executable)
/// plus its `LICENSE`, for a packaged install's resources directory. The binary
/// is resolved through a content-addressed cache (keyed by tag + SHA-256), so a
/// repeat `make dmg` reuses it and downloads nothing; the bundle gets a *copy*, so
/// codesigning it in place never touches the shared cache. Verifies size + SHA-256
/// before the cache is trusted. Errors if the host target has no pin (so packaging
/// fails loudly rather than silently shipping an ffmpeg-less app).
pub async fn provision_into(dir: &Path) -> anyhow::Result<()> {
    let pin = pin().ok_or_else(|| {
        anyhow!(
            "no pinned static ffmpeg for {}-{} (tag {RELEASE_TAG}); macOS arm64 and windows x86_64 \
             are the targets wired up",
            std::env::consts::OS,
            std::env::consts::ARCH,
        )
    })?;

    let cached = ensure_cached(pin).await?;

    tokio::fs::create_dir_all(dir)
        .await
        .with_context(|| format!("creating {}", dir.display()))?;
    let bin = dir.join(bundled_name());
    tokio::fs::copy(&cached.bin, &bin)
        .await
        .with_context(|| format!("copying ffmpeg into {}", bin.display()))?;
    make_executable(&bin).await?;
    if let Some(license) = &cached.license {
        let _ = tokio::fs::copy(license, dir.join("LICENSE")).await;
    }

    tracing::info!(path = %bin.display(), "static ffmpeg ready");
    Ok(())
}

/// A static ffmpeg in the content-addressed cache: the verified binary and, when
/// present, its upstream license text (best-effort, so it may be absent).
struct CachedFfmpeg {
    bin: PathBuf,
    license: Option<PathBuf>,
}

/// Ensure the pinned ffmpeg (and, best-effort, its `LICENSE`) exist in a
/// content-addressed cache dir, downloading + verifying only on a miss, and return
/// their paths. Reused across `make dmg` runs so the binary is fetched at most once
/// per pin per machine. Publishes via temp-then-rename so an interrupted run never
/// leaves a partial binary in the cache.
async fn ensure_cached(pin: &FfmpegPin) -> anyhow::Result<CachedFfmpeg> {
    let dir = cache_dir(pin)?;
    tokio::fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("creating {}", dir.display()))?;
    let bin = dir.join("ffmpeg");
    let license = dir.join("LICENSE");

    let fresh = matches!(tokio::fs::metadata(&bin).await, Ok(m) if m.len() == pin.size);
    if !fresh {
        let _ = tokio::fs::remove_file(&bin).await;
        hint(&format!("downloading static ffmpeg {RELEASE_TAG} (~{} MB)…", pin.size / 1_000_000));
        let tmp = dir.join(format!(".ffmpeg.tmp.{}", std::process::id()));
        if let Err(e) = crate::net::with_retries("ffmpeg", || download_verify(pin, &tmp)).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e);
        }
        make_executable(&tmp).await?;
        tokio::fs::rename(&tmp, &bin)
            .await
            .with_context(|| format!("publishing ffmpeg to {}", bin.display()))?;

        // License text beside the binary — best-effort (distribution hygiene, not
        // a gate): a missing/changed asset must not fail the bundle.
        if let Ok(resp) = crate::net::http_client().get(pin.license_url).send().await {
            if let Ok(resp) = resp.error_for_status() {
                if let Ok(bytes) = resp.bytes().await {
                    let _ = tokio::fs::write(&license, &bytes).await;
                }
            }
        }
    }

    Ok(CachedFfmpeg {
        bin,
        license: license.exists().then_some(license),
    })
}

/// Content-addressed cache dir for the pinned static ffmpeg, keyed by the upstream
/// tag + SHA-256 prefix so a pin bump lands in a fresh dir instead of reusing a
/// stale binary. Mirrors the runtime/model/esbuild caches under the OS cache dir.
fn cache_dir(pin: &FfmpegPin) -> anyhow::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "human-interface", "hi-agent")
        .ok_or_else(|| anyhow!("cannot determine OS cache dir"))?;
    Ok(dirs
        .cache_dir()
        .join("ffmpeg")
        .join(format!("{RELEASE_TAG}-{}", &pin.sha256[..16])))
}

/// Stream `pin.url` to `tmp`, hashing as we go; fail if the final length or digest
/// disagrees with the pin so `tmp` is never trusted on a mismatch.
async fn download_verify(pin: &FfmpegPin, tmp: &Path) -> anyhow::Result<()> {
    let resp = crate::net::http_client()
        .get(pin.url)
        .send()
        .await
        .with_context(|| format!("requesting {}", pin.url))?
        .error_for_status()
        .with_context(|| format!("downloading {}", pin.url))?;

    let mut file = tokio::fs::File::create(tmp)
        .await
        .with_context(|| format!("creating {}", tmp.display()))?;
    let mut hasher = Sha256::new();
    let mut len: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading ffmpeg download body")?;
        hasher.update(&chunk);
        len += chunk.len() as u64;
        file.write_all(&chunk).await.context("writing ffmpeg chunk")?;
    }
    file.flush().await.context("flushing ffmpeg file")?;

    if len != pin.size {
        bail!("ffmpeg: downloaded {len} bytes, expected {} (truncated?)", pin.size);
    }
    let digest = hex_lower(&hasher.finalize());
    if digest != pin.sha256 {
        bail!("ffmpeg: sha256 {digest}, expected {} (wrong or corrupt file)", pin.sha256);
    }
    Ok(())
}

/// Set the owner-execute bit (and group/other read+execute) on a freshly
/// downloaded binary so it can be spawned.
#[cfg(unix)]
async fn make_executable(p: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o755);
    tokio::fs::set_permissions(p, perms)
        .await
        .with_context(|| format!("chmod +x {}", p.display()))
}

#[cfg(not(unix))]
async fn make_executable(_p: &Path) -> anyhow::Result<()> {
    Ok(())
}

/// First-run user-facing hint — straight to stderr (not `tracing`) so it shows
/// regardless of `RUST_LOG`, mirroring the runtime/model provisioners.
fn hint(msg: &str) {
    eprintln!("hi-agent: {msg}");
}

/// Lowercase hex of a byte slice (for comparing a computed digest to the pin).
fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pin_is_well_formed() {
        // Every row, not just the one this host resolves. A pin only a host
        // nobody builds on can reach would otherwise never be checked, and a typo
        // in it surfaces as a broken bundle on whichever machine tries first.
        for p in [&MACOS_ARM64, &WINDOWS_X64] {
            assert_eq!(p.sha256.len(), 64, "{}", p.url);
            assert!(p.sha256.chars().all(|c| c.is_ascii_hexdigit()), "{}", p.url);
            assert!(p.url.starts_with("https://"), "{}", p.url);
            assert!(p.url.contains(RELEASE_TAG), "{}", p.url);
            assert!(p.license_url.starts_with("https://"), "{}", p.license_url);
            assert!(p.size > 0, "{}", p.url);
        }
    }
}
