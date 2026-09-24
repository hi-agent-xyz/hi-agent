//! `immutable` decides what leaves the machine.
//!
//! It used to govern a browser cache; it now marks a response as one a core may
//! upload to the community's cache and redirect to from then on
//! (`docs/arch/topology.md` § *Content*). An object wrongly marked goes
//! permanently stale out there, so every route that says it is pinned here, case
//! by case — including the cases on the same routes that must *not* say it,
//! because each of those was a way the bytes at one path can change.

use chrono::{TimeZone, Utc};
use hi_agent::foundation::server;
use hi_agent::foundation::surfaces::{Acceptor, accepted_on};
use hi_agent::mind::memory::{Memory, layout, media};
use hi_agent::types::Channel;
use tempfile::tempdir;
use tokio::net::TcpListener;

async fn spawn() -> (String, tempfile::TempDir, server::ServerSeams) {
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
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let router = accepted_on(router, Acceptor::Loopback);
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            router.into_make_service(),
        )
        .await;
    });
    (format!("http://{addr}"), dir, seams)
}

fn put(path: std::path::PathBuf, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[tokio::test]
async fn only_bytes_that_never_change_at_their_path_are_called_immutable() {
    let (base, dir, _seams) = spawn().await;
    let data = dir.path();

    // A signal's own blob: written once, at a name that is its timestamp.
    let ts = Utc.with_ymd_and_hms(2026, 9, 22, 14, 3, 22).unwrap();
    let rel = media::store_blob(data, Channel::File, ts, layout::MediaSlot::InputOneOff, "jpg", b"orig")
        .await
        .unwrap();
    let blob = media::signal_ref(Channel::File, ts, &rel);

    // A day that faded: the original is gone and a keepsake answers for it.
    let faded_at = Utc.with_ymd_and_hms(2026, 9, 1, 10, 0, 0).unwrap();
    put(layout::channel_day_dir(data, Channel::Vision, faded_at).join("keep/100000.jpg"), b"small");
    let faded = media::signal_ref(Channel::Vision, faded_at, "10/00-00.jpg");

    // The drive, edited in place.
    put(media::drive_root(data).join("notes/pic.png"), b"png");

    // Views: a content-addressed module; a record picture, which is healed in place
    // when an older renderer left the wrong shape; a named surface's picture, re-taken
    // on a clock; and source.
    let views = data.join("views");
    put(views.join("_compiled/0a1b.mjs"), b"export default 1");
    put(views.join("_shots/0a1b.png"), b"png");
    put(views.join("_shots/ref/home/board.png"), b"png");
    put(views.join("home/board.jsx"), b"<div/>");

    let cases: &[(String, bool)] = &[
        (format!("/api/media/{blob}"), true),
        (format!("/api/media/{faded}"), false),
        ("/views/_compiled/0a1b.mjs".into(), true),
        ("/views/_shots/0a1b.png".into(), false),
        ("/views/_shots/ref/home/board.png?v=1".into(), false),
        ("/views/home/board.jsx".into(), false),
    ];

    let client = reqwest::Client::new();
    // The drive has one address, and it is not this one: `/api/media` answered for the same
    // bytes under a second cache rule until phase 4 of `docs/arch/showing.md`.
    let drive = client
        .get(format!("{base}/api/media/drive/notes/pic.png"))
        .send()
        .await
        .expect("send");
    assert_eq!(drive.status(), 404, "a drive file is served by /api/drive/file alone");

    for (path, immutable) in cases {
        let res = client.get(format!("{base}{path}")).send().await.expect("send");
        assert_eq!(res.status(), 200, "{path}");
        let cache = res
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        assert_eq!(cache.contains("immutable"), *immutable, "{path} answered cache-control: {cache:?}");
        assert!(!cache.contains("public"), "{path} answered cache-control: {cache:?}");
    }
}
