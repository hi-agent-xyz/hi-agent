//! The file channel: handing the agent a file (a contract, a passport scan).
//!
//! A file is a **handed artifact**, not something the agent perceives — so it
//! does not ride the vision sense. The user gives the agent a file by reference
//! through an upload *carrier*; we keep the bytes verbatim under the `file`
//! channel and journal a signal that says who handed over what. This is the
//! bundled built-in carrier (a real route + a seeded view), the platform's
//! "stdlib" for receiving files; the agent's filing/recall on top is agentic.
//!
//! Two doors, one core:
//! - `POST /api/in/file` — drag-drop from the agent's own page.
//! - phone handoff — `POST /api/handoff` mints a short-lived upload token and
//!   returns a `/up/<token>` URL; the built-in view renders it as a QR
//!   (`GET /api/qr`). A phone opens `GET /up/<token>` (a tiny uploader) and posts
//!   to `POST /api/up/<token>`.
//!
//! Every door funnels into [`receive_file`], which mirrors the text path: store
//! the bytes ([`media::store_blob`]), journal a `SignalIn`, echo to observers,
//! and — crucially — send the `Signal` inbound so the reaction *wakes* and the
//! agent reacts — as a presence change does ([`super::vision`], which journals
//! *and* sends `Inbound::Observed`). A handed file must wake the mind.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::mind::memory::layout::MediaSlot;
use crate::mind::memory::media;
use crate::foundation::server::AppState;
use crate::types::{Author, Channel, Content, FileRef, Inbound, JournalEntry, Message, Sender};

/// How long a minted phone-upload token stays valid. Long enough to pick up your
/// phone and scan; short enough that a leaked QR doesn't linger.
const HANDOFF_TTL: Duration = Duration::from_secs(600);

/// A short-lived phone-upload grant.
pub struct Handoff {
    pub expires: Instant,
}

#[derive(Debug, Serialize)]
struct UploadFailure {
    index: usize,
    name: String,
    error: String,
}

#[derive(Debug, Default, Serialize)]
struct UploadResult {
    attempted: usize,
    received: usize,
    failed: Vec<UploadFailure>,
}

impl UploadResult {
    fn status(&self) -> StatusCode {
        if self.attempted == 0 {
            StatusCode::BAD_REQUEST
        } else if self.failed.is_empty() {
            StatusCode::OK
        } else {
            StatusCode::MULTI_STATUS
        }
    }
}

// -----------------------------------------------------------------------------
// Core: receive one handed file
// -----------------------------------------------------------------------------

