//! The attention lane — the web face reports what its *own* window is doing.
//!
//! `POST /api/in/attention` is a first-party report the page sends when its window
//! changes state: it came forward, it went behind something, or it was shut. The
//! body is the state — `active`, `background` or `closed` — and an empty body
//! means `active`, which is what a body-less POST always meant.
//!
//! **The other two states exist because nothing else can see them.** Reach only
//! knows whether a channel is open, and the face drops its channels for
//! `background` and `closed` alike, so from the wire they are the same nothing.
//! They are different situations — background is ambient, closed is a decision —
//! and only the client can tell us which, so only the client is asked.
//!
//! Strictly first-party: the page reports about its own window, never about other
//! apps or the wider system. It just pokes [`crate::body::presence::Presence`].

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;

use crate::body::presence::WindowState;
use crate::foundation::server::AppState;

/// Record what the window is doing.
pub async fn post_attention(
    State(state): State<Arc<AppState>>,
    body: String,
) -> StatusCode {
    let Some(window) = parse_state(&body) else {
        return StatusCode::BAD_REQUEST;
    };
    // The one signal that can mean "they came back" — see `Presence::returns`. Worth
    // a line at info: it is the cause of a turn nobody typed, so a reader of
    // `server.log` needs to be able to account for one.
    if state.presence.note_window(window) {
        tracing::info!("attention: they're back after an absence");
    }
    StatusCode::ACCEPTED
}

/// The body as a window state. Empty is `Active` — a body-less POST is what the
/// face sent when there was nothing to say but "I'm here", and it still reads that
/// way. Anything unrecognized is rejected rather than guessed: presence defaults
/// keep-biased toward *present* everywhere else, so a typo quietly meaning
/// "active" would be a report that can never say the person left.
fn parse_state(body: &str) -> Option<WindowState> {
    match body.trim() {
        "" | "active" => Some(WindowState::Active),
        "background" => Some(WindowState::Background),
        "closed" => Some(WindowState::Closed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_states_and_the_empty_body_parse() {
        assert_eq!(parse_state(""), Some(WindowState::Active));
        assert_eq!(parse_state("active"), Some(WindowState::Active));
        assert_eq!(parse_state("background"), Some(WindowState::Background));
        assert_eq!(parse_state("closed"), Some(WindowState::Closed));
        assert_eq!(parse_state("  closed\n"), Some(WindowState::Closed));
    }

    /// Keep-biased elsewhere means resolving toward "still here"; here it would
    /// mean a client that can never report leaving, so an unknown state is an
    /// error rather than a default.
    #[test]
    fn an_unknown_state_is_rejected_not_taken_as_active() {
        assert_eq!(parse_state("gone"), None);
        assert_eq!(parse_state("Active"), None);
    }
}
