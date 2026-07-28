//! Headless Chrome, driven over the DevTools protocol — the one vendor behind the
//! [view render capability](crate::body::capabilities::view_render).
//!
//! ## Why CDP and not `--screenshot`
//!
//! Chrome can screenshot a URL with a single command line, and it would be a
//! third of this code. It would also be the exact failure the architecture warns
//! about: *"the command exited zero" is not "the thing worked"*. A view whose
//! `@hi/ui` import fails to resolve still produces a perfectly valid PNG — of an
//! empty white page — and `--screenshot` exits 0. The most common real defect
//! would be reported as a pass.
//!
//! So we attach a real session and read the page's own account of itself
//! alongside the pixels, from four sources at once:
//!
//! | Source | Catches |
//! |---|---|
//! | `Runtime.exceptionThrown` | uncaught exceptions, including a crashing component |
//! | `Runtime.consoleAPICalled` (level `error`) | React's own error logging |
//! | `Log.entryAdded` (level `error`) | failed subresource loads, CSP, CORS |
//! | `Network.loadingFailed` / 4xx-5xx responses | the compiled `.mjs` 404ing, with its URL |
//!
//! plus a fifth, page-side: `window.__hiRender.errors`, which the render page
//! fills from `window.onerror`, `unhandledrejection`, a `console.error` shim and
//! — crucially — the `catch` on its dynamic `import()`. That last one is where
//! `Failed to resolve module specifier "@hi/ui"` arrives in readable form; CDP
//! alone reports it only as a rejected promise inside the page's own handler.
//!
//! The same session also gives us a **deterministic settle point**: the page sets
//! `__hiRender.ready` once React has committed, two frames have painted, fonts
//! have loaded and every `<img>` has resolved. We wait for that rather than
//! guessing with a sleep, so a slow view is not captured half-painted and a
//! fast one is not waited on for nothing.
//!
//! Stateless free functions taking their config explicitly, per the vendor-layer
//! contract in [`super`].

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::process::{Child, Command};
use tokio_tungstenite::tungstenite::Message;

use crate::runtime::browser::ResolvedBrowser;

/// How long to wait for the browser to publish its DevTools port.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a single CDP request may take before we give up on the browser.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// What to render, and at what size.
#[derive(Debug, Clone)]
pub struct PageRequest {
    /// The full URL of the host page to load (the `/render/view` route with its
    /// query string).
    pub url: String,
    pub width: u32,
    pub height: u32,
    /// Device pixel ratio. 2.0 gives a retina-density PNG, which is what you want
    /// if a person (or a model) is going to look at the text.
    pub scale: f64,
    /// Upper bound on waiting for the page to declare itself settled.
    pub settle_timeout: Duration,
}

/// What the page did and what it said about it.
#[derive(Debug, Clone)]
pub struct PageCapture {
    pub png: Vec<u8>,
    /// Everything that went wrong, de-duplicated, in the order first seen.
    pub problems: Vec<String>,
    /// The page's own verdict: it failed to load or mount the view.
    pub page_failed: bool,
    /// The page never declared itself settled within `settle_timeout`.
    pub timed_out: bool,
}

/// Launch `browser`, load `req.url`, wait for the page to settle, and return the
/// screenshot together with everything that went wrong while doing it.
///
/// The browser is a fresh process with a throwaway profile per render: no shared
/// cache, no leftover state, nothing to reset between reviews. It is always
/// killed and its profile removed, including on the error paths.
pub async fn capture(browser: &ResolvedBrowser, req: &PageRequest) -> anyhow::Result<PageCapture> {
    let profile = std::env::temp_dir().join(format!(
        "hi-render-{}-{}",
        std::process::id(),
        uuid::Uuid::now_v7().simple()
    ));
    tokio::fs::create_dir_all(&profile)
        .await
        .with_context(|| format!("creating the browser profile dir {}", profile.display()))?;

    let mut child = spawn(browser, &profile)?;

    // Drain stderr into a buffer rather than letting it fill the pipe (a full
    // pipe would block the browser mid-render). It is also the only diagnostic
    // there is when the browser refuses to start at all — a missing shared
    // library, a sandbox refusal — so a launch failure quotes it back.
    let log = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    if let Some(stderr) = child.stderr.take() {
        let sink = log.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(mut buf) = sink.lock() {
                    if buf.len() < 8192 {
                        buf.push_str(&line);
                        buf.push('\n');
                    }
                }
            }
        });
    }

    let result = drive(&profile, req).await;

    let _ = child.kill().await;
    let _ = tokio::fs::remove_dir_all(&profile).await;

    result.map_err(|e| {
        let tail = log.lock().ok().map(|b| b.trim().to_string()).unwrap_or_default();
        if tail.is_empty() { e } else { e.context(format!("browser stderr:\n{tail}")) }
    })
}

