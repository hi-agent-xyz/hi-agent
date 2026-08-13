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

/// `GET /api/handle` — the names this account owns, and how many it may.
///
/// An empty list is a normal answer: a core with no name works, and is simply
/// reachable from its own machine only.
pub async fn get_handle(State(state): State<Arc<AppState>>) -> Response {
    match community::current(&state.data_dir).await {
        Ok(h) => axum::Json(h).into_response(),
        // No account, or no community to ask: this core has no name, which is a
        // normal state and not a failure. Answering with an error would make a
        // first run look broken on a screen whose whole job is to say what is
        // there. The reason rides along so the page can show it.
        Err(e) => {
            tracing::debug!(error = %e, "this core has no name");
            axum::Json(serde_json::json!({
                "handles": [],
                "limit": 0,
                "why": e.to_string(),
            }))
            .into_response()
        }
    }
}

/// `POST /api/handle` — claim a name, permanently.
///
/// The registry's refusal travels through verbatim: "that handle is in use", or
/// "sign in first", is the whole content of the answer, and a person choosing a
/// name is entitled to the reason rather than a status code.
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
