//! The text channel: `POST /api/in/text`, `POST /api/in/text/typing`,
//! `GET /api/in/text`, `GET /api/out/text`.
//!
//! `POST /api/in/text` is the typed-input path: the body is dispatched to the
//! mind (journalled + queued on `inbound`), echoed to live channel observers,
//! and appended to the conversation as one message.
//!
//! **Its body is streamed and unbounded, and its size decides what it becomes.**
//! Under [`INLINE_MAX`] it is words, and takes the path just described. Over it the
//! same bytes are a handed artifact — written through to a blob as they arrive, kept
//! verbatim under the file channel, and delivered as a `Content::File` carrying a ref,
//! a size and a peek instead of the whole payload. One person, one channel, one act of
//! handing something over; only the shape of what they handed over differs.
//!
//! `POST /api/in/text/typing` is the only thing here that is not input: a
//! contentless ping saying a line is being written. It never becomes a message, a
//! journal entry, or an observed frame — it goes to the floor and nowhere else, so
//! the agent does not answer half a thought. See [`crate::body::reaction::floor`].
//!
//! `GET /api/out/text` is one long-lived NDJSON stream of the conversation: the
//! current window whole, then one frame per message appended. `GET /api/messages`
//! reads further back through the journal. Messages carry the journal's ids so the
//! two agree, but no client ever sends one back — there is no cursor, no
//! acknowledgement and no read position anywhere in this file.
//!
//! `GET /api/in/text` is a live observe stream (see [`crate::foundation::server::observe`]):
//! no buffering, just the inputs as they cross the boundary.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::foundation::server::headers::{AuthBearer, StreamHeader};
use crate::foundation::server::{AppState, observe, transcript};
use crate::mind::memory::journal;
use crate::mind::memory::layout::MediaSlot;
use crate::types::{Author, Channel, Content, Inbound, JournalEntry, Message, Sender};
use futures::stream::unfold;

/// The largest typed body that goes into the prompt **as words**.
///
/// Above this the same body still arrives whole and is still kept whole — it simply
/// stops being an utterance and becomes a handed artifact (see [`post_text`]). The
/// number is a judgement about what a person can be *saying*: a long paragraph, a
/// stack trace, a page of notes all sit far below it; a logfile, a CSV export or a
/// pasted book sit far above. 64 KiB is roughly 16k tokens — big enough that no
/// ordinary line trips it, small enough that one paste cannot take a sixth of a
/// 100k-token window.
const INLINE_MAX: usize = 64 * 1024;

/// How much of an oversized body's opening the prompt quotes.
///
/// The same number [`crate::foundation::channel_log`] already clips a logged body to,
/// because it answers the same question — how much is a readable excerpt.
const PEEK_CHARS: usize = 2000;

/// How much of an artifact the credential scan reads back.
///
/// The scan exists because somebody pastes a key without thinking, and a key pasted
/// without thinking is at the top of what they pasted. Beyond this the read-back would
/// cost more than the case is worth, and the boundary says so in the log rather than
/// pretending it scanned.
const SCAN_MAX: u64 = 4 * 1024 * 1024;

/// `POST /api/in/text` — somebody typed (or pasted) something.
///
/// **The body is streamed, and the size of what arrives is not bounded.** It used to
/// be taken as `Bytes`, which buffers the whole thing before the handler runs and so
/// inherited axum's 2 MB default limit — a ceiling nobody chose, enforced by a 413 the
/// face swallowed. A person handing over a 50 MB export was told nothing and lost it.
/// Now at most [`INLINE_MAX`] is ever held in memory: past that the rest is written
/// through to a blob as it arrives, so the route needs no limit and has none.
///
/// **Which also decides what the thing *is*.** Under the seam a body is words, and
/// takes the path it always took — `Content::Text`, journalled on the text channel,
/// into the prompt verbatim. Over it, the same bytes are a **handed artifact**: kept
/// verbatim under the file channel (which forgetting exempts), delivered as a
/// `Content::File` carrying a ref, a size and a peek. The mind gets an opening it can
/// judge and a path it can open, instead of a megabyte it must carry every turn.
/// See `docs/arch/message.md`.
pub async fn post_text(
    State(state): State<Arc<AppState>>,
    StreamHeader(stream): StreamHeader,
    AuthBearer(auth): AuthBearer,
    body: Body,
) -> Response {
    let ts = Utc::now();
    // A typed line has no capture source to tell apart, so the stream header stops
    // here rather than riding along on a message that has no use for it.
    let _ = stream;

    let received = match receive_body(&state, ts, body).await {
        Ok(received) => received,
        Err(response) => return response,
    };

    tracing::info!(auth = ?auth, len = received.total, "POST /api/in/text");

    match received.kind {
        Received::Words(text) => post_words(&state, ts, text).await,
        Received::Artifact { rel, peek } => post_artifact(&state, ts, rel, peek, received.total).await,
    }
}

