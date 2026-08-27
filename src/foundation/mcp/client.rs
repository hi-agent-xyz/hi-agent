//! An MCP **client** — the other direction from [`super`], which is the server we
//! publish to codex.
//!
//! # Why this exists at all
//!
//! `docs/arch/tools.md`: **MCP is a command, not a carrier class.** A service that
//! speaks only MCP is reached through one small program rather than through a loader,
//! a per-carrier dispatch and a dead-server-kills-the-thread failure mode. That
//! program is `hi mcp`, and this is its engine. The payoff is that every such server
//! becomes an ordinary note in the workshop, indistinguishable at the call site from
//! a CLI someone installed with a package manager.
//!
//! # Two transports, told apart by the endpoint
//!
//! An endpoint that starts with `http://` or `https://` is **streamable HTTP**;
//! anything else is a **command** to spawn and speak to over its stdin/stdout. That is
//! the whole of the dispatch, and it is one `if` rather than a registry — a server's
//! address already says how to reach it.
//!
//! # The cost this accepts, stated
//!
//! A one-shot call **drops the protocol's push half**. MCP servers can emit progress
//! notifications and can ask the client for things mid-call (sampling, elicitation);
//! through a command none of that arrives. `tools.md` names this as a real amputation
//! rather than an oversight: a server whose value *is* the stream is not served by a
//! note, and that is when to reach for something other than this.
//!
//! Notifications that do arrive are drained and ignored rather than treated as
//! answers — a server that emits progress must not desynchronise the reply we are
//! waiting for.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// The protocol version we announce. MCP servers negotiate: a server may answer with
/// a different one, and we do not refuse it — refusing would turn a working server
/// into an unreachable one over a string, which is the opposite of the point.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// How long any single request may take. A wedged server must fail the command rather
/// than hang the job that ran it — the note's reader can then read the error and
/// decide, which is the whole reason readiness is *running it*.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Where an MCP server lives, and therefore how to reach it.
pub enum Endpoint {
    /// Streamable HTTP: JSON-RPC over `POST`, answered as JSON or as an SSE stream.
    Http(String),
    /// A program to spawn, spoken to over stdin/stdout as newline-delimited JSON-RPC.
    Stdio(Vec<String>),
}

impl Endpoint {
    /// Read an endpoint from one command-line argument. A URL is HTTP; anything else
    /// is a command line, split on whitespace.
    ///
    /// Whitespace splitting rather than a shell parser: an MCP server is started by a
    /// program and some plain arguments (`npx -y @scope/pkg`), and a server whose
    /// launch genuinely needs quoting or globbing wants a wrapper script of its own —
    /// which is a tool note like any other, not a parser living here.
    pub fn parse(endpoint: &str) -> Result<Self> {
        let endpoint = endpoint.trim();
        if endpoint.is_empty() {
            bail!("no endpoint given");
        }
        if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
            if endpoint.split_whitespace().count() > 1 {
                bail!("a URL endpoint is one word; got {endpoint:?}");
            }
            return Ok(Endpoint::Http(endpoint.to_string()));
        }
        Ok(Endpoint::Stdio(endpoint.split_whitespace().map(str::to_string).collect()))
    }
}

/// List a server's tools: `name — description`, one per line.
///
/// This is the same bargain the workshop registry makes — a line you can match a job
/// against — and it is why a note never copies a schema. The server publishes its own,
/// on demand, and a copied one would be a second truth that drifts.
pub async fn list(endpoint: Endpoint) -> Result<String> {
    let mut conn = Connection::open(endpoint).await?;
    let result = conn.request("tools/list", json!({})).await?;
    let tools = result.get("tools").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut out = String::new();
    for tool in &tools {
        let name = tool.get("name").and_then(Value::as_str).unwrap_or("?");
        let description = tool
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .lines()
            .next()
            .unwrap_or("");
        out.push_str(&format!("{name} — {description}\n"));
    }
    if tools.is_empty() {
        out.push_str("(the server published no tools)\n");
    }
    conn.shutdown().await;
    Ok(out)
}

/// Show one tool's input schema, so a caller can work out what to pass without
/// guessing. The schema is fetched at call time and never written down.
pub async fn schema(endpoint: Endpoint, tool: &str) -> Result<String> {
    let mut conn = Connection::open(endpoint).await?;
    let result = conn.request("tools/list", json!({})).await?;
    conn.shutdown().await;
    let tools = result.get("tools").and_then(Value::as_array).cloned().unwrap_or_default();
    let found = tools
        .iter()
        .find(|t| t.get("name").and_then(Value::as_str) == Some(tool))
        .ok_or_else(|| {
            let names: Vec<&str> =
                tools.iter().filter_map(|t| t.get("name").and_then(Value::as_str)).collect();
            anyhow!("no tool named {tool:?}. This server publishes: {}", names.join(", "))
        })?;
    Ok(serde_json::to_string_pretty(found)?)
}

