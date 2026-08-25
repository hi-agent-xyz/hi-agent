//! Smoke test for the HTTP route surface.
//!
//! Builds the axum router via [`hi_agent::foundation::server::build`] directly. The
//! reaction seams are returned alongside so the test holds them past the
//! handlers' send into `inbound` — otherwise the receiver drops and
//! POST /api/in/text returns 503.

use std::time::Duration;

use hi_agent::mind::memory::Memory;
use hi_agent::foundation::surfaces::{Acceptor, accepted_on};
use hi_agent::foundation::server::{self, ServerSeams};
use hi_agent::types::{Channel, Content, JournalEntry};
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
        hi_agent::foundation::codex::WireTap::new(),
        hi_agent::foundation::privacy::PrivacyBoundary::open(dir.path()).unwrap(),
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

    (format!("http://{addr}"), dir, seams)
}

/// Read every per-channel `*.jsonl` under `memory/raw/` into typed entries.
fn read_journal(dir: &std::path::Path) -> Vec<JournalEntry> {
    let mut out = Vec::new();
    for log in walk_files(&dir.join("memory").join("raw")) {
        if log.extension().and_then(|n| n.to_str()) != Some("jsonl") {
            continue;
        }
        let contents = std::fs::read_to_string(&log).expect("read log");
        for line in contents.lines().filter(|l| !l.trim().is_empty()) {
            out.push(serde_json::from_str(line).expect("decode journal entry"));
        }
    }
    out
}

/// Every file (recursively) under `root`; empty if `root` does not exist.
fn walk_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&path) else {
            continue;
        };
        for ent in rd.flatten() {
            let p = ent.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                files.push(p);
            }
        }
    }
    files
}

fn multipart(files: &[(&str, &str, &[u8])]) -> (String, Vec<u8>) {
    let boundary = "hi-agent-http-smoke-boundary";
    let mut body = Vec::new();
    for (name, mime, bytes) in files {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\n\
                 Content-Type: {mime}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

#[tokio::test]
async fn post_thought_accepts_and_journals() {
    let (base, dir, _seams) = spawn_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/api/in/text"))
        .body("hi")
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), reqwest::StatusCode::ACCEPTED);

    tokio::time::sleep(Duration::from_millis(50)).await;
    let entries = read_journal(dir.path());
    assert_eq!(entries.len(), 1, "expected one journal entry, got {entries:?}");
    match &entries[0] {
        JournalEntry::Message { message, .. } => {
            assert_eq!(message.content.text(), Some("hi"));
        }
        other => panic!("expected SignalIn, got {other:?}"),
    }
}

/// A body far past the old framework default, which used to answer 413 and lose the
/// text. It is accepted, kept verbatim, and arrives as something the prompt can carry:
/// a ref and an opening, not a megabyte of words.
#[tokio::test]
async fn a_large_paste_is_kept_whole_and_arrives_as_an_artifact() {
    let (base, dir, _seams) = spawn_server().await;
    let client = reqwest::Client::new();

    // Distinguishable rows, so a wrong offset or a lost chunk fails loudly rather than
    // passing on a body that happens to be the right length.
    let pasted: String = (0..120_000).map(|i| format!("row {i} of the pasted log\n")).collect();
    assert!(pasted.len() > 2 * 1024 * 1024, "the point is to exceed the old 2 MB ceiling");

    let resp = client
        .post(format!("{base}/api/in/text"))
        .body(pasted.clone())
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), reqwest::StatusCode::ACCEPTED);

    tokio::time::sleep(Duration::from_millis(200)).await;
    let entries = read_journal(dir.path());
    assert_eq!(entries.len(), 1, "expected one journal entry, got {} ", entries.len());
    let JournalEntry::Message { channel, message } = &entries[0] else {
        panic!("expected a message, got {:?}", entries[0]);
    };
    // The file channel, which forgetting exempts — a person's own words do not fade.
    assert_eq!(*channel, Channel::File);
    let Content::File(file) = &message.content else {
        panic!("expected a file, got {:?}", message.content);
    };
    assert_eq!(file.bytes, Some(pasted.len() as u64));
    assert!(file.name.starts_with("pasted-"), "{}", file.name);
    let peek = file.peek.as_deref().expect("a text artifact opens somewhere");
    assert!(peek.starts_with("row 0 of the pasted log"), "{peek:?}");
    assert!(peek.len() < pasted.len(), "a peek is an opening, not the content");

    // Verbatim: the bytes on disk are the bytes that were sent, to the last row.
    // A ref is `<channel>/<day>/<rel>` under `memory/raw`, so it is already the path.
    let blob = dir.path().join("memory").join("raw").join(&file.reff);
    let stored = std::fs::read_to_string(&blob)
        .unwrap_or_else(|e| panic!("read {}: {e}", blob.display()));
    assert_eq!(stored, pasted, "the artifact must hold exactly what was handed over");
}

