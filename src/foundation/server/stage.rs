//! The stage lane — the desktop window reports the frame it is showing.
//!
//! `POST /api/stage` carries `{"width":…,"height":…,"scale":…}` in CSS pixels, and
//! the only consumer is `review_view`: it renders into whatever the window last
//! reported, so a builder composes for the frame the person actually has and a
//! reviewer signs off on that same frame.
//!
//! **Why the page is asked rather than the window.** The number is a rendering
//! parameter, not a perception, and the page already holds it (`innerWidth` /
//! `innerHeight`) with no OS call at all. Reading it off `NSWindow` instead would
//! turn a portable fact into a macOS mechanism for nothing — the wrong side of the
//! mechanism/policy line `docs/arch/` draws, and one more thing to re-port when the
//! native shell takes over the process.
//!
//! Strictly first-party, the same rule the attention lane keeps: the page reports
//! its own frame, never another window's. It is deliberately NOT under `/api/in/*`
//! — that namespace is perception, world→agent, and nothing about a window's size
//! reaches the agent's senses.
//!
//! The client sends only from the desktop window (see `stageReport.ts`); the
//! popover and a browser tab stay quiet, because a review rendered at the
//! popover's 380×540 portrait frame would be a review of something nobody is
//! composing for.

use axum::Json;
use axum::http::StatusCode;
use serde::Deserialize;

use crate::body::capabilities::view_render;

/// A window frame in CSS pixels, plus its device pixel ratio.
#[derive(Debug, Deserialize)]
pub struct StageFrame {
    width: f64,
    height: f64,
    /// `window.devicePixelRatio`. Absent reads as the review default.
    #[serde(default)]
    scale: Option<f64>,
}

/// Record the frame the desktop window is showing.
///
/// A rejected frame answers `400` rather than failing quietly: the client reports
/// on every resize, so a silently-dropped report would leave reviews rendering at
/// a stale size with nothing anywhere saying why.
pub async fn post_stage(Json(frame): Json<StageFrame>) -> StatusCode {
    // Fractional CSS pixels are normal on a scaled display; the viewport wants
    // whole device-independent pixels.
    let width = frame.width.round();
    let height = frame.height.round();
    if !width.is_finite() || !height.is_finite() || width < 0.0 || height < 0.0 {
        return StatusCode::BAD_REQUEST;
    }
    let scale = frame.scale.unwrap_or(view_render::DEFAULT_SCALE);
    if view_render::set_stage_frame(width as u32, height as u32, scale) {
        StatusCode::ACCEPTED
    } else {
        StatusCode::BAD_REQUEST
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(json: &str) -> StageFrame {
        serde_json::from_str(json).expect("valid frame json")
    }

    /// Only the rejecting cases are exercised here, on purpose: an accepted frame
    /// writes the process-global `STAGE`, and the store's own semantics (newest
    /// wins, absurd frames dropped, scale clamped) are asserted in
    /// `view_render`'s tests. Asserting them from a second test in the same
    /// binary would just race that one through the global.
    #[tokio::test]
    async fn a_frame_that_is_not_a_window_is_refused() {
        for json in [
            r#"{"width":-1,"height":800}"#,
            r#"{"width":0,"height":0}"#,
            // A window mid-drag, or a hidden tab reporting a collapsed box.
            r#"{"width":8,"height":6}"#,
        ] {
            assert_eq!(
                post_stage(Json(body(json))).await,
                StatusCode::BAD_REQUEST,
                "should have refused {json}"
            );
        }
    }

    /// Fractional CSS pixels are ordinary on a scaled display, and `scale` is
    /// optional because a client that cannot name its DPR still knows its frame.
    #[test]
    fn a_fractional_frame_parses_and_scale_is_optional() {
        let f = body(r#"{"width":1512.5,"height":944.5}"#);
        assert_eq!(f.scale, None);
        assert_eq!(f.width.round(), 1513.0);
        assert_eq!(f.height.round(), 945.0);
    }
}
