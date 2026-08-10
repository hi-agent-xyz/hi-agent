//! The text channel: `POST /api/in/text`, `GET /api/in/text`, `GET /api/out/text`.
//!
//! `POST /api/in/text` is the typed-input path: the body is dispatched to the
//! mind (journalled + queued on `inbound`), echoed to live channel observers,
//! and folded into the backend-owned current text appearance.
//!
//! `GET /api/out/text` is one long-lived NDJSON stream of whole current-state
//! snapshots. A subscriber receives the present exchange immediately, then a
//! replacement whenever it changes. There are no message ids, client ids,
//! cursors or replay. The journal is history; this endpoint is the appearance's
//! present.
//!
//! `GET /api/in/text` is a live observe stream (see [`crate::foundation::server::observe`]):
//! no buffering, just the inputs as they cross the boundary.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use uuid::Uuid;

use crate::foundation::server::AppState;
use crate::foundation::server::headers::{AuthBearer, StreamHeader};
use crate::foundation::server::observe;
use crate::types::{Channel, JournalEntry, Origin, Signal};
use futures::stream::unfold;

pub async fn post_text(
    State(state): State<Arc<AppState>>,
    StreamHeader(stream): StreamHeader,
    AuthBearer(auth): AuthBearer,
    body: Bytes,
) -> impl IntoResponse {
    let body_str = match std::str::from_utf8(&body) {
        Ok(s) => s.to_owned(),
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "text body must be utf-8").into_response();
        }
    };

    let signal = Signal {
        channel: Channel::Text,
        body: body_str,
        stream,
        ts: Utc::now(),
    };

    tracing::info!(
        auth = ?auth,
        len = signal.body.len(),
        "POST /api/in/text"
    );
    crate::foundation::channel_log::inbound(Channel::Text, &signal.body);

    let entry = JournalEntry::SignalIn {
        id: Uuid::now_v7().to_string(),
        ts: signal.ts,
        channel: signal.channel,
        body: signal.body.clone(),
        stream: signal.stream.clone(),
        media: None,
        origin: Some(Origin::Human),
    };
    if let Err(err) = state.memory.journal.append(entry).await {
        tracing::error!(error = %err, "journal append failed; accepting signal anyway");
    }

    // A human message is engagement — refresh presence and start the owed-reply
    // clock (covers both typed text and transcribed voice, which lands here too).
    state.presence.note_activity();

    // Fold into the shared text appearance and echo to live observers before
    // dispatching inward.
    state.echo_input(Channel::Text, &signal.body, true);

    if let Err(err) = state.inbound.send(signal).await {
        tracing::error!(error = %err, "inbound channel closed");
        return (StatusCode::SERVICE_UNAVAILABLE, "inbound channel closed").into_response();
    }

    StatusCode::ACCEPTED.into_response()
}

/// `GET /api/out/text` — the current text appearance, then every replacement.
pub async fn get_out_text(
    State(state): State<Arc<AppState>>,
    AuthBearer(auth): AuthBearer,
) -> Response {
    tracing::info!(auth = ?auth, "GET /api/out/text state stream opened");

    // Opening this state stream is a presence signal: warm up so the process +
    // session + upstream cache are hot before the first utterance.
    state.warm();

    // Count this reader as live presence for as long as its body stream exists.
    let presence = state
        .presence
        .connect(crate::body::presence::OutChannel::Text);
    let rx = state.text_appearance.subscribe();
    let stream = unfold(
        (rx, true, presence),
        |(mut rx, first, presence)| async move {
            if !first && rx.changed().await.is_err() {
                return None;
            }
            let snapshot = { rx.borrow_and_update().clone() };
            let mut line = serde_json::to_vec(&snapshot).expect("text state is serializable");
            line.push(b'\n');
            Some((
                Ok::<Bytes, std::convert::Infallible>(Bytes::from(line)),
                (rx, false, presence),
            ))
        },
    );

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-ndjson; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(stream))
        .unwrap()
}

/// `GET /api/in/text` — observe typed inputs live (NDJSON).
pub async fn get_in_text(
    State(state): State<Arc<AppState>>,
    AuthBearer(auth): AuthBearer,
) -> Response {
    tracing::info!(auth = ?auth, "GET /api/in/text observe opened");
    observe::stream_input(state, Channel::Text)
}
