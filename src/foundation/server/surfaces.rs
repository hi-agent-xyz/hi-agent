//! The surface endpoints — exchange a credential, pair a new one, list and
//! revoke.
//!
//! The gate itself lives in [`crate::foundation::surfaces`]; this is only its
//! HTTP face. Four routes, and the split between them is who may call:
//!
//! - `POST /api/session` is **open**, because it is how anything stops being
//!   unauthorized. It takes either a credential or a one-time pairing code.
//! - `POST /api/pair`, `GET /api/surfaces`, `DELETE /api/surfaces/{id}` are
//!   gated like everything else, so pairing a phone means asking from the Mac —
//!   or from the machine itself, which is `authorized_keys` again.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::foundation::server::AppState;
use crate::foundation::surfaces;

#[derive(Debug, Default, Deserialize)]
struct SessionBody {
    /// What to call this surface in the device list. Only read when a pairing
    /// code is being spent — an existing credential already has its label.
    #[serde(default)]
    label: String,
}

/// `POST /api/session` — exchange the long-lived credential for a short session.
///
/// Two callers, one route. An app sends its credential and keeps the cookie *and*
/// the bearer; a browser sends a pairing code, and the credential minted for it
/// comes back in the body for an app to store and is simply ignored by the page,
/// which keeps the cookie instead. That asymmetry is the point: the webview never
/// holds a credential, and the row behind the cookie is still what revocation
/// removes.
pub async fn post_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Some(presented) = bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, "present a credential as a bearer token\n")
            .into_response();
    };
    // Parsed leniently: an app with nothing to say sends no body at all, and a
    // missing label is a default, not a failure.
    let parsed: SessionBody = serde_json::from_str(&body).unwrap_or_default();
    let label = if parsed.label.trim().is_empty() {
        "paired surface".to_string()
    } else {
        parsed.label.trim().chars().take(80).collect()
    };

    let Some((session, id, minted)) = state.surfaces.exchange(&presented, &label) else {
        return (StatusCode::UNAUTHORIZED, "that credential was not accepted\n").into_response();
    };
    tracing::info!(surface = %id, paired = minted.is_some(), "surface session opened");

    let cookie = surfaces::session_cookie(
        &session,
        &cookie_path(&headers),
        surfaces::over_tls(&headers),
    );
    let payload = serde_json::json!({ "id": id, "credential": minted });
    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        axum::Json(payload),
    )
        .into_response()
}

/// `POST /api/pair` — mint a one-time pairing code and the URL that carries it.
///
/// The same shape as the phone-upload handoff (`/api/handoff`), for the same
/// reason: the thing being handed across is a short-lived grant, and a QR is how
/// it crosses to a device with no keyboard worth using.
pub async fn post_pair(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let code = state.surfaces.mint_pairing_code();
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("localhost");
    let scheme = if surfaces::over_tls(&headers) { "https" } else { "http" };
    // No trailing slash on a prefixed address: `https://hi-agent.xyz/ana` is what
    // the community calls this core, and what a person reads off a QR should be
    // the same string, not a variant of it. Only a core at its own root gets the
    // bare `/`, because `https://host` with no path at all is a stranger thing to
    // hand someone than `https://host/`.
    let prefix = surfaces::base_path(&headers);
    let url = if prefix.is_empty() {
        format!("{scheme}://{host}/")
    } else {
        format!("{scheme}://{host}{prefix}")
    };
    let app_url = pairing_app_url(&url, &code);
    tracing::info!("pairing code minted");
    axum::Json(serde_json::json!({
        "code": code,
        "url": url,
        "app_url": app_url,
        "expires_in": 600
    }))
    .into_response()
}

fn pairing_app_url(core_url: &str, code: &str) -> String {
    let mut url = url::Url::parse("hiagent://pair").expect("the static pairing URL is valid");
    url.query_pairs_mut()
        .append_pair("url", core_url)
        .append_pair("code", code);
    url.to_string()
}

/// `GET /api/surfaces` — the device list. Labels, when each was added, and when
/// each was last seen; never a credential.
pub async fn get_surfaces(State(state): State<Arc<AppState>>) -> Response {
    match surfaces::store::list(state.surfaces.data_dir()) {
        Ok(list) => axum::Json(serde_json::json!({ "surfaces": list })).into_response(),
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "listing surfaces");
            (StatusCode::INTERNAL_SERVER_ERROR, "could not read the surface list\n")
                .into_response()
        }
    }
}

/// `DELETE /api/surfaces/{id}` — revoke one surface, here at the core. No
/// community is involved, so losing a phone does not need the community to be
/// reachable, or trusted, to fix.
pub async fn delete_surface(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    match state.surfaces.revoke(&id) {
        Ok(true) => {
            tracing::info!(surface = %id, "surface revoked");
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, "no such surface\n").into_response(),
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "revoking a surface");
            (StatusCode::INTERNAL_SERVER_ERROR, "could not revoke\n").into_response()
        }
    }
}

#[cfg(test)]
mod pairing_url_tests {
    use super::pairing_app_url;

    #[test]
    fn the_app_pairing_url_round_trips_a_prefixed_core_and_code() {
        let app_url = pairing_app_url("https://hi-agent.xyz/ana", "code-with_-symbols");
        let parsed = url::Url::parse(&app_url).expect("app URL");
        assert_eq!(parsed.scheme(), "hiagent");
        assert_eq!(parsed.host_str(), Some("pair"));

        let query: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(
            query.get("url").map(String::as_str),
            Some("https://hi-agent.xyz/ana")
        );
        assert_eq!(
            query.get("code").map(String::as_str),
            Some("code-with_-symbols")
        );
    }
}

/// `GET /healthz` — the process is alive. Open by definition, and the only route
/// that answers before anything has been paired.
pub async fn get_healthz() -> Response {
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], "ok\n").into_response()
}

/// The cookie's `Path` — [`surfaces::base_path`] as a path, so the root case is
/// `/` rather than the empty string a URL wants.
///
/// `Path=/ana` limits transmission between cores on the shared origin, which is a
/// scoping, not a security boundary (see `topology.md`).
fn cookie_path(headers: &HeaderMap) -> String {
    let prefix = surfaces::base_path(headers);
    if prefix.is_empty() { "/".to_string() } else { prefix }
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let rest = v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer "))?;
    let rest = rest.trim();
    (!rest.is_empty()).then(|| rest.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn the_cookie_path_takes_a_prefix_and_refuses_nonsense() {
        let mut h = HeaderMap::new();
        assert_eq!(cookie_path(&h), "/");

        h.insert("x-forwarded-prefix", HeaderValue::from_static("/ana"));
        assert_eq!(cookie_path(&h), "/ana");

        h.insert("x-forwarded-prefix", HeaderValue::from_static("/ana/"));
        assert_eq!(cookie_path(&h), "/ana");

        // A prefix is a path, and a relative or climbing one is not one we will
        // scope a cookie to.
        h.insert("x-forwarded-prefix", HeaderValue::from_static("ana"));
        assert_eq!(cookie_path(&h), "/");
        h.insert("x-forwarded-prefix", HeaderValue::from_static("/../admin"));
        assert_eq!(cookie_path(&h), "/");
    }
}
