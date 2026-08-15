//! `/api/out/text` is the conversation: an append-only message list the backend
//! owns. These tests pin the contract at the HTTP boundary — that the list is
//! whole on connect, that it survives a restart, that it never carries reader
//! identity, and that nothing is ever rewritten or suppressed.

use std::pin::Pin;
use std::time::Duration;

use axum::Router;
use bytes::Bytes;
use chrono::Utc;
use futures::{Stream, StreamExt};
use hi_agent::body::reaction::OutboundSignal;
use hi_agent::foundation::surfaces::{Acceptor, accepted_on};
use hi_agent::foundation::server::{self, ServerSeams};
use hi_agent::mind::memory::Memory;
use serde::Deserialize;
use tempfile::tempdir;
use tokio::net::TcpListener;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct Message {
    id: String,
    role: String,
    text: String,
    #[serde(default)]
    attachment: Option<Attachment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct Attachment {
    #[serde(rename = "ref")]
    reff: String,
    mime: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Frame {
    Reset {
        messages: Vec<Message>,
        interim: Option<String>,
    },
    Append(Message),
    Interim(Option<String>),
}

impl Frame {
    fn reset(self) -> Vec<Message> {
        match self {
            Frame::Reset { messages, .. } => messages,
            other => panic!("expected the opening window, got {other:?}"),
        }
    }

    fn appended(self) -> Message {
        match self {
            Frame::Append(m) => m,
            other => panic!("expected an append, got {other:?}"),
        }
    }
}

struct Feed {
    chunks: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
    buffer: Vec<u8>,
}

impl Feed {
    async fn open(base: &str) -> Self {
        let response = reqwest::Client::new()
            .get(format!("{base}/api/out/text"))
            .send()
            .await
            .expect("open the conversation stream");
        assert!(response.status().is_success());
        assert_eq!(
            response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/x-ndjson; charset=utf-8")
        );
        // The retired delivery protocol stays retired: no utterance number, no
        // epoch, nothing a client could send back to claim progress.
        assert!(response.headers().get("X-HI-Utterance").is_none());
        assert!(response.headers().get("X-HI-Text-Epoch").is_none());
        Self {
            chunks: Box::pin(response.bytes_stream()),
            buffer: Vec::new(),
        }
    }

    async fn next(&mut self) -> Frame {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(newline) = self.buffer.iter().position(|b| *b == b'\n') {
                    let mut line: Vec<u8> = self.buffer.drain(..=newline).collect();
                    line.pop();
                    return serde_json::from_slice(&line).unwrap_or_else(|e| {
                        panic!("valid frame: {e} in {:?}", String::from_utf8_lossy(&line))
                    });
                }
                let chunk = self
                    .chunks
                    .next()
                    .await
                    .expect("the stream stays open")
                    .expect("stream chunk");
                self.buffer.extend_from_slice(&chunk);
            }
        })
        .await
        .expect("next frame")
    }
}

fn serve(dir: &std::path::Path, memory: Memory) -> (Router, ServerSeams) {
    let (router, seams) = server::build(
        memory,
        dir.to_path_buf(),
        hi_agent::foundation::observatory::Observatory::new(None),
        hi_agent::foundation::codex::WireTap::new(),
        hi_agent::body::reaction::ToolRegistry::new(),
        hi_agent::body::reaction::Floor::new(),
        hi_agent::body::attachments::Attachments::new(),
        None,
    );
    // A test is a local caller, and says so: without an acceptor the gate
    // fails closed and every request here would be a 401.
    (accepted_on(router, Acceptor::Loopback), seams)
}

async fn bind(router: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    format!("http://{addr}")
}

async fn spawn_server() -> (String, tempfile::TempDir, ServerSeams, Memory) {
    let dir = tempdir().expect("tempdir");
    let memory = Memory::open(dir.path()).await.expect("memory");
    let (router, seams) = serve(dir.path(), memory.clone());
    let base = bind(router).await;
    (base, dir, seams, memory)
}