/// Spawn the browser with a throwaway profile and an ephemeral DevTools port.
fn spawn(browser: &ResolvedBrowser, profile: &Path) -> anyhow::Result<Child> {
    let mut cmd = Command::new(&browser.bin);
    // `chrome-headless-shell` IS headless and rejects the flag; a full
    // Chrome/Chromium/Edge found on the system needs it.
    if !browser.headless_shell {
        cmd.arg("--headless");
    }
    cmd.arg("--remote-debugging-port=0")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-gpu")
        .arg("--hide-scrollbars")
        // Small /dev/shm in containers makes Chrome crash on startup.
        .arg("--disable-dev-shm-usage")
        // Nothing here should phone home; a review render must be reproducible
        // and must not sit behind a component update.
        .arg("--disable-background-networking")
        .arg("--disable-component-update")
        .arg("--disable-default-apps")
        .arg("--disable-extensions")
        .arg("--disable-sync")
        .arg("--metrics-recording-only")
        .arg("--mute-audio")
        // Start blank: we attach *before* navigating, so no console error from
        // the view's own load can happen before we are listening.
        .arg("about:blank");
    // On Linux the server shape runs in Docker, usually as root, where Chrome
    // refuses to start with its sandbox enabled. The desktop shapes (macOS,
    // Windows) keep the sandbox. What we load is our own loopback page and the
    // agent's own compiled view — the same trust level as the rest of the
    // process — so this is a deployment accommodation, not a widened surface.
    if cfg!(target_os = "linux") {
        cmd.arg("--no-sandbox").arg("--disable-setuid-sandbox");
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    cmd.spawn()
        .with_context(|| format!("spawning the headless browser at {}", browser.bin.display()))
}

/// Attach to the launched browser and perform one render.
async fn drive(profile: &Path, req: &PageRequest) -> anyhow::Result<PageCapture> {
    let port = wait_for_devtools_port(profile).await?;
    let ws_url = page_socket_url(port).await?;

    let (mut socket, _) = tokio_tungstenite::connect_async(ws_url.as_str())
        .await
        .with_context(|| format!("connecting to the DevTools socket at {ws_url}"))?;

    let mut session = Session { next_id: 1, problems: Vec::new(), seen: Default::default() };

    // Enable the reporting domains BEFORE navigating, so nothing the view does on
    // load happens off-camera.
    for domain in ["Runtime", "Log", "Page", "Network"] {
        session.call(&mut socket, &format!("{domain}.enable"), json!({})).await?;
    }
    session
        .call(
            &mut socket,
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": req.width,
                "height": req.height,
                "deviceScaleFactor": req.scale,
                "mobile": false,
            }),
        )
        .await?;

    session.call(&mut socket, "Page.navigate", json!({ "url": req.url })).await?;
    // Best-effort: a page that never fires `load` (a hung subresource) should
    // still be probed and screenshotted rather than aborting the whole render.
    let _ = session.wait_for_event(&mut socket, "Page.loadEventFired", CALL_TIMEOUT).await;

    let report = session
        .call(
            &mut socket,
            "Runtime.evaluate",
            json!({
                "expression": settle_script(req.settle_timeout),
                "awaitPromise": true,
                "returnByValue": true,
            }),
        )
        .await?;

    let (page_failed, timed_out, page_errors) = read_report(&report);
    for e in page_errors {
        session.note(e);
    }

    let shot = session
        .call(
            &mut socket,
            "Page.captureScreenshot",
            json!({ "format": "png", "captureBeyondViewport": false }),
        )
        .await?;
    let b64 = shot
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Page.captureScreenshot returned no data"))?;
    let png = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .context("decoding the screenshot base64")?
    };

    let _ = socket.close(None).await;

    Ok(PageCapture { png, problems: session.problems, page_failed, timed_out })
}