/// A filesystem-safe lowercase extension for the stored blob, from the original
/// filename when it has one, else mapped from the mime, else `bin`.
/// The inverse of [`ext_for`], for serving bytes back. Deliberately a short list
/// rather than a mime database: the only extensions this route can ever see are the
/// ones `ext_for` wrote, and anything it doesn't recognize is safest served as
/// opaque bytes the browser will not try to execute.
fn mime_for_ext(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain; charset=utf-8",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

fn ext_for(name: &str, mime: &str) -> String {
    if let Some(dot) = name.rfind('.') {
        let raw = &name[dot + 1..];
        let ext: String = raw.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
        if !ext.is_empty() && ext.len() <= 8 {
            return ext;
        }
    }
    match mime.split(';').next().unwrap_or("").trim() {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        _ => "bin",
    }
    .to_string()
}

/// Store one handed file under the `file` channel and deliver it as an inbound
/// signal that wakes the reaction. `body` is the text surface the mind reacts to
/// (it never sees the bytes); the blob path rides the journal entry's `media`.
/// The framing in `body` is the caller's — a neutral handoff ([`receive_file`])
/// or the macOS "come and see this" gesture ([`receive_screenshot`]) — and this appends
/// the locator, so no caller can frame a file the mind then cannot open.
///
/// **The locator is the whole point of the signal.** `docs/arch/agents.md` retires
/// the perception tool on the grounds that "a photo or a file arrives as a **ref**,
/// a ref is a path, and an agent that can read files can open it" — and
/// `cognition.md` teaches exactly that, naming this line as an example. It was
/// not true: the body said `The user handed you a file: passport.jpg` and nothing
/// else, while the bytes landed under a generated name that is not `passport.jpg`.
/// The one rung that can read was told a file existed and given no way to reach it.
///
/// The ref is `<day>/<rel>` under the channel's own root — the same grammar the
/// vision channel already emits and [`media::resolve`] already reads — and it is
/// deliberately *relative*: this string is appended to the durable log, and
/// invariant 11 forbids persisting a host path into `data/`. The absolute root
/// reaches the rung through its prompt (`{raw_dir}`), which is reinstalled from
/// the binary every boot and so may hold one.
async fn ingest_file(
    state: &AppState,
    name: &str,
    mime: &str,
    bytes: &Bytes,
    note: Option<String>,
) -> Result<(), String> {
    let ts = Utc::now();
    let ext = ext_for(name, mime);
    let rel = media::store_blob(&state.data_dir, Channel::File, ts, MediaSlot::InputOneOff, &ext, bytes)
        .await
        .map_err(|e| format!("store file: {e}"))?;
    let reff = media::signal_ref(Channel::File, ts, &rel);

    crate::foundation::channel_log::inbound(Channel::File, name);

    deliver_artifact(
        state,
        ts,
        FileRef {
            reff,
            mime: mime.to_string(),
            name: name.to_string(),
            bytes: Some(bytes.len() as u64),
            // No peek: a carrier that hands over a photo or a PDF has no opening
            // worth quoting, and this one does not read what it stores. The typed
            // channel's own artifacts take theirs from the bytes as they arrive.
            peek: None,
        },
        note,
    )
    .await
}

/// Put an artifact whose bytes are already on disk into the conversation: journal it,
/// append it to the live window, and wake the reaction.
///
/// **The tail of every handed artifact, shared because the head is what differs.** A
/// dragged file arrives whole and is written in one call; a typed body arrives in
/// chunks and is written as it streams (see
/// [`crate::foundation::server::text::post_text`]). By the time either has a
/// [`FileRef`] the remaining work is identical, and a second copy of it is how the two
/// paths would drift into journalling the same thing two ways.
pub(crate) async fn deliver_artifact(
    state: &AppState,
    ts: DateTime<Utc>,
    file: FileRef,
    note: Option<String>,
) -> Result<(), String> {
    // Addressed, like text: a file is *handed over*, and the hander is the owner
    // unless something says otherwise. `Channel::File`'s own definition already
    // promised "the signal says who handed over what"; this is that field.
    let sender = Sender::owner_or_unknown(crate::foundation::config::tunables::owner().as_deref());

    // **One arrival, one message per thing communicated.** A note the person
    // effectively said — the screen gesture's own first-person line — is its own
    // message and comes first; the file is its own. Nothing frames the file in prose
    // any more: its name is a field, and the sentence the mind reads is rendered from
    // that field rather than stored in the words.
    let mut messages = Vec::new();
    if let Some(note) = note {
        messages.push(Message {
            id: Uuid::now_v7().to_string(),
            ts,
            from: Author::Person(sender.clone()),
            content: Content::Text(note),
        });
    }
    messages.push(Message {
        id: Uuid::now_v7().to_string(),
        ts,
        from: Author::Person(sender),
        content: Content::File(file),
    });

    // Journaled first, all of them, so durability precedes reaction — and then
    // enqueued back to back with nothing awaited between, which is what keeps the
    // settle window from splitting one arrival across two turns.
    for message in &messages {
        let entry = JournalEntry::Message { channel: Channel::File, message: message.clone() };
        if let Err(err) = state.memory.journal.append(entry).await {
            tracing::error!(error = %format!("{err:#}"), "journal append failed; accepting file anyway");
        }
    }
    for message in messages {
        state.note_message(Channel::File, message.clone());
        state
            .inbound
            .send(Inbound::Message(message))
            .await
            .map_err(|_| "inbound channel closed".to_string())?;
    }
    Ok(())
}

/// A photo the person handed over is part of what they said, so the face has to be
/// able to show it rather than a grey placeholder naming a path. The ref is the
/// same channel-qualified locator the journal and the agent use, resolved through
/// [`media::resolve_ref`] — which means a day that has since faded degrades to its
/// keepsake, or to a 404, instead of pretending the original is still there.
///
/// Read-only, and confined to what the ref grammar can express: `parse_ref` accepts
/// `<channel>/<date>/<HH>/<MM>-<SS>.<ext>` and nothing else, so no path this route
/// resolves can escape the media tree.
pub async fn get_media(
    State(state): State<Arc<AppState>>,
    Path(reff): Path<String>,
) -> Response {
    let Some(path) = media::resolve_ref(&state.data_dir, &reff).await else {
        return (StatusCode::NOT_FOUND, "no such media").into_response();
    };
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!(reff = %reff, error = %err, "media read failed");
            return (StatusCode::NOT_FOUND, "no such media").into_response();
        }
    };
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    (
        [
            (header::CONTENT_TYPE, mime_for_ext(&ext)),
            (header::CACHE_CONTROL, "private, max-age=31536000, immutable"),
        ],
        bytes,
    )
        .into_response()
}

