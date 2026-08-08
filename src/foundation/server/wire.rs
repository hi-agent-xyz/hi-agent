//! Raw wire endpoint — the operator's rawest window into agent sessions.
//!
//! - `GET /api/wire/frames/events` — Server-Sent Events carrying one frame type,
//!   `frame`: every JSON-RPC line that crosses the wire between hi-agent and a
//!   session's `codex app-server` subprocess. The buffered ring replays on connect (so
//!   a late subscriber sees recent context), then live frames stream as they happen.
//!
//! This reads the [`WireTap`](crate::foundation::codex::WireTap) ring + feed; it never
//! mutates it and knows nothing about the reaction. The inspect SPA's Sessions tab
//! groups the frames by connection to reconstruct per-session conversations, including
//! the system prompt carried as `baseInstructions` on `thread/start`.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::{self, Stream, StreamExt};
use tokio::sync::broadcast;

use crate::foundation::codex::RawFrame;
use crate::foundation::server::AppState;

/// `GET /api/wire/frames/events` — SSE of raw JSON-RPC frames. On connect the buffered
/// ring replays first (each as a `frame` event), then live frames stream. Replay
/// and live are cut atomically by [`WireTap::subscribe`](crate::foundation::codex::WireTap::subscribe),
/// so no frame is dropped or duplicated across the seam.
pub async fn get_wire_frames_events(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (replay, rx) = state.wire_tap.subscribe();
    let replay = stream::iter(replay);
    let live = frame_stream(rx);
    let frames = replay.chain(live).map(|f| Ok::<Event, Infallible>(frame_event(&f)));
    Sse::new(frames).keep_alive(KeepAlive::default())
}

fn frame_event(frame: &RawFrame) -> Event {
    Event::default()
        .event("frame")
        .json_data(frame)
        .unwrap_or_else(|_| Event::default().comment("serialize error"))
}

/// Turn a broadcast receiver into a stream of frames, skipping lag gaps and
/// ending on close — the [`WireTap`](crate::foundation::codex::WireTap) twin of
/// [`event_stream`](crate::foundation::observatory::event_stream).
fn frame_stream(rx: broadcast::Receiver<RawFrame>) -> impl Stream<Item = RawFrame> {
    stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(frame) => return Some((frame, rx)),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "wire tap: SSE subscriber lagged");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
}
