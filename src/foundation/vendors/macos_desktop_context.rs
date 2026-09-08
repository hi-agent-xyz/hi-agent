//! macOS frontmost-app/window read for [`crate::body::capabilities::desktop_context`].
//!
//! The "API" here is the OS's stable CLI surface rather than framework
//! bindings — zero extra crate dependencies, and each tool degrades cleanly
//! when a permission is missing:
//!
//!   /usr/bin/lsappinfo          frontmost app name  no permission
//!   /usr/bin/osascript          front window title  Automation ("System
//!                                                    Events"), prompted once
//!                                                    per host app
//!
//! Permissions attach to the *responsible* app: in dev that is the terminal
//! that launched the process; a bundled .app must hold the grants itself.
//!
//! This file **used to capture the screen, both clipboard flavors and the
//! frontmost browser's URL**, and all of that is gone. Not because it stopped
//! working — because driving a machine is a note over what the machine already
//! has, and a mechanism kept in here has to be written again for X11, Wayland,
//! Windows and Android. What is left is the one field a caller in this process
//! still reads. Output parsing stays in pure functions so the wire shapes are
//! unit-testable without a GUI session.

use anyhow::Context;
use chrono::Utc;
use tokio::process::Command;

use crate::body::capabilities::desktop_context::ContextSnapshot;

/// Read which app and window are in front. Never fails: each field is
/// independently read and a failed field is `None`.
pub async fn capture() -> ContextSnapshot {
    let captured_at = Utc::now();
    let (app, title) = tokio::join!(frontmost_app(), frontmost_window_title());

    ContextSnapshot {
        captured_at,
        frontmost_app: warn_missing("frontmost app", app),
        frontmost_window_title: debug_missing("window title", title)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    }
}

/// A field whose absence usually means a denied permission or broken tool.
fn warn_missing<T>(field: &str, r: anyhow::Result<T>) -> Option<T> {
    r.inspect_err(|e| tracing::warn!("desktop context: {field} unavailable: {e:#}")).ok()
}

/// A field whose absence is routine (a windowless app, an SSH session, …).
fn debug_missing<T>(field: &str, r: anyhow::Result<T>) -> Option<T> {
    r.inspect_err(|e| tracing::debug!("desktop context: {field} absent: {e:#}")).ok()
}

async fn frontmost_app() -> anyhow::Result<String> {
    let asn = run("/usr/bin/lsappinfo", &["front"]).await?;
    let asn = asn.trim();
    anyhow::ensure!(!asn.is_empty(), "lsappinfo front returned nothing");
    let info = run("/usr/bin/lsappinfo", &["info", "-only", "name", asn]).await?;
    parse_quoted_value(&info)
        .ok_or_else(|| anyhow::anyhow!("unexpected lsappinfo output: {info:?}"))
}

async fn frontmost_window_title() -> anyhow::Result<String> {
    run(
        "/usr/bin/osascript",
        &[
            "-e",
            "tell application \"System Events\" to tell \
             (first application process whose frontmost is true) to get name of front window",
        ],
    )
    .await
}

async fn run(cmd: &str, args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .await
        .with_context(|| format!("spawning {cmd}"))?;
    anyhow::ensure!(
        out.status.success(),
        "{cmd} exited {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Pull the value out of lsappinfo's `"key"="value"` line.
fn parse_quoted_value(line: &str) -> Option<String> {
    let value = line.trim().split_once('=')?.1.trim().trim_matches('"');
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lsappinfo_name_line() {
        // Observed key on macOS 26 is LSDisplayName; the parser is key-agnostic.
        assert_eq!(
            parse_quoted_value("\"LSDisplayName\"=\"Safari\"\n").as_deref(),
            Some("Safari")
        );
        assert_eq!(
            parse_quoted_value("\"name\"=\"Google Chrome\"").as_deref(),
            Some("Google Chrome")
        );
        assert!(parse_quoted_value("garbage").is_none());
        // No frontmost GUI app (e.g. SSH session) yields an empty value.
        assert!(parse_quoted_value("\"LSDisplayName\"=\"\"").is_none());
    }
}
