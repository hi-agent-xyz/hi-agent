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
        hi_agent::foundation::privacy::PrivacyBoundary::open(dir.path()).unwrap(),
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
                router.into_make_service(),
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
async fn a_share_grants_declared_attachments_even_when_loaded_after_the_page() {
    use hi_agent::foundation::{attachments, credentials};

    let (_loopback, off_box, dir, _seams) = spawn().await;
    let source = dir.path().join("picture.png");
    image::RgbImage::from_pixel(32, 32, image::Rgb([20, 130, 80]))
        .save(&source)
        .unwrap();
    let placed = attachments::place(dir.path(), &source).await.unwrap();
    let private_source = dir.path().join("private.png");
    image::RgbImage::from_pixel(32, 32, image::Rgb([130, 20, 80]))
        .save(&private_source)
        .unwrap();
    let private = attachments::place(dir.path(), &private_source).await.unwrap();

    let share = server::share::Share {
        of: "view:court/review".into(),
        module_url: "/views/_compiled/ab12.mjs".into(),
        key_hash: None,
        created_at: chrono::Utc::now(),
        description: String::new(),
        attachments: vec![placed.id.clone()],
        resources: vec![],
    };
    credentials::set_setting(dir.path(), "view_shares", &serde_json::to_string(&vec![share]).unwrap())
        .unwrap();
    let page = dir.path().join("views/_shares/court/review.html");
    std::fs::create_dir_all(page.parent().unwrap()).unwrap();
    std::fs::write(&page, "<html><head></head><body>Review</body></html>").unwrap();
    let resource = dir.path().join("views/court/review.assets/late.png");
    std::fs::create_dir_all(resource.parent().unwrap()).unwrap();
    std::fs::write(&resource, b"published image").unwrap();
    std::fs::write(dir.path().join("views/court/other.jsx"), "private view").unwrap();
    std::fs::write(dir.path().join("views/court/review.share.json"), "[]").unwrap();
    let other_resource = dir.path().join("views/court/other.assets/private.png");
    std::fs::create_dir_all(other_resource.parent().unwrap()).unwrap();
    std::fs::write(&other_resource, b"private image").unwrap();

    let client = reqwest::Client::new();
    let response = client.get(format!("{off_box}/court/review")).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-security-policy"], "connect-src 'self'");
    assert_eq!(client.get(format!("{off_box}/views/court/review.assets/late.png")).send().await.unwrap().status(), 200);
    for path in ["court/other.jsx", "court/review.share.json", "court/other.assets/private.png"] {
        assert_eq!(
            client.get(format!("{off_box}/views/{path}")).send().await.unwrap().status(),
            401,
            "private view file: {path}",
        );
    }
    for suffix in ["about.v1", "preview.v1"] {
        let response = client
            .get(format!("{off_box}/api/attachments/{}/{suffix}", placed.id))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "declared attachment: {suffix}");
    }
    let response = client
        .get(format!("{off_box}/api/attachments/{}/preview.v1", private.id))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401, "another private attachment remains gated");
    assert_eq!(client.get(format!("{off_box}/api/tools")).send().await.unwrap().status(), 401);
    #[cfg(unix)]
    {
        let outside = dir.path().join("private.txt");
        std::fs::write(&outside, "not public").unwrap();
        std::os::unix::fs::symlink(&outside, dir.path().join("views/court/review.assets/linked.txt")).unwrap();
        assert_eq!(client.get(format!("{off_box}/views/court/review.assets/linked.txt")).send().await.unwrap().status(), 404);
    }

    credentials::set_setting(dir.path(), "view_shares", "[]").unwrap();
    assert_eq!(
        client
            .get(format!("{off_box}/api/attachments/{}/preview.v1", placed.id))
            .send()
            .await
            .unwrap()
            .status(),
        401,
        "withdrawing the share withdraws its attachments",
    );
}