/// Read the body, spilling to a blob once it crosses [`INLINE_MAX`].
///
/// The spill is one-way: the first `INLINE_MAX` bytes stay in `head` after the crossing
/// and become the peek, and nothing accumulates after it. So the memory this holds is
/// bounded by the constant whatever the body's size — that bound is the whole reason
/// the route can advertise no limit.
async fn receive_body(
    state: &AppState,
    ts: DateTime<Utc>,
    body: Body,
) -> Result<ReceivedBody, Response> {
    use futures::StreamExt as _;

    let mut stream = body.into_data_stream();
    let mut head: Vec<u8> = Vec::new();
    let mut spill: Option<(String, tokio::fs::File)> = None;
    let mut total: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            // A body that stopped arriving mid-flight. Whatever was written stays on
            // disk unreferenced rather than becoming a message that claims to be the
            // whole of what somebody sent.
            Err(err) => {
                tracing::warn!(error = %err, bytes = total, "typed body cut short; nothing accepted");
                return Err((StatusCode::BAD_REQUEST, "body ended early").into_response());
            }
        };
        total += chunk.len() as u64;

        // Already spilling: straight through to disk, nothing retained.
        if let Some((_, file)) = spill.as_mut() {
            write_through(file, &chunk).await?;
            continue;
        }

        // Still inline, and it fits.
        let room = INLINE_MAX - head.len();
        if chunk.len() <= room {
            head.extend_from_slice(&chunk);
            continue;
        }

        // The crossing. **The head is filled to the brim first**, rather than this whole
        // chunk going to disk: a body can arrive as a single chunk — nothing about the
        // wire promises otherwise — and handing that one straight through would leave
        // `head` empty, so the largest pastes would be the ones with no peek at all.
        let (fill, rest) = chunk.split_at(room);
        head.extend_from_slice(fill);
        let opened = crate::mind::memory::media::create_blob(
            &state.data_dir,
            Channel::File,
            ts,
            MediaSlot::InputOneOff,
            "txt",
        )
        .await;
        let (rel, mut file) = match opened {
            Ok(opened) => opened,
            Err(err) => {
                tracing::error!(error = %format!("{err:#}"), "opening the artifact failed");
                return Err((StatusCode::INTERNAL_SERVER_ERROR, "cannot store that").into_response());
            }
        };
        write_through(&mut file, &head).await?;
        write_through(&mut file, rest).await?;
        spill = Some((rel, file));
    }

    let Some((rel, mut file)) = spill else {
        let text = match String::from_utf8(head) {
            Ok(text) => text,
            Err(_) => return Err((StatusCode::BAD_REQUEST, "text body must be utf-8").into_response()),
        };
        return Ok(ReceivedBody { total, kind: Received::Words(text) });
    };

    // Durability declared here, at the one point that knows the whole body arrived.
    use tokio::io::AsyncWriteExt as _;
    let synced = match file.flush().await {
        Ok(()) => file.sync_data().await,
        Err(err) => Err(err),
    };
    if let Err(err) = synced {
        tracing::error!(error = %err, "flushing the artifact failed");
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "cannot store that").into_response());
    }
    Ok(ReceivedBody { total, kind: Received::Artifact { rel, peek: peek_of(&head) } })
}

/// Write to the artifact, turning a failed write into the answer the caller returns.
///
/// The three call sites are one situation — the disk would not take it — and there is
/// nothing useful to say about which of them was unlucky.
async fn write_through(file: &mut tokio::fs::File, bytes: &[u8]) -> Result<(), Response> {
    use tokio::io::AsyncWriteExt as _;
    file.write_all(bytes).await.map_err(|err| {
        tracing::error!(error = %err, "writing the artifact failed");
        (StatusCode::INTERNAL_SERVER_ERROR, "cannot store that").into_response()
    })
}