/// Receive one handed file (drag-drop / picker / phone handoff) — a neutral
/// document handoff, framed as such.
async fn receive_file(
    state: &AppState,
    name: &str,
    mime: &str,
    bytes: &Bytes,
) -> Result<(), String> {
    ingest_file(state, name, mime, bytes, None).await
}

/// Receive a screenshot pushed with the "come and see this" gesture (double-tap
/// Command; see [`crate::body::gesture`]). Same `file` channel, same wake — only the
/// framing differs: this is the user's current screen, handed over to look at and
/// help with, not a neutral document.
///
/// **First person, because the gesture is the user's.** ⌘⌘ is a shortcut they pressed —
/// the same act as dragging an image into the box — so writing it as their own line is a
/// transcription of what they did, not an invention. It also matches the row the mind
/// reads it on, `>/file …`, where `>` is their side of the conversation; a third-person
/// `The user is showing you…` sits on that row one person out of step. What was wrong
/// with the old framing was never the person but the content: a quoted "come and see
/// this" they never said (which a model will read as speech and repeat), the keystroke
/// that produced the PNG, and its byte size.
///
/// **One clause, and it is written for Reaction.** Cognition joins the ref to
/// `{raw_dir}` and opens the PNG, so it needs no help knowing what it is looking at.
/// Reaction never sees a pixel — no `{raw_dir}`, and `reaction.md` tells it "you don't
/// read files" — so this sentence is the entire basis on which it decides to hand the
/// turn down. "My screen right now" is decidable on that basis; a keystroke and a
/// payload size are not.
#[cfg(target_os = "macos")]
pub(crate) async fn receive_screenshot(
    state: &AppState,
    bytes: &Bytes,
) -> Result<(), String> {
    let name = format!("screen-{}.png", Utc::now().format("%Y%m%d-%H%M%S"));
    ingest_file(state, &name, "image/png", bytes, Some("Here's my screen right now.".to_string())).await
}

/// Drain a multipart body, storing every file part. A failed file does not hide
/// siblings that already landed or prevent later parts from being attempted.
async fn drain_multipart(
    state: &AppState,
    mut mp: Multipart,
) -> Result<UploadResult, (StatusCode, String)> {
    let mut result = UploadResult::default();
    loop {
        match mp.next_field().await {
            Ok(Some(field)) => {
                // Only parts that carry a filename are files; skip plain fields,
                // but still drain their bytes so the stream advances.
                let name = field.file_name().map(str::to_owned);
                let mime = field
                    .content_type()
                    .map(str::to_owned)
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                let Some(name) = name else {
                    let _ = field.bytes().await;
                    continue;
                };
                let index = result.attempted;
                result.attempted += 1;
                match field.bytes().await {
                    Ok(bytes) => match receive_file(state, &name, &mime, &bytes).await {
                        Ok(()) => result.received += 1,
                        Err(error) => {
                            tracing::error!(file = %name, %error, "file upload failed");
                            result.failed.push(UploadFailure { index, name, error });
                        }
                    }
                    Err(e) => return Err((StatusCode::BAD_REQUEST, format!("reading upload: {e}"))),
                }
            }
            Ok(None) => break,
            Err(e) => return Err((StatusCode::BAD_REQUEST, format!("malformed multipart: {e}"))),
        }
    }
    Ok(result)
}

