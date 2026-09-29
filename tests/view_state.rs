//! /out/view as retained shared appearance state: a view shown with no
//! client connected is served to any later GET (refresh, second device), and
//! the state survives a server restart. The old `tokio::broadcast` delivered
//! envelopes only to receivers that existed at send time, so every one of
//! those paths used to come up blank.

use std::path::Path;
use std::time::Duration;

use hi_agent::mind::memory::Memory;
use hi_agent::body::reaction::OutboundSignal;
use hi_agent::foundation::surfaces::{Acceptor, accepted_on};
use hi_agent::foundation::server::{self, ServerSeams};
use hi_agent::types::{ViewEnvelope, ViewOp};
use tempfile::tempdir;
use tokio::net::TcpListener;

async fn spawn_server_at(dir: &Path) -> (String, ServerSeams) {
    let memory = Memory::open(dir).await.expect("memory");
    let observatory =
        hi_agent::foundation::observatory::Observatory::new(None);
    let (router, seams) = server::build(
        memory,
        dir.to_path_buf(),
        observatory,
        hi_agent::foundation::codex::WireTap::new(),
        hi_agent::foundation::privacy::PrivacyBoundary::open(dir).unwrap(),
        hi_agent::body::reaction::ToolRegistry::new(),
        hi_agent::body::reaction::Floor::new(),
        hi_agent::body::attachments::Attachments::new(),
        None,
    );
    // A test is a local caller, and says so: without an acceptor the gate
    // fails closed and every request here would be a 401.
    let router = accepted_on(router, Acceptor::Loopback);

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    (format!("http://{addr}"), seams)
}

/// Drive a view through the reaction's outbound seam — binder → bus — exactly
/// as the mind emits it: every view the agent shows goes up by its ref, here
/// `notes/<id>`.
async fn emit_view(
    seams: &ServerSeams, id: &str, op: ViewOp, url: Option<&str>) {
    seams
        .out_tx
        .send(OutboundSignal::View {
            keep: false,
            envelope: ViewEnvelope {
                id: id.to_string(),
                op,
                module_url: url.map(str::to_string),
                view_ref: Some(format!("notes/{id}")),
            },
        })
        .await
        .expect("out_tx send");
    // The binder drains the seam asynchronously.
    tokio::time::sleep(Duration::from_millis(50)).await;
}

async fn get_state(
    base: &str,
    since: Option<u64>,
    budget: Duration,
) -> Result<serde_json::Value, ()> {
    let query = since.map(|s| format!("?since={s}")).unwrap_or_default();
    let client = reqwest::Client::new();
    tokio::time::timeout(budget, async {
        client
            .get(format!("{base}/api/out/view{query}"))
            .send()
            .await
            .expect("send")
            .json::<serde_json::Value>()
            .await
            .expect("body")
    })
    .await
    .map_err(|_| ())
}

/// Somewhere the person can move the screen that needs no view compiler, which this
/// server never publishes: a picture goes up as the host's own stage module.
async fn a_picture(dir: &Path) -> String {
    let mut png = Vec::new();
    image::DynamicImage::new_rgb8(16, 9)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("encode");
    let placed = hi_agent::foundation::attachments::place_bytes(dir, &png, "drive.png")
        .await
        .expect("attached");
    format!("{}{}", hi_agent::foundation::attachments::PREFIX, placed.id)
}

fn ids(state: &serde_json::Value) -> Vec<&str> {
    state["views"]
        .as_array()
        .expect("views array")
        .iter()
        .map(|v| v["id"].as_str().expect("id"))
        .collect()
}

/// A view shown before any client connects is served to a late GET — and to
/// every GET after it (refresh / second device), because it is state, not a
/// drained queue.
#[tokio::test]
async fn late_and_repeat_subscribers_see_the_same_appearance() {
    let dir = tempdir().expect("tempdir");
    let (base, seams) = spawn_server_at(dir.path()).await;

    emit_view(&seams, "card", ViewOp::Show, Some("/m/card.mjs")).await;

    let first = get_state(&base, None, Duration::from_millis(500))
        .await
        .expect("late GET should receive the retained state");
    assert_eq!(first["version"], 1);
    assert_eq!(ids(&first), vec!["card"]);
    assert_eq!(first["views"][0]["module_url"], "/m/card.mjs");

    // A refresh (or a second device) syncs to the identical state.
    let second = get_state(&base, None, Duration::from_millis(500))
        .await
        .expect("repeat GET");
    assert_eq!(second, first);
}

