//! The text channel: `POST /api/in/text`, `POST /api/in/text/typing`,
//! `GET /api/in/text`, `GET /api/out/text`.
//!
//! `POST /api/in/text` is the typed-input path: the body is dispatched to the
//! mind (journalled + queued on `inbound`), echoed to live channel observers,
//! and appended to the conversation as one message.
//!
//! `POST /api/in/text/typing` is the only thing here that is not input: a
//! contentless ping saying a line is being written. It never becomes a message, a
//! journal entry, or an observed frame — it goes to the floor and nowhere else, so
//! the agent does not answer half a thought. See [`crate::body::reaction::floor`].
//!
//! `GET /api/out/text` is one long-lived NDJSON stream of the conversation: the
//! current window whole, then one frame per message appended. `GET /api/messages`
//! reads further back through the journal. Messages carry the journal's ids so the
//! two agree, but no client ever sends one back — there is no cursor, no
//! acknowledgement and no read position anywhere in this file.
//!
//! `GET /api/in/text` is a live observe stream (see [`crate::foundation::server::observe`]):
//! no buffering, just the inputs as they cross the boundary.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::foundation::server::headers::{AuthBearer, StreamHeader};
use crate::foundation::server::{AppState, observe, transcript};
use crate::mind::memory::journal;
use crate::types::{Author, Channel, Content, Inbound, JournalEntry, Message, Sender};
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

    let ts = Utc::now();
    // A typed line has no capture source to tell apart, so the stream header stops
    // here rather than riding along on a message that has no use for it.
    let _ = stream;

    tracing::info!(
        auth = ?auth,
        len = body_str.len(),
        "POST /api/in/text"
    );

    // The one place credentials are looked for. A key somebody typed without
    // thinking is written to `drive/accounts/secrets/`, and every model prompt
    // gets that path in its place (`AgentSession::prompt`). Nothing below this
    // line changes: the journal, the conversation and `/api/out/text` all carry
    // the message exactly as it was sent — the person is not the one being kept
    // from their own key.
    //
    // A scan that fails must not cost them the message. It is logged and the
    // signal goes on, because the alternative — refusing input — is a worse
    // failure than a key reaching the model, which is what happened before any
    // of this existed.
    match state.privacy.filter().file_secrets(&body_str) {
        Ok(filed) if !filed.is_empty() => tracing::info!(
            count = filed.len(),
            refs = ?filed.iter().map(|f| f.reference.as_str()).collect::<Vec<_>>(),
            "filed credentials from an inbound message; sessions will see their paths"
        ),
        Ok(_) => {}
        Err(error) => tracing::error!(
            error = %format!("{error:#}"),
            "secret scan failed; the message is accepted unscanned"
        ),
    }
    crate::foundation::channel_log::inbound(Channel::Text, &body_str);

    // Addressed: somebody typed this *to* the agent, so absent evidence otherwise it
    // is the owner. Labelled `owner` rather than written bare, so a later pass can
    // tell the default from a recognition — see `docs/arch/signal-attribution.md`.
    // Decided once and handed to both the journal and the conversation, so the face
    // beside the message and the name in the log are one answer, not two.
    let sender = Sender::owner_or_unknown(crate::foundation::config::tunables::owner().as_deref());
    // Minted once and used three times: the journal entry, the conversation and
    // Reaction are the same value under the same key, which is what lets the list be
    // rebuilt from the log without a merge — and what gives Reaction something to
    // name when it concludes anything about this line.
    let message = Message {
        id: Uuid::now_v7().to_string(),
        ts,
        from: Author::Person(sender),
        content: Content::Text(body_str),
    };
    let entry = JournalEntry::Message { channel: Channel::Text, message: message.clone() };
    if let Err(err) = state.memory.journal.append(entry).await {
        tracing::error!(error = %format!("{err:#}"), "journal append failed; accepting signal anyway");
    }

    state.floor.note_sent().await;

    // Append to the conversation and echo to live observers before dispatching
    // inward.
    state.note_message(Channel::Text, message.clone());

    if let Err(err) = state.inbound.send(Inbound::Message(message)).await {
        tracing::error!(error = %err, "inbound channel closed");
        return (StatusCode::SERVICE_UNAVAILABLE, "inbound channel closed").into_response();
    }

    StatusCode::ACCEPTED.into_response()
}

/// `POST /api/in/text/typing` — a keystroke landed in a line that has not been
/// sent yet.
///
/// Contentless on purpose. The draft's *text* is nobody's business but the window
/// holding it: this reports only that a thought is in progress, which is all the
/// floor needs to not answer over it. It is deliberately not the recognition
/// `interim`, which exists to be *shown* — a half-typed line is not shown anywhere,
/// and routing drafts into that slot would put one window's unsent keystrokes on
/// every other window's screen.
///
/// Cheap, idempotent, and fire-and-forget: a composer pings this while the person
/// writes, and stopping is expressed by not pinging. Nothing is stored per client
/// and no reply body is produced.
pub async fn post_text_typing(
    State(state): State<Arc<AppState>>,
    AuthBearer(auth): AuthBearer,
) -> impl IntoResponse {
    tracing::debug!(auth = ?auth, "POST /api/in/text/typing");
    state.floor.note_typing(tokio::time::Instant::now()).await;
    StatusCode::ACCEPTED
}

