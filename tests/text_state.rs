//! `/out/text` is one backend-owned current appearance, not a message-delivery
//! log. These tests pin the identity-free contract at the HTTP boundary.

use std::pin::Pin;
use std::time::Duration;

use bytes::Bytes;
use futures::{Stream, StreamExt};
use hi_agent::body::reaction::OutboundSignal;
use hi_agent::foundation::server::{self, ServerSeams};
use hi_agent::mind::memory::Memory;
use serde::Deserialize;
use tempfile::tempdir;
use tokio::net::TcpListener;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct AgentText {
    text: String,
    #[serde(rename = "final")]
    is_final: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
struct TextState {
    user: Option<String>,
    agent: Option<AgentText>,
    interim: Option<String>,
}

struct TextFeed {
    chunks: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
    buffer: Vec<u8>,
}

impl TextFeed {
    async fn open(base: &str) -> Self {
        let response = reqwest::Client::new()
            .get(format!("{base}/api/out/text"))
            .send()
            .await
            .expect("open text state stream");
        assert!(response.status().is_success());
        assert_eq!(
            response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/x-ndjson; charset=utf-8")
        );
        assert!(response.headers().get("X-HI-Utterance").is_none());
        assert!(response.headers().get("X-HI-Text-Epoch").is_none());
        Self {
            chunks: Box::pin(response.bytes_stream()),
            buffer: Vec::new(),
        }
    }

    async fn next(&mut self) -> TextState {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(newline) = self.buffer.iter().position(|b| *b == b'\n') {
                    let mut line: Vec<u8> = self.buffer.drain(..=newline).collect();
                    line.pop();
                    return serde_json::from_slice(&line).expect("valid text state");
                }
                let chunk = self
                    .chunks
                    .next()
                    .await
                    .expect("state stream stays open")
                    .expect("state stream chunk");
                self.buffer.extend_from_slice(&chunk);
            }
        })
        .await
        .expect("next text state")
    }
}

