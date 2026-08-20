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
    // Transport metadata passes through verbatim. This proxy exists to rewrite two
    // things — the body (the projection) and the credential — and a header it did not
    // have to touch is a header it must not touch. The rule used to be a whitelist, and
    // it silently ate everything codex 0.147 uses to identify a turn upstream:
    // `Session-Id`, `Thread-Id`, `Originator`, `X-Client-Request-Id`, `X-Codex-*`, and
    // the `X-Openai-Internal-*` feature flags. Worse, neither prefix it allowed matched
    // anything codex actually sends (`openai-` never matches `x-openai-…`, and
    // `x-stainless-` is a TS/Python SDK convention the Rust client has no idea about),
    // so the effective forward set was `Accept` + `User-Agent`.
    //
    // Headers are *not* projected. The projector's contract is the serialized Responses
    // request (docs/arch/privacy.md § Projection); codex's transport metadata is ids and
    // feature flags it generated itself, never person-supplied text.
    let mut outgoing = state.privacy.http().post(url);
    for (name, value) in &headers {
        if forwards_upstream(name.as_str()) {
            outgoing = outgoing.header(name, value);
        }
    }
    // After the pass-through, so a forwarded `Content-Type` wins over the default; and
    // `bearer_auth` *appends*, so it may only run with the client's own `Authorization`
    // already excluded above.
    let outgoing = outgoing.json(&request).bearer_auth(&upstream.upstream_key);

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

/// Whether an inbound request header rides along to the provider.
///
/// Everything does, except the few this proxy is obliged to own:
///
/// - `authorization` — the child holds only the per-boot proxy token; the upstream
///   credential is substituted here and never enters the codex process.
/// - `host` — the client addressed loopback; reqwest derives the real one from the URL.
/// - `content-length` — projection changes the body's length; reqwest recomputes it.
/// - `accept-encoding` — we decode the upstream response before restreaming it, so the
///   content coding is negotiated between this proxy and the provider, not end to end.
/// - hop-by-hop headers, which are per-connection by definition.
fn forwards_upstream(name: &str) -> bool {
    !matches!(
        name,
        "authorization" | "host" | "content-length" | "accept-encoding"
    ) && !is_hop_by_hop(name)
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

    /// The exact header set `codex 0.147.0` puts on a Responses request, captured off the
    /// wire. Every one of these reaches the provider — the whitelist this replaced
    /// forwarded only the first two.
    #[test]
    fn codex_transport_metadata_reaches_the_provider() {
        for name in [
            "accept",
            "user-agent",
            "originator",
            "session-id",
            "thread-id",
            "x-client-request-id",
            "x-codex-beta-features",
            "x-codex-turn-metadata",
            "x-codex-window-id",
            "x-openai-internal-codex-responses-lite",
            "content-type",
        ] {
            assert!(forwards_upstream(name), "{name} must reach the provider");
        }
    }

    #[test]
    fn the_proxy_keeps_only_what_it_must_rewrite() {
        // The credential is substituted; the client's own never rides along.
        assert!(!forwards_upstream("authorization"));
        // Loopback host, pre-projection length, and a coding we terminate ourselves.
        assert!(!forwards_upstream("host"));
        assert!(!forwards_upstream("content-length"));
        assert!(!forwards_upstream("accept-encoding"));
        // Per-connection by definition.
        assert!(!forwards_upstream("connection"));
        assert!(!forwards_upstream("transfer-encoding"));
    }
}
