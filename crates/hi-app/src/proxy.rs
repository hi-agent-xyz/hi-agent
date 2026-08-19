//! The local proxy — the only address the face ever knows.
//!
//! Everything the face asks for goes to `http://127.0.0.1:<app port>` and is
//! forwarded to whichever core is attached, with this app's credential added on
//! the way out. Loopback, direct and relayed are three carriers of one protocol,
//! so there is nothing here that knows which of them it is talking over: one
//! request in, the same request out against a different base URL.
//!
//! The app keeps one namespace for itself, `/api/app/*` — the roster, and
//! attaching. Nothing else is ours, and the core will never serve that prefix.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{FromRequestParts, Path, Request, State, WebSocketUpgrade, ws};
use axum::http::{HeaderMap, HeaderName, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;

use super::{App, roster};

/// Headers that describe *this* hop and must not be forwarded to the next one.
/// `host` goes too — the upstream's host comes from its own base URL.
const HOP_BY_HOP: &[HeaderName] = &[
    header::CONNECTION,
    header::PROXY_AUTHENTICATE,
    header::PROXY_AUTHORIZATION,
    header::TE,
    header::TRAILER,
    header::TRANSFER_ENCODING,
    header::UPGRADE,
    header::HOST,
];

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        // The app's own screen. `/app` is the app's and never forwarded, which is
        // why it has to be a path no core serves — a browser pointed straight at a
        // core gets nothing here, correctly: a core has no roster.
        .route("/app", get(get_screen))
        .route("/app/", get(get_screen))
        .route("/api/app/roster", get(get_roster).post(post_roster))
        .route("/api/app/roster/{id}", axum::routing::delete(delete_roster))
        .route("/api/app/roster/{id}/health", get(get_health))
        .route("/api/app/roster/{id}/attach", post(post_attach))
        .with_state(app.clone())
        .fallback(axum::routing::any(forward))
        .with_state(app)
}

// -----------------------------------------------------------------------------
// The app's own surface
// -----------------------------------------------------------------------------

/// `GET /app` — the roster, as a page. See [`crate::screen`] for why the
/// app serves this itself and why it carries no assets.
async fn get_screen() -> Response {
    axum::response::Html(crate::screen::page()).into_response()
}

/// `GET /api/app/roster` — the cores this app may be with, and which one it is
/// with now. Credentials are never in it: the face renders this, and the face is
/// exactly what must not hold one.
async fn get_roster(State(app): State<Arc<App>>) -> Response {
    match roster::list(app.data_dir()) {
        Ok(entries) => axum::Json(serde_json::json!({ "roster": entries })).into_response(),
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "reading the roster");
            (StatusCode::INTERNAL_SERVER_ERROR, "could not read the roster\n").into_response()
        }
    }
}

#[derive(Deserialize)]
struct AddBody {
    base_url: String,
    /// A pairing code from the core, or a credential already in hand.
    code: String,
    #[serde(default)]
    label: String,
}

/// `POST /api/app/roster` — pair with a core and add it. Adding a core *is*
/// acquiring a credential for it; there is no other way to be on the list.
async fn post_roster(State(app): State<Arc<App>>, body: String) -> Response {
    let Ok(add) = serde_json::from_str::<AddBody>(&body) else {
        return (StatusCode::BAD_REQUEST, "expected {base_url, code, label?}\n").into_response();
    };
    let label = if add.label.trim().is_empty() { "a core" } else { add.label.trim() };
    match app.pair(&add.base_url, add.code.trim(), label).await {
        Ok(id) => axum::Json(serde_json::json!({ "id": id })).into_response(),
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "pairing with a core");
            (StatusCode::BAD_GATEWAY, format!("{e}\n")).into_response()
        }
    }
}

/// `DELETE /api/app/roster/{id}` — forget a core. Local to this app; the
/// credential stays live at the core until it is revoked *there*, which is what
/// lets losing a device be fixed without the device.
async fn delete_roster(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    app.drop_session(&id);
    match roster::forget(app.data_dir(), &id) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "no such entry\n").into_response(),
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "forgetting a core");
            (StatusCode::INTERNAL_SERVER_ERROR, "could not forget it\n").into_response()
        }
    }
}

