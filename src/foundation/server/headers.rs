//! Axum extractors for the spec's identity headers.
//!
//! - `Authorization: Bearer ...` is accepted but not validated in v0.

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;

const HDR_STREAM: &str = "x-hi-stream";
const HDR_AUTH: &str = "authorization";

/// `X-HI-Stream`. Names one source among several feeding a channel (`webcam`,
/// `headset`), so the reaction can tell concurrent sources apart. Defaults to
/// `None` — the default stream — when missing or empty, so a client that never
/// sets it behaves exactly as before. This is the single place `""` is folded to
/// `None`, so a bare default never leaks downstream as `Some("")`.
#[derive(Debug, Clone)]
pub struct StreamHeader(pub Option<String>);

impl<S> FromRequestParts<S> for StreamHeader
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let stream = parts
            .headers
            .get(HDR_STREAM)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        Ok(StreamHeader(stream))
    }
}

/// Optional `Authorization: Bearer ...`. Logged, not validated.
#[derive(Debug, Clone)]
pub struct AuthBearer(pub Option<String>);

impl<S> FromRequestParts<S> for AuthBearer
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let Some(value) = parts.headers.get(HDR_AUTH) else {
            return Ok(AuthBearer(None));
        };
        let s = value
            .to_str()
            .map_err(|_| (StatusCode::BAD_REQUEST, "invalid Authorization"))?
            .trim();
        let token = s.strip_prefix("Bearer ").or_else(|| s.strip_prefix("bearer "));
        match token {
            Some(t) if !t.is_empty() => {
                tracing::debug!(token = %t, "authorization bearer token (not validated)");
                Ok(AuthBearer(Some(t.to_owned())))
            }
            _ => Ok(AuthBearer(None)),
        }
    }
}
