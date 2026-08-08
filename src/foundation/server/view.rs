//! GET /api/out/view — long-poll for the retained appearance state.
//!
//! Appearance is state, not a stream: the response is the whole set of
//! active views (z-ordered) plus a version, served by the [`ViewBus`]. A call
//! without `?since=` returns the current state immediately — even when empty —
//! so a fresh page syncs on open; passing the last seen version parks until
//! the state changes. The reaction mutates the state when the agent calls
//! `show` and the view compiler has turned its source into a module.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::IntoResponse;

use crate::foundation::server::AppState;
use crate::foundation::server::headers::AuthBearer;

#[derive(serde::Deserialize)]
pub struct ViewQuery {
    /// The last appearance version this client has rendered; the response is
    /// held until the appearance version exceeds it.
    since: Option<u64>,
}

pub async fn get_out_view(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ViewQuery>,
    AuthBearer(auth): AuthBearer,
) -> impl IntoResponse {
    // A held view long-poll = a screen is attached; counted until this handler returns.
    let _presence = state.presence.connect(crate::body::presence::OutChannel::View);

    tracing::info!(since = ?query.since, auth = ?auth, "GET /api/out/view long-poll opened");

    // Opening this long-poll is a presence signal: warm up so the
    // process + session + upstream cache are hot before the first utterance.
    state.warm();

    axum::Json(state.views.wait_state(query.since).await)
}

/// DELETE /api/out/view — clear the appearance (close all views, back
/// to the default empty room). A user control: the screen is the agent's
/// presentation, but the user can reclaim it. The clear bumps the version, so
/// every device's long-poll converges on the empty state. Returns 204.
pub async fn clear_out_view(
    State(state): State<Arc<AppState>>,
    AuthBearer(auth): AuthBearer,
) -> impl IntoResponse {
    tracing::info!(auth = ?auth, "DELETE /api/out/view — user cleared the screen");
    state.views.clear().await;
    axum::http::StatusCode::NO_CONTENT
}