/// `GET /api/app/roster/{id}/health` — does this core answer, and how.
///
/// Per entry rather than folded into the roster listing: a core that is off takes
/// the whole timeout to say so, and one unreachable entry must not hold up the
/// list of the others.
async fn get_health(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    let Ok(entries) = roster::list(app.data_dir()) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "could not read the roster\n").into_response();
    };
    let Some(entry) = entries.into_iter().find(|e| e.id == id) else {
        return (StatusCode::NOT_FOUND, "no such entry\n").into_response();
    };
    let state = app.reachable(&entry.base_url).await;
    axum::Json(serde_json::json!({ "state": state })).into_response()
}

/// `POST /api/app/roster/{id}/attach` — be with this one now.
///
/// The face is not told: it is repointed. Its channels drop when the proxy
/// changes upstream, it reconnects the way it does after any interruption, and
/// what comes back is the other core's conversation whole — which is exactly the
/// reconnection the transcript already covers.
async fn post_attach(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    match roster::attach(app.data_dir(), &id) {
        Ok(()) => {
            tracing::info!(entry = %id, "attached");
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => (StatusCode::NOT_FOUND, format!("{e}\n")).into_response(),
    }
}

// -----------------------------------------------------------------------------
// Everything else: the attached core's own API, unchanged
// -----------------------------------------------------------------------------

async fn forward(State(app): State<Arc<App>>, req: Request) -> Response {
    let Some(entry) = roster::attached(app.data_dir()) else {
        return (StatusCode::SERVICE_UNAVAILABLE, "this app is not with anyone yet\n")
            .into_response();
    };
    if is_websocket(req.headers()) {
        return forward_ws(app, entry, req).await;
    }
    forward_http(app, entry, req).await
}

/// Split a base URL into its origin and its path prefix, if it has one.
/// `https://hi-agent.xyz/ana` → `("https://hi-agent.xyz", "/ana")`;
/// `http://localhost:12358` → `("http://localhost:12358", "")`.
fn split_base(base_url: &str) -> (&str, &str) {
    let base = base_url.trim_end_matches('/');
    let after_scheme = base.find("://").map(|i| i + 3).unwrap_or(0);
    match base[after_scheme..].find('/') {
        Some(i) => base.split_at(after_scheme + i),
        None => (base, ""),
    }
}

/// Where a path the face asked for actually lives.
///
/// A relayed core is under a subpath and is told so by the relay's
/// `x-forwarded-prefix`, so every root-absolute path it emits — `/ana/assets/*`,
/// the import map, `window.__HI_BASE__` — already carries the prefix. The face
/// asks this app for those paths verbatim, so pasting them onto a base URL that
/// *ends* in `/ana` requests `/ana/ana/assets/*`, which is a 404 from the core.
/// The prefix belongs to the address, not to the request: strip the one the path
/// already carries before the base URL puts it back.
///
/// A path that merely starts with the same letters (`/analytics`) is not
/// prefixed by `/ana` — only a whole segment counts.
fn upstream_url(base_url: &str, path_and_query: &str) -> String {
    let (origin, prefix) = split_base(base_url);
    if prefix.is_empty() {
        return format!("{origin}{path_and_query}");
    }
    let rest = match path_and_query.strip_prefix(prefix) {
        Some(r) if r.is_empty() => "/",
        Some(r) if r.starts_with('/') || r.starts_with('?') => r,
        _ => path_and_query,
    };
    let sep = if rest.starts_with('?') { "/" } else { "" };
    format!("{origin}{prefix}{sep}{rest}")
}

async fn forward_http(app: Arc<App>, entry: roster::Entry, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let path_and_query = parts.uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
    let url = upstream_url(&entry.base_url, path_and_query);

    let mut out = app.client.request(parts.method.clone(), &url);
    for (name, value) in forwardable(&parts.headers) {
        out = out.header(name, value);
    }
    if let Some(cookie) = app.session(&entry).await {
        out = out.header(header::COOKIE, cookie);
        // A proxied request is never a cross-site *simple* request — the browser
        // talked to this app, and this app constructed what goes upstream. So the
        // app asserts that about its own traffic rather than requiring the face to
        // know about a rule that is not the face's to satisfy. Only sent alongside
        // the session, because that is the only presentation the rule is about.
        out = out.header(hi_wire::CSRF_HEADER, "1");
    }

    let res = match out.body(reqwest::Body::wrap_stream(body.into_data_stream())).send().await {
        Ok(res) => res,
        Err(e) => {
            tracing::warn!(core = %entry.label, %url, error = %e, "the core did not answer");
            return (StatusCode::BAD_GATEWAY, format!("{} did not answer\n", entry.label))
                .into_response();
        }
    };

    // A session that lapsed and a credential that was revoked look identical
    // from here, so drop the cached session and let the next request find out
    // which it was. The body is a stream and cannot be replayed, so this one is
    // still answered honestly with the core's own 401.
    if res.status() == StatusCode::UNAUTHORIZED {
        app.drop_session(&entry.id);
    }

    let mut response = Response::builder().status(res.status());
    for (name, value) in res.headers() {
        if HOP_BY_HOP.contains(name) || name == header::SET_COOKIE {
            // The core's cookie is for the core's own origin and is this app's
            // to hold, not the browser's — handing it down would put a
            // credential-bearing cookie in the very place the design keeps
            // credentials out of.
            continue;
        }
        response = response.header(name, value);
    }
    response
        .body(Body::from_stream(res.bytes_stream()))
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}

/// Bridge one WebSocket. The two capture channels (`/api/in/audio/stream`,
/// `/api/in/vision/stream`) ride this, which is why a remote surface can hold a
/// mic at all.
///
/// Frames are re-encoded rather than the connection being spliced byte-for-byte:
/// splicing would need a TLS client of its own for the relayed shape, and
/// nothing we serve negotiates a subprotocol or an extension that survives one
/// hop and not the other.
async fn forward_ws(app: Arc<App>, entry: roster::Entry, req: Request) -> Response {
    let (mut parts, _) = req.into_parts();
    let ws = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
        Ok(ws) => ws,
        Err(e) => return e.into_response(),
    };
    let path_and_query = parts.uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
    // Resolved as a URL first, then re-schemed: the prefix rule is the same one
    // the request path follows, and it must not be re-derived here.
    let upstream = upstream_url(&entry.base_url, path_and_query)
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    let cookie = app.session(&entry).await;

    ws.on_upgrade(move |client| async move {
        let mut request = match tokio_tungstenite::tungstenite::client::IntoClientRequest::
            into_client_request(upstream.as_str())
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "the core's stream address is not one");
                return;
            }
        };
        if let Some(cookie) = cookie {
            if let Ok(v) = cookie.parse() {
                request.headers_mut().insert(header::COOKIE, v);
            }
        }
        let (upstream, _) = match tokio_tungstenite::connect_async(request).await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(core = %entry.label, error = %e, "could not open the core's stream");
                return;
            }
        };
        bridge(client, upstream).await;
    })
}

