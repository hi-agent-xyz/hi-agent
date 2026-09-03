//! Isolated register → authenticated MCP use → release proof for external auto-runs.

use hi_agent::body::attachments::Attachments;
use hi_agent::body::reaction::{Floor, ToolRegistry};
use hi_agent::foundation::codex::WireTap;
use hi_agent::foundation::observatory::Observatory;
use hi_agent::foundation::registry::{self, Delivery, SessionSlug};
use hi_agent::foundation::server::{self, ServerSeams};
use hi_agent::foundation::surfaces::{self, Acceptor};
use hi_agent::mind::memory::Memory;
use serde_json::{Value, json};
use std::sync::Mutex;
use tempfile::tempdir;
use tokio::net::TcpListener;

static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Serialize these tests without letting one failure impersonate four.
///
/// They share one process-wide registry, so they must not overlap — but a plain
/// `lock().unwrap()` means the first test to panic *while holding the lock* poisons it, and
/// every test after that dies on the unwrap instead of running. The count then depends on
/// scheduling: one stale assertion in this file reported as 2, 3 or 4 failures across
/// consecutive runs, and none of the extra three named anything real. Recovering the guard
/// keeps the mutual exclusion and lets exactly the broken test be the broken one.
fn serialized() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct SessionGuard(SessionSlug);

impl Drop for SessionGuard {
    fn drop(&mut self) {
        registry::global().unregister(&self.0);
    }
}

async fn spawn(acceptor: Acceptor) -> (String, tempfile::TempDir, ServerSeams, SessionGuard) {
    spawn_with_subject(acceptor, true).await
}

async fn spawn_with_subject(
    acceptor: Acceptor,
    create_task: bool,
) -> (String, tempfile::TempDir, ServerSeams, SessionGuard) {
    let dir = tempdir().expect("tempdir");
    if create_task {
        let task_dir = dir.path().join("memory/facets/tasks/weekly-report-friday");
        std::fs::create_dir_all(&task_dir).expect("task directory");
        std::fs::write(
            task_dir.join("facet.md"),
            "---\nstatus: doing\n---\n\n## Timeline\n",
        )
        .expect("task facet");
    } else {
        let agent_dir = dir.path().join("agents/weekly-report-friday");
        std::fs::create_dir_all(&agent_dir).expect("agent definition directory");
        std::fs::write(
            agent_dir.join("weekly-report-friday.md"),
            "---\nname: Weekly Report Friday\nagent: opencode\ntrigger: cron\nschedule: \"30 9 * * 5\"\nenabled: true\n---\n",
        )
        .expect("agent definition");
    }

    let memory = Memory::open(dir.path()).await.expect("memory");
    let cognition = registry::mint(hi_agent::identity::Role::Cognition, None);
    registry::global().register(
        cognition.clone(),
        hi_agent::identity::Role::Cognition,
        None,
        "the shared brain".into(),
        None,
    );
    let owner = SessionGuard(cognition);

    let (router, seams) = server::build(
        memory,
        dir.path().to_path_buf(),
        Observatory::new(None),
        WireTap::new(),
        hi_agent::foundation::privacy::PrivacyBoundary::open(dir.path()).unwrap(),
        ToolRegistry::new(),
        Floor::new(),
        Attachments::new(),
        None,
    );
    let router = surfaces::accepted_on(router, acceptor);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{addr}"), dir, seams, owner)
}

