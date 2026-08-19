//! Protocol smoke test for the `/mcp` tool endpoint.
//!
//! Builds the axum router via [`hi_agent::foundation::server::build`] and exercises the
//! hand-rolled MCP "Streamable HTTP" surface directly: the initialize handshake,
//! role-gated `tools/list`, the `202` for notifications, the `405` for the GET
//! SSE stream we decline, and a `tools/call` with no live owning loop.

use hi_agent::mind::memory::Memory;
use hi_agent::foundation::surfaces::{Acceptor, accepted_on};
use hi_agent::foundation::server::{self, ServerSeams};
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio::net::TcpListener;

async fn spawn_server() -> (String, tempfile::TempDir, ServerSeams) {
    let dir = tempdir().expect("tempdir");
    let memory = Memory::open(dir.path()).await.expect("memory");
    let observatory =
        hi_agent::foundation::observatory::Observatory::new(None);
    let (router, seams) = server::build(
        memory,
        dir.path().to_path_buf(),
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
    (format!("http://{addr}"), dir, seams)
}

async fn post_mcp(client: &reqwest::Client, base: &str, role: &str, msg: Value) -> reqwest::Response {
    client
        .post(format!("{base}/mcp"))
        .header("X-HI-Role", role)
        .header("Content-Type", "application/json")
        .body(msg.to_string())
        .send()
        .await
        .expect("send POST /mcp")
}

fn tool_names(list: &Value) -> Vec<String> {
    list["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[tokio::test]
async fn initialize_returns_server_info() {
    let (base, _dir, _seams) = spawn_server().await;
    let client = reqwest::Client::new();
    let resp = post_mcp(
        &client,
        &base,
        "reaction",
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2025-06-18" } }),
    )
    .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: Value = resp.json().await.expect("json");
    assert_eq!(body["result"]["serverInfo"]["name"], "hi-agent");
    assert_eq!(body["result"]["protocolVersion"], "2025-06-18");
    assert!(body["result"]["capabilities"]["tools"].is_object());
}

#[tokio::test]
async fn tools_list_is_role_gated() {
    let (base, _dir, _seams) = spawn_server().await;
    let client = reqwest::Client::new();

    let reaction = post_mcp(
        &client,
        &base,
        "reaction",
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    )
    .await
    .json::<Value>()
    .await
    .expect("json");
    let names = tool_names(&reaction);
    // The reaction is the fast conversational rung: its two expression channels, both
    // calls, plus the one verb that reaches another agent. Nothing that reads or fetches.
    // This asserted one tool while `hi_say` sat in the unreachable fallback arm and
    // Reaction fell back to plain message text — the test agreed with the code and both
    // were wrong about the design.
    let mut names = names;
    names.sort();
    assert_eq!(
        names,
        vec!["hi_say".to_string(), "hi_send_message".to_string(), "hi_show".to_string()],
        "got {names:?}"
    );

    let worker = post_mcp(
        &client,
        &base,
        "worker",
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" }),
    )
    .await
    .json::<Value>()
    .await
    .expect("json");
    let names = tool_names(&worker);
    // One verb reaches another agent, and it is the only one.
    assert!(names.contains(&"hi_send_message".to_string()), "got {names:?}");
    assert!(!names.contains(&"hi_say".to_string()), "worker must not see say");
    // The screen pair is withdrawn. `do_look`/`do_act` still exist and still dispatch by
    // name, so the only thing standing between a worker and the user's live cursor is
    // this surface — which makes it a thing to assert, not a thing to leave untested.
    for gone in ["hi_look", "hi_act"] {
        assert!(!names.contains(&gone.to_string()), "`{gone}` is withdrawn; got {names:?}");
    }
    // The retired channel, in all three of its names. A worker that can still reach
    // Reaction by a second route is a worker that will, and then two paths are live.
    for gone in ["ask", "surface", "delegate"] {
        assert!(!names.contains(&gone.to_string()), "`{gone}` is retired; got {names:?}");
    }
    // Only the standing owner roles make workers.
    assert!(!names.contains(&"hi_create_worker".to_string()), "got {names:?}");
}

/// The switchboard is process-wide, so reaching it must not require a tool sink.
///
/// A standing caller must get a real answer (even "no live session 9999"),
/// never fail because an unrelated owning loop is absent.
#[tokio::test]
async fn the_switchboard_needs_no_tool_sink() {
    let (base, _dir, _seams) = spawn_server().await;
    let client = reqwest::Client::new();

    for (id, call) in [
        // `id` is declared a string in both schemas, so send one.
        (10, json!({ "name": "hi_session_status", "arguments": { "id": "9999" } })),
        (11, json!({ "name": "hi_session_messages", "arguments": { "id": "9999" } })),
    ] {
        let resp = post_mcp(
            &client,
            &base,
            "reflection",
            json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": call }),
        )
        .await
        .json::<Value>()
        .await
        .expect("json");
        let text = serde_json::to_string(&resp).expect("serialize");
        assert!(!text.contains("owning loop is not up"), "switchboard call required a sink: {text}");
        assert!(text.contains("no live session 9999"), "expected a real answer: {text}");
    }
}

#[tokio::test]
async fn notification_is_accepted_without_body() {
    let (base, _dir, _seams) = spawn_server().await;
    let client = reqwest::Client::new();
    // No `id` ⇒ a notification ⇒ 202 Accepted, empty body.
    let resp = post_mcp(
        &client,
        &base,
        "reaction",
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await;
    assert_eq!(resp.status(), reqwest::StatusCode::ACCEPTED);
    assert!(resp.bytes().await.expect("body").is_empty());
}

#[tokio::test]
async fn get_declines_sse_stream() {
    let (base, _dir, _seams) = spawn_server().await;
    let resp = reqwest::Client::new()
        .get(format!("{base}/mcp"))
        .send()
        .await
        .expect("GET /mcp");
    assert_eq!(resp.status(), reqwest::StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn tool_call_without_an_owning_loop_is_a_tool_error() {
    // No reaction loop is registered (server::build doesn't start one), so a
    // delegate call resolves to a tool error rather than a transport failure —
    // the JSON-RPC envelope still succeeds.
    let (base, _dir, _seams) = spawn_server().await;
    let client = reqwest::Client::new();
    let body: Value = post_mcp(
        &client,
        &base,
        "reaction",
        json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                "params": { "name": "delegate", "arguments": { "task": "look something up" } } }),
    )
    .await
    .json()
    .await
    .expect("json");
    assert_eq!(body["result"]["isError"], true);
}