/// The seam is a size test, and a body just under it is still words. Guards against a
/// creeping threshold quietly turning ordinary long messages into files.
#[tokio::test]
async fn a_body_under_the_seam_is_still_words() {
    let (base, dir, _seams) = spawn_server().await;
    let client = reqwest::Client::new();

    let long = "x".repeat(64 * 1024 - 1);
    let resp = client
        .post(format!("{base}/api/in/text"))
        .body(long.clone())
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), reqwest::StatusCode::ACCEPTED);

    tokio::time::sleep(Duration::from_millis(100)).await;
    let entries = read_journal(dir.path());
    let JournalEntry::Message { channel, message } = &entries[0] else {
        panic!("expected a message, got {:?}", entries[0]);
    };
    assert_eq!(*channel, Channel::Text);
    assert_eq!(message.content.text(), Some(long.as_str()));
}

#[tokio::test]
async fn post_files_returns_a_structured_batch_result_and_journals_each_file() {
    let (base, dir, _seams) = spawn_server().await;
    let client = reqwest::Client::new();
    let files = [
        ("notes.txt", "text/plain", b"hello".as_slice()),
        ("scan.pdf", "application/pdf", b"%PDF".as_slice()),
    ];
    let (content_type, body) = multipart(&files);

    let resp = client
        .post(format!("{base}/api/in/file"))
        .header("Content-Type", content_type)
        .body(body)
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let result: serde_json::Value = resp.json().await.expect("json result");
    assert_eq!(result["attempted"], 2);
    assert_eq!(result["received"], 2);
    assert_eq!(result["failed"], serde_json::json!([]));

    tokio::time::sleep(Duration::from_millis(50)).await;
    let entries = read_journal(dir.path());
    assert_eq!(entries.len(), 2, "each uploaded file should be journaled");
    let mut bodies = entries
        .iter()
        .map(|entry| match entry {
            JournalEntry::Message { channel, message } => {
                assert_eq!(*channel, Channel::File);
                let Content::File(f) = &message.content else {
                    panic!("a file handoff is file content, got {message:?}");
                };
                assert!(!f.reff.is_empty(), "file handoff carries its stored blob");
                (f.name.as_str(), f.reff.as_str())
            }
            other => panic!("expected a message, got {other:?}"),
        })
        .collect::<Vec<_>>();
    bodies.sort_unstable();
    // The name a person chose and the locator for the bytes are two fields now, and
    // neither is recovered from prose: the blob sits on a timestamp grid, so the ref
    // never contained the filename in the first place.
    assert_eq!(bodies[0].0, "notes.txt");
    assert_eq!(bodies[1].0, "scan.pdf");

    // The locator, checked by *opening* it. `docs/arch/agents.md` retires the
    // perception tool on the grounds that a handed file arrives as a ref and a ref
    // is a path — so asserting the text merely contains `⟨ref:` would pin the shape
    // of the lie this fixes. The body used to name only the original filename, which
    // is not what the bytes are stored as.
    // The ref names its own channel, so it joins onto the *raw root* — a reader that
    // has to supply the channel is the thing the grammar removed.
    let raw = dir.path().join("memory").join("raw");
    // The ref is a field on the file's content now, not a marker fished back out of
    // prose — which is the whole of what `docs/arch/message.md` moved.
    for (_, reff) in &bodies {
        assert!(reff.starts_with("file/"), "a handed file's ref must name its channel: {reff}");
        let path = raw.join(reff);
        assert!(path.is_file(), "the ref must resolve under {raw:?}: {reff}");
        assert!(!std::fs::read(&path).unwrap().is_empty(), "the ref must reach the bytes");
    }
}