/// Call one tool and print what it returned.
///
/// Text content is printed as text — the common case, and the one a shell pipeline can
/// use. Anything else is printed as JSON rather than described, because a caller that
/// asked for an image should get the bytes' reference, not a sentence about them.
///
/// A tool that reports `isError` exits non-zero: **readiness is running it**, and the
/// shell's own failure signal is how that reaches the caller.
pub async fn call(endpoint: Endpoint, tool: &str, arguments: Value) -> Result<(String, bool)> {
    let mut conn = Connection::open(endpoint).await?;
    let result = conn.request("tools/call", json!({ "name": tool, "arguments": arguments })).await?;
    conn.shutdown().await;

    let is_error = result.get("isError").and_then(Value::as_bool).unwrap_or(false);
    let mut out = String::new();
    match result.get("content").and_then(Value::as_array) {
        Some(items) => {
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        out.push_str(item.get("text").and_then(Value::as_str).unwrap_or(""));
                        out.push('\n');
                    }
                    _ => {
                        out.push_str(&serde_json::to_string(item)?);
                        out.push('\n');
                    }
                }
            }
        }
        // A server that answered with something other than `content` still answered;
        // print it rather than claiming nothing came back.
        None => out.push_str(&serde_json::to_string_pretty(&result)?),
    }
    Ok((out, is_error))
}

// ── the connection ────────────────────────────────────────────────────────────

enum Connection {
    Http { client: reqwest::Client, url: String, session: Option<String>, next_id: u64 },
    Stdio { child: tokio::process::Child, next_id: u64 },
}

impl Connection {
    /// Open a connection and complete the MCP handshake: `initialize`, then the
    /// `notifications/initialized` that tells the server it may start serving.
    async fn open(endpoint: Endpoint) -> Result<Self> {
        let mut conn = match endpoint {
            Endpoint::Http(url) => Connection::Http {
                client: reqwest::Client::builder()
                    .timeout(REQUEST_TIMEOUT)
                    .build()
                    .context("building the HTTP client")?,
                url,
                session: None,
                next_id: 1,
            },
            Endpoint::Stdio(argv) => {
                let (program, args) = argv.split_first().ok_or_else(|| anyhow!("empty command"))?;
                let child = tokio::process::Command::new(program)
                    .args(args)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    // The server's own diagnostics go to our stderr, where a person
                    // running this in a shell will actually see them.
                    .stderr(Stdio::inherit())
                    .kill_on_drop(true)
                    .spawn()
                    .with_context(|| format!("spawning the MCP server {program:?}"))?;
                Connection::Stdio { child, next_id: 1 }
            }
        };

        conn.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "hi-agent", "version": env!("CARGO_PKG_VERSION") },
            }),
        )
        .await
        .context("the MCP handshake failed")?;
        conn.notify("notifications/initialized").await?;
        Ok(conn)
    }

    fn take_id(&mut self) -> u64 {
        let id = match self {
            Connection::Http { next_id, .. } | Connection::Stdio { next_id, .. } => next_id,
        };
        let this = *id;
        *id += 1;
        this
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.take_id();
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let reply = match self {
            Connection::Http { client, url, session, .. } => {
                http_roundtrip(client, url, session, &body).await?
            }
            Connection::Stdio { child, .. } => stdio_roundtrip(child, &body, id).await?,
        };
        if let Some(error) = reply.get("error") {
            let message = error.get("message").and_then(Value::as_str).unwrap_or("unknown error");
            bail!("{method} failed: {message}");
        }
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    }

    /// A notification carries no id and expects no reply.
    async fn notify(&mut self, method: &str) -> Result<()> {
        let body = json!({ "jsonrpc": "2.0", "method": method, "params": {} });
        match self {
            Connection::Http { client, url, session, .. } => {
                let mut req = client.post(url.as_str()).json(&body);
                if let Some(id) = session {
                    req = req.header("Mcp-Session-Id", id.as_str());
                }
                // A server may answer 202 with no body; either way there is nothing to
                // read, and a transport hiccup here must not fail the actual call.
                let _ = req.send().await;
            }
            Connection::Stdio { child, .. } => {
                let stdin = child.stdin.as_mut().ok_or_else(|| anyhow!("server has no stdin"))?;
                stdin.write_all(format!("{body}\n").as_bytes()).await?;
                stdin.flush().await?;
            }
        }
        Ok(())
    }

    /// Let a spawned server exit. `kill_on_drop` is the backstop; closing stdin is the
    /// polite half, and a server that ignores it is killed rather than waited on.
    async fn shutdown(self) {
        if let Connection::Stdio { mut child, .. } = self {
            drop(child.stdin.take());
            let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
        }
    }
}

