//! Axum extractors for the spec's identity headers.
//!
//! - `Authorization: Bearer ...` is accepted but not validated in v0.

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;

const HDR_STREAM: &str = "x-hi-stream";
const HDR_AUTH: &str = "authorization";
const HDR_FACE: &str = "x-hi-face";

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

/// `X-HI-Face`. Which attached face is asking — the id it mints in `stageReport.ts`
/// and reports its frame under on `POST /api/stage`. Read by the two view calls a
/// person makes (`GET /api/views`, `POST /api/views/open`), so a thumbnail is rendered
/// at the frame of whoever is looking at the band rather than at whichever face
/// reported most recently.
///
/// **Not `x-hi-surface`**, which is taken: that is the CSRF header
/// ([`hi_wire::CSRF_HEADER`]) and carries a constant `1`. The two would collide, and
/// this repo calls a window a *face* everywhere else anyway.
///
/// Absent reads as `None` — every render then falls back to the primary surface, which
/// is exactly the behaviour from before the header existed.
#[derive(Debug, Clone)]
pub struct FaceHeader(pub Option<String>);

impl<S> FromRequestParts<S> for FaceHeader
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let face = parts
            .headers
            .get(HDR_FACE)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        Ok(FaceHeader(face))
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