/// Say something as the agent, exactly the way `reaction::emit_message` does: mint
/// one id, journal the `SignalOut` under it, then send it down the outbound seam
/// for the binder to append.
///
/// **The single id is the part under test.** The conversation is rebuilt from the
/// journal at boot, so if these two ever drifted apart — a key each, or a journal
/// row that never happens — a restart would show a different conversation from the
/// one that was live. `the_conversation_survives_a_restart` is what catches that.
async fn agent_says(seams: &ServerSeams, memory: &Memory, text: &str) {
    let id = uuid::Uuid::now_v7().to_string();
    let ts = Utc::now();
    memory
        .journal
        .append(hi_agent::types::JournalEntry::SignalOut {
            id: id.clone(),
            ts,
            channel: hi_agent::types::Channel::Text,
            body: text.to_owned(),
            media: None,
            origin: Some(hi_agent::types::Origin::Reaction),
        })
        .await
        .expect("journal the message");
    seams
        .out_tx
        .send(OutboundSignal::Text { id, ts, text: text.to_owned() })
        .await
        .expect("outbound seam");
}

async fn post_text(base: &str, body: &str) {
    let response = reqwest::Client::new()
        .post(format!("{base}/api/in/text"))
        .body(body.to_owned())
        .send()
        .await
        .expect("post text");
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
}

/// The change this whole contract exists for: a window that opens later sees the
/// conversation, not just whatever was said most recently.
#[tokio::test]
async fn a_late_surface_receives_the_conversation_not_just_the_present() {
    let (base, _dir, seams, memory) = spawn_server().await;
    post_text(&base, "first").await;
    agent_says(&seams, &memory, "old answer").await;
    post_text(&base, "second").await;
    agent_says(&seams, &memory, "current answer").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut feed = Feed::open(&base).await;
    let texts: Vec<String> = feed.next().await.reset().into_iter().map(|m| m.text).collect();
    assert_eq!(texts, ["first", "old answer", "second", "current answer"]);
}

#[tokio::test]
async fn an_open_connection_receives_each_message_as_it_is_appended() {
    let (base, _dir, seams, memory) = spawn_server().await;
    let mut feed = Feed::open(&base).await;
    assert!(feed.next().await.reset().is_empty());

    post_text(&base, "hello").await;
    let m = feed.next().await.appended();
    assert_eq!(m.text, "hello");
    assert_eq!(m.role, "user");

    agent_says(&seams, &memory, "hi").await;
    let m = feed.next().await.appended();
    assert_eq!(m.text, "hi");
    assert_eq!(m.role, "agent");
}

#[tokio::test]
async fn two_windows_converge_without_reader_identity() {
    let (base, _dir, _seams, _memory) = spawn_server().await;
    let mut first = Feed::open(&base).await;
    let mut second = Feed::open(&base).await;
    assert!(first.next().await.reset().is_empty());
    assert!(second.next().await.reset().is_empty());

    post_text(&base, "shared").await;
    assert_eq!(first.next().await, second.next().await);
}

/// A dropped connection is nothing to recover from. The window reconnects and gets
/// the conversation whole — no cursor, no resume, no gap.
#[tokio::test]
async fn reconnecting_receives_the_whole_conversation_again() {
    let (base, _dir, seams, memory) = spawn_server().await;
    let mut first = Feed::open(&base).await;
    assert!(first.next().await.reset().is_empty());
    agent_says(&seams, &memory, "before the drop").await;
    assert_eq!(first.next().await.appended().text, "before the drop");
    drop(first);

    agent_says(&seams, &memory, "while nobody was connected").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut reconnected = Feed::open(&base).await;
    let texts: Vec<String> = reconnected
        .next()
        .await
        .reset()
        .into_iter()
        .map(|m| m.text)
        .collect();
    assert_eq!(texts, ["before the drop", "while nobody was connected"]);
}

/// The eligibility rule is gone with the slot it protected. A reply from a turn
/// that started before a new human line is not suppressed — it lands after that
/// line, which is what actually happened and how a person reads a crossed message.
#[tokio::test]
async fn a_reply_that_crossed_with_a_new_line_lands_after_it() {
    let (base, _dir, seams, memory) = spawn_server().await;
    let mut feed = Feed::open(&base).await;
    assert!(feed.next().await.reset().is_empty());

    seams.state.floor.note_turn_started(7);
    post_text(&base, "actually never mind").await;
    assert_eq!(feed.next().await.appended().text, "actually never mind");

    agent_says(&seams, &memory, "answering the older question").await;
    let m = feed.next().await.appended();
    assert_eq!(m.text, "answering the older question");
    assert_eq!(m.role, "agent");
}