// -----------------------------------------------------------------------------
// Routes
// -----------------------------------------------------------------------------

/// `POST /api/in/file` — drag-drop / picker from the agent's own page.
pub async fn post_file(
    State(state): State<Arc<AppState>>,
    mp: Multipart,
) -> Response {
    tracing::info!("POST /api/in/file");
    match drain_multipart(&state, mp).await {
        Ok(result) => (result.status(), Json(result)).into_response(),
        Err((code, msg)) => (code, msg).into_response(),
    }
}

/// `POST /api/handoff` — mint a short-lived upload token and return the
/// `/up/<token>` URL (absolute) for the built-in view to render as a QR. Reusable
/// until it expires.
///
/// The address comes from [`crate::foundation::server::surfaces::public_base_url`]
/// for the reason written there: the panel showing the QR is open on this machine,
/// over loopback, and the phone about to scan it cannot reach `127.0.0.1`.
pub async fn post_handoff(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let token = Uuid::now_v7().to_string();
    let base = crate::foundation::server::surfaces::public_base_url(&state.data_dir, &headers)
        .await
        .trim_end_matches('/')
        .to_string();
    let url = format!("{base}/up/{token}");

    {
        let mut map = state.handoffs.lock().unwrap();
        prune_expired(&mut map);
        map.insert(token.clone(), Handoff { expires: Instant::now() + HANDOFF_TTL });
    }
    Json(serde_json::json!({ "url": url, "token": token })).into_response()
}

/// `GET /up/{token}` — the phone's uploader page. A self-contained HTML form that
/// posts to `POST /api/up/<token>`.
pub async fn get_up_page(State(state): State<Arc<AppState>>, Path(token): Path<String>) -> Response {
    if !resolve_token(&state, &token) {
        return (StatusCode::GONE, Html(EXPIRED_PAGE.to_string())).into_response();
    }
    Html(upload_page(&token)).into_response()
}

/// `POST /api/up/{token}` — the phone uploads here.
pub async fn post_up(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
    mp: Multipart,
) -> Response {
    if !resolve_token(&state, &token) {
        return (StatusCode::GONE, Html(EXPIRED_PAGE.to_string())).into_response();
    }
    tracing::info!("POST /api/up/<token>");
    match drain_multipart(&state, mp).await {
        Ok(result) if result.attempted == 0 => {
            (StatusCode::BAD_REQUEST, Html(result_page("没有选择文件", false))).into_response()
        }
        Ok(result) if result.failed.is_empty() => (
            StatusCode::OK,
            Html(result_page(
                &format!("已发送 {} 个文件给 agent，可以关掉本页了。", result.received),
                true,
            )),
        )
            .into_response(),
        Ok(result) => (
            StatusCode::MULTI_STATUS,
            Html(result_page(
                &format!(
                    "已发送 {} 个文件，{} 个失败。回到 agent 可以重试。",
                    result.received,
                    result.failed.len()
                ),
                false,
            )),
        )
            .into_response(),
        Err((code, msg)) => (code, Html(result_page(&msg, false))).into_response(),
    }
}

#[derive(Deserialize)]
pub struct QrQuery {
    data: String,
}