/// `GET /api/out/text` — the conversation: the current window whole, then every
/// message as it is appended.
pub async fn get_out_text(
    State(state): State<Arc<AppState>>,
    AuthBearer(auth): AuthBearer,
) -> Response {
    tracing::info!(auth = ?auth, "GET /api/out/text conversation stream opened");

    // Opening this stream means a surface just attached: warm up so the process +
    // session + upstream cache are hot before the first utterance.
    state.warm();

    // Hold an attachment guard for as long as the body stream exists. This counts
    // toward one question only — whether a speaker is attached, which decides
    // whether speech is synthesized. Nothing infers from it whether anyone is
    // reading; see `docs/arch/host.md#attachment`.
    let attached = state
        .attachments
        .connect(crate::body::attachments::OutChannel::Text);
    let (opening, rx) = state.transcript.subscribe();

    // `Some(frame)` is pending output; after it, the stream pulls from `rx`. A
    // lagged receiver resyncs with a fresh whole window rather than skipping
    // messages — the list is the contract, and a gap in it would be a lie.
    let transcript = state.transcript.clone();
    let stream = unfold(
        (rx, Some(opening), attached, transcript),
        |(mut rx, pending, attached, transcript)| async move {
            let frame = match pending {
                Some(frame) => frame,
                None => match rx.recv().await {
                    Ok(frame) => frame,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(missed = n, "text subscriber lagged; resyncing");
                        let (resync, _) = transcript.subscribe();
                        resync
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                },
            };
            let mut line = serde_json::to_vec(&frame).expect("a frame is serializable");
            line.push(b'\n');
            Some((
                Ok::<Bytes, std::convert::Infallible>(Bytes::from(line)),
                (rx, None, attached, transcript),
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

/// `GET /api/messages?before=<id>&limit=<n>` — older messages, for scrollback.
///
/// Read from the journal through the same mapping the boot seed uses, so what you
/// scroll into is the same conversation you were already looking at rather than a
/// second, differently-shaped view of the log.
///
/// `before` is a message id, which is a journal id. It is **not** a cursor the
/// backend remembers: it is where *this one request* starts, sent by a window that
/// already has the newer messages in hand and is asking for what precedes them.
pub async fn get_messages(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ScrollbackParams>,
    AuthBearer(auth): AuthBearer,
) -> Response {
    let limit = params.limit.unwrap_or(SCROLLBACK_DEFAULT).min(SCROLLBACK_MAX);
    let before = params
        .before
        .or_else(|| state.transcript.oldest_id())
        .unwrap_or_default();
    tracing::info!(auth = ?auth, before = %before, limit, "GET /api/messages");

    let since = journal::uuidv7_ts(&before)
        .map(|ts| ts - chrono::Duration::days(SCROLLBACK_DAYS))
        .unwrap_or_else(|| DateTime::from_timestamp(0, 0).expect("unix epoch is valid"));
    let entries = match state.memory.journal.recent(since, JOURNAL_SCAN_MAX).await {
        Ok(entries) => entries,
        Err(err) => {
            tracing::error!(error = %format!("{err:#}"), "scrollback read failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "journal read failed").into_response();
        }
    };

    let mut messages = transcript::from_journal(entries);
    messages.retain(|m| m.id.as_str() < before.as_str());
    if messages.len() > limit {
        let drop = messages.len() - limit;
        messages.drain(0..drop);
    }
    axum::Json(messages).into_response()
}

/// How far back one scrollback request reaches, in days of journal, before it
/// reports what it found. A page of chat is minutes of conversation on an active
/// day and months on a quiet one; this bounds the scan without bounding the
/// conversation, since the next request starts from whatever came back.
const SCROLLBACK_DAYS: i64 = 30;
const SCROLLBACK_DEFAULT: usize = 50;
const SCROLLBACK_MAX: usize = 200;
/// Journal lines to consider per scrollback request. Generous because most of what
/// it reads is not conversation (views, clock wakes, recognition) and is filtered out.
const JOURNAL_SCAN_MAX: usize = 5000;

#[derive(serde::Deserialize)]
pub struct ScrollbackParams {
    /// The oldest message the caller already has. Defaults to the oldest in the
    /// live window.
    before: Option<String>,
    limit: Option<usize>,
}

/// `GET /api/in/text` — observe typed inputs live (NDJSON).
pub async fn get_in_text(
    State(state): State<Arc<AppState>>,
    AuthBearer(auth): AuthBearer,
) -> Response {
    tracing::info!(auth = ?auth, "GET /api/in/text observe opened");
    observe::stream_input(state, Channel::Text)
}
