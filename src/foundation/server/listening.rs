//! `GET /api/listening` — whether the agent has its ear open right now.
//!
//! One boolean, and it is true for exactly one reason: somebody is holding the
//! attention key and the mic the press opened has crossed into live processing
//! ([`crate::body::gesture`]). Not "the mic is on" in general — a browser page with
//! `getUserMedia` open is the person's own doing and says nothing about whether the
//! agent is being spoken to.
//!
//! ## Why this is state to read rather than a push to an app
//!
//! `docs/arch/mechanisms.md` used to count "the tray pushes" among the things that
//! must cross the app seam outward, from when the only tray was AppKit inside this
//! process. It is not one. **A tray is a renderer, and there is more than one** — the
//! macOS menu bar, the Windows notification area, a phone, a browser face — so the
//! question each of them is asking is the same question, and the answer belongs on the
//! core where anyone may read it. Making it a [`mechanisms`](super::mechanisms) call
//! would hand the fact to whichever app answered and hide it from the rest.
//!
//! So the direction here is the ordinary one: the app asks, the core answers. The
//! Windows shell holds this subscription open for its notification-area icon's third
//! state (`app/windows/HiAgentWindows/Core/ListeningWatch.cs`); the macOS menu bar is
//! in this process and reads the same value without the round trip.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::{self, Stream, StreamExt};
use serde::Serialize;
use tokio::sync::watch;

use super::AppState;

/// Whether the agent's ear is open, and the seam anything watching it reads.
///
/// A `watch` rather than a broadcast on purpose: a subscriber that attaches mid-hold
/// must learn that it is mid-hold, and a subscriber that falls behind must converge on
/// *now* rather than replay a queue of flips. Both are what `watch` is.
#[derive(Clone)]
pub struct ListeningState {
    tx: watch::Sender<bool>,
}

impl Default for ListeningState {
    fn default() -> Self {
        Self::new()
    }
}

impl ListeningState {
    pub fn new() -> Self {
        Self { tx: watch::channel(false).0 }
    }

    /// Open or close the ear. Idempotent — `watch` only wakes subscribers when the
    /// value actually changes, so calling this with the value it already holds costs
    /// nothing and wakes nobody.
    pub fn set(&self, on: bool) {
        self.tx.send_if_modified(|current| {
            let changed = *current != on;
            *current = on;
            changed
        });
    }

    pub fn get(&self) -> bool {
        *self.tx.borrow()
    }

    fn subscribe(&self) -> watch::Receiver<bool> {
        self.tx.subscribe()
    }
}

/// What one frame on the wire says.
#[derive(Serialize)]
struct Frame {
    listening: bool,
}

/// `GET /api/listening` — the current value, then every change.
///
/// The first frame arrives immediately and without waiting for anything to happen,
/// which is the property a tray needs: an app that starts while the person happens to
/// be mid-hold must not draw "idle" until they let go. That is what `first` below is —
/// the held value, sent before the loop ever awaits a change.
///
/// Keep-alive is on for the reason it always is: a shell holding this open across a
/// sleep needs the socket's death to be noticed rather than to sit there looking like
/// a very quiet agent.
pub async fn get_listening(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.listening.subscribe();
    let first = *rx.borrow_and_update();
    let changes = stream::unfold(rx, |mut rx| async move {
        // Ends only when the sender is dropped, which is process shutdown.
        rx.changed().await.ok()?;
        let value = *rx.borrow_and_update();
        Some((value, rx))
    });
    let frames = stream::iter([first]).chain(changes).map(|listening| {
        Ok(Event::default()
            .event("listening")
            .json_data(Frame { listening })
            .unwrap_or_else(|_| Event::default().comment("serialize error")))
    });
    Sse::new(frames).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_subscriber_learns_the_value_it_arrived_in_the_middle_of() {
        let state = ListeningState::new();
        state.set(true);

        // Attaching after the hold began: the first thing read is `true`, not a wait
        // for the release. This is the tray-restarted-mid-hold case.
        let mut rx = state.subscribe();
        assert!(*rx.borrow_and_update());

        state.set(false);
        rx.changed().await.expect("the release arrives");
        assert!(!*rx.borrow());
    }

    #[tokio::test]
    async fn setting_the_value_it_already_holds_wakes_nobody() {
        let state = ListeningState::new();
        let mut rx = state.subscribe();
        rx.borrow_and_update();

        state.set(false);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), rx.changed())
                .await
                .is_err(),
            "an unchanged value is not a change"
        );

        state.set(true);
        rx.changed().await.expect("a real change does wake it");
        assert!(state.get());
    }
}
