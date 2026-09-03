//! `WS /api/mechanisms` — the one seam where the core does the asking.
//!
//! See [`docs/arch/mechanisms.md`](../../../../docs/arch/mechanisms.md). Every other
//! endpoint has an app asking and the core answering. A capability the core needs —
//! a screen frame, a synthesized click, the accessibility tree — runs the other way,
//! because the app is the process holding the OS session and the grants.
//!
//! The app still **dials**, because a core that had to dial an app could not reach one
//! behind NAT; what inverts is who asks, not who connects.
//!
//! ## The wire
//!
//! Text frames are JSON, tagged by `type`. Binary frames are a reply payload: the
//! first 16 bytes are the call's uuid, the rest is the bytes. That prefix is what
//! makes concurrent calls safe — a reply is matched by id and never by arrival order,
//! so a slow screen grab cannot be mistaken for the answer to a fast one.
//!
//! ```text
//! app  → core   {"type":"hello","app":"macos","mechanisms":["screen","input"]}
//! core → app    {"type":"call","id":"…","method":"screen.grab","params":{…}}
//! app  → core   {"type":"reply","id":"…","ok":{…}}          // or "err":"…"
//! app  → core   <binary: [16-byte id][payload]>              // a reply carrying bytes
//! app  → core   {"type":"event","kind":"hotkey.edge","data":{…}}
//! ```
//!
//! ## What is built here, and what is not
//!
//! This module is the seam and nothing else: the registry of attached apps, the call
//! plumbing, and the socket. **No capability is wired to it yet** —
//! [`crate::body::capabilities`] still reaches macOS through `cfg`-gated in-process
//! calls. That wiring is the next step and is deliberately separate, so the seam can
//! be tested against a fake app before any working capability is re-pointed at it.
//!
//! So the only caller of [`Mechanisms::call`] in the tree is this module's own
//! `POST /api/mechanisms/call`, which exists to exercise the seam and to give a
//! shell author something to develop against. Real and not yet load-bearing, which
//! is a state this repo allows; what it is not is described-and-absent.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot};
use uuid::Uuid;

use super::AppState;

/// How long a call waits before giving up on the app that took it.
///
/// This is a **liveness backstop, not a latency budget**. The transport is
/// microseconds and the mechanisms themselves are tens of milliseconds
/// (`docs/arch/mechanisms.md` § *The hop is not the cost*), so nothing legitimate
/// comes near this. It exists so that an app which accepted a call and then wedged
/// — a beachballed window server, a grant dialog nobody answered — fails the call
/// instead of holding a turn open forever.
///
/// Deliberately generous rather than tuned: no real mechanism duration has been
/// measured yet, and a too-tight value here would turn a slow screen grab into a
/// spurious failure. Tighten it when there are numbers, not before.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// How many events may queue for a subscriber before the slowest one starts losing
/// them. Events are advisory signals (a key edge), not a durable log; a subscriber
/// too slow to keep up has already lost the moment they described.
const EVENT_CAPACITY: usize = 64;

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

/// A frame from an app.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum FromApp {
    /// First frame on the socket. Declares what this app can do.
    Hello {
        app: String,
        #[serde(default)]
        mechanisms: Vec<String>,
    },
    /// The answer to a [`ToApp::Call`]. Exactly one of `ok`/`err` is meaningful; a
    /// reply carrying bytes arrives as a binary frame instead.
    Reply {
        id: Uuid,
        #[serde(default)]
        ok: Option<Value>,
        #[serde(default)]
        err: Option<String>,
    },
    /// Something the app noticed. Nothing replies to it.
    Event {
        kind: String,
        #[serde(default)]
        data: Value,
    },
}

/// A frame to an app.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ToApp {
    Call {
        id: Uuid,
        method: String,
        params: Value,
    },
}

/// What an app sent back for a call.
#[derive(Debug, Clone)]
pub enum Reply {
    Json(Value),
    Bytes(Bytes),
}