/// Pump both directions until either side closes. Either end closing ends the
/// other: a half-open stream is a mic that is still recording into nothing.
async fn bridge(
    client: ws::WebSocket,
    upstream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    use tokio_tungstenite::tungstenite::Message as Up;

    let (mut client_tx, mut client_rx) = client.split();
    let (mut up_tx, mut up_rx) = upstream.split();

    let to_upstream = async {
        while let Some(Ok(msg)) = client_rx.next().await {
            let up = match msg {
                ws::Message::Text(t) => Up::Text(t.as_str().into()),
                ws::Message::Binary(b) => Up::Binary(b.into()),
                ws::Message::Ping(p) => Up::Ping(p.into()),
                ws::Message::Pong(p) => Up::Pong(p.into()),
                ws::Message::Close(_) => break,
            };
            if up_tx.send(up).await.is_err() {
                break;
            }
        }
        let _ = up_tx.close().await;
    };

    let to_client = async {
        while let Some(Ok(msg)) = up_rx.next().await {
            let down = match msg {
                Up::Text(t) => ws::Message::Text(t.as_str().into()),
                Up::Binary(b) => ws::Message::Binary(b.into()),
                Up::Ping(p) => ws::Message::Ping(p.into()),
                Up::Pong(p) => ws::Message::Pong(p.into()),
                Up::Close(_) => break,
                // A frame kind that only exists mid-reassembly upstream; there is
                // nothing to hand on.
                Up::Frame(_) => continue,
            };
            if client_tx.send(down).await.is_err() {
                break;
            }
        }
        let _ = client_tx.close().await;
    };

    tokio::select! {
        _ = to_upstream => {}
        _ = to_client => {}
    }
}

