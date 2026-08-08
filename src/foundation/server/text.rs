//! The text channel: `POST /api/in/text`, `GET /api/in/text`, `GET /api/out/text`.
//!
//! `POST /api/in/text` is the typed-input path: the body is dispatched to the
//! mind (journalled + queued on `inbound`) and echoed on the
//! input-echo bus so every client observing `GET /api/in/text` renders the same
//! line — the human's words fan out the way the agent's do.
//!
//! `GET /api/out/text` is a long-poll for the agent's reply. The handler binds
//! to the reader's next utterance, holds the
//! connection open, and streams each chunk into the response body until the
//! utterance completes. Closing the body is the spec's "end of utterance". A
//! fresh GET re-subscribes for the next utterance; because the
//! [`TextBus`](crate::foundation::server::TextBus) retains utterances, a reply produced
//! between polls (or before the first poll) is retained rather than lost — see
//! that module for the race this fixes.
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

use futures::StreamExt as _;

use axum::extract::Query;
use crate::foundation::server::observe;
use crate::foundation::server::headers::{AuthBearer, StreamHeader};
use crate::foundation::server::AppState;
use crate::types::{Channel, JournalEntry, Origin, Signal};

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

    // Echo to observers (live, no buffer) before dispatching inward, so a
    // typed line shows on every client just like recognized speech does.
    state.echo_input(Channel::Text, &signal.body, true);

    if let Err(err) = state.inbound.send(signal).await {
        tracing::error!(error = %err, "inbound channel closed");
        return (StatusCode::SERVICE_UNAVAILABLE, "inbound channel closed").into_response();
    }

    StatusCode::ACCEPTED.into_response()
}

/// Query for `GET /api/out/text`: where this readerhas got to.
#[derive(Debug, Default, serde::Deserialize)]
pub struct OutTextQuery {
    /// The id of the last utterance this reader received in full. Absent means
    /// "start at the oldest still retained", so a client that has never connected
    /// still gets a reply produced before it arrived.
    pub after: Option<u64>,
}

/// `GET /api/out/text` — the agent's reply, one utterance per long-poll.
///
/// Reading does not consume: the utterance stays for every other attached
/// surface. The client says where it is with `?after=`, and the id it just
/// received comes back on `X-HI-Utterance` for the next request.
pub async fn get_out_text(
    State(state): State<Arc<AppState>>,
    Query(q): Query<OutTextQuery>,
    AuthBearer(auth): AuthBearer,
) -> Response {
    tracing::info!(auth = ?auth, after = ?q.after, "GET /api/out/text long-poll opened");

    // Opening this long-poll is a presence signal: warm up so the process +
    // session + upstream cache are hot before the first utterance.
    state.warm();

    // Count this reader as live presence for as long as its body stream exists.
    let presence = state.presence.connect(crate::body::presence::OutChannel::Text);
    // Resolved before the response head is written — the header has to be on the
    // response, and the body comes after it. Parks here if nothing is pending,
    // which is what makes this a long-poll rather than an empty 200.
    let id = state.text_bus.next_id_after(q.after).await;
    let stream = state.text_bus.subscribe(q.after).map(move |item| {
        let _held = &presence;
        item
    });

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(crate::foundation::server::text_bus::UTTERANCE_HEADER, id.to_string())
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