async fn spawn_server() -> (String, tempfile::TempDir, ServerSeams) {
    let dir = tempdir().expect("tempdir");
    let memory = Memory::open(dir.path()).await.expect("memory");
    let observatory = hi_agent::foundation::observatory::Observatory::new(None);
    let (router, seams) = server::build(
        memory,
        dir.path().to_path_buf(),
        observatory,
        hi_agent::foundation::codex::WireTap::new(),
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

#[tokio::test]
async fn a_fresh_surface_receives_the_present_state_immediately() {
    let (base, _dir, seams) = spawn_server().await;
    seams
        .text_appearance
        .note_user("current question", true, None);
    seams.text_appearance.begin_reaction_turn(0);
    seams
        .text_appearance
        .push_agent_chunk("current answer".into());
    seams.text_appearance.end_agent_utterance();

    let mut feed = TextFeed::open(&base).await;
    assert_eq!(
        feed.next().await,
        TextState {
            user: Some("current question".into()),
            agent: Some(AgentText {
                text: "current answer".into(),
                is_final: true,
            }),
            interim: None,
        }
    );
}

#[tokio::test]
async fn one_connection_receives_whole_state_replacements() {
    let (base, _dir, seams) = spawn_server().await;
    let mut feed = TextFeed::open(&base).await;
    assert_eq!(feed.next().await, TextState::default());

    seams.text_appearance.note_user("hello", true, None);
    assert_eq!(
        feed.next().await,
        TextState {
            user: Some("hello".into()),
            ..TextState::default()
        }
    );

    seams.text_appearance.begin_reaction_turn(0);
    seams.text_appearance.push_agent_chunk("hi".into());
    assert_eq!(
        feed.next().await.agent,
        Some(AgentText {
            text: "hi".into(),
            is_final: false,
        })
    );
}

#[tokio::test]
async fn a_late_surface_sees_only_the_latest_exchange() {
    let (base, _dir, seams) = spawn_server().await;
    seams.text_appearance.note_user("first", true, None);
    seams.text_appearance.begin_reaction_turn(0);
    seams.text_appearance.push_agent_chunk("old".into());
    seams.text_appearance.end_agent_utterance();

    seams.text_appearance.note_user("second", true, Some(0));
    seams.text_appearance.begin_reaction_turn(1);
    seams.text_appearance.push_agent_chunk("new".into());

    let mut feed = TextFeed::open(&base).await;
    let state = feed.next().await;
    assert_eq!(state.user.as_deref(), Some("second"));
    assert_eq!(state.agent.map(|a| a.text), Some("new".into()));
}

#[tokio::test]
async fn two_windows_converge_without_reader_identity() {
    let (base, _dir, seams) = spawn_server().await;
    let mut first = TextFeed::open(&base).await;
    let mut second = TextFeed::open(&base).await;
    assert_eq!(first.next().await, TextState::default());
    assert_eq!(second.next().await, TextState::default());

    seams.text_appearance.note_user("shared", true, None);
    assert_eq!(first.next().await, second.next().await);
}

#[tokio::test]
async fn posting_text_updates_the_shared_appearance() {
    let (base, _dir, _seams) = spawn_server().await;
    let mut feed = TextFeed::open(&base).await;
    assert_eq!(feed.next().await, TextState::default());

    let response = reqwest::Client::new()
        .post(format!("{base}/api/in/text"))
        .body("typed here")
        .send()
        .await
        .expect("post text");
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    assert_eq!(feed.next().await.user.as_deref(), Some("typed here"));
}

#[tokio::test]
async fn reconnecting_mid_reply_receives_the_whole_current_text() {
    let (base, _dir, seams) = spawn_server().await;
    let mut first = TextFeed::open(&base).await;
    assert_eq!(first.next().await, TextState::default());

    seams.text_appearance.begin_reaction_turn(0);
    seams.text_appearance.push_agent_chunk("inter".into());
    assert_eq!(
        first.next().await.agent,
        Some(AgentText {
            text: "inter".into(),
            is_final: false,
        })
    );
    drop(first);

    seams.text_appearance.push_agent_chunk("rupted".into());
    let mut reconnected = TextFeed::open(&base).await;
    assert_eq!(
        reconnected.next().await.agent,
        Some(AgentText {
            text: "interrupted".into(),
            is_final: false,
        })
    );
}

#[tokio::test]
async fn a_delayed_old_turn_marker_cannot_reclaim_a_new_human_line() {
    let (base, _dir, seams) = spawn_server().await;
    let mut feed = TextFeed::open(&base).await;
    assert_eq!(feed.next().await, TextState::default());

    // The reaction turn really started before this POST, but its marker is
    // deliberately delayed in the outbound binder queue.
    seams.state.interrupts.note_turn_started(7);
    let response = reqwest::Client::new()
        .post(format!("{base}/api/in/text"))
        .body("new question")
        .send()
        .await
        .expect("post text");
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    assert_eq!(feed.next().await.user.as_deref(), Some("new question"));

    seams
        .out_tx
        .send(OutboundSignal::TextTurnStart { turn: 7 })
        .await
        .expect("old turn marker");
    seams
        .out_tx
        .send(OutboundSignal::Text {
            chunk: "stale answer".into(),
        })
        .await
        .expect("old turn text");
    tokio::time::sleep(Duration::from_millis(20)).await;

    let mut reconnected = TextFeed::open(&base).await;
    let state = reconnected.next().await;
    assert_eq!(state.user.as_deref(), Some("new question"));
    assert_eq!(state.agent, None);

    seams
        .out_tx
        .send(OutboundSignal::TextTurnStart { turn: 8 })
        .await
        .expect("new turn marker");
    seams
        .out_tx
        .send(OutboundSignal::Text {
            chunk: "current answer".into(),
        })
        .await
        .expect("new turn text");
    assert_eq!(
        reconnected.next().await.agent,
        Some(AgentText {
            text: "current answer".into(),
            is_final: false,
        })
    );
}