#[tokio::test]
async fn named_view_resource_is_read_through_its_view_without_global_attachment_access() {
    use hi_agent::foundation::{attachments, credentials};
    let (_loopback, off_box, dir, _seams) = spawn().await;
    let source = dir.path().join("image.png");
    image::RgbImage::from_pixel(32, 32, image::Rgb([30, 100, 60])).save(&source).unwrap();
    let image = attachments::place(dir.path(), &source).await.unwrap();
    let bindings = dir.path().join("views/court/review.resources.json");
    std::fs::create_dir_all(bindings.parent().unwrap()).unwrap();
    std::fs::write(&bindings, format!("{{\"hero\":\"att:{}\"}}", image.id)).unwrap();
    let share = server::share::Share {
        of: "view:court/review".into(),
        module_url: "/views/_compiled/ab12.mjs".into(),
        key_hash: None,
        created_at: chrono::Utc::now(),
        description: String::new(),
        attachments: vec![],
        resources: vec!["hero".into()],
    };
    credentials::set_setting(dir.path(), "view_shares", &serde_json::to_string(&vec![share]).unwrap()).unwrap();
    let client = reqwest::Client::new();
    let url = format!("{off_box}/views/court/review.assets/_resources/hero");
    let about: serde_json::Value = client.get(format!("{url}/about.v1")).send().await.unwrap().json().await.unwrap();
    assert_eq!(about["preview"], "/views/court/review.assets/_resources/hero/preview.v1");
    assert_eq!(client.get(format!("{url}/preview.v1")).send().await.unwrap().status(), 200);
    assert_eq!(client.get(format!("{url}/original")).send().await.unwrap().status(), 200);
    assert_eq!(client.get(format!("{url}/missing")).send().await.unwrap().status(), 401);
    let next = dir.path().join("new-image.png");
    image::RgbImage::from_pixel(32, 32, image::Rgb([160, 35, 100])).save(&next).unwrap();
    let replaced = attachments::place(dir.path(), &next).await.unwrap();
    std::fs::write(&bindings, format!("{{\"hero\":\"att:{}\",\"extra\":\"att:{}\"}}", replaced.id, image.id)).unwrap();
    let updated: serde_json::Value = client.get(format!("{url}/about.v1")).send().await.unwrap().json().await.unwrap();
    assert_eq!(updated["ref"], format!("att:{}", replaced.id));
    assert_eq!(client.get(format!("{off_box}/views/court/review.assets/_resources/extra/preview.v1")).send().await.unwrap().status(), 401);
    assert_eq!(client.get(format!("{off_box}/api/attachments/{}/preview.v1", image.id)).send().await.unwrap().status(), 401);
    assert_eq!(client.get(format!("{off_box}/views/court/other.assets/_resources/hero/preview.v1")).send().await.unwrap().status(), 401);
    credentials::set_setting(dir.path(), "view_shares", "[]").unwrap();
    assert_eq!(client.get(format!("{url}/preview.v1")).send().await.unwrap().status(), 401);
}

