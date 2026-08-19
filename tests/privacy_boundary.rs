use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use hi_agent::foundation::credentials::{Credentials, LlmCredentials, Mode};
use hi_agent::foundation::privacy::{PrivacyBoundary, broker};
use hi_agent::foundation::server;
use hi_agent::foundation::surfaces::{Acceptor, accepted_on};
use hi_agent::mind::memory::Memory;
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, mpsc};

async fn bind(router: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn responses_proxy_replaces_private_values_before_upstream() {
    async fn upstream(
        State(tx): State<mpsc::Sender<(HeaderMap, Value)>>,
        headers: HeaderMap,
        axum::Json(body): axum::Json<Value>,
    ) -> axum::Json<Value> {
        tx.send((headers, body)).await.unwrap();
        axum::Json(json!({ "id": "resp_test", "object": "response", "output": [] }))
    }

    let (seen_tx, mut seen_rx) = mpsc::channel(1);
    let upstream_base = bind(
        Router::new()
            .route("/v1/responses", post(upstream))
            .with_state(seen_tx),
    )
    .await;

    let dir = tempdir().unwrap();
    Credentials {
        mode: Mode::Byok,
        llm: LlmCredentials {
            base_url: format!("{upstream_base}/v1"),
            api_key: "upstream-provider-key".into(),
            model: Some("test-model".into()),
            ..Default::default()
        },
        ..Default::default()
    }
    .save(dir.path())
    .unwrap();

    let memory = Memory::open(dir.path()).await.unwrap();
    let privacy = PrivacyBoundary::open(dir.path()).unwrap();
    let proxy_token = privacy
        .child_env()
        .into_iter()
        .find(|(name, _)| name == hi_agent::foundation::privacy::ENV_MODEL_PROXY_KEY)
        .unwrap()
        .1;
    let (router, _seams) = server::build(
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

    let key = [
        "sk-proj-",
        "abcdefghij_klmnopqrst-uvwxyz0123456789ABCDEFGHIJ",
    ]
    .concat();
    let raw = format!("Email alice@example.com. Test {key} at https://api.example.test/v1/me");
    let response = reqwest::Client::new()
        .post(format!("{base}/internal/model/v1/responses"))
        .bearer_auth(&proxy_token)
        .json(&json!({ "model": "test-model", "input": raw }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let (headers, projected) = seen_rx.recv().await.unwrap();
    assert_eq!(
        headers["authorization"], "Bearer upstream-provider-key",
        "only the trusted proxy injects the upstream credential"
    );
    let projected = projected.to_string();
    assert!(!projected.contains(&key));
    assert!(!projected.contains("alice@example.com"));
    assert!(projected.contains("SECRET_REF"));
    assert!(projected.contains("PII:EMAIL_ADDRESS"));

    let stored = privacy.store().active_values().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].value, key);
    let portable_store = dir.path().join(&stored[0].reference);
    assert!(
        portable_store.is_file(),
        "the reference must be the ordinary file that travels with drive"
    );
    assert!(
        privacy
            .store()
            .resolve_for_http(&stored[0].reference)
            .is_ok(),
        "the HTTP broker reads the referenced text file"
    );
    let file = std::fs::read_to_string(&portable_store).unwrap();
    assert_eq!(file, key);
    assert!(stored[0].reference.starts_with("drive/accounts/secrets/"));

    let response = reqwest::Client::new()
        .post(format!("{base}/internal/model/v1/responses"))
        .bearer_auth(&proxy_token)
        .json(&json!({
            "model": "test-model",
            "input": [{
                "type": "function_call_output",
                "output": file
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let (_, projected_cli_output) = seen_rx.recv().await.unwrap();
    let projected_cli_output = projected_cli_output.to_string();
    assert!(!projected_cli_output.contains(&key));
    assert!(projected_cli_output.contains(&stored[0].reference));

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
}

#[tokio::test]
async fn broker_injects_a_bound_secret_and_redacts_an_echo() {
    async fn api(State(seen): State<Arc<Mutex<Option<String>>>>, headers: HeaderMap) -> String {
        let authorization = headers["authorization"].to_str().unwrap().to_string();
        *seen.lock().await = Some(authorization.clone());
        format!("received {authorization}")
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
    let secret = ["private", "-", "api", "-", "token", "-", "123456"].concat();
    let secret_ref = privacy
        .store()
        .upsert_detected(&secret, "GENERIC_SECRET")
        .unwrap();

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
    let expected = format!("Bearer {secret}");
    assert_eq!(seen.lock().await.as_deref(), Some(expected.as_str()));
    assert!(!response.body.contains(&secret));
    assert!(
        response
            .body
            .contains("[SECRET_REF:drive/accounts/secrets/")
    );
}
