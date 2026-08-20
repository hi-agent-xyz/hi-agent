//! The two seams, end to end: a key typed into the text channel is filed, and the
//! text a model session would be handed carries the file's path instead.
//!
//! What this asserts as loudly as the masking is what is **not** touched — the
//! journal and `/api/out/text` still carry the exact characters that were typed.
//! `docs/arch/privacy.md` scopes this to the unconscious moment: a person pasting
//! a credential mid-sentence. It is not a vault, and everything downstream of the
//! person is deliberately left alone.

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use hi_agent::foundation::privacy::{PrivacyBoundary, broker};
use hi_agent::foundation::server;
use hi_agent::foundation::surfaces::{Acceptor, accepted_on};
use hi_agent::mind::memory::Memory;
use serde_json::json;
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

async fn bind(router: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

/// Everything written under the raw root, concatenated — the day folder's name is
/// not this test's business, only that the exact characters survived into it.
fn raw_journal_text(root: &std::path::Path) -> String {
    let mut out = String::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(text) = std::fs::read_to_string(&path) {
                out.push_str(&text);
            }
        }
    }
    out
}

fn a_key() -> String {
    [
        "sk-proj-",
        "abcdefghij_klmnopqrst-uvwxyz0123456789ABCDEFGHIJ",
    ]
    .concat()
}

#[tokio::test]
async fn a_typed_key_is_filed_while_the_conversation_keeps_it_verbatim() {
    let dir = tempdir().unwrap();
    let memory = Memory::open(dir.path()).await.unwrap();
    let privacy = PrivacyBoundary::open(dir.path()).unwrap();
    let (router, seams) = server::build(
        memory,
        dir.path().to_path_buf(),
        hi_agent::foundation::observatory::Observatory::new(None),
        hi_agent::foundation::codex::WireTap::new(),
        privacy.clone(),
        hi_agent::body::reaction::ToolRegistry::new(),
        hi_agent::body::reaction::Floor::new(),
        hi_agent::body::attachments::Attachments::new(),
        None,
    );
    let base = bind(accepted_on(router, Acceptor::Loopback)).await;

    let key = a_key();
    let typed = format!("here's the key {key}, check it works against the API");
    let response = reqwest::Client::new()
        .post(format!("{base}/api/in/text"))
        .body(typed.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);

    // Filed as one ordinary file whose whole content is the credential, at the
    // path the agent is told to use.
    let stored = privacy.store().active_values().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].value, key);
    assert_eq!(
        stored[0].reference,
        "drive/accounts/secrets/openai-api-key.txt"
    );
    let on_disk = dir.path().join(&stored[0].reference);
    assert_eq!(std::fs::read_to_string(&on_disk).unwrap(), key);

    // The seam: what a session is handed names the file and never the value.
    let masked = privacy.store().mask_known(&typed);
    assert!(!masked.contains(&key));
    assert!(masked.contains("⟨secret: drive/accounts/secrets/openai-api-key.txt⟩"));
    assert!(masked.contains("check it works against the API"));

    // …and the person's own record of what they said is untouched. Both of these
    // would have been masked by an egress filter; neither may be here.
    let mut inbound_rx = seams.inbound_rx;
    let signal = inbound_rx.recv().await.expect("the signal reaches the mind");
    assert_eq!(signal.body, typed, "the mind receives the message verbatim");

    let journalled = raw_journal_text(&hi_agent::mind::memory::layout::raw_root(dir.path()));
    assert!(
        journalled.contains(&key),
        "the journal is the person's record and keeps the exact text"
    );

    // The file is an ordinary drive file: visible, listable, and readable, which is
    // what lets a command consume it without the value entering a prompt.
    let listing = reqwest::get(format!("{base}/api/drive"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(listing.contains("accounts/secrets/"));
    let direct = reqwest::get(format!(
        "{base}/api/drive/file/{}",
        stored[0].reference.trim_start_matches("drive/")
    ))
    .await
    .unwrap();
    assert_eq!(direct.status(), reqwest::StatusCode::OK);
    assert_eq!(direct.text().await.unwrap(), key);
}

/// The turn after next is where the old design leaked: the journal snapshot renders
/// the same message back into the prompt, so masking has to be a property of the
/// text rather than of the moment it arrived.
#[tokio::test]
async fn the_same_message_masks_identically_on_a_later_turn() {
    let dir = tempdir().unwrap();
    let privacy = PrivacyBoundary::open(dir.path()).unwrap();
    let key = a_key();
    let typed = format!("key {key}");
    privacy.filter().file_secrets(&typed).unwrap();

    let first = privacy.store().mask_known(&typed).into_owned();
    // A fresh boundary over the same drive — a restart — must agree.
    let reopened = PrivacyBoundary::open(dir.path()).unwrap();
    assert_eq!(reopened.store().mask_known(&typed), first);
    assert!(!first.contains(&key));
}

/// The capability the whole design is built around keeping: the agent spends the
/// credential without ever being shown it.
#[tokio::test]
async fn the_broker_spends_a_filed_key_the_model_never_saw() {
    async fn api(State(seen): State<Arc<Mutex<Option<String>>>>, headers: HeaderMap) -> String {
        let authorization = headers["authorization"].to_str().unwrap().to_string();
        *seen.lock().await = Some(authorization.clone());
        "ok".to_string()
    }

    let seen = Arc::new(Mutex::new(None));
    let api_base = bind(
        Router::new()
            .route("/me", post(api))
            .with_state(seen.clone()),
    )
    .await;

    let dir = tempdir().unwrap();
    let privacy = PrivacyBoundary::open(dir.path()).unwrap();
    let key = a_key();
    let filed = privacy.filter().file_secrets(&format!("use {key}")).unwrap();
    let secret_ref = filed[0].reference.clone();

    let response = broker::http_request(
        &privacy,
        &json!({
            "method": "POST",
            "url": format!("{api_base}/me"),
            "auth_ref": secret_ref,
            "auth_scheme": "bearer"
        }),
    )
    .await
    .unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(
        seen.lock().await.as_deref(),
        Some(format!("Bearer {key}").as_str()),
        "the real credential reaches the destination"
    );
}
