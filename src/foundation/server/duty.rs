//! `POST /api/in/duty/{key}` — a listener saying something arrived.
//!
//! The world→agent side of a standing duty. A `serving` task's machinery is the agent's
//! own — a detached process holding a WebSocket or a long-poll — and this is the door it
//! knocks on when it has something, so the duty is handled in seconds rather than at the
//! next glance-up.
//!
//! **Not a wake channel for the conversation, and deliberately not a sense.** `host.md`
//! rules out `/api/in/text` for machine-originated input because a timer firing into it
//! puts a line the person never wrote into their chat. This route avoids that by not
//! going near the conversation at all: it reaches the working session that holds the
//! duty, and the person hears about it only if that session decides they should. Nothing
//! here appends to the transcript, emits an `InputEcho`, or counts as activity — which is
//! also why no [`crate::types::Channel`] variant was added. A duty is not something the
//! agent perceives; it is work arriving for a session that already exists.
//!
//! **Accepting is not handling.** The response says only that the delivery was queued.
//! Coalescing, the decision to open a handler, and the handler's own turn all happen
//! behind [`crate::body::reaction::DutyDelivery`], and none of them are worth making a
//! listener wait on. A `202` here does not promise a turn ran — a key no `serving` task
//! claims is dropped after this point, on purpose (see the module doc on the duty inbox).
//! What makes that safe is that the listener's own ledger, not this call, is the record.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::body::reaction::DutyDelivery;
use crate::foundation::server::AppState;

/// Longest routing key accepted. A `start_key` is a slug an agent wrote for itself, not a
/// free-text field.
const MAX_KEY: usize = 64;

/// Longest body accepted in one delivery. A listener with more than this to say is
/// summarising badly, or should be handing over a pointer into its own ledger.
const MAX_BODY: usize = 32_000;

pub async fn post_duty(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    body: String,
) -> Response {
    // Validated rather than sanitised, and rejected rather than repaired. The key is
    // matched against `Liveness::start_key` in the ledger and is never interpolated into
    // a prompt or a path — but a routing key that can carry arbitrary bytes is one that
    // will eventually be put somewhere that cares, and the shape a slug actually has
    // costs nothing to insist on now.
    if key.is_empty()
        || key.len() > MAX_KEY
        || !key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return (StatusCode::BAD_REQUEST, "duty key must be a slug\n").into_response();
    }

    let text = body.trim().to_owned();
    if text.is_empty() {
        return (StatusCode::BAD_REQUEST, "nothing to deliver\n").into_response();
    }
    if text.len() > MAX_BODY {
        return (StatusCode::PAYLOAD_TOO_LARGE, "delivery too large\n").into_response();
    }

    // `try_send`, not `send`: a listener is a machine and must never be given a reason to
    // hold a connection open against a busy agent. A full queue means the inbox is
    // saturated, which the cadence glance-up recovers from — the same fallback every
    // other lost nudge has.
    match state.duties.try_send(DutyDelivery { key, text }) {
        Ok(()) => (StatusCode::ACCEPTED, "queued\n").into_response(),
        Err(err) => {
            tracing::warn!(error = %err, "duty delivery dropped; inbox saturated or gone");
            (StatusCode::SERVICE_UNAVAILABLE, "duty inbox unavailable\n").into_response()
        }
    }
}
