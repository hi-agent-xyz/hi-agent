//! Regression: the /out/text delivery race that dropped an utterance when the
//! subscriber was not connected at send time.
//!
//! The field symptom (journal-confirmed): the reaction produced a reply
//! ("Hey! What's up?") and emitted its chunks, but the web client's GET
//! GET /out/text re-subscribed ~150ms too late. The old `tokio::broadcast` delivered
//! nothing to a receiver created after the send, so "send hi, nothing
//! responds". `TextBus` retains utterances, so a late GET still receives the
//! pending one — and because reading no longer consumes, several attached
//! surfaces each receive it rather than racing for it.

use std::time::Duration;

use hi_agent::mind::memory::Memory;
use hi_agent::foundation::server::{self, ServerSeams, TextBus};
use tempfile::tempdir;
use tokio::net::TcpListener;

async fn spawn_server() -> (String, tempfile::TempDir, ServerSeams) {
    let dir = tempdir().expect("tempdir");
    let memory = Memory::open(dir.path()).await.expect("memory");
    let observatory = hi_agent::foundation::observatory::Observatory::new(None);
    let (router, seams) = server::build(
        memory,
        dir.path().to_path_buf(),
        observatory,
        hi_agent::foundation::acp::AcpTap::new(),
        hi_agent::body::reaction::ToolRegistry::new(),
        hi_agent::body::reaction::InterruptRegistry::new(),
        hi_agent::body::presence::Presence::new(),
        None,
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    (format!("http://{addr}"), dir, seams)
}

async fn emit_utterance(bus: &TextBus, chunks: &[&str]) {
    for c in chunks {
        bus.push_chunk(c.to_string()).await;
    }
    bus.end_utterance().await;
}

async fn get_out_text(base: &str, after: Option<u64>, budget: Duration) -> Result<String, ()> {
    let client = reqwest::Client::new();
    let qs = after.map(|a| format!("?after={a}")).unwrap_or_default();
    tokio::time::timeout(budget, async {
        client
            .get(format!("{base}/api/out/text{qs}"))
            .send()
            .await
            .expect("send")
            .text()
            .await
            .expect("body")
    })
    .await
    .map_err(|_| ())
}

/// The original bug: produce the whole reply, *then* subscribe. The buffered
/// utterance must still be delivered.
#[tokio::test]
async fn late_subscriber_still_gets_the_utterance() {
    let (base, _dir, seams) = spawn_server().await;

    emit_utterance(&seams.text_bus, &["Hey! What", "'s up?"]).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let body = get_out_text(&base, None, Duration::from_millis(500))
        .await
        .expect("late GET should receive the buffered utterance, not hang");
    assert_eq!(body, "Hey! What's up?");
}

/// A subscriber connected *before* the reply still streams it (live path).
#[tokio::test]
async fn connected_subscriber_streams_live() {
    let (base, _dir, seams) = spawn_server().await;

    let bus = seams.text_bus.clone();
    let base2 = base.clone();
    let reader = tokio::spawn(async move {
        get_out_text(&base2, None, Duration::from_millis(800)).await
    });

    // Let the GET subscribe, then emit.
    tokio::time::sleep(Duration::from_millis(80)).await;
    emit_utterance(&bus, &["live ", "stream"]).await;

    let body = reader.await.expect("join").expect("should not hang");
    assert_eq!(body, "live stream");
}

/// Two sequential utterances are delivered one-per-GET, in order. The reader says
/// where it is with `after`; without it a re-GET would land on the first one again,
/// which is the price of retaining rather than draining.
#[tokio::test]
async fn sequential_gets_advance_by_cursor() {
    let (base, _dir, seams) = spawn_server().await;

    emit_utterance(&seams.text_bus, &["first"]).await;
    emit_utterance(&seams.text_bus, &["second"]).await;

    let a = get_out_text(&base, None, Duration::from_millis(500))
        .await
        .expect("first GET");
    assert_eq!(a, "first");

    let b = get_out_text(&base, Some(0), Duration::from_millis(500))
        .await
        .expect("second GET");
    assert_eq!(b, "second");
}

/// Reading does not consume: a second surface, positioned where the first one
/// started, receives the same utterance rather than finding it gone. This is the
/// property a drain-and-delete bus could not offer, and the reason the desktop
/// window and the menu-bar popover can watch one conversation at once.
#[tokio::test]
async fn a_second_reader_receives_the_same_utterance() {
    let (base, _dir, seams) = spawn_server().await;

    emit_utterance(&seams.text_bus, &["shared"]).await;

    let first = get_out_text(&base, None, Duration::from_millis(500))
        .await
        .expect("first reader");
    let second = get_out_text(&base, None, Duration::from_millis(500))
        .await
        .expect("second reader");
    assert_eq!(first, "shared");
    assert_eq!(second, "shared", "the first read must not have consumed it");
}

/// A reader caught up with the log parks rather than closing empty — the long-poll
/// contract. Nothing is pending past cursor 0, so this must time out.
#[tokio::test]
async fn a_caught_up_reader_parks() {
    let (base, _dir, seams) = spawn_server().await;

    emit_utterance(&seams.text_bus, &["only one"]).await;
    tokio::time::sleep(Duration::from_millis(30)).await;

    let caught_up = get_out_text(&base, Some(0), Duration::from_millis(250)).await;
    assert!(caught_up.is_err(), "should park, got {caught_up:?}");
}
