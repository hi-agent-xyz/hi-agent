//! macOS screen grab — one still of the main display, via `/usr/sbin/screencapture`.
//!
//! **This exists for one gesture and nothing else.** The person double-taps the right
//! ⌘ and hands the agent what they are looking at ([`crate::body::gesture::glance`]);
//! this is the grab behind that. It is *not* a perceive surface the agent can reach
//! for — nothing else calls it, no tool exposes it, and the way an agent looks at a
//! machine is a note over that machine's own tools
//! ([`crate::mind::skills`]'s `driving-a-desktop.md`).
//!
//! That is why the capability wrapper is gone. A `capabilities::screencast` module
//! existed to select a vendor per platform and to offer the work to an attached shell;
//! with one caller, on one platform, that indirection described a generality the code
//! no longer has. The gesture is macOS-only by construction — it is triggered by a
//! macOS key tap — so it calls this directly.
//!
//! It used to also enumerate windows (`CGWindowList`) and grab one by number, for a
//! cast-to-view consumer that was never built. Deleted rather than kept: see
//! `docs/arch/mechanisms.md`.
//!
//! Needs the **Screen Recording** grant for real pixels; without it the grab returns a
//! desktop-only image. Only compiled on macOS.

use anyhow::Context;
use bytes::Bytes;
use tokio::process::Command;

/// Grab the whole screen as PNG. With no `-l<id>`, `screencapture` captures the
/// main display fullscreen; `-x` silences the shutter sound.
pub async fn grab_screen_png() -> anyhow::Result<Bytes> {
    let path = std::env::temp_dir().join(format!("hi-agent-screen-{}.png", uuid::Uuid::now_v7()));
    let path_str = path.to_string_lossy().into_owned();
    let status = Command::new("/usr/sbin/screencapture")
        .args(grab_screen_args(&path_str))
        .status()
        .await
        .context("spawning screencapture")?;
    anyhow::ensure!(status.success(), "screencapture exited {status}");
    let bytes = tokio::fs::read(&path).await.context("reading screencapture output")?;
    let _ = tokio::fs::remove_file(&path).await;
    Ok(Bytes::from(bytes))
}

/// The `screencapture` argv for a whole-screen PNG grab. Pure so the flags are
/// unit-testable without a GUI session.
fn grab_screen_args(path: &str) -> Vec<String> {
    vec!["-x".into(), "-t".into(), "png".into(), path.into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grab_screen_args_have_no_window_target() {
        let args = grab_screen_args("/tmp/s.png");
        assert!(!args.iter().any(|a| a.starts_with("-l")), "whole-screen grab has no -l");
        assert!(args.contains(&"png".to_string()));
        assert_eq!(args.last().unwrap(), "/tmp/s.png");
    }
}