#[tokio::test]
async fn external_session_is_scoped_and_released() {
    let _lock = serialized();
    let (base, _dir, seams, owner) = spawn(Acceptor::Loopback).await;
    let client = reqwest::Client::new();
    let (_surface_id, surface_token) = seams.state.surfaces.mint("test").expect("surface");

    let response = client
        .post(format!("{base}/api/external-sessions"))
        .bearer_auth(&surface_token)
        .json(&json!({
            "title": "weekly report auto-run",
            "subject": "weekly-report-friday"
        }))
        .send()
        .await
        .expect("register");
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let registered: Value = response.json().await.expect("registration JSON");
    let slug = registered["slug"].as_str().expect("slug").to_owned();
    let capability = registered["capability"].as_str().expect("capability").to_owned();
    assert_ne!(slug, owner.0.as_str());
    assert_eq!(registered["mcp_url"], "/mcp/external");

    let tools = client
        .post(format!("{base}/mcp/external"))
        .bearer_auth(&capability)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
        .send()
        .await
        .expect("tools/list")
        .json::<Value>()
        .await
        .expect("tools JSON");
    let tools = tools["result"]["tools"].as_array().expect("tools");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "hi_send_message");

    let refused = client
        .post(format!("{base}/mcp/external"))
        .bearer_auth(&capability)
        .json(&json!({
            "jsonrpc":"2.0",
            "id":11,
            "method":"tools/call",
            "params":{"name":"hi_review_view","arguments":{}}
        }))
        .send()
        .await
        .expect("refused tool")
        .json::<Value>()
        .await
        .expect("refusal JSON");
    assert_eq!(refused["result"]["isError"], true);

    let body = client
        .post(format!("{base}/mcp/external"))
        .bearer_auth(&capability)
        // These headers are deliberately forged; external MCP derives its sender from
        // the capability, not from either native identity header.
        .header("X-HI-Role", "cognition")
        .header("X-HI-Session-Slug", "cognition")
        .json(&json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"tools/call",
            "params":{"name":"hi_send_message","arguments":{
                "to":owner.0.as_str(),
                "message":"external review is ready"
            }}
        }))
        .send()
        .await
        .expect("send message")
        .json::<Value>()
        .await
        .expect("message JSON");
    assert_eq!(body["result"]["isError"], false);
    // The receipt names *which* delivery this was: the owner here is idle, so the message is
    // taken on its next prompt rather than waiting behind a turn already open. Asserting the
    // whole sentence rather than a `delivered` prefix is the point — the two answers were one
    // word until `delivered_line` split them, and a prefix match would pass for either.
    // Built from the slug the test already holds, because the number in it is assigned by the
    // process-wide registry and so depends on what else has registered.
    assert_eq!(
        body["result"]["content"][0]["text"],
        format!("delivered — `{}` is idle, so it takes this on its next prompt.", owner.0.as_str())
    );

    let pending = registry::global()
        .take_pending(&owner.0)
        .expect("receiver mailbox");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].text, "external review is ready");
    assert_eq!(pending[0].from.as_ref().map(SessionSlug::as_str), Some(slug.as_str()));

    // Only the capability paired with the named slug can release it.
    let wrong_release = client
        .delete(format!("{base}/api/external-sessions/{slug}"))
        .bearer_auth("wrong-capability")
        .send()
        .await
        .expect("wrong release");
    assert_eq!(wrong_release.status(), reqwest::StatusCode::UNAUTHORIZED);

    let released = client
        .delete(format!("{base}/api/external-sessions/{slug}"))
        .bearer_auth(&capability)
        .send()
        .await
        .expect("release");
    assert_eq!(released.status(), reqwest::StatusCode::NO_CONTENT);

    let repeated_release = client
        .delete(format!("{base}/api/external-sessions/{slug}"))
        .bearer_auth(&capability)
        .send()
        .await
        .expect("repeated release");
    assert_eq!(repeated_release.status(), reqwest::StatusCode::NO_CONTENT);

    let after_release = client
        .post(format!("{base}/mcp/external"))
        .bearer_auth(&capability)
        .json(&json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}))
        .send()
        .await
        .expect("post after release");
    assert_eq!(after_release.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(
        registry::global().send(
            &slug.parse().expect("slug"),
            &owner.0,
            "must not deliver".into()
        ),
        Delivery::UnknownSender
    );
}

#[tokio::test]
async fn external_session_accepts_a_known_agent_definition_subject() {
    let _lock = serialized();
    let (base, _dir, seams, _owner) = spawn_with_subject(Acceptor::Loopback, false).await;
    let client = reqwest::Client::new();
    let (_surface_id, surface_token) = seams.state.surfaces.mint("test").expect("surface");

    let response = client
        .post(format!("{base}/api/external-sessions"))
        .bearer_auth(&surface_token)
        .json(&json!({
            "title": "weekly report auto-run",
            "subject": "weekly-report-friday"
        }))
        .send()
        .await
        .expect("register");
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let registered: Value = response.json().await.expect("registration JSON");
    let slug = registered["slug"].as_str().expect("slug");
    let capability = registered["capability"].as_str().expect("capability");

    let release = client
        .delete(format!("{base}/api/external-sessions/{slug}"))
        .bearer_auth(capability)
        .send()
        .await
        .expect("release");
    assert_eq!(release.status(), reqwest::StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn external_session_rejects_an_unknown_subject() {
    let _lock = serialized();
    let (base, _dir, seams, _owner) = spawn(Acceptor::Loopback).await;
    let client = reqwest::Client::new();
    let (_surface_id, surface_token) = seams.state.surfaces.mint("test").expect("surface");

    let response = client
        .post(format!("{base}/api/external-sessions"))
        .bearer_auth(&surface_token)
        .json(&json!({
            "title": "weekly report auto-run",
            "subject": "not-a-real-task-or-agent"
        }))
        .send()
        .await
        .expect("register");
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn external_session_routes_refuse_off_box_even_with_surface_credentials() {
    let _lock = serialized();
    let (base, _dir, seams, _owner) = spawn(Acceptor::OffBox).await;
    let client = reqwest::Client::new();
    let (_surface_id, surface_token) = seams.state.surfaces.mint("test").expect("surface");

    // The ordinary gate accepts the surface credential, but the route-specific check remains
    // fail-closed because the request arrived through the OffBox acceptor.
    let response = client
        .post(format!("{base}/api/external-sessions"))
        .bearer_auth(&surface_token)
        .json(&json!({"title":"report","subject":"weekly-report-friday"}))
        .send()
        .await
        .expect("register");
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
}
