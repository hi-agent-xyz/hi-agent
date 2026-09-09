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

use crate::foundation::community;
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

    let cookie = surfaces::session_cookie(&session, surfaces::over_tls(&headers));
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
    let url = public_base_url(&state.data_dir, &headers).await;
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

/// The address to hand another device — the one a phone should dial, not the one
/// the asking request happened to arrive on.
///
/// **The claimed name comes first, and that is the whole point of this function.**
/// A QR is only ever read by a *different* machine, and the panel that shows it is
/// opened from the app on this one, over loopback — so the request's own `Host` is
/// `127.0.0.1`, which is the one address that cannot work anywhere else. The
/// community says where this core is (`Handle::base_url`, e.g.
/// `https://ana.hi-agent.xyz`), and the name on the same screen is read from the
/// same place, so the QR and the address a person can see now agree.
///
/// The request falls back to naming itself when there is no name yet, no account,
/// or no community reachable: a core with no name is a normal core, and pairing
/// from the same network still works if the browser reached it by an address that
/// is not loopback. That address is the best guess available, not a good one — a
/// core with no name has nothing better to say about where it is.
///
/// A relayed address is `https://ana.hi-agent.xyz`, which is what a person reads
/// off the screen, and what they scan should be the same string. `Url`'s own
/// normalisation gives the trailing slash a bare host wants.
pub(crate) async fn public_base_url(
    data_dir: &std::path::Path,
    headers: &HeaderMap,
) -> String {
    // The name this core *serves*, not the first one the account ever took: a QR
    // carrying a name nothing is dialling is a pairing that fails with "asleep"
    // on the other device, which reads as the app being broken.
    let named = community::current(data_dir).await.ok().and_then(|names| {
        let serving = crate::foundation::tunnel::choose(data_dir, &names.handles)?;
        names
            .handles
            .iter()
            .find(|h| h.handle == serving)
            .map(|h| h.base_url.trim().trim_end_matches('/').to_string())
            .filter(|base| !base.is_empty())
    });
    let raw = named.unwrap_or_else(|| {
        let host =
            headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("localhost");
        let scheme = if surfaces::over_tls(headers) { "https" } else { "http" };
        format!("{scheme}://{host}")
    });
    url::Url::parse(&raw).map(|u| u.to_string()).unwrap_or(raw)
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
pub async fn get_surfaces(
    State(state): State<Arc<AppState>>,
    acceptor: Option<axum::Extension<surfaces::Acceptor>>,
) -> Response {
    // Whether this caller may register a device to a person — the page needs to know
    // before it draws, or it shows a control that would only ever be refused.
    let settable = acceptor.map(|axum::Extension(a)| a) == Some(surfaces::Acceptor::Loopback);
    match surfaces::store::list(state.surfaces.data_dir()) {
        Ok(list) => axum::Json(serde_json::json!({
            "surfaces": list,
            "subject_settable": settable,
        }))
        .into_response(),
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "listing surfaces");
            (StatusCode::INTERNAL_SERVER_ERROR, "could not read the surface list\n")
                .into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct SubjectBody {
    /// The `people/` subject this device belongs to, or empty to unregister it.
    subject: String,
}

/// `POST /api/surfaces/{id}/subject` — register this device to a person.
///
/// **What it buys is attribution on the addressed channels**: what somebody types,
/// hands over, or opens on a registered device is recorded as theirs, with basis
/// `stated`, in preference to the install's owner default. A device nobody registered
/// states nothing and falls through to that default.
///
/// **It says nothing about the microphone.** A microphone records whatever was
/// audible, and no property of the device it is attached to changes that — a phone on
/// a train hears the train. Audio is answered by the voiceprint or by nobody.
///
/// **Loopback only**, for the same reason declaring the install's owner is: this
/// silently changes who future messages are attributed to, and the gate lets any
/// paired client reach this router — so without the check a borrowed phone could
/// register itself to the owner and have everything typed on it filed under them.
/// Revoking and renaming stay reachable from anywhere; they are corrections.
pub async fn post_subject(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    acceptor: Option<axum::Extension<surfaces::Acceptor>>,
    axum::Json(body): axum::Json<SubjectBody>,
) -> Response {
    if acceptor.map(|axum::Extension(a)| a) != Some(surfaces::Acceptor::Loopback) {
        return (StatusCode::FORBIDDEN, "registering a device is loopback-only\n").into_response();
    }
    let subject = crate::mind::memory::facets::slug(&body.subject);
    if subject.is_empty() && !body.subject.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "subject must contain a usable character\n")
            .into_response();
    }
    match surfaces::store::bind(state.surfaces.data_dir(), &id, &subject) {
        Ok(true) => {
            tracing::info!(surface = %id, subject = %subject, "surface registered to a person");
            axum::Json(serde_json::json!({
                "ok": true,
                "subject": (!subject.is_empty()).then_some(subject),
            }))
            .into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, "no such surface\n").into_response(),
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "registering a surface");
            (StatusCode::INTERNAL_SERVER_ERROR, "could not write the surface\n").into_response()
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
    use super::{HeaderMap, header, pairing_app_url, public_base_url};

    /// A core with no account cannot ask the community where it is, so it names
    /// itself off the request: scheme and host, and nothing after them — a core
    /// is the whole of its own origin, so there is no path to carry.
    #[tokio::test]
    async fn with_no_name_the_pairing_address_is_what_the_request_says() {
        let dir = std::env::temp_dir().join(format!("hi-pair-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp data dir");

        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:12358".parse().unwrap());
        assert_eq!(public_base_url(&dir, &headers).await, "http://127.0.0.1:12358/");

        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        assert_eq!(public_base_url(&dir, &headers).await, "https://127.0.0.1:12358/");
    }

    #[test]
    fn the_app_pairing_url_round_trips_a_core_address_and_code() {
        let app_url = pairing_app_url("https://ana.hi-agent.xyz", "code-with_-symbols");
        let parsed = url::Url::parse(&app_url).expect("app URL");
        assert_eq!(parsed.scheme(), "hiagent");
        assert_eq!(parsed.host_str(), Some("pair"));

        let query: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(
            query.get("url").map(String::as_str),
            Some("https://ana.hi-agent.xyz")
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

fn bearer(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let rest = v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer "))?;
    let rest = rest.trim();
    (!rest.is_empty()).then(|| rest.to_string())
}