#[tokio::test]
async fn post_files_reports_each_failed_part_when_the_inbound_lane_is_closed() {
    let (base, _dir, seams) = spawn_server().await;
    drop(seams);
    let client = reqwest::Client::new();
    let files = [
        ("a.txt", "text/plain", b"a".as_slice()),
        ("b.txt", "text/plain", b"b".as_slice()),
    ];
    let (content_type, body) = multipart(&files);

    let resp = client
        .post(format!("{base}/api/in/file"))
        .header("Content-Type", content_type)
        .body(body)
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), reqwest::StatusCode::MULTI_STATUS);
    let result: serde_json::Value = resp.json().await.expect("json result");
    assert_eq!(result["attempted"], 2);
    assert_eq!(result["received"], 0);
    let failed = result["failed"].as_array().expect("failed array");
    assert_eq!(failed.len(), 2);
    assert_eq!(failed[0]["index"], 0);
    assert_eq!(failed[0]["name"], "a.txt");
    assert_eq!(failed[1]["index"], 1);
    assert_eq!(failed[1]["name"], "b.txt");
}

#[tokio::test]
async fn post_vision_journals_and_persists() {
    // A still is accepted (202), persisted as bytes, AND journaled as a vision
    // signal whose `body` is a caption — from the vision capability, or a
    // placeholder when none is configured (the case here: capabilities are never
    // initialized in this test). Perception is spawned, so we poll for the line.
    let (base, dir, _seams) = spawn_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/api/in/vision"))
        .header("Content-Type", "image/jpeg")
        .body(vec![0xFFu8, 0xD8, 0xFF, 0xD9]) // minimal JPEG-ish bytes
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), reqwest::StatusCode::ACCEPTED);

    let mut entries = read_journal(dir.path());
    for _ in 0..40 {
        if !entries.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        entries = read_journal(dir.path());
    }
    assert_eq!(entries.len(), 1, "vision still should journal one entry, got {entries:?}");
    match &entries[0] {
        JournalEntry::Observation { channel, body, media, .. } => {
            assert_eq!(*channel, Channel::Vision);
            assert!(!body.is_empty(), "vision signal carries a caption (placeholder when no provider)");
            let media = media.as_ref().expect("vision signal carries media");
            assert!(media.file.ends_with(".jpg"), "relative blob path, got {}", media.file);
        }
        other => panic!("expected SignalIn, got {other:?}"),
    }

    // The bytes landed as a `.jpg` under the vision channel folder.
    let raw = dir.path().join("memory").join("raw");
    let frames = walk_files(&raw)
        .into_iter()
        .filter(|p| p.extension().and_then(|n| n.to_str()) == Some("jpg"))
        .count();
    assert_eq!(frames, 1, "expected one persisted vision frame under {raw:?}");
}

#[tokio::test]
async fn all_sensory_stubs_return_501() {
    // touch/smell/taste are still 501 in v0. /audio returns 501 only when
    // STT is unconfigured, which the test forces by never calling
    // capabilities::init — the STT global stays uninitialized, so
    // stt::available() is false.
    let (base, _dir, _seams) = spawn_server().await;
    let client = reqwest::Client::new();

    for ch in ["touch", "smell", "taste"] {
        let resp = client
            .post(format!("{base}/api/in/{ch}"))
            .body("...")
            .send()
            .await
            .expect("send");
        assert_eq!(
            resp.status(),
            reqwest::StatusCode::NOT_IMPLEMENTED,
            "POST /api/in/{ch} should be 501"
        );
    }

    // POST /api/in/audio with no STT configured: 501 with the new (capability-gated)
    // body.
    let resp = client
        .post(format!("{base}/api/in/audio"))
        .header("Content-Type", "audio/wav")
        .body(vec![0u8; 16])
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_IMPLEMENTED);
    let body = resp.text().await.expect("body");
    assert!(
        body.contains("capability not configured"),
        "501 body should explain the capability gate, got: {body}"
    );
}

#[tokio::test]
async fn homepage_returns_html() {
    let (base, _dir, _seams) = spawn_server().await;
    let client = reqwest::Client::new();

    let resp = client.get(format!("{base}/")).send().await.expect("send");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap_or("").to_string())
        .unwrap_or_default();
    assert!(
        ct.starts_with("text/html"),
        "expected text/html, got {ct:?}"
    );
    let body = resp.text().await.expect("body");
    assert!(body.contains("<html") || body.contains("<!doctype"));
}

