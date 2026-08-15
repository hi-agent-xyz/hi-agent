//! The gate: an off-box request is answered only with a credential.
//!
//! Both servers here bind `127.0.0.1`, and one of them still gates everything —
//! which is the point. Trust is decided by **which acceptor took the request**
//! ([`Acceptor`]), never by an address, because in the relayed shape every
//! request shares the community's source address and an address check would be
//! inert exactly where it was needed.

use hi_agent::foundation::server::{self, ServerSeams};
use hi_agent::foundation::surfaces::{Acceptor, accepted_on};
use hi_agent::mind::memory::Memory;
use tempfile::tempdir;
use tokio::net::TcpListener;

/// Stand the one router up twice — once as loopback, once as off-box — over the
/// same state, and hand back both base URLs.
async fn spawn() -> (String, String, tempfile::TempDir, ServerSeams) {
    let dir = tempdir().expect("tempdir");
    let memory = Memory::open(dir.path()).await.expect("memory");
    let (router, seams) = server::build(
        memory,
        dir.path().to_path_buf(),
        hi_agent::foundation::observatory::Observatory::new(None),
        hi_agent::foundation::codex::WireTap::new(),
        hi_agent::body::reaction::ToolRegistry::new(),
        hi_agent::body::reaction::Floor::new(),
        hi_agent::body::attachments::Attachments::new(),
        None,
    );

    let mut bases = Vec::new();
    for acceptor in [Acceptor::Loopback, Acceptor::OffBox] {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let router = accepted_on(router.clone(), acceptor);
        tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await;
        });
        bases.push(format!("http://{addr}"));
    }
    let off_box = bases.pop().unwrap();
    let loopback = bases.pop().unwrap();
    (loopback, off_box, dir, seams)
}

#[tokio::test]
async fn loopback_is_ungated_and_off_box_is_not() {
    let (loopback, off_box, _dir, _seams) = spawn().await;
    let client = reqwest::Client::new();

    let res = client.get(format!("{loopback}/api/tools")).send().await.expect("send");
    assert_eq!(res.status(), 200, "the same route, from this machine");

    let res = client.get(format!("{off_box}/api/tools")).send().await.expect("send");
    assert_eq!(res.status(), 401, "the same route, from anywhere else");
    let body: serde_json::Value = res.json().await.expect("json");
    assert_eq!(body["error"], "unauthorized");
}