/// Why a call did not produce a reply.
#[derive(Debug, thiserror::Error)]
pub enum CallError {
    /// Nothing attached declares this mechanism. **Not an error condition** — a core
    /// in Docker has no hands and never will. Callers degrade rather than fail; see
    /// `docs/arch/surfaces.md` § *Degradation*.
    #[error("no attached app offers `{0}`")]
    Unavailable(String),
    /// The app answered, and the answer was a failure.
    #[error("{0}")]
    Failed(String),
    /// The app went away with the call outstanding.
    #[error("the app detached before answering")]
    Detached,
    /// The app took the call and never answered.
    #[error("the app did not answer within {0:?}")]
    TimedOut(Duration),
}

/// One event an app pushed up, fanned out to whoever is listening.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub app: String,
    pub kind: String,
    pub data: Value,
}

// ---------------------------------------------------------------------------
// The registry
// ---------------------------------------------------------------------------

/// One attached app.
struct Attached {
    mechanisms: HashSet<String>,
    out: mpsc::UnboundedSender<ToApp>,
    pending: HashMap<Uuid, oneshot::Sender<Result<Reply, CallError>>>,
}

#[derive(Default)]
struct Inner {
    apps: HashMap<String, Attached>,
}

/// The apps currently reachable, and the calls in flight to them.
///
/// Cloneable and cheap: one `Arc` inside, so `AppState` holds it and anything with
/// the state can ask.
#[derive(Clone)]
pub struct Mechanisms {
    inner: Arc<Mutex<Inner>>,
    events: broadcast::Sender<Event>,
}

impl Default for Mechanisms {
    fn default() -> Self {
        Self::new()
    }
}

impl Mechanisms {
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        Self { inner: Arc::new(Mutex::new(Inner::default())), events }
    }

    /// Subscribe to app-pushed events (key edges today).
    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// **Whether anything attached can do this right now.**
    ///
    /// The replacement for `capabilities::*::available()`, which is a compile-time
    /// `cfg!(target_os = …)`. Over this seam the answer is a fact about who is
    /// attached, and it changes while the process runs — a laptop sleeps, a shell
    /// is restarted. Callers must re-ask rather than cache.
    pub fn available(&self, mechanism: &str) -> bool {
        let inner = self.inner.lock().expect("mechanisms registry poisoned");
        inner.apps.values().any(|a| a.mechanisms.contains(mechanism))
    }

    /// What is attached, for the debug view. Sorted so the output is stable.
    pub fn attached(&self) -> Vec<(String, Vec<String>)> {
        let inner = self.inner.lock().expect("mechanisms registry poisoned");
        let mut out: Vec<(String, Vec<String>)> = inner
            .apps
            .iter()
            .map(|(id, a)| {
                let mut ms: Vec<String> = a.mechanisms.iter().cloned().collect();
                ms.sort();
                (id.clone(), ms)
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Ask an attached app to do something, and wait for its answer.
    ///
    /// `method` is dotted (`screen.grab`); the part before the dot is the mechanism
    /// used to pick an app, so declaring `screen` is what makes every `screen.*`
    /// call routable.
    pub async fn call(&self, method: &str, params: Value) -> Result<Reply, CallError> {
        let mechanism = method.split('.').next().unwrap_or(method).to_string();
        let id = Uuid::new_v4();
        let rx = {
            let mut inner = self.inner.lock().expect("mechanisms registry poisoned");
            // Pick deterministically by app id so a two-desktop setup is at least
            // stable rather than arbitrary. Choosing *which* app deliberately is an
            // open question in the design doc; this is not that.
            let mut ids: Vec<String> = inner
                .apps
                .iter()
                .filter(|(_, a)| a.mechanisms.contains(&mechanism))
                .map(|(id, _)| id.clone())
                .collect();
            ids.sort();
            let Some(app_id) = ids.into_iter().next() else {
                return Err(CallError::Unavailable(mechanism));
            };
            let app = inner.apps.get_mut(&app_id).expect("just selected");
            let (tx, rx) = oneshot::channel();
            app.pending.insert(id, tx);
            let frame = ToApp::Call { id, method: method.to_string(), params };
            if app.out.send(frame).is_err() {
                app.pending.remove(&id);
                return Err(CallError::Detached);
            }
            rx
        };

        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(result)) => result,
            // The oneshot was dropped without a value: the app detached and
            // `detach` cleared its pending calls.
            Ok(Err(_)) => Err(CallError::Detached),
            Err(_) => {
                self.forget(id);
                Err(CallError::TimedOut(CALL_TIMEOUT))
            }
        }
    }

    /// Register an app and hand back the channel its socket should pump.
    ///
    /// Re-attaching under an id that is already present replaces the old entry, and
    /// the old entry's outstanding calls fail as [`CallError::Detached`] — a shell
    /// that reconnected is not going to answer what its predecessor was asked.
    fn attach(
        &self,
        app: &str,
        mechanisms: Vec<String>,
    ) -> mpsc::UnboundedReceiver<ToApp> {
        let (out, rx) = mpsc::unbounded_channel();
        let entry = Attached {
            mechanisms: mechanisms.into_iter().collect(),
            out,
            pending: HashMap::new(),
        };
        let previous = {
            let mut inner = self.inner.lock().expect("mechanisms registry poisoned");
            inner.apps.insert(app.to_string(), entry)
        };
        if let Some(previous) = previous {
            fail_all(previous);
        }
        rx
    }

    /// Drop an app. Every call it still owed fails now rather than waiting out the
    /// timeout — the socket closing is proof no answer is coming.
    fn detach(&self, app: &str) {
        let gone = {
            let mut inner = self.inner.lock().expect("mechanisms registry poisoned");
            inner.apps.remove(app)
        };
        if let Some(gone) = gone {
            fail_all(gone);
        }
    }

    /// Resolve a pending call. Silently ignores an id nobody is waiting for: a reply
    /// that lost a race with its own timeout is not worth a log line.
    fn resolve(&self, id: Uuid, result: Result<Reply, CallError>) {
        let waiting = {
            let mut inner = self.inner.lock().expect("mechanisms registry poisoned");
            inner.apps.values_mut().find_map(|a| a.pending.remove(&id))
        };
        if let Some(tx) = waiting {
            let _ = tx.send(result);
        }
    }

    /// Drop a pending call without resolving it (the timeout path).
    fn forget(&self, id: Uuid) {
        let mut inner = self.inner.lock().expect("mechanisms registry poisoned");
        for app in inner.apps.values_mut() {
            if app.pending.remove(&id).is_some() {
                break;
            }
        }
    }

    fn publish(&self, event: Event) {
        // No subscribers is the normal case today; the send failing means exactly
        // that and is not a problem.
        let _ = self.events.send(event);
    }
}