/// The JS the page is asked to run: resolve once `window.__hiRender.ready`, or
/// give up after `timeout` and report whatever the page had collected by then.
/// Returned by value, so a page that never defines `__hiRender` at all (the
/// render page's own script failed to load) is itself reported as a failure.
fn settle_script(timeout: Duration) -> String {
    let ms = timeout.as_millis();
    format!(
        r#"new Promise((resolve) => {{
  const started = Date.now();
  const poll = () => {{
    const r = window.__hiRender;
    if (r && r.ready) return resolve({{ failed: !!r.failed, timedOut: false, errors: r.errors || [] }});
    if (Date.now() - started > {ms}) {{
      return resolve({{
        failed: true,
        timedOut: true,
        errors: (r && r.errors) || ["the render page never reported a result (its own script may have failed to load)"],
      }});
    }}
    setTimeout(poll, 50);
  }};
  poll();
}})"#
    )
}

/// Pull the page's verdict out of a `Runtime.evaluate` result.
fn read_report(result: &Value) -> (bool, bool, Vec<String>) {
    let value = result.get("result").and_then(|r| r.get("value"));
    let Some(value) = value else {
        return (
            true,
            false,
            vec!["the page returned no render report".to_string()],
        );
    };
    let failed = value.get("failed").and_then(Value::as_bool).unwrap_or(false);
    let timed_out = value.get("timedOut").and_then(Value::as_bool).unwrap_or(false);
    let errors: Vec<String> = value
        .get("errors")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    (failed, timed_out, errors)
}