#[tokio::test]
async fn the_open_routes_answer_before_anything_is_paired() {
    let (_loopback, off_box, _dir, _seams) = spawn().await;
    let client = reqwest::Client::new();

    let res = client.get(format!("{off_box}/healthz")).send().await.expect("send");
    assert_eq!(res.status(), 200);

    // Open, but still not a way in without something to present.
    let res = client.post(format!("{off_box}/api/session")).send().await.expect("send");
    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn html_navigation_gets_somewhere_to_start_rather_than_a_bare_401() {
    let (_loopback, off_box, _dir, _seams) = spawn().await;
    let client = reqwest::Client::new();

    let res = client
        .get(format!("{off_box}/"))
        .header("Accept", "text/html")
        .send()
        .await
        .expect("send");
    assert_eq!(res.status(), 401);
    let body = res.text().await.expect("text");
    assert!(body.contains("Pair this surface"), "the pairing page, not a bare 401");
}

#[tokio::test]
async fn a_credential_opens_both_doors_it_is_supposed_to() {
    let (_loopback, off_box, _dir, seams) = spawn().await;
    let client = reqwest::Client::new();
    let (_id, token) = seams.state.surfaces.mint("the test").expect("mint");

    // Presentation one: the bearer header, which is what an app and curl use.
    let res = client
        .get(format!("{off_box}/api/tools"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("send");
    assert_eq!(res.status(), 200);

    // Presentation two: exchange it once for a session, then ride the cookie —
    // which is the only thing SSE, WebSocket and plain navigation can carry.
    let res = client
        .post(format!("{off_box}/api/session"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("send");
    assert_eq!(res.status(), 200);
    let cookie = res
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .expect("a session cookie")
        .split(';')
        .next()
        .expect("the name=value pair")
        .to_string();
    assert!(cookie.starts_with("hi_surface="));

    let res = client
        .get(format!("{off_box}/api/tools"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("send");
    assert_eq!(res.status(), 200);

    // And a wrong credential is simply not one.
    let res = client
        .get(format!("{off_box}/api/tools"))
        .bearer_auth("not-a-credential")
        .send()
        .await
        .expect("send");
    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn a_pairing_code_is_how_a_second_surface_gets_in() {
    let (loopback, off_box, _dir, _seams) = spawn().await;
    let client = reqwest::Client::new();

    // Minted from a surface that already has access — here, this machine.
    let res = client.post(format!("{loopback}/api/pair")).send().await.expect("send");
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.expect("json");
    let code = body["code"].as_str().expect("a code").to_string();

    // Spending it mints a real credential for the new surface to keep.
    let res = client
        .post(format!("{off_box}/api/session"))
        .bearer_auth(&code)
        .header("Content-Type", "application/json")
        .body(r#"{"label":"the phone"}"#)
        .send()
        .await
        .expect("send");
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.expect("json");
    let credential = body["credential"].as_str().expect("a credential to keep").to_string();

    let res = client
        .get(format!("{off_box}/api/tools"))
        .bearer_auth(&credential)
        .send()
        .await
        .expect("send");
    assert_eq!(res.status(), 200);

    // One-time: the same code cannot admit a second surface.
    let res = client
        .post(format!("{off_box}/api/session"))
        .bearer_auth(&code)
        .send()
        .await
        .expect("send");
    assert_eq!(res.status(), 401);

    // It shows up in the device list under the label it was given, and revoking
    // it there is the end of it — no community involved.
    let res = client.get(format!("{loopback}/api/surfaces")).send().await.expect("send");
    let body: serde_json::Value = res.json().await.expect("json");
    let phone = body["surfaces"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|s| s["label"] == "the phone")
        .expect("the phone")
        .clone();
    assert!(!phone["last_seen_at"].as_str().unwrap_or("").is_empty(), "it has been seen");

    let res = client
        .delete(format!("{loopback}/api/surfaces/{}", phone["id"].as_str().unwrap()))
        .send()
        .await
        .expect("send");
    assert_eq!(res.status(), 204);

    let res = client
        .get(format!("{off_box}/api/tools"))
        .bearer_auth(&credential)
        .send()
        .await
        .expect("send");
    assert_eq!(res.status(), 401, "revoked is revoked");
}

#[tokio::test]
async fn a_cookie_alone_cannot_drive_a_state_change_from_another_site() {
    let (_loopback, off_box, _dir, seams) = spawn().await;
    let client = reqwest::Client::new();
    let (_id, token) = seams.state.surfaces.mint("the test").expect("mint");
    let res = client
        .post(format!("{off_box}/api/session"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("send");
    let cookie = res
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .expect("cookie")
        .split(';')
        .next()
        .unwrap()
        .to_string();

    // `text/plain` is one of the three content types a cross-site *simple*
    // request can carry, so a cookie alone is not enough for it.
    let res = client
        .post(format!("{off_box}/api/in/text"))
        .header("Cookie", &cookie)
        .header("Content-Type", "text/plain")
        .body("hi")
        .send()
        .await
        .expect("send");
    assert_eq!(res.status(), 403);

    // The header forces a preflight a simple request cannot satisfy, so it is
    // proof enough that this was not one.
    let res = client
        .post(format!("{off_box}/api/in/text"))
        .header("Cookie", &cookie)
        .header("Content-Type", "text/plain")
        .header("X-HI-Surface", "1")
        .body("hi")
        .send()
        .await
        .expect("send");
    assert_eq!(res.status(), 202);

    // A bearer is never sent ambiently by another site, so it needs none of this.
    let res = client
        .post(format!("{off_box}/api/in/text"))
        .bearer_auth(&token)
        .header("Content-Type", "text/plain")
        .body("hi")
        .send()
        .await
        .expect("send");
    assert_eq!(res.status(), 202);
}

/// **Nothing the gate protects may be labelled `public`.**
///
/// A gated `200` was served *because* a credential checked out, so a shared cache
/// storing it and replaying it to the next caller hands out exactly what the gate
/// refused. This is not theoretical: relayed, a core sits behind a CDN, and an
/// authorized fetch of `/assets/*` turned a later unauthenticated fetch of the
/// same path from the core's `401` into a `200` served from the edge.
///
/// `private` is the fix rather than `no-store`, because the browser cache was the
/// whole point and only the *shared* one is the problem. Content-addressed names
/// are why these may be cached *forever*; they have never been a reason to cache
/// them *shared*.
#[tokio::test]
async fn nothing_behind_the_gate_is_cacheable_by_a_shared_cache() {
    let (_loopback, off_box, _dir, seams) = spawn().await;
    let client = reqwest::Client::new();
    let (_id, token) = seams.state.surfaces.mint("the test").expect("mint");

    // Everything static enough to carry a long TTL, which is the whole risk set:
    // the host bundle, a non-hashed embedded file, and the agent's own compiled
    // views. A 404 is fine — the header is what is on trial, not the body.
    for path in ["/assets/index.js", "/vite.svg", "/generated/_compiled/anything.mjs", "/api/tools"] {
        let res = client
            .get(format!("{off_box}{path}"))
            .bearer_auth(&token)
            .send()
            .await
            .expect("send");
        let cache = res
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        assert!(
            !cache.contains("public"),
            "{path} answered {} with cache-control: {cache:?} — a shared cache may keep this \
             and serve it to a request with no credential at all",
            res.status(),
        );
    }
}
