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
/// as the mind emits it.
async fn emit_view(
    seams: &ServerSeams, id: &str, op: ViewOp, url: Option<&str>) {
    seams
        .out_tx
        .send(OutboundSignal::View {
            envelope: ViewEnvelope {
                id: id.to_string(),
                op,
                module_url: url.map(str::to_string),
                traits: None,
                view_ref: None,
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
/// The second `hi_show` here also pins the screen's one-at-a-time rule end to end:
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
