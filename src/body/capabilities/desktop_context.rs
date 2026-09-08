//! Desktop context capability — which application and window are in front on
//! the machine this process runs on.
//!
//! **This is the coarse gate for reflex recognition and nothing else.** Its one
//! caller is [`crate::foundation::server::reflex::post_invoke`], which uses the
//! app name and window title to decide *which* taught reflex could apply before
//! [`super::accessibility`] looks for the field.
//!
//! It used to be wider — a screenshot, both clipboard flavors, the frontmost
//! browser's URL — and none of that had a reader. The screenshot the person
//! hands over with the ⌘ glance gesture comes from [`super::screencast`], not
//! from here, and everything an agent needs in order to *drive* a machine is a
//! note over the tools that machine already has ([`crate::mind::skills`]), not a
//! capability in this process: a mechanism kept here has to be written again for
//! X11, Wayland, Windows and Android, while the judgment that reads a screenshot
//! is the same code everywhere.
//!
//! Unlike the API-backed capabilities, the "vendor" here is the operating
//! system, so selection is compile-time (`cfg(target_os)`) rather than env
//! config — there is no `init_from_env` and nothing to configure. Both fields
//! are best-effort: a missing OS permission (Automation) or a windowless app
//! yields `None` for that field, never an error, so one denied prompt does not
//! take recognition down — it just leaves a reflex that gates on the app unable
//! to match. Failures are logged at `warn` for diagnosability.

use chrono::{DateTime, Utc};

/// What is in front right now. `captured_at` is always present; both other
/// fields are best-effort.
#[derive(Debug, Clone)]
pub struct ContextSnapshot {
    pub captured_at: DateTime<Utc>,
    /// Name of the frontmost application (e.g. `Safari`).
    pub frontmost_app: Option<String>,
    /// Title of the frontmost window (e.g. `flight booking — Safari`).
    pub frontmost_window_title: Option<String>,
}

/// Whether this build has an impl for the current platform. Note this is a
/// compile-time fact, not a permission check — a macOS build in an SSH session
/// reports `true` but will read little.
pub fn available() -> bool {
    cfg!(target_os = "macos")
}

/// Best-effort read of the frontmost app and window.
/// Errs only where [`available`] is `false`.
pub async fn capture() -> anyhow::Result<ContextSnapshot> {
    #[cfg(target_os = "macos")]
    {
        Ok(crate::foundation::vendors::macos_desktop_context::capture().await)
    }
    #[cfg(not(target_os = "macos"))]
    {
        anyhow::bail!("desktop context is not supported on this platform")
    }
}
