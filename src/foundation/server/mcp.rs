//! HTTP glue for the MCP tool endpoint.
//!
//! Binds the MCP "Streamable HTTP" transport to the reaction's tool carrier
//! ([`crate::foundation::mcp`]). A POST carries one JSON-RPC message; we route by the
//! `X-HI-Role`/`X-HI-Session-Slug` headers a session's MCP attach sets
//! (see `agent::AgentLayer::session`). A request gets a single `application/json`
//! response; a notification gets `202`. We push no server-initiated messages, so
//! the optional GET SSE stream is declined with `405`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde_json::Value;

use crate::foundation::config::{HEADER_ROLE, HEADER_SESSION_SLUG};
use crate::foundation::mcp::{self, McpReply};
use crate::foundation::server::AppState;

/// One MCP message over POST. Parses the JSON-RPC body, resolves the routing
/// identity from headers, and returns either a JSON-RPC response or an empty 202.
pub async fn post_mcp(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let role = header(HEADER_ROLE);
    let slug = header(HEADER_SESSION_SLUG)
        .and_then(|v| v.parse::<crate::foundation::registry::SessionSlug>().ok());

    let msg: Value = match serde_json::from_slice(body.as_ref()) {
        Ok(v) => v,
        Err(err) => {
            return (StatusCode::BAD_REQUEST, format!("invalid JSON-RPC body: {err}"))
                .into_response();
        }
    };

    match mcp::handle(
        &state.tool_registry,
        &state.data_dir,
        &state.privacy,
        &state.video_in_partial,
        &state.observatory,
        role.as_deref(),
        slug,
        &msg,
    )
    .await
    {
        McpReply::Json(value) => Json(value).into_response(),
        McpReply::Accepted => StatusCode::ACCEPTED.into_response(),
    }
}

/// The optional server→client SSE stream — declined; we never push to the agent.
pub async fn get_mcp() -> Response {
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}