async fn http_roundtrip(
    client: &reqwest::Client,
    url: &str,
    session: &mut Option<String>,
    body: &Value,
) -> Result<Value> {
    let mut req = client
        .post(url)
        // Streamable HTTP lets the server answer either way, so both are accepted and
        // the response's own content type decides how it is read.
        .header("Accept", "application/json, text/event-stream")
        .json(body);
    if let Some(id) = session.as_deref() {
        req = req.header("Mcp-Session-Id", id);
    }
    let response = req.send().await.context("the MCP server did not answer")?;

    // A server may mint a session on the first response and expect it back on the rest.
    if let Some(id) = response.headers().get("Mcp-Session-Id").and_then(|v| v.to_str().ok()) {
        *session = Some(id.to_string());
    }
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let text = response.text().await.context("reading the MCP server's answer")?;
    if !status.is_success() {
        bail!("the MCP server answered {status}: {}", text.trim());
    }
    if content_type.contains("text/event-stream") {
        return sse_reply(&text);
    }
    serde_json::from_str(&text).with_context(|| format!("the answer was not JSON: {}", text.trim()))
}

/// Pull the JSON-RPC reply out of an SSE body.
///
/// **The last `data:` payload carrying a `result` or `error` is the answer**; earlier
/// events are progress notifications, which a one-shot call has nowhere to put. This is
/// exactly the push half the module doc says is dropped — it is dropped *here*, and
/// visibly, rather than by pretending the protocol has no such thing.
fn sse_reply(body: &str) -> Result<Value> {
    let mut answer = None;
    for line in body.lines() {
        let Some(payload) = line.strip_prefix("data:") else { continue };
        let Ok(value) = serde_json::from_str::<Value>(payload.trim()) else { continue };
        if value.get("result").is_some() || value.get("error").is_some() {
            answer = Some(value);
        }
    }
    answer.ok_or_else(|| anyhow!("the event stream carried no reply: {}", body.trim()))
}

async fn stdio_roundtrip(
    child: &mut tokio::process::Child,
    body: &Value,
    id: u64,
) -> Result<Value> {
    {
        let stdin = child.stdin.as_mut().ok_or_else(|| anyhow!("server has no stdin"))?;
        stdin.write_all(format!("{body}\n").as_bytes()).await?;
        stdin.flush().await?;
    }
    let stdout = child.stdout.as_mut().ok_or_else(|| anyhow!("server has no stdout"))?;
    let mut lines = BufReader::new(stdout).lines();

    // Read until *our* id comes back. A server is free to interleave notifications and
    // replies to other requests; treating the next line as the answer is how a client
    // desynchronises and then reports the wrong result for the rest of its life.
    let deadline = tokio::time::sleep(REQUEST_TIMEOUT);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => bail!("the MCP server did not reply within {REQUEST_TIMEOUT:?}"),
            line = lines.next_line() => {
                let Some(line) = line.context("reading from the MCP server")? else {
                    bail!("the MCP server closed its output before replying");
                };
                let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
                if value.get("id").and_then(Value::as_u64) == Some(id) {
                    return Ok(value);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_endpoint_tells_its_transport_from_its_address() {
        assert!(matches!(Endpoint::parse("https://example.com/mcp").unwrap(), Endpoint::Http(_)));
        assert!(matches!(Endpoint::parse("http://127.0.0.1:8080/mcp").unwrap(), Endpoint::Http(_)));
        // Anything that is not a URL is a command to spawn, arguments included. A
        // server's own flags (`-y`) belong to it, which is why the whole endpoint is
        // one argument rather than a greedy list clap would try to interpret.
        match Endpoint::parse("npx -y @modelcontextprotocol/server-everything").unwrap() {
            Endpoint::Stdio(parts) => {
                assert_eq!(parts, ["npx", "-y", "@modelcontextprotocol/server-everything"])
            }
            Endpoint::Http(_) => panic!("a command is not a URL"),
        }
        assert!(Endpoint::parse("   ").is_err());
        // A URL with trailing words is a typo, not a command line — saying so beats
        // silently spawning `https://…` as a program.
        assert!(Endpoint::parse("https://x/mcp call").is_err());
    }

    #[test]
    fn the_sse_reply_is_the_last_payload_that_answers() {
        // Progress first, then the answer — the shape a server that streams produces.
        let body = concat!(
            "event: message\n",
            "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{}}\n",
            "\n",
            "event: message\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"tools\":[]}}\n",
        );
        let reply = sse_reply(body).unwrap();
        assert!(reply.get("result").is_some(), "{reply}");

        // A stream with nothing but notifications carried no answer, and says so
        // rather than returning an empty result that reads like success.
        let only_progress =
            "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{}}\n";
        assert!(sse_reply(only_progress).is_err());
    }
}