/// What arrived, and how much of it.
struct ReceivedBody {
    total: u64,
    kind: Received,
}

enum Received {
    /// Under the seam: what somebody said.
    Words(String),
    /// Over it: bytes on disk, plus the opening kept back for the prompt.
    Artifact { rel: String, peek: String },
}

/// The opening of a body, as the prompt will quote it.
///
/// Lossy on purpose: `head` is cut at a byte count, so its last character is very
/// likely split, and a peek that refuses to render because one trailing byte is
/// incomplete would be worse than a peek that drops it. The cut to [`PEEK_CHARS`]
/// removes the replacement character with it in every case a real body can produce —
/// `head` is 64 KiB, which is at least 16k characters.
fn peek_of(head: &[u8]) -> String {
    String::from_utf8_lossy(head).chars().take(PEEK_CHARS).collect()
}

/// Under the seam: a typed line, exactly as before.
async fn post_words(state: &AppState, ts: DateTime<Utc>, body_str: String) -> Response {
    // The one place credentials are looked for. A key somebody typed without
    // thinking is written to `drive/accounts/secrets/`, and every model prompt
    // gets that path in its place (`AgentSession::prompt`). Nothing below this
    // line changes: the journal, the conversation and `/api/out/text` all carry
    // the message exactly as it was sent — the person is not the one being kept
    // from their own key.
    //
    // A scan that fails must not cost them the message. It is logged and the
    // signal goes on, because the alternative — refusing input — is a worse
    // failure than a key reaching the model, which is what happened before any
    // of this existed.
    match state.privacy.filter().file_secrets(&body_str) {
        Ok(filed) if !filed.is_empty() => tracing::info!(
            count = filed.len(),
            refs = ?filed.iter().map(|f| f.reference.as_str()).collect::<Vec<_>>(),
            "filed credentials from an inbound message; sessions will see their paths"
        ),
        Ok(_) => {}
        Err(error) => tracing::error!(
            error = %format!("{error:#}"),
            "secret scan failed; the message is accepted unscanned"
        ),
    }
    crate::foundation::channel_log::inbound(Channel::Text, &body_str);

    // Addressed: somebody typed this *to* the agent, so absent evidence otherwise it
    // is the owner. Labelled `owner` rather than written bare, so a later pass can
    // tell the default from a recognition — see `docs/arch/signal-attribution.md`.
    // Decided once and handed to both the journal and the conversation, so the face
    // beside the message and the name in the log are one answer, not two.
    let sender = Sender::owner_or_unknown(crate::foundation::config::tunables::owner().as_deref());
    // Minted once and used three times: the journal entry, the conversation and
    // Reaction are the same value under the same key, which is what lets the list be
    // rebuilt from the log without a merge — and what gives Reaction something to
    // name when it concludes anything about this line.
    let message = Message {
        id: Uuid::now_v7().to_string(),
        ts,
        from: Author::Person(sender),
        content: Content::Text(body_str),
    };
    let entry = JournalEntry::Message { channel: Channel::Text, message: message.clone() };
    if let Err(err) = state.memory.journal.append(entry).await {
        tracing::error!(error = %format!("{err:#}"), "journal append failed; accepting signal anyway");
    }

    state.floor.note_sent().await;

    // Append to the conversation and echo to live observers before dispatching
    // inward.
    state.note_message(Channel::Text, message.clone());

    if let Err(err) = state.inbound.send(Inbound::Message(message)).await {
        tracing::error!(error = %err, "inbound channel closed");
        return (StatusCode::SERVICE_UNAVAILABLE, "inbound channel closed").into_response();
    }

    StatusCode::ACCEPTED.into_response()
}