/// `GET /api/qr?data=…` — render `data` as an SVG QR code. A dumb encoder; the
/// upload view passes it the absolute `/up/<token>` URL.
pub async fn get_qr(Query(q): Query<QrQuery>) -> Response {
    use qrcode::render::svg;
    match qrcode::QrCode::new(q.data.as_bytes()) {
        Ok(code) => {
            let svg = code
                .render::<svg::Color>()
                .min_dimensions(220, 220)
                .quiet_zone(true)
                .build();
            ([(header::CONTENT_TYPE, "image/svg+xml")], svg).into_response()
        }
        Err(_) => (StatusCode::BAD_REQUEST, "bad qr data\n").into_response(),
    }
}

// -----------------------------------------------------------------------------
// Token helpers
// -----------------------------------------------------------------------------

/// Resolve a token, dropping it if expired. Prunes other stale entries while
/// holding the lock.
fn resolve_token(state: &AppState, token: &str) -> bool {
    let mut map = state.handoffs.lock().unwrap();
    prune_expired(&mut map);
    map.contains_key(token)
}

fn prune_expired(map: &mut std::collections::HashMap<String, Handoff>) {
    let now = Instant::now();
    map.retain(|_, h| h.expires > now);
}

// -----------------------------------------------------------------------------
// Small helpers
// -----------------------------------------------------------------------------

/// A filesystem-safe lowercase extension for the stored blob, from the original
/// filename when it has one, else mapped from the mime, else `bin`.
/// The inverse of [`ext_for`], for serving bytes back. Deliberately a short list
/// rather than a mime database: the only extensions this route can ever see are the
/// ones `ext_for` wrote, and anything it doesn't recognize is safest served as

// -----------------------------------------------------------------------------
// Phone pages (self-contained, no shared assets)
// -----------------------------------------------------------------------------

fn upload_page(token: &str) -> String {
    format!(
        r#"<!doctype html><html lang="zh"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>传给 agent</title>
<style>
 :root {{ color-scheme: light dark; }}
 body {{ font: 16px/1.5 -apple-system,system-ui,sans-serif; margin: 0; padding: 24px;
        display:flex; flex-direction:column; gap:16px; max-width:480px; margin:0 auto; }}
 h1 {{ font-size: 20px; margin: 8px 0; }}
 .drop {{ border: 2px dashed #888; border-radius: 14px; padding: 28px 16px; text-align:center; }}
 input[type=file] {{ width: 100%; }}
 button {{ font-size: 17px; padding: 12px 16px; border-radius: 12px; border: 0;
           background: #2563eb; color: #fff; width: 100%; }}
 .hint {{ color: #888; font-size: 14px; }}
</style></head>
<body>
 <h1>把文件传给 agent</h1>
 <form action="/api/up/{token}" method="post" enctype="multipart/form-data">
   <div class="drop">
     <p class="hint">选择要传的文件（可多选）</p>
     <input type="file" name="file" multiple required>
   </div>
   <p><button type="submit">发送</button></p>
 </form>
 <p class="hint">这个链接很快会过期，只把它给你信任的设备。</p>
</body></html>"#
    )
}

fn result_page(msg: &str, ok: bool) -> String {
    let mark = if ok { "✓" } else { "⚠" };
    format!(
        r#"<!doctype html><html lang="zh"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>已发送</title>
<style>
 :root {{ color-scheme: light dark; }}
 body {{ font: 17px/1.6 -apple-system,system-ui,sans-serif; margin:0; padding:48px 24px;
        text-align:center; max-width:480px; margin:0 auto; }}
 .mark {{ font-size: 48px; }}
</style></head>
<body><div class="mark">{mark}</div><p>{msg}</p></body></html>"#
    )
}

const EXPIRED_PAGE: &str = r#"<!doctype html><html lang="zh"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>链接已过期</title>
<style>:root{color-scheme:light dark;}body{font:17px/1.6 -apple-system,system-ui,sans-serif;
 padding:48px 24px;text-align:center;max-width:480px;margin:0 auto;}</style></head>
<body><div style="font-size:48px">⌛</div><p>这个上传链接已过期。回到 agent 让它再给你一个二维码。</p></body></html>"#;
