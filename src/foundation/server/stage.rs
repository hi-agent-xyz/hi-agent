//! The stage lane — every face reports the frame it is showing.
//!
//! `POST /api/stage` carries `{"width":…,"height":…,"scale":…,"theme":…}` in CSS
//! pixels, and both consumers are renderers: `review_view` renders into whatever the
//! window last reported, so a builder composes for the frame the person actually has
//! and a reviewer signs off on that same frame; and the views band's thumbnails
//! (`view_shots`) render into that frame *and* that skin, so the picture in the
//! history row is of the screen the person was looking at.
//!
//! The skin is reported here rather than passed per render for the same reason the
//! frame is: the page is the only thing that knows it, it is a property of the
//! window and not of any one call, and asking the OS instead would turn a portable
//! fact into a platform mechanism.
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
//! **Every face that can show a view reports**, each under its own `surface` id: the
//! desktop window, a browser tab, and the iPhone client's `WKWebView`, which loads
//! the same page. The one that stays quiet is the menu-bar popover, which says so
//! with `?chrome=popover` — it is a chat panel, and a review rendered at its 380×540
//! portrait frame would be a review of a frame nobody reads a view on. The store
//! keeps one entry per surface with the most recent reporter at the head, because a
//! report follows a resize, a skin flip or a load, and all three are someone looking.

use axum::Json;
use axum::http::StatusCode;
use serde::Deserialize;

use crate::body::capabilities::view_render;

/// One face's frame in CSS pixels, plus its device pixel ratio, its skin and who
/// is speaking.
#[derive(Debug, Deserialize)]
pub struct StageFrame {
    width: f64,
    height: f64,
    /// `window.devicePixelRatio`. Absent reads as the review default.
    #[serde(default)]
    scale: Option<f64>,
    /// The skin this face is in — `data-theme` when one is forced, else what
    /// `prefers-color-scheme` resolves to. Absent on a client older than the field,
    /// which keeps whatever this surface reported last.
    #[serde(default)]
    theme: Option<String>,
    /// The face's own id, minted per page load. Absent on a client older than the
    /// field, which then reports as the one unnamed face — the behaviour from when
    /// there was only ever one.
    #[serde(default)]
    surface: Option<String>,
}

/// Record the frame one face is showing.
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
    // A theme we don't know is dropped inside the store rather than failing the
    // report: the frame is the half anything depends on, and rejecting the whole POST
    // over the decoration would leave every later review rendering at a stale size.
    let surface = frame.surface.as_deref().unwrap_or("face");
    if view_render::report_surface(surface, width as u32, height as u32, scale, frame.theme.as_deref()) {
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
    /// writes the process-global `STAGE`, and the store's own semantics (the newest
    /// reporter leads, one entry per surface, absurd frames dropped, scale clamped)
    /// are asserted in `view_render`'s tests. Asserting them from a second test in the same
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
    /// `surface` is optional for the same reason the skin is: a client that predates
    /// the field still knows the number the renderer actually needs.
    #[test]
    fn a_fractional_frame_parses_and_the_optional_fields_are_optional() {
        let f = body(r#"{"width":1512.5,"height":944.5}"#);
        assert_eq!(f.scale, None);
        assert_eq!(f.surface, None);
        assert_eq!(f.width.round(), 1513.0);
        assert_eq!(f.height.round(), 945.0);

        let named = body(r#"{"width":390,"height":844,"surface":"a1b2","theme":"dark"}"#);
        assert_eq!(named.surface.as_deref(), Some("a1b2"));
        assert_eq!(named.theme.as_deref(), Some("dark"));
    }
}