/// Over the seam: the bytes are already on disk, so this only has to say what they are.
///
/// It hands off to [`crate::foundation::server::files::deliver_artifact`] — the same tail
/// a dragged file takes — because by this point the two arrivals differ in nothing. What
/// this adds is the framing only the typed channel can supply: the peek, and a name that
/// says where the thing came from, since a paste has none of its own.
async fn post_artifact(
    state: &AppState,
    ts: DateTime<Utc>,
    rel: String,
    peek: String,
    total: u64,
) -> Response {
    let reff = crate::mind::memory::media::signal_ref(Channel::File, ts, &rel);
    let name = format!("pasted-{}.txt", ts.format("%Y%m%d-%H%M%S"));

    scan_artifact(state, &rel, &peek, total, ts).await;
    crate::foundation::channel_log::inbound(Channel::File, &name);

    state.floor.note_sent().await;

    let file = crate::types::FileRef {
        reff,
        mime: "text/plain; charset=utf-8".to_string(),
        name,
        bytes: Some(total),
        peek: Some(peek),
    };
    match crate::foundation::server::files::deliver_artifact(state, ts, file, None).await {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(err) => {
            tracing::error!(error = %err, "delivering the pasted artifact failed");
            (StatusCode::SERVICE_UNAVAILABLE, err).into_response()
        }
    }
}

/// Look for credentials in an artifact the way [`post_words`] looks for them in a line.
///
/// **The scan does not follow the bytes to disk for free, and this is where that is
/// paid for.** The typed channel is the one place credentials are looked for at all —
/// a dragged file has never been scanned — so letting a 200 KB paste become "a file"
/// would quietly retire the check for exactly the case that motivated it: somebody
/// pasting a config. So the artifact is read back and scanned, up to [`SCAN_MAX`].
///
/// Past that ceiling only the peek is scanned and the log says `partial`, because a
/// boundary that cannot honestly claim it read everything should say which it did.
/// Nothing here refuses or rewrites the input: as in the typed path, a finding is filed
/// and the artifact still arrives exactly as it was sent.
async fn scan_artifact(state: &AppState, rel: &str, peek: &str, total: u64, ts: DateTime<Utc>) {
    let (text, partial) = if total <= SCAN_MAX {
        let path = crate::mind::memory::layout::channel_day_dir(&state.data_dir, Channel::File, ts)
            .join(rel);
        match tokio::fs::read_to_string(&path).await {
            Ok(text) => (text, false),
            Err(err) => {
                tracing::warn!(error = %err, "re-reading the artifact for the scan failed; scanning its opening");
                (peek.to_string(), true)
            }
        }
    } else {
        (peek.to_string(), true)
    };

    match state.privacy.filter().file_secrets(&text) {
        Ok(filed) if !filed.is_empty() => tracing::info!(
            count = filed.len(),
            partial,
            refs = ?filed.iter().map(|f| f.reference.as_str()).collect::<Vec<_>>(),
            "filed credentials from a pasted artifact; sessions will see their paths"
        ),
        Ok(_) => {
            if partial {
                tracing::info!(bytes = total, "artifact too large to scan whole; scanned its opening only");
            }
        }
        Err(error) => tracing::error!(
            error = %format!("{error:#}"),
            "secret scan failed; the artifact is accepted unscanned"
        ),
    }
}

/// `POST /api/in/text/typing` — a keystroke landed in a line that has not been
/// sent yet.
///
/// Contentless on purpose. The draft's *text* is nobody's business but the window
/// holding it: this reports only that a thought is in progress, which is all the
/// floor needs to not answer over it. It is deliberately not the recognition
/// `interim`, which exists to be *shown* — a half-typed line is not shown anywhere,
/// and routing drafts into that slot would put one window's unsent keystrokes on
/// every other window's screen.
///
/// Cheap, idempotent, and fire-and-forget: a composer pings this while the person
/// writes, and stopping is expressed by not pinging. Nothing is stored per client
/// and no reply body is produced.
pub async fn post_text_typing(
    State(state): State<Arc<AppState>>,
    AuthBearer(auth): AuthBearer,
) -> impl IntoResponse {
    tracing::debug!(auth = ?auth, "POST /api/in/text/typing");
    state.floor.note_typing(tokio::time::Instant::now()).await;
    StatusCode::ACCEPTED
}

