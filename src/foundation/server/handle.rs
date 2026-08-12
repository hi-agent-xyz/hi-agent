//! The handle — this core's address in the community.
//!
//! Two routes over [`crate::foundation::community`]: read what this core holds,
//! and claim or rename it. Gated like everything else, so claiming a name is
//! something a surface that already has access does — the community never
//! decides who may ask.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::foundation::community;
use crate::foundation::server::AppState;

#[derive(Deserialize)]
struct ClaimBody {
    handle: String,
}

/// `GET /api/handle` — what this core is called and where it is reachable.
///
/// Answers with an empty handle rather than an error when nothing is claimed: a
/// core with no name is a normal core, working, reachable from its own machine.
pub async fn get_handle(State(state): State<Arc<AppState>>) -> Response {
    match community::current(&state.data_dir).await {
        Ok(h) => axum::Json(h).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "reading this core's handle");
            (StatusCode::BAD_GATEWAY, format!("{e}\n")).into_response()
        }
    }
}

/// `POST /api/handle` — claim a handle, or rename to a different one.
///
/// The registry's refusal travels through verbatim: "that handle is in use" is
/// the whole content of a conflict, and a person choosing a name is entitled to
/// the reason rather than a status code.
pub async fn post_handle(State(state): State<Arc<AppState>>, body: String) -> Response {
    let Ok(req) = serde_json::from_str::<ClaimBody>(&body) else {
        return (StatusCode::BAD_REQUEST, "expected {handle}\n").into_response();
    };
    let handle = req.handle.trim().to_ascii_lowercase();
    match community::claim(&state.data_dir, &handle).await {
        Ok(h) => {
            tracing::info!(handle = %h.handle, base_url = %h.base_url, "handle claimed");
            axum::Json(h).into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "claiming a handle");
            (StatusCode::CONFLICT, format!("{e}\n")).into_response()
        }
    }
}
