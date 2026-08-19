use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures::StreamExt;
use serde_json::Value;

use crate::foundation::config::AgentConfig;
use crate::foundation::server::AppState;

pub const MODEL_PROXY_BASE_PATH: &str = "/internal/model/v1";

pub async fn post_responses(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(token) = bearer_token(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if !state.privacy.accepts_proxy_token(token) {
        return StatusCode::FORBIDDEN.into_response();
    }

    let mut request: Value = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("invalid Responses request JSON: {error}"),
            )
                .into_response();
        }
    };
    let findings = match state.privacy.filter().project_json(&mut request) {
        Ok(findings) => findings,
        Err(error) => {
            tracing::error!(error = %error, "privacy projection failed; blocking model request");
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                "model request blocked: privacy projection failed",
            )
                .into_response();
        }
    };
    if !findings.is_empty() {
        let secret_refs = findings
            .iter()
            .filter(|finding| finding.reference.is_some())
            .count();
        tracing::info!(
            findings = findings.len(),
            secret_refs,
            "projected sensitive data at the external model boundary"
        );
    }

    let upstream = AgentConfig::resolve(&state.data_dir);
    if !upstream.is_configured() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "no upstream LLM credential is configured",
        )
            .into_response();
    }
    let url = responses_url(&upstream.upstream_base_url);
    let mut outgoing = state
        .privacy
        .http()
        .post(url)
        .bearer_auth(&upstream.upstream_key)
        .json(&request);
    for name in [header::ACCEPT, header::USER_AGENT] {
        if let Some(value) = headers.get(&name) {
            outgoing = outgoing.header(name, value);
        }
    }
    for (name, value) in &headers {
        if name.as_str().starts_with("openai-") || name.as_str().starts_with("x-stainless-") {
            outgoing = outgoing.header(name, value);
        }
    }

    let response = match outgoing.send().await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(error = %error, "external model transport failed");
            return (StatusCode::BAD_GATEWAY, "external model transport failed").into_response();
        }
    };

    let status = response.status();
    let response_headers = response.headers().clone();
    let stream = response
        .bytes_stream()
        .map(|chunk| chunk.map_err(std::io::Error::other));
    let mut builder = Response::builder().status(status);
    for (name, value) in &response_headers {
        if !is_hop_by_hop(name.as_str()) && name != header::CONTENT_LENGTH {
            builder = builder.header(name, value);
        }
    }
    builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

fn responses_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with("/responses") {
        base.to_string()
    } else {
        format!("{base}/responses")
    }
}

fn is_hop_by_hop(name: &str) -> bool {
    matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_roots_and_full_routes_are_both_accepted() {
        assert_eq!(
            responses_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/responses"
        );
        assert_eq!(
            responses_url("https://gateway.example/v1/responses/"),
            "https://gateway.example/v1/responses"
        );
    }
}