#[tokio::test]
async fn view_owned_json_and_delayed_video_ranges_work_without_opening_the_project() {
    use hi_agent::foundation::credentials;
    let (_loopback, off_box, dir, _seams) = spawn().await;
    let share = server::share::Share {
        of: "view:court/review".into(),
        module_url: "/views/_compiled/ab12.mjs".into(),
        key_hash: None,
        created_at: chrono::Utc::now(),
        description: String::new(),
        attachments: vec![],
        resources: vec![],
    };
    credentials::set_setting(dir.path(), "view_shares", &serde_json::to_string(&vec![share]).unwrap()).unwrap();
    let assets = dir.path().join("views/court/review.assets");
    std::fs::create_dir_all(&assets).unwrap();
    let data = serde_json::json!({"clip": {
        "src": "/views/court/review.assets/clip-b.MOV",
        "alt": "/views/court/review.assets/clip-b.mp4"
    }});
    std::fs::write(assets.join("ids-b.json"), data.to_string()).unwrap();
    for name in ["clip-b.MOV", "clip-b.mp4"] {
        std::fs::write(assets.join(name), b"video range probe").unwrap();
    }
    let private = dir.path().join("views/court/assets");
    std::fs::create_dir_all(&private).unwrap();
    std::fs::write(private.join("ids-b.json"), data.to_string()).unwrap();

    // No owner credential and no observed-request grant: B can load after a click.
    let client = reqwest::Client::new();
    let response = client.get(format!("{off_box}/views/court/review.assets/ids-b.json"))
        .send().await.unwrap();
    assert_eq!(response.status(), 200);
    let received: serde_json::Value = response.json().await.unwrap();
    assert_eq!(received, data);
    for field in ["src", "alt"] {
        let path = received["clip"][field].as_str().unwrap();
        let response = client.get(format!("{off_box}{path}"))
            .header("Range", "bytes=0-4").send().await.unwrap();
        assert_eq!(response.status(), 206);
        assert_eq!(response.headers()["content-range"], "bytes 0-4/17");
        assert_eq!(response.bytes().await.unwrap().as_ref(), b"video");
    }
    assert_eq!(client.get(format!("{off_box}/views/court/assets/ids-b.json"))
        .send().await.unwrap().status(), 401);
    credentials::set_setting(dir.path(), "view_shares", "[]").unwrap();
    assert_eq!(client.get(format!("{off_box}/views/court/review.assets/clip-b.mp4"))
        .header("Range", "bytes=0-4").send().await.unwrap().status(), 401);
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
    // On what the page *does*, not what it is headed. The old assertion was on the
    // heading, which the copy moved out from under when asking to be let in replaced
    // typing a pairing code — leaving a red test that said nothing about the gate.
    assert!(
        body.contains(r#"fetch("/api/session""#),
        "a page that can let you in, not a bare 401"
    );
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
    let app_url = url::Url::parse(body["app_url"].as_str().expect("an app pairing URL"))
        .expect("valid app pairing URL");
    assert_eq!(app_url.scheme(), "hiagent");
    assert_eq!(app_url.host_str(), Some("pair"));
    let query: std::collections::HashMap<_, _> = app_url.query_pairs().into_owned().collect();
    assert_eq!(query.get("url").map(String::as_str), body["url"].as_str());
    assert_eq!(query.get("code").map(String::as_str), Some(code.as_str()));

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
    let (_loopback, off_box, dir, seams) = spawn().await;
    let client = reqwest::Client::new();
    let (_id, token) = seams.state.surfaces.mint("the test").expect("mint");

    // A compiled view has to exist to be answered with its header at all. This
    // asked for `/generated/_compiled/…` until 2026-09-22 — a route that does not
    // exist, so the shared-view fallback answered and the views route was never on
    // trial.
    let compiled = dir.path().join("views/_compiled/anything.mjs");
    std::fs::create_dir_all(compiled.parent().unwrap()).unwrap();
    std::fs::write(&compiled, "export default 1").unwrap();

    // Everything static enough to carry a long TTL, which is the whole risk set:
    // the host bundle, a non-hashed embedded file, and the agent's own compiled
    // views. A 404 is fine for the embedded two — the header is what is on trial,
    // not the body.
    for path in ["/assets/index.js", "/vite.svg", "/views/_compiled/anything.mjs", "/api/tools"] {
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

/// The settings routes are this machine's alone — a paired device is let through the
/// gate and still refused here, by the listener that took it and not by its address.
///
/// Both servers bind `127.0.0.1`, which is what makes this the test: a check on the
/// peer's address would pass the off-box one. The tunnel had no peer address to read
/// at all, so that check answered a remote page load with a 500 instead of this 403.
#[tokio::test]
async fn settings_answer_this_machine_and_refuse_a_paired_device() {
    let (loopback, off_box, _dir, seams) = spawn().await;
    let client = reqwest::Client::new();
    let (_id, token) = seams.state.surfaces.mint("the test").expect("mint");

    let res = client.get(format!("{loopback}/api/settings")).send().await.expect("send");
    assert_eq!(res.status(), 200, "the Settings window, on this machine");

    let res = client
        .get(format!("{off_box}/api/settings"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("send");
    assert_eq!(res.status(), 403, "a paired device, through the gate and no further");
    let body: serde_json::Value = res.json().await.expect("json");
    assert_eq!(body["error"], "loopback_only");
}