/// `?since=` parks until the state changes, then delivers the new whole state.
#[tokio::test]
async fn since_long_polls_until_the_state_changes() {
    let dir = tempdir().expect("tempdir");
    let (base, seams) = spawn_server_at(dir.path()).await;

    emit_view(&seams, "card", ViewOp::Show, Some("/m/card.mjs")).await;

    // Up to date → the poll parks.
    let parked = get_state(&base, Some(1), Duration::from_millis(250)).await;
    assert!(parked.is_err(), "in-sync poll should hang; got {parked:?}");

    let base2 = base.clone();
    let waiter = tokio::spawn(async move {
        get_state(&base2, Some(1), Duration::from_millis(800)).await
    });
    tokio::time::sleep(Duration::from_millis(80)).await;
    emit_view(&seams, "card", ViewOp::Dismiss, None).await;

    let state = waiter.await.expect("join").expect("dismiss should wake the poll");
    assert_eq!(state["version"], 2);
    assert!(ids(&state).is_empty());
}

/// Every attached client converges on the same screen. The appearance is retained
/// state, not a per-client stream, so a second window syncing after a view was
/// shown sees exactly what the first one sees — which is what makes "show it on
/// the screen" mean one screen rather than whichever window asked first.
#[tokio::test]
async fn every_client_sees_the_same_screen() {
    let dir = tempdir().expect("tempdir");
    let (base, seams) = spawn_server_at(dir.path()).await;

    emit_view(&seams, "hers", ViewOp::Show, Some("/m/h.mjs")).await;

    let first = get_state(&base, None, Duration::from_millis(500))
        .await
        .expect("first client syncs");
    let second = get_state(&base, None, Duration::from_millis(500))
        .await
        .expect("second client syncs");

    assert_eq!(ids(&first), vec!["hers"]);
    assert_eq!(ids(&second), vec!["hers"]);
    assert_eq!(first["version"], second["version"]);
}

/// The whole point: the appearance survives a server restart. A fresh server
/// over the same data dir serves the same state (version included).
///
/// The second show here also pins the screen's one-at-a-time rule end to end:
/// `b` takes the screen from `a` rather than stacking on it, and it is that
/// single view — not a pile — that comes back after the restart.
#[tokio::test]
async fn appearance_survives_restart() {
    let dir = tempdir().expect("tempdir");

    let before = {
        let (base, seams) = spawn_server_at(dir.path()).await;
        emit_view(&seams, "a", ViewOp::Show, Some("/m/a.mjs")).await;
        emit_view(&seams, "b", ViewOp::Show, Some("/m/b.mjs")).await;
        get_state(&base, None, Duration::from_millis(500))
            .await
            .expect("GET before restart")
    };
    assert_eq!(ids(&before), vec!["b"]);

    // "Restart": a second server over the same data dir.
    let (base, _seams) = spawn_server_at(dir.path()).await;
    let after = get_state(&base, None, Duration::from_millis(500))
        .await
        .expect("GET after restart");
    assert_eq!(after, before);
}

/// One screen: the person going somewhere **is** a write of the appearance, and a poll
/// parked in another window wakes on it. The moment this stops being true, the phone and
/// the desktop are looking at different things with no way to say so.
#[tokio::test]
async fn a_move_takes_every_window_with_it() {
    let dir = tempdir().expect("tempdir");
    let (base, seams) = spawn_server_at(dir.path()).await;
    emit_view(&seams, "card", ViewOp::Show, Some("/m/card.mjs")).await;

    let picture = a_picture(dir.path()).await;
    let before = get_state(&base, None, Duration::from_millis(500))
        .await
        .expect("state");
    let version = before["version"].as_u64().expect("version");

    // A second window, parked on the state it has already rendered.
    let waiting = {
        let base = base.clone();
        tokio::spawn(async move { get_state(&base, Some(version), Duration::from_secs(2)).await })
    };
    tokio::task::yield_now().await;

    let client = reqwest::Client::new();
    let moved = client
        .post(format!("{base}/api/views/open"))
        .header("X-HI-Face", "desk")
        .json(&serde_json::json!({ "ref": picture }))
        .send()
        .await
        .expect("send");
    assert!(moved.status().is_success(), "moved: {}", moved.status());

    let after = waiting.await.expect("join").expect("the parked window woke");
    assert_eq!(after["cursor"], picture.as_str(), "the other window follows");
    // The slot is still the agent's: what it showed is what it will refer to out loud.
    assert_eq!(ids(&after), vec!["card"]);
    assert_eq!(after["live"], "notes/card");

    // And going live drops the cursor rather than clearing the screen.
    let live = client
        .post(format!("{base}/api/views/open"))
        .header("X-HI-Face", "desk")
        .json(&serde_json::json!({ "live": true }))
        .send()
        .await
        .expect("send");
    assert_eq!(live.status(), 202);
    let home = get_state(&base, None, Duration::from_millis(500)).await.expect("state");
    assert!(home["cursor"].is_null());
    assert_eq!(ids(&home), vec!["card"]);
}