/// Every header worth passing on: the request's own, minus the ones that
/// describe this hop and minus any credential the face tried to set (it has
/// none, and if it ever did it would not be one this core accepts).
fn forwardable(headers: &HeaderMap) -> Vec<(HeaderName, axum::http::HeaderValue)> {
    headers
        .iter()
        .filter(|(name, _)| {
            !HOP_BY_HOP.contains(name)
                && *name != header::AUTHORIZATION
                && *name != header::COOKIE
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn is_websocket(headers: &HeaderMap) -> bool {
    headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use tower::ServiceExt as _;

    #[test]
    fn this_hop_stays_on_this_hop() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, HeaderValue::from_static("127.0.0.1:12357"));
        h.insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
        h.insert(header::ACCEPT, HeaderValue::from_static("text/event-stream"));
        h.insert(header::COOKIE, HeaderValue::from_static("hi_surface=someone-elses"));
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer nope"));

        let out = forwardable(&h);
        let names: Vec<_> = out.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["accept"], "the app supplies the credential, not the face");
    }

    /// A relayed core lives under a subpath, and the paths it emits already carry
    /// it. Concatenation asked the community for `/ana/ana/assets/*` and got a
    /// 404 for every asset, view and channel on the page — invisible until an
    /// entry's base URL has a path at all, which no loopback one does.
    #[test]
    fn a_subpath_is_part_of_the_address_and_is_added_once() {
        let relayed = "https://hi-agent.xyz/ana";

        // The first load, and then everything that page goes on to ask for.
        assert_eq!(upstream_url(relayed, "/"), "https://hi-agent.xyz/ana/");
        assert_eq!(
            upstream_url(relayed, "/ana/assets/index-abc.js"),
            "https://hi-agent.xyz/ana/assets/index-abc.js"
        );
        assert_eq!(
            upstream_url(relayed, "/ana/api/out/text?since=3"),
            "https://hi-agent.xyz/ana/api/out/text?since=3"
        );
        // The bare prefix, with and without a query, still names the page.
        assert_eq!(upstream_url(relayed, "/ana"), "https://hi-agent.xyz/ana/");
        assert_eq!(upstream_url(relayed, "/ana?x=1"), "https://hi-agent.xyz/ana/?x=1");

        // A whole segment, or nothing: `/analytics` is not under `/ana`.
        assert_eq!(
            upstream_url(relayed, "/analytics"),
            "https://hi-agent.xyz/ana/analytics"
        );

        // A core with no prefix is untouched — loopback and directly-public both.
        assert_eq!(
            upstream_url("http://localhost:12358", "/api/messages"),
            "http://localhost:12358/api/messages"
        );
        assert_eq!(
            upstream_url("https://agent.example.com/", "/api/messages"),
            "https://agent.example.com/api/messages"
        );
    }

    #[test]
    fn an_upgrade_is_recognized_however_it_is_spelled() {
        let mut h = HeaderMap::new();
        assert!(!is_websocket(&h));
        h.insert(header::UPGRADE, HeaderValue::from_static("WebSocket"));
        assert!(is_websocket(&h));
    }

    /// The roster screen is the app's, and it is reachable when nothing else is.
    ///
    /// Both facts are one route: `/app` must be answered by the app rather than
    /// forwarded, because the attached core neither serves it nor knows what a
    /// roster is — and because the two moments this screen exists for are adding
    /// your first core and finding the attached one asleep, when forwarding would
    /// reach nobody.
    #[tokio::test]
    async fn the_roster_screen_is_the_apps_own_and_needs_no_core() {
        let dir = tempfile::tempdir().expect("tempdir");
        // No roster entry at all: the emptiest an app gets, and exactly when a
        // person needs somewhere to add one.
        let app = Arc::new(App::new(dir.path().to_path_buf()).expect("app"));
        let router = router(app);

        for path in ["/app", "/app/"] {
            let res = router
                .clone()
                .oneshot(
                    Request::builder().uri(path).body(axum::body::Body::empty()).unwrap(),
                )
                .await
                .expect("response");
            assert_eq!(res.status(), StatusCode::OK, "{path} is the app's own");
            let kind = res
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            assert!(kind.starts_with("text/html"), "{path} serves a page, got {kind:?}");
        }
    }
}