#[tokio::test]
async fn stats_returns_the_requested_window_and_rejects_an_unknown_one() {
    let (base, _dir, _seams) = spawn_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base}/api/stats?range=7d"))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("stats JSON");
    assert_eq!(body["period"]["range"], "7d");
    assert_eq!(body["period"]["timezone"], "UTC");
    assert_eq!(body["series"].as_array().expect("daily series").len(), 7);
    for key in ["tokens", "sessions", "conversation", "tools", "tasks", "energy"] {
        assert!(body["summary"].get(key).is_some(), "summary is missing {key}");
    }
    for key in ["facets", "episodes", "skills", "people", "drive", "custom_views"] {
        assert!(body["inventory"].get(key).is_some(), "inventory is missing {key}");
    }

    let resp = client
        .get(format!("{base}/api/stats?range=forever"))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.json().await.expect("error JSON");
    assert_eq!(body["error"], "range must be 7d, 30d, 90d or all");
}

#[tokio::test]
async fn vision_get_streams_camera_video() {
    // "Vision is video": the camera streams WebM over WS /api/in/vision/stream and
    // GET /api/in/vision plays it back — one camera session per long-poll response,
    // carrying the stream's Content-Type. The first chunk is the init segment; the
    // GET body is the concatenation of every chunk the camera sent.
    use futures::SinkExt;
    use tokio_tungstenite::tungstenite::Message;

    let (base, _dir, _seams) = spawn_server().await;

    // The GET blocks until a camera starts, so drive it from a task and open the
    // streaming WS after it has had time to subscribe.
    let get_base = base.clone();
    let getter = tokio::spawn(async move {
        let c = reqwest::Client::new();
        let resp = c
            .get(format!("{get_base}/api/in/vision"))
            .send()
            .await
            .expect("send GET");
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        let ct = resp
            .headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap_or("").to_string())
            .unwrap_or_default();
        let body = resp.bytes().await.expect("body");
        (ct, body)
    });

    tokio::time::sleep(Duration::from_millis(80)).await;

    let ws_url = format!(
        "{}/api/in/vision/stream?mime=video/webm",
        base.replace("http://", "ws://")
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url).await.expect("ws connect");
    let init = vec![0x1A, 0x45, 0xDF, 0xA3]; // EBML magic — stands in for the init segment
    let frame = vec![0x42u8, 0x82, 0x88];
    ws.send(Message::binary(init.clone())).await.expect("send init");
    ws.send(Message::binary(frame.clone())).await.expect("send frame");
    // Give the frames time to fan out to the GET body before closing the source.
    tokio::time::sleep(Duration::from_millis(80)).await;
    ws.close(None).await.expect("close ws");

    let (ct, body) = tokio::time::timeout(Duration::from_secs(2), getter)
        .await
        .expect("vision GET within timeout")
        .expect("getter task");
    assert!(ct.starts_with("video/webm"), "content-type echoes stream mime, got {ct:?}");
    let mut expected = init.clone();
    expected.extend_from_slice(&frame);
    assert_eq!(body.as_ref(), expected.as_slice(), "GET body is the streamed chunks");
}

/// The stage lane, end to end over HTTP: a frame the window posts is the frame a
/// review renders into.
///
/// The unit tests either side of this prove the store's semantics and the
/// handler's validation; what only a routed request can show is that the two are
/// actually wired to each other — a `/api/stage` that answered 202 and updated
/// nothing would pass both of them and still leave every review at the fallback
/// size, which is the exact defect this lane exists to remove.
#[tokio::test]
async fn a_posted_window_frame_becomes_the_review_viewport() {
    use hi_agent::body::capabilities::view_render;

    let (base, _dir, _seams) = spawn_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/api/stage"))
        .json(&serde_json::json!({ "width": 1728, "height": 1080, "scale": 2.0 }))
        .send()
        .await
        .expect("post stage");
    assert_eq!(resp.status(), 202, "a good frame is accepted");
    assert_eq!(
        (view_render::stage_frame().width, view_render::stage_frame().height),
        (1728, 1080),
        "the reported frame is what a review will render into",
    );

    // A frame no window has is refused, and the last good one stands — a resize
    // drag must not be able to leave reviews rendering at a sliver.
    let resp = client
        .post(format!("{base}/api/stage"))
        .json(&serde_json::json!({ "width": 2, "height": 1 }))
        .send()
        .await
        .expect("post bad stage");
    assert_eq!(resp.status(), 400);
    assert_eq!(view_render::stage_frame().width, 1728, "the last good frame survives");
}