/// A show takes every window with it, whatever any of them had gone back to.
#[tokio::test]
async fn a_show_catches_up_with_a_parked_screen() {
    let dir = tempdir().expect("tempdir");
    let (base, seams) = spawn_server_at(dir.path()).await;
    emit_view(&seams, "card", ViewOp::Show, Some("/m/card.mjs")).await;

    let picture = a_picture(dir.path()).await;
    let moved = reqwest::Client::new()
        .post(format!("{base}/api/views/open"))
        .header("X-HI-Face", "desk")
        .json(&serde_json::json!({ "ref": picture }))
        .send()
        .await
        .expect("send");
    assert!(moved.status().is_success(), "moved: {}", moved.status());

    emit_view(&seams, "next", ViewOp::Show, Some("/m/next.mjs")).await;
    let state = get_state(&base, None, Duration::from_millis(500)).await.expect("state");
    assert!(state["cursor"].is_null(), "the show took them along; got {state:?}");
    assert_eq!(ids(&state), vec!["next"]);
}

/// A move to nowhere is a client bug. Answering 202 to it would hide the bug behind a
/// screen that silently never moves.
#[tokio::test]
async fn a_move_with_no_destination_is_refused() {
    let dir = tempdir().expect("tempdir");
    let (base, _seams) = spawn_server_at(dir.path()).await;

    let empty = reqwest::Client::new()
        .post(format!("{base}/api/views/open"))
        .header("X-HI-Face", "desk")
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("send");
    assert_eq!(empty.status(), 400);
}

/// A move is made from a face. Loopback presents no credential, so without the header
/// this route cannot tell a window from any process on the box — and every move it
/// takes is journalled as the person going there. Watched on 2026-09-24: a reviewer's
/// scripts posting here for a module URL pulled every window off what was being read.
#[tokio::test]
async fn a_move_no_face_made_is_refused_and_moves_nothing() {
    let dir = tempdir().expect("tempdir");
    let (base, seams) = spawn_server_at(dir.path()).await;
    emit_view(&seams, "card", ViewOp::Show, Some("/m/card.mjs")).await;
    let before = get_state(&base, None, Duration::from_millis(500)).await.expect("state");

    let faceless = reqwest::Client::new()
        .post(format!("{base}/api/views/open"))
        .json(&serde_json::json!({ "ref": "factory/drive" }))
        .send()
        .await
        .expect("send");
    assert_eq!(faceless.status(), 400);
    let said = faceless.text().await.expect("body");
    assert!(said.contains("/api/views/module"), "the refusal names the call that moves nothing: {said}");

    let after = get_state(&base, None, Duration::from_millis(500)).await.expect("state");
    assert_eq!(after, before, "nothing moved");
}

/// Seed a view's source into the tree the server reads, as a builder would have written it.
fn seed_view(dir: &Path, view_ref: &str) {
    let path = dir.join("views").join(format!("{view_ref}.jsx"));
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, "export default () => null").expect("write");
}

async fn delete(base: &str, view_ref: &str, face: bool) -> reqwest::Response {
    let mut request = reqwest::Client::new().post(format!("{base}/api/views/delete"));
    if face {
        request = request.header("X-HI-Face", "desk");
    }
    request.json(&serde_json::json!({ "ref": view_ref })).send().await.expect("send")
}

/// Deleting a view from its card deletes the view: the source goes to the trash, its card
/// leaves the trail on every window, and the agent's slot empties if it had it up.
#[tokio::test]
async fn a_deleted_view_goes_to_the_trash_and_its_card_goes_with_it() {
    let dir = tempdir().expect("tempdir");
    let (base, seams) = spawn_server_at(dir.path()).await;
    seed_view(dir.path(), "notes/card");
    std::fs::write(dir.path().join("views/notes/data.json"), "{}").expect("write");
    emit_view(&seams, "card", ViewOp::Show, Some("/m/card.mjs")).await;
    let before = get_state(&base, None, Duration::from_millis(500)).await.expect("state");
    assert_eq!(before["live"], "notes/card");

    let deleted = delete(&base, "notes/card", true).await;
    assert_eq!(deleted.status(), 204);

    let after = get_state(&base, None, Duration::from_millis(500)).await.expect("state");
    assert!(after["history"].as_array().expect("history").is_empty(), "{after:?}");
    assert!(ids(&after).is_empty());
    assert!(after["live"].is_null());

    // The only view in its folder took the folder, data and all, into one trash entry.
    assert!(!dir.path().join("views/notes").exists());
    let trash = dir.path().join("views/_trash");
    let entries: Vec<_> = std::fs::read_dir(&trash).expect("trash").flatten().collect();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].path().join("notes/card.jsx").is_file());
    assert!(entries[0].path().join("notes/data.json").is_file());

    // And a deleted view is not a place any more, nor something to delete again.
    let listed: serde_json::Value = reqwest::get(format!("{base}/api/views"))
        .await
        .expect("send")
        .json()
        .await
        .expect("json");
    assert!(listed.as_array().expect("list").iter().all(|v| v["view_ref"] != "notes/card"));
    assert_eq!(delete(&base, "notes/card", true).await.status(), 404);
}