/// The failure that started all of this: after a restart the only record of what
/// the agent had said was the log. Now the process comes back up into the
/// conversation it was already having.
#[tokio::test]
async fn the_conversation_survives_a_restart() {
    let dir = tempdir().expect("tempdir");

    {
        let memory = Memory::open(dir.path()).await.expect("memory");
        let (router, seams) = serve(dir.path(), memory.clone());
        let base = bind(router).await;
        post_text(&base, "before the restart").await;
        agent_says(&seams, &memory, "answered before the restart").await;
        tokio::time::sleep(Duration::from_millis(80)).await;
    }

    // A second process over the same data dir — nothing carried in memory.
    let memory = Memory::open(dir.path()).await.expect("memory");
    let (router, _seams) = serve(dir.path(), memory);
    let base = bind(router).await;
    // The seed runs off the boot path; give it a moment to land.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut feed = Feed::open(&base).await;
    let texts: Vec<String> = feed.next().await.reset().into_iter().map(|m| m.text).collect();
    assert_eq!(texts, ["before the restart", "answered before the restart"]);
}

/// Scrollback reads the same conversation the live window shows, through the same
/// mapping — so what you scroll into is not a second, differently-shaped view of
/// the log.
#[tokio::test]
async fn scrollback_reads_older_messages_through_the_same_mapping() {
    let (base, _dir, seams, memory) = spawn_server().await;
    post_text(&base, "oldest").await;
    agent_says(&seams, &memory, "middle").await;
    post_text(&base, "newest").await;
    tokio::time::sleep(Duration::from_millis(80)).await;

    let mut feed = Feed::open(&base).await;
    let window = feed.next().await.reset();
    assert_eq!(window.len(), 3);
    let newest_id = window.last().unwrap().id.clone();

    let older: Vec<Message> = reqwest::Client::new()
        .get(format!("{base}/api/messages?before={newest_id}"))
        .send()
        .await
        .expect("scrollback")
        .json()
        .await
        .expect("scrollback json");
    let texts: Vec<String> = older.into_iter().map(|m| m.text).collect();
    assert_eq!(texts, ["oldest", "middle"]);
}

/// A file the person hands over is a message. It used to be invisible on the face
/// entirely — journaled, reacted to, and never shown.
#[tokio::test]
async fn a_handed_file_becomes_a_message_with_its_bytes_reachable() {
    let (base, _dir, _seams, _memory) = spawn_server().await;
    let mut feed = Feed::open(&base).await;
    assert!(feed.next().await.reset().is_empty());

    // Hand-rolled rather than `reqwest::multipart`, which is behind a feature this
    // crate's dev-dependencies do not enable. The shape is fixed and tiny.
    const BOUNDARY: &str = "hiagenttestboundary";
    let body = format!(
        "--{BOUNDARY}\r\n\
         Content-Disposition: form-data; name=\"file\"; filename=\"shot.png\"\r\n\
         Content-Type: image/png\r\n\r\n\
         not really a png\r\n\
         --{BOUNDARY}--\r\n"
    );
    let response = reqwest::Client::new()
        .post(format!("{base}/api/in/file"))
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(body)
        .send()
        .await
        .expect("post file");
    assert!(response.status().is_success(), "{:?}", response.status());

    let m = feed.next().await.appended();
    assert_eq!(m.role, "user");
    assert!(m.text.contains("shot.png"), "the framing names the file: {}", m.text);
    assert!(
        !m.text.contains("⟨ref:"),
        "the locator is for the agent, not the chat: {}",
        m.text
    );
    let attachment = m.attachment.expect("the file rides along");
    assert_eq!(attachment.mime, "image/png");

    // And the bytes are actually fetchable, so the face can render it.
    let fetched = reqwest::Client::new()
        .get(format!("{base}/api/media/{}", attachment.reff))
        .send()
        .await
        .expect("fetch media");
    assert!(fetched.status().is_success());
    assert_eq!(
        fetched.headers().get(reqwest::header::CONTENT_TYPE).unwrap(),
        "image/png"
    );
    assert_eq!(fetched.bytes().await.unwrap().as_ref(), b"not really a png");
}

/// A rolling recognition partial is a preview of a message, not a message: it
/// never enters the list, and the settled line clears it.
#[tokio::test]
async fn an_interim_is_not_a_message() {
    let (base, _dir, seams, _memory) = spawn_server().await;
    let mut feed = Feed::open(&base).await;
    assert!(feed.next().await.reset().is_empty());

    seams
        .state
        .note_interim(hi_agent::types::Channel::Text, "what day is");
    assert_eq!(feed.next().await, Frame::Interim(Some("what day is".into())));

    post_text(&base, "what day is it?").await;
    assert_eq!(feed.next().await, Frame::Interim(None));
    assert_eq!(feed.next().await.appended().text, "what day is it?");
}