/// `GET /api/out/text` — the conversation: the current window whole, then every
/// message as it is appended.
pub async fn get_out_text(
    State(state): State<Arc<AppState>>,
    AuthBearer(auth): AuthBearer,
) -> Response {
    tracing::info!(auth = ?auth, "GET /api/out/text conversation stream opened");

    // Opening this stream means a surface just attached: warm up so the process +
    // session + upstream cache are hot before the first utterance.
    state.warm();

    // Hold an attachment guard for as long as the body stream exists. This counts
    // toward one question only — whether a speaker is attached, which decides
    // whether speech is synthesized. Nothing infers from it whether anyone is
    // reading; see `docs/arch/host.md#attachment`.
    let attached = state
        .attachments
        .connect(crate::body::attachments::OutChannel::Text);
    let (opening, rx) = state.transcript.subscribe();

    // `Some(frame)` is pending output; after it, the stream pulls from `rx`. A
    // lagged receiver resyncs with a fresh whole window rather than skipping
    // messages — the list is the contract, and a gap in it would be a lie.
    let transcript = state.transcript.clone();
    let stream = unfold(
        (rx, Some(opening), attached, transcript),
        |(mut rx, pending, attached, transcript)| async move {
            let frame = match pending {
                Some(frame) => frame,
                None => match rx.recv().await {
                    Ok(frame) => frame,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(missed = n, "text subscriber lagged; resyncing");
                        let (resync, _) = transcript.subscribe();
                        resync
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                },
            };
            let mut line = serde_json::to_vec(&frame).expect("a frame is serializable");
            line.push(b'\n');
            Some((
                Ok::<Bytes, std::convert::Infallible>(Bytes::from(line)),
                (rx, None, attached, transcript),
            ))
        },
    );

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-ndjson; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(stream))
        .unwrap()
}

/// `GET /api/messages?before=<id>&limit=<n>` — older messages, for scrollback.
///
/// Read from the journal through the same mapping the boot seed uses, so what you
/// scroll into is the same conversation you were already looking at rather than a
/// second, differently-shaped view of the log.
///
/// `before` is a message id, which is a journal id. It is **not** a cursor the
/// backend remembers: it is where *this one request* starts, sent by a window that
/// already has the newer messages in hand and is asking for what precedes them.
pub async fn get_messages(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ScrollbackParams>,
    AuthBearer(auth): AuthBearer,
) -> Response {
    let limit = params.limit.unwrap_or(SCROLLBACK_DEFAULT).min(SCROLLBACK_MAX);
    let before = params
        .before
        .or_else(|| state.transcript.oldest_id())
        .unwrap_or_default();
    tracing::info!(auth = ?auth, before = %before, limit, "GET /api/messages");

    let since = journal::uuidv7_ts(&before)
        .map(|ts| ts - chrono::Duration::days(SCROLLBACK_DAYS))
        .unwrap_or_else(|| DateTime::from_timestamp(0, 0).expect("unix epoch is valid"));
    let entries = match state.memory.journal.recent(since, JOURNAL_SCAN_MAX).await {
        Ok(entries) => entries,
        Err(err) => {
            tracing::error!(error = %format!("{err:#}"), "scrollback read failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "journal read failed").into_response();
        }
    };

    let mut messages = transcript::from_journal(entries);
    messages.retain(|m| m.id.as_str() < before.as_str());
    if messages.len() > limit {
        let drop = messages.len() - limit;
        messages.drain(0..drop);
    }
    axum::Json(messages).into_response()
}

/// How far back one scrollback request reaches, in days of journal, before it
/// reports what it found. A page of chat is minutes of conversation on an active
/// day and months on a quiet one; this bounds the scan without bounding the
/// conversation, since the next request starts from whatever came back.
const SCROLLBACK_DAYS: i64 = 30;
const SCROLLBACK_DEFAULT: usize = 50;
const SCROLLBACK_MAX: usize = 200;
/// Journal lines to consider per scrollback request. Generous because most of what
/// it reads is not conversation (views, clock wakes, recognition) and is filtered out.
const JOURNAL_SCAN_MAX: usize = 5000;

#[derive(serde::Deserialize)]
pub struct ScrollbackParams {
    /// The oldest message the caller already has. Defaults to the oldest in the
    /// live window.
    before: Option<String>,
    limit: Option<usize>,
}

/// `GET /api/in/text` — observe typed inputs live (NDJSON).
pub async fn get_in_text(
    State(state): State<Arc<AppState>>,
    AuthBearer(auth): AuthBearer,
) -> Response {
    tracing::info!(auth = ?auth, "GET /api/in/text observe opened");
    observe::stream_input(state, Channel::Text)
}