/// Fail every call an app still owed.
fn fail_all(app: Attached) {
    for (_, tx) in app.pending {
        let _ = tx.send(Err(CallError::Detached));
    }
}

// ---------------------------------------------------------------------------
// The socket
// ---------------------------------------------------------------------------

/// `WS /api/mechanisms` — an app offering its hands.
///
/// **No gate of its own, on purpose.** Which listener accepted a request is the
/// whole trust decision here and it is made once, centrally
/// ([`crate::foundation::surfaces::gate`], layered on the router): the loopback
/// listener binds `127.0.0.1` only and passes ungated, and the off-box listener
/// requires a surface credential. Re-checking the peer address in this handler would
/// be the second copy of that decision — and the wrong copy, because it would refuse
/// a properly credentialed remote app, which the design allows.
pub async fn get_mechanisms(
    State(state): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| run(state, socket))
}

/// `GET /api/mechanisms/attached` — what is attached and what it says it can do.
///
/// A debug view in the plain sense: the registry as it stands, not a reshaping of
/// it. This exists because a seam with no endpoint reads as broken to anyone
/// looking, however correct it is.
pub async fn get_attached(State(state): State<Arc<AppState>>) -> Response {
    let apps: Vec<Value> = state
        .mechanisms
        .attached()
        .into_iter()
        .map(|(app, mechanisms)| serde_json::json!({ "app": app, "mechanisms": mechanisms }))
        .collect();
    Json(serde_json::json!({ "attached": apps })).into_response()
}