/// Poll `<profile>/DevToolsActivePort` for the ephemeral port Chrome chose. More
/// reliable than scraping stderr (which varies by build and is silent under some
/// logging flags); the file's first line is the port, the second the browser
/// endpoint path.
async fn wait_for_devtools_port(profile: &Path) -> anyhow::Result<u16> {
    let marker = profile.join("DevToolsActivePort");
    let deadline = tokio::time::Instant::now() + LAUNCH_TIMEOUT;
    loop {
        if let Ok(text) = tokio::fs::read_to_string(&marker).await {
            if let Some(port) = text.lines().next().and_then(|l| l.trim().parse::<u16>().ok()) {
                return Ok(port);
            }
        }
        if tokio::time::Instant::now() >= deadline {
            bail!(
                "the headless browser never published a DevTools port at {} \
                 (it may have failed to start)",
                marker.display()
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Ask the browser's HTTP endpoint for the blank starting tab's page socket. We
/// talk to the *page* target directly rather than plumbing flat-protocol session
/// ids through every message.
async fn page_socket_url(port: u16) -> anyhow::Result<String> {
    let url = format!("http://127.0.0.1:{port}/json/list");
    let deadline = tokio::time::Instant::now() + LAUNCH_TIMEOUT;
    let client = reqwest::Client::new();
    loop {
        if let Ok(resp) = client.get(&url).send().await {
            if let Ok(targets) = resp.json::<Vec<Value>>().await {
                let found = targets
                    .iter()
                    .find(|t| t.get("type").and_then(Value::as_str) == Some("page"))
                    .and_then(|t| t.get("webSocketDebuggerUrl"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if let Some(ws) = found {
                    return Ok(ws);
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            bail!("the headless browser exposed no page target at {url}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

type Socket = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

/// One CDP conversation: a request-id counter, everything the page reported going
/// wrong while we were waiting for replies, and which events have already gone
/// past (so waiting for one that already fired returns immediately instead of
/// burning the whole timeout).
struct Session {
    next_id: u64,
    problems: Vec<String>,
    seen: std::collections::HashSet<String>,
}

impl Session {
    /// Record a problem, skipping blanks and exact repeats (the page's own
    /// `console.error` shim re-emits through the real console, so the same text
    /// legitimately arrives on two channels).
    fn note(&mut self, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() || self.problems.iter().any(|p| p == &text) {
            return;
        }
        if self.problems.len() < 50 {
            self.problems.push(text);
        }
    }

    /// Send a command and read until its reply arrives, folding every event seen
    /// on the way into [`Session::problems`].
    async fn call(
        &mut self,
        socket: &mut Socket,
        method: &str,
        params: Value,
    ) -> anyhow::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let payload = json!({ "id": id, "method": method, "params": params });
        socket
            .send(Message::text(payload.to_string()))
            .await
            .with_context(|| format!("sending {method} to the browser"))?;

        // `Runtime.evaluate` carries the page's own settle timeout, so give every
        // call the larger of the two budgets rather than cutting it short.
        let budget = CALL_TIMEOUT + Duration::from_secs(60);
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            let msg = self.next_message(socket, deadline, method).await?;
            if msg.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(err) = msg.get("error") {
                    bail!("{method} failed: {err}");
                }
                return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }

    /// Read until `event` arrives (or the budget runs out), collecting problems.
    async fn wait_for_event(
        &mut self,
        socket: &mut Socket,
        event: &str,
        budget: Duration,
    ) -> anyhow::Result<()> {
        if self.seen.contains(event) {
            return Ok(());
        }
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            let msg = self.next_message(socket, deadline, event).await?;
            if msg.get("method").and_then(Value::as_str) == Some(event) {
                return Ok(());
            }
        }
    }

    /// One protocol message, with events folded in as they pass.
    async fn next_message(
        &mut self,
        socket: &mut Socket,
        deadline: tokio::time::Instant,
        waiting_for: &str,
    ) -> anyhow::Result<Value> {
        loop {
            let next = tokio::time::timeout_at(deadline, socket.next())
                .await
                .map_err(|_| anyhow!("the browser stopped responding while waiting for {waiting_for}"))?;
            let msg = match next {
                Some(Ok(m)) => m,
                Some(Err(e)) => bail!("DevTools socket error while waiting for {waiting_for}: {e}"),
                None => bail!("the browser closed the DevTools socket while waiting for {waiting_for}"),
            };
            let text: String = match msg {
                Message::Text(t) => t.as_str().to_owned(),
                Message::Close(_) => {
                    bail!("the browser closed the DevTools socket while waiting for {waiting_for}")
                }
                // Binary/ping/pong: nothing CDP sends that we care about.
                _ => continue,
            };
            let value: Value = serde_json::from_str(&text)
                .with_context(|| format!("parsing a DevTools message: {text}"))?;
            if let Some(method) = value.get("method").and_then(Value::as_str) {
                self.seen.insert(method.to_string());
                self.absorb_event(&value);
            }
            return Ok(value);
        }
    }

    /// Turn one CDP event into a problem line, if it is one.
    fn absorb_event(&mut self, event: &Value) {
        let method = event.get("method").and_then(Value::as_str).unwrap_or("");
        let null = Value::Null;
        let p = event.get("params").unwrap_or(&null);
        match method {
            "Runtime.exceptionThrown" => {
                let d = p.get("exceptionDetails");
                let text = d
                    .and_then(|d| d.get("exception"))
                    .and_then(|e| e.get("description"))
                    .and_then(Value::as_str)
                    .or_else(|| d.and_then(|d| d.get("text")).and_then(Value::as_str))
                    .unwrap_or("uncaught exception");
                self.note(format!("uncaught: {text}"));
            }
            "Runtime.consoleAPICalled" => {
                if p.get("type").and_then(Value::as_str) != Some("error") {
                    return;
                }
                let args = p
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|arg| {
                                arg.get("description")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned)
                                    .or_else(|| {
                                        arg.get("value").map(|v| match v.as_str() {
                                            Some(s) => s.to_string(),
                                            None => v.to_string(),
                                        })
                                    })
                                    .unwrap_or_default()
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                self.note(format!("console.error: {args}"));
            }
            "Log.entryAdded" => {
                let entry = p.get("entry");
                if entry.and_then(|e| e.get("level")).and_then(Value::as_str) != Some("error") {
                    return;
                }
                let text = entry
                    .and_then(|e| e.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("log error");
                let url = entry.and_then(|e| e.get("url")).and_then(Value::as_str);
                self.note(match url {
                    Some(u) => format!("{text} ({u})"),
                    None => text.to_string(),
                });
            }
            "Network.loadingFailed" => {
                let err = p.get("errorText").and_then(Value::as_str).unwrap_or("load failed");
                let kind = p.get("type").and_then(Value::as_str).unwrap_or("resource");
                self.note(format!("{kind} failed to load: {err}"));
            }
            "Network.responseReceived" => {
                let resp = p.get("response");
                let status =
                    resp.and_then(|r| r.get("status")).and_then(Value::as_u64).unwrap_or(0);
                if status < 400 {
                    return;
                }
                let url = resp
                    .and_then(|r| r.get("url"))
                    .and_then(Value::as_str)
                    .unwrap_or("(unknown url)");
                self.note(format!("HTTP {status} for {url}"));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session { next_id: 1, problems: Vec::new(), seen: Default::default() }
    }

    #[test]
    fn uncaught_exceptions_are_reported() {
        let mut s = session();
        s.absorb_event(&json!({
            "method": "Runtime.exceptionThrown",
            "params": { "exceptionDetails": { "exception": {
                "description": "TypeError: Failed to resolve module specifier \"@hi/ui\""
            }}}
        }));
        assert_eq!(s.problems.len(), 1);
        assert!(s.problems[0].contains("@hi/ui"), "{:?}", s.problems);
    }

    #[test]
    fn a_missing_module_is_reported_with_its_url() {
        let mut s = session();
        s.absorb_event(&json!({
            "method": "Network.responseReceived",
            "params": { "response": { "status": 404, "url": "http://x/views/_compiled/ab.mjs" } }
        }));
        assert_eq!(s.problems, vec!["HTTP 404 for http://x/views/_compiled/ab.mjs"]);
    }

    #[test]
    fn healthy_traffic_is_not_a_problem() {
        let mut s = session();
        s.absorb_event(&json!({
            "method": "Network.responseReceived",
            "params": { "response": { "status": 200, "url": "http://x/assets/a.js" } }
        }));
        s.absorb_event(&json!({
            "method": "Runtime.consoleAPICalled",
            "params": { "type": "log", "args": [{ "value": "hello" }] }
        }));
        s.absorb_event(&json!({
            "method": "Log.entryAdded",
            "params": { "entry": { "level": "warning", "text": "deprecated" } }
        }));
        assert!(s.problems.is_empty(), "{:?}", s.problems);
    }

    #[test]
    fn console_errors_are_captured_and_deduped() {
        let mut s = session();
        let ev = json!({
            "method": "Runtime.consoleAPICalled",
            "params": { "type": "error", "args": [{ "value": "boom" }] }
        });
        s.absorb_event(&ev);
        s.absorb_event(&ev);
        assert_eq!(s.problems, vec!["console.error: boom"], "the page's shim re-emits; dedupe it");
    }

    #[test]
    fn log_entries_carry_their_url() {
        let mut s = session();
        s.absorb_event(&json!({
            "method": "Log.entryAdded",
            "params": { "entry": {
                "level": "error",
                "text": "Failed to load resource: 404",
                "url": "http://x/views/deck/leader.jpg"
            }}
        }));
        assert_eq!(
            s.problems,
            vec!["Failed to load resource: 404 (http://x/views/deck/leader.jpg)"]
        );
    }

    #[test]
    fn the_page_report_is_read_back() {
        let (failed, timed_out, errors) = read_report(&json!({
            "result": { "value": { "failed": true, "timedOut": false, "errors": ["nope"] } }
        }));
        assert!(failed);
        assert!(!timed_out);
        assert_eq!(errors, vec!["nope"]);
    }

    #[test]
    fn a_page_that_reports_nothing_counts_as_failed() {
        // The render page's own script failing to load must not read as success.
        let (failed, _, errors) = read_report(&json!({ "result": {} }));
        assert!(failed);
        assert!(!errors.is_empty());
    }

    #[test]
    fn settle_script_carries_the_timeout_and_is_a_promise() {
        let js = settle_script(Duration::from_millis(1234));
        assert!(js.starts_with("new Promise("));
        assert!(js.contains("1234"));
        assert!(js.contains("__hiRender"));
    }
}