/// What is not a view in the tree cannot be deleted from here: a system view is re-seeded
/// on every boot, an attachment is not in the tree at all, and a ref cannot reach into the
/// trash. And like a move, it is made from a face.
#[tokio::test]
async fn only_a_view_in_the_tree_can_be_deleted_and_only_from_a_face() {
    let dir = tempdir().expect("tempdir");
    let (base, _seams) = spawn_server_at(dir.path()).await;
    seed_view(dir.path(), "factory/tasks");
    seed_view(dir.path(), "notes/card");

    assert_eq!(delete(&base, "factory/tasks", true).await.status(), 400);
    assert_eq!(delete(&base, "att:3f9a0c11d2e4b5a6", true).await.status(), 400);
    assert_eq!(delete(&base, "_trash/x", true).await.status(), 400);
    assert_eq!(delete(&base, "../etc", true).await.status(), 400);
    assert_eq!(delete(&base, "notes/card", false).await.status(), 400);
    assert!(dir.path().join("views/notes/card.jsx").is_file(), "nothing was moved");
    assert!(dir.path().join("views/factory/tasks.jsx").is_file());
}

/// The owner is handed the link to give out, by the same rule the agent is: a core with no
/// name has only a path on this machine, and says so. Sharing again replaces the record, so
/// an unlisted link that was re-issued stops opening under its old key.
#[tokio::test]
async fn a_share_answers_with_its_link_and_a_new_one_retires_the_old_key() {
    let dir = tempdir().expect("tempdir");
    let (base, _seams) = spawn_server_at(dir.path()).await;
    let picture = a_picture(dir.path()).await;
    let client = reqwest::Client::new();
    let share = |unlisted: bool| {
        client
            .post(format!("{base}/api/shares"))
            .json(&serde_json::json!({ "ref": picture, "on": true, "unlisted": unlisted }))
            .send()
    };

    let first: serde_json::Value = share(true).await.expect("send").json().await.expect("json");
    let key = first["key"].as_str().expect("an unlisted share has a key").to_string();
    let path = first["path"].as_str().expect("path").to_string();
    assert_eq!(first["reachable"], "this_machine");
    assert_eq!(first["link"], format!("{path}?key={key}"));

    let asked: serde_json::Value = client
        .get(format!("{base}/api/shares"))
        .query(&[("ref", picture.as_str())])
        .send()
        .await
        .expect("send")
        .json()
        .await
        .expect("json");
    assert_eq!(asked["kind"], "unlisted");
    assert!(asked["link"].is_null(), "an unlisted key is kept only as a hash");

    let second: serde_json::Value = share(true).await.expect("send").json().await.expect("json");
    assert_ne!(second["key"], first["key"]);
    // Asked with no credential, which is how a share is opened.
    let opened = |key: String| {
        let url = format!("{base}{path}?key={key}");
        async move { reqwest::get(url).await.expect("send").status() }
    };
    assert_eq!(opened(key).await, 404, "the old link stops opening");
    // Past the key, which is all this asks: a build with no web bundle has no render page
    // to draw the attachment on, and answers `503` for that rather than `404`.
    assert_ne!(opened(second["key"].as_str().expect("key").to_string()).await, 404, "the new one does");

    let public: serde_json::Value = share(false).await.expect("send").json().await.expect("json");
    assert!(public["key"].is_null());
    let asked: serde_json::Value = client
        .get(format!("{base}/api/shares"))
        .query(&[("ref", picture.as_str())])
        .send()
        .await
        .expect("send")
        .json()
        .await
        .expect("json");
    assert_eq!(asked["kind"], "public");
    assert_eq!(asked["link"], path, "a public link can be handed out again");
}