/// What `POST /api/mechanisms/call` takes.
#[derive(Debug, Deserialize)]
pub struct CallRequest {
    method: String,
    #[serde(default)]
    params: Value,
}

/// `POST /api/mechanisms/call` — make one mechanism call and return its answer.
///
/// This is the seam's own test surface, and it is the only caller of
/// [`Mechanisms::call`] in the tree today. It exists for two reasons that outlast
/// that: a shell author needs to exercise their `screen.grab` without standing up
/// cognition first, and a seam with no endpoint is indistinguishable from a broken
/// one to anyone looking at a running instance.
///
/// A JSON reply comes back as JSON; a byte reply comes back as the bytes, with the
/// content type left deliberately vague — this hands back exactly what the app sent
/// and does not interpret it.
pub async fn post_call(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CallRequest>,
) -> Response {
    match state.mechanisms.call(&req.method, req.params).await {
        Ok(Reply::Json(v)) => Json(v).into_response(),
        Ok(Reply::Bytes(b)) => (
            [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
            b,
        )
            .into_response(),
        // Nothing offers it. Not a server fault: a core with no app attached has no
        // hands, which is a normal state the caller degrades around.
        Err(e @ CallError::Unavailable(_)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn run(state: Arc<AppState>, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();

    // The first frame must be `hello`. Until an app has said what it can do there is
    // nothing to route to it, so anything else closes the socket.
    let hello = stream.next().await;
    let (app, mechanisms) = match hello {
        Some(Ok(WsMessage::Text(t))) => match serde_json::from_str::<FromApp>(&t) {
            Ok(FromApp::Hello { app, mechanisms }) => (app, mechanisms),
            Ok(_) => {
                tracing::warn!("mechanisms: first frame was not `hello`; closing");
                return;
            }
            Err(e) => {
                tracing::warn!(error = %e, "mechanisms: unparseable `hello`; closing");
                return;
            }
        },
        _ => return,
    };

    tracing::info!(app = %app, ?mechanisms, "mechanisms: app attached");
    let mut outbound = state.mechanisms.attach(&app, mechanisms);

    // Pump calls out. A send failure means the socket is gone; the read loop below
    // sees the same thing and does the detaching.
    let pump = tokio::spawn(async move {
        while let Some(frame) = outbound.recv().await {
            let text = match serde_json::to_string(&frame) {
                Ok(t) => t,
                Err(e) => {
                    tracing::error!(error = %e, "mechanisms: un-serializable call");
                    continue;
                }
            };
            if sink.send(WsMessage::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(message)) = stream.next().await {
        match message {
            WsMessage::Text(text) => match serde_json::from_str::<FromApp>(&text) {
                Ok(FromApp::Reply { id, ok, err }) => {
                    let result = match err {
                        Some(e) => Err(CallError::Failed(e)),
                        None => Ok(Reply::Json(ok.unwrap_or(Value::Null))),
                    };
                    state.mechanisms.resolve(id, result);
                }
                Ok(FromApp::Event { kind, data }) => {
                    state.mechanisms.publish(Event { app: app.clone(), kind, data });
                }
                // A second `hello` is a protocol error, not a re-declaration: the
                // registry keys on the id this socket already gave.
                Ok(FromApp::Hello { .. }) => {
                    tracing::warn!(app = %app, "mechanisms: unexpected second `hello`; ignoring");
                }
                Err(e) => {
                    tracing::warn!(app = %app, error = %e, "mechanisms: unparseable frame");
                }
            },
            WsMessage::Binary(bytes) => match split_payload(&bytes) {
                Some((id, payload)) => {
                    state.mechanisms.resolve(id, Ok(Reply::Bytes(payload)));
                }
                None => {
                    tracing::warn!(
                        app = %app,
                        len = bytes.len(),
                        "mechanisms: binary frame too short to carry a call id"
                    );
                }
            },
            WsMessage::Close(_) => break,
            WsMessage::Ping(_) | WsMessage::Pong(_) => {}
        }
    }

    tracing::info!(app = %app, "mechanisms: app detached");
    state.mechanisms.detach(&app);
    pump.abort();
}

/// Split `[16-byte call id][payload]`. `None` if the frame cannot even carry an id.
fn split_payload(frame: &[u8]) -> Option<(Uuid, Bytes)> {
    if frame.len() < 16 {
        return None;
    }
    let (head, rest) = frame.split_at(16);
    let id = Uuid::from_slice(head).ok()?;
    Some((id, Bytes::copy_from_slice(rest)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Attach a fake app and hand back the registry plus the channel it would pump.
    fn attach(app: &str, mechanisms: &[&str]) -> (Mechanisms, mpsc::UnboundedReceiver<ToApp>) {
        let reg = Mechanisms::new();
        let rx = reg.attach(app, mechanisms.iter().map(|s| s.to_string()).collect());
        (reg, rx)
    }

    #[test]
    fn availability_is_about_who_is_attached_not_how_we_were_built() {
        let (reg, _rx) = attach("macos", &["screen", "input"]);
        assert!(reg.available("screen"));
        assert!(reg.available("input"));
        assert!(!reg.available("ax"), "undeclared mechanisms are simply absent");

        reg.detach("macos");
        assert!(!reg.available("screen"), "detaching takes the hands with it");
    }

    #[tokio::test]
    async fn a_call_reaches_the_app_and_its_answer_comes_back() {
        let (reg, mut rx) = attach("macos", &["desktop"]);

        let caller = {
            let reg = reg.clone();
            tokio::spawn(async move { reg.call("desktop.context", Value::Null).await })
        };

        let ToApp::Call { id, method, .. } = rx.recv().await.expect("the call reaches the app");
        assert_eq!(method, "desktop.context");
        reg.resolve(id, Ok(Reply::Json(serde_json::json!({ "app": "Xcode" }))));

        let reply = caller.await.expect("caller joins").expect("the call succeeds");
        match reply {
            Reply::Json(v) => assert_eq!(v["app"], "Xcode"),
            other => panic!("expected json, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_reply_is_matched_by_id_never_by_arrival_order() {
        let (reg, mut rx) = attach("macos", &["screen"]);

        let first = {
            let reg = reg.clone();
            tokio::spawn(async move { reg.call("screen.grab", serde_json::json!({"n": 1})).await })
        };
        let ToApp::Call { id: id1, .. } = rx.recv().await.expect("first call");
        let second = {
            let reg = reg.clone();
            tokio::spawn(async move { reg.call("screen.grab", serde_json::json!({"n": 2})).await })
        };
        let ToApp::Call { id: id2, .. } = rx.recv().await.expect("second call");
        assert_ne!(id1, id2);

        // Answer them backwards. Each caller must still get its own answer.
        reg.resolve(id2, Ok(Reply::Json(serde_json::json!("second"))));
        reg.resolve(id1, Ok(Reply::Json(serde_json::json!("first"))));

        let a = first.await.unwrap().unwrap();
        let b = second.await.unwrap().unwrap();
        match (a, b) {
            (Reply::Json(a), Reply::Json(b)) => {
                assert_eq!(a, serde_json::json!("first"));
                assert_eq!(b, serde_json::json!("second"));
            }
            other => panic!("expected two json replies, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn bytes_come_back_on_a_binary_frame_tagged_with_the_call_id() {
        let (reg, mut rx) = attach("macos", &["screen"]);
        let caller = {
            let reg = reg.clone();
            tokio::spawn(async move { reg.call("screen.grab", Value::Null).await })
        };
        let ToApp::Call { id, .. } = rx.recv().await.expect("the call reaches the app");

        // Exactly what an app puts on the wire: the id, then the PNG.
        let mut frame = id.as_bytes().to_vec();
        frame.extend_from_slice(b"\x89PNG-not-really");
        let (parsed, payload) = split_payload(&frame).expect("a well-formed binary frame");
        assert_eq!(parsed, id);
        reg.resolve(parsed, Ok(Reply::Bytes(payload)));

        match caller.await.unwrap().unwrap() {
            Reply::Bytes(b) => assert_eq!(&b[..], b"\x89PNG-not-really"),
            other => panic!("expected bytes, got {other:?}"),
        }
    }

    #[test]
    fn a_binary_frame_too_short_to_hold_an_id_is_refused_not_guessed() {
        assert!(split_payload(b"short").is_none());
        assert!(split_payload(&[0u8; 15]).is_none());
        assert!(split_payload(&[0u8; 16]).is_some(), "an empty payload is still a reply");
    }

    #[tokio::test]
    async fn an_unavailable_mechanism_is_its_own_answer() {
        let reg = Mechanisms::new();
        match reg.call("screen.grab", Value::Null).await {
            Err(CallError::Unavailable(m)) => assert_eq!(m, "screen"),
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn detaching_fails_the_calls_in_flight_rather_than_leaving_them_to_time_out() {
        let (reg, mut rx) = attach("macos", &["screen"]);
        let caller = {
            let reg = reg.clone();
            tokio::spawn(async move { reg.call("screen.grab", Value::Null).await })
        };
        rx.recv().await.expect("the call reaches the app");

        reg.detach("macos");

        match caller.await.unwrap() {
            Err(CallError::Detached) => {}
            other => panic!("expected Detached, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reattaching_under_the_same_id_does_not_leave_the_old_calls_hanging() {
        let (reg, mut rx) = attach("macos", &["screen"]);
        let caller = {
            let reg = reg.clone();
            tokio::spawn(async move { reg.call("screen.grab", Value::Null).await })
        };
        rx.recv().await.expect("the call reaches the app");

        // The shell restarted and dialed again.
        let _rx2 = reg.attach("macos", vec!["screen".to_string()]);

        match caller.await.unwrap() {
            Err(CallError::Detached) => {}
            other => panic!("expected Detached, got {other:?}"),
        }
        assert!(reg.available("screen"), "the new attachment is live");
    }

    #[tokio::test]
    async fn an_app_that_answers_with_an_error_fails_the_call_with_it() {
        let (reg, mut rx) = attach("macos", &["screen"]);
        let caller = {
            let reg = reg.clone();
            tokio::spawn(async move { reg.call("screen.grab", Value::Null).await })
        };
        let ToApp::Call { id, .. } = rx.recv().await.expect("the call reaches the app");
        reg.resolve(id, Err(CallError::Failed("screen recording not granted".into())));

        match caller.await.unwrap() {
            Err(CallError::Failed(m)) => assert!(m.contains("not granted")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn events_reach_a_subscriber() {
        let reg = Mechanisms::new();
        let mut events = reg.events();
        reg.publish(Event {
            app: "macos".into(),
            kind: "hotkey.edge".into(),
            data: serde_json::json!({ "edge": "down" }),
        });
        let got = events.recv().await.expect("the event arrives");
        assert_eq!(got.kind, "hotkey.edge");
        assert_eq!(got.data["edge"], "down");
    }

    #[test]
    fn the_wire_frames_are_what_the_design_doc_says_they_are() {
        // hello
        let hello: FromApp =
            serde_json::from_str(r#"{"type":"hello","app":"macos","mechanisms":["screen"]}"#)
                .expect("hello parses");
        match hello {
            FromApp::Hello { app, mechanisms } => {
                assert_eq!(app, "macos");
                assert_eq!(mechanisms, vec!["screen".to_string()]);
            }
            other => panic!("expected hello, got {other:?}"),
        }

        // an error reply
        let reply: FromApp =
            serde_json::from_str(r#"{"type":"reply","id":"00000000-0000-0000-0000-000000000001","err":"no grant"}"#)
                .expect("reply parses");
        match reply {
            FromApp::Reply { err, ok, .. } => {
                assert_eq!(err.as_deref(), Some("no grant"));
                assert!(ok.is_none());
            }
            other => panic!("expected reply, got {other:?}"),
        }

        // a call, as the app will see it
        let call = ToApp::Call {
            id: Uuid::nil(),
            method: "screen.grab".into(),
            params: serde_json::json!({ "window": 7 }),
        };
        let wire: Value = serde_json::from_str(&serde_json::to_string(&call).unwrap()).unwrap();
        assert_eq!(wire["type"], "call");
        assert_eq!(wire["method"], "screen.grab");
        assert_eq!(wire["params"]["window"], 7);
    }
}
