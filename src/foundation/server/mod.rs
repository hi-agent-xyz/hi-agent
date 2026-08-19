//! HTTP front — axum router and shared application state.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::AtomicU64;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::routing::{get, patch, post, put};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use tokio::sync::{broadcast, mpsc};
use tower_http::compression::{CompressionLayer, CompressionLevel};
use tower_http::trace::TraceLayer;

use crate::mind::memory::Memory;
use crate::foundation::codex::WireTap;
use crate::foundation::observatory::Observatory;
use crate::body::reaction::{Floor, OutboundSignal, ToolRegistry};
use crate::types::{Channel, Signal, ViewEnvelope};

pub mod account;
pub mod activity;
pub mod audio;
pub mod binder;
pub mod channels;
pub mod drive;
pub mod duty;
pub mod facets;
pub mod files;
pub mod generated;
pub mod handle;
pub mod headers;
pub mod mcp;
pub mod observe;
pub mod people;
pub mod reflex;
pub mod sessions;
pub mod settings;
pub mod skills;
pub mod stage;
pub mod stats;
pub mod stubs;
pub mod surfaces;
pub mod tasks;
pub mod text;
pub mod transcript;
pub mod tools;
pub mod view;
pub mod view_bus;
pub mod view_shots;
pub mod vision;
pub mod wire;
pub mod workers;

pub use transcript::{Attachment, Frame, Message, Role, Transcript};
pub use view_bus::ViewBus;

/// Outbound synthesized-audio event. One turn's speech is a continuous stream:
/// a `Start` (carrying the mime so GET /audio can set `Content-Type` before the
/// first byte), then a run of `Frame`s as the brain synthesizes them, then an
/// `End`. The GET /audio handler turns one such run into one chunked HTTP
/// response — the client just appends bytes and plays, no per-clip reassembly.
///
/// `turn` is the monotonic cognition turn, used to keep a handler's response
/// bound to a single turn so frames from a later turn never bleed into an
/// earlier response.
#[derive(Debug, Clone)]
pub enum AudioEvent {
    Start { turn: u64, mime: String },
    Frame { turn: u64, bytes: Bytes },
    End { turn: u64 },
}

impl AudioEvent {
    /// The cognition turn this event belongs to.
    pub fn turn(&self) -> u64 {
        match self {
            AudioEvent::Start { turn, .. }
            | AudioEvent::Frame { turn, .. }
            | AudioEvent::End { turn, .. } => *turn,
        }
    }
}

/// Inbound audio event — the read side of the audio *input* channel, the mirror
/// of [`AudioEvent`]. "Audio is audio": the bytes the world feeds the agent are
/// observable as bytes, not summarized to a transcript. One source (a mic stream
/// or a posted clip) is a `Start`/`Frame`*/`End` run; `GET /api/in/audio` turns
/// one run into one chunked HTTP response a client can play.
///
/// `turn` is a per-source id (one WS connection or one POST), keeping a
/// listener's response bound to a single source so concurrent uploaders never
/// interleave in one body. `mime` carries the format so a listener
/// can decode — `audio/pcm;rate=16000;channels=1` for the live mic stream, the
/// clip's own type for a posted clip. Like the other channel broadcasts this is
/// lossy presence with no replay; the transcript the agent actually consumes
/// rides the *text* channel.
#[derive(Debug, Clone)]
pub enum AudioInEvent {
    Start { turn: u64, mime: String },
    Frame { turn: u64, bytes: Bytes },
    End { turn: u64 },
}

impl AudioInEvent {
    /// The source this event belongs to (one mic stream or one posted clip).
    pub fn turn(&self) -> u64 {
        match self {
            AudioInEvent::Start { turn, .. }
            | AudioInEvent::Frame { turn, .. }
            | AudioInEvent::End { turn, .. } => *turn,
        }
    }
}

/// Outbound agent-authored view event. Carries the view envelope (compiled
/// module URL + op) for the GET /out/view long-poll.
#[derive(Debug, Clone)]
pub struct ViewEvent {
    pub envelope: ViewEnvelope,
    pub ts: DateTime<Utc>,
}

/// Inbound video event — the read side of the vision *input* channel, the visual
/// twin of [`AudioInEvent`]. "Vision is video": the camera streams continuously
/// and the bytes are observable as bytes (the backend never decodes or samples
/// frames — that's a future perception path's job). One camera session is a
/// `Start`/`Frame`*/`End` run; `GET /api/in/vision` turns one run into one chunked
/// HTTP response a client can play.
///
/// `turn` is a per-source id (one WS connection); `mime` is the container/codec
/// (`video/webm;codecs=…`). Like the other channel broadcasts this is lossy
/// presence with no replay — but unlike audio frames a WebM stream is only
/// decodable from its first chunk (the initialization segment), so the active
/// source's init bytes are cached separately (see [`VideoSource`]) to let a late
/// observer join mid-stream.
#[derive(Debug, Clone)]
pub enum VideoInEvent {
    Start { turn: u64, mime: String },
    Frame { turn: u64, bytes: Bytes },
    End { turn: u64 },
}

impl VideoInEvent {
    /// The source this event belongs to (one camera WS connection).
    pub fn turn(&self) -> u64 {
        match self {
            VideoInEvent::Start { turn, .. }
            | VideoInEvent::Frame { turn, .. }
            | VideoInEvent::End { turn, .. } => *turn,
        }
    }
}

/// The currently-active inbound-video source: its turn id, mime, and
/// cached WebM initialization segment (the first chunk). A `GET /api/in/vision`
/// observer that connects after the camera started writes this init before the
/// live frames so MSE can decode the stream; without it the `<video>` stalls.
#[derive(Debug, Clone)]
pub struct VideoSource {
    pub turn: u64,
    pub mime: String,
    pub init: Bytes,
}

/// A snapshot of the in-progress (not-yet-flushed) camera minute, so a
/// tool can grab "what just happened" without waiting for the minute to roll over and
/// flush to disk. Holds the cached init segment plus the media bytes accumulated so
/// far this minute; `init` followed by `buf` is an independently-decodable clip — the
/// same shape every persisted minute file has. Refreshed as chunks arrive and cleared
/// when the camera closes (see [`vision`]).
#[derive(Debug, Clone)]
pub struct PartialMinute {
    pub turn: u64,
    pub mime: String,
    pub init: Bytes,
    pub buf: Bytes,
}

/// One recognized input on the live observer tap `GET /api/in/<channel>`.
///
/// This broadcast feeds reflexes and the channel inspector. It is deliberately
/// lossy presence, not UI state and not a log. The conversation the person reads
/// is owned separately by [`Transcript`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct InputEcho {
    pub channel: Channel,
    pub text: String,
    /// `false` for a rolling partial (e.g. live STT), `true` once the utterance
    /// is settled. Serialized as `final` for the client.
    #[serde(rename = "final")]
    pub is_final: bool,
    pub ts: DateTime<Utc>,
}

/// One spoken/typed reply, echoed to observers — the outbound mirror of
/// [`InputEcho`]. It carries the reply as a live presence signal, the way
/// `InputEcho` carries inbound text, so the channel inspector sees both sides on
/// the same terms. Presence, not a log: broadcast, lossy, no replay.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OutputEcho {
    pub channel: Channel,
    pub text: String,
    /// `false` while a reply is still streaming chunks, `true` at end-of-utterance.
    #[serde(rename = "final")]
    pub is_final: bool,
    pub ts: DateTime<Utc>,
}

/// Face-presence state for the presence lane (`POST /api/in/vision/presence`).
///
/// The presence still-loop posts a low-res camera frame every couple of seconds;
/// the handler recognizes faces locally and edge-triggers a perception signal only
/// when *who is present* changes. `last_seen` times each identity label's most
/// recent sighting (so a momentarily-missed detection doesn't flap a leave), and
/// `announced` is the set we currently treat as on-camera (so each appear/leave
/// fires exactly once per transition). See [`vision::post_presence`].
#[derive(Default)]
pub struct FacePresence {
    pub last_seen: HashMap<String, DateTime<Utc>>,
    pub announced: HashSet<String>,
}

/// Shared state passed to every handler via `axum::extract::State`.
pub struct AppState {
    /// Inbound signals from every channel POST. The reaction consumes these.
    pub inbound: mpsc::Sender<Signal>,

    /// Traffic a listener hands in for a standing duty (`POST /api/in/duty/{key}`).
    ///
    /// A separate seam from `inbound` because it is a separate boundary: `inbound`
    /// carries what the person did, into the conversation; this carries what a machine
    /// received, to the working session holding that duty. Folding them would put
    /// machine traffic in the transcript, which is the failure `host.md` names when it
    /// rules out `/api/in/text` as a wake channel.
    pub duties: mpsc::Sender<crate::body::reaction::DutyDelivery>,

    /// Warm-up requests. A presence GET (`GET /api/out/*`, the long-polls a client
    /// opens when it attaches) asks here so the reaction stands itself up —
    /// spawning the subprocess and opening the agent session — before the first
    /// utterance lands, keeping that cold-start off the first reply's critical
    /// path. Bounded and best-effort: a full channel only means warm-ups are
    /// already queued, so a dropped request costs at most the cold-start it would
    /// have saved.
    pub warm: mpsc::Sender<()>,

    /// The conversation: one backend-owned, append-only message list. GET
    /// /api/out/text sends the current window whole and then every later message
    /// as it is appended. There is no reader identity, cursor, acknowledgement or
    /// read receipt — see [`transcript`].
    pub transcript: Transcript,

    /// Outbound audio broadcast. GET /api/out/audio subscribers receive from
    /// this; the reaction produces TTS clips here when a TTS provider is set.
    pub audio_out: broadcast::Sender<AudioEvent>,

    /// Retained appearance state. GET /api/out/view serves whole-state
    /// snapshots from this; the binder folds each reaction-emitted envelope in.
    /// Unlike a broadcast, a view shown while no client is connected is retained —
    /// refresh, a second device, or a restart all converge on the same screen.
    pub views: ViewBus,

    /// Outbound view-event broadcast — the non-draining debug tap of the view
    /// channel, observed by the channel inspector. Delivery rides `views`.
    pub view_out: broadcast::Sender<ViewEvent>,

    /// Inbound audio broadcast — the read side of the audio *input* channel.
    /// `POST /api/in/audio` and `WS /api/in/audio/stream` publish the raw audio
    /// bytes here; `GET /api/in/audio` subscribers play them. Written directly by
    /// the ingest handlers, not the binder — it is input data, not the reaction's
    /// voice. The transcript the agent consumes rides the *text* channel instead.
    pub audio_in: broadcast::Sender<AudioInEvent>,

    /// Hands each inbound-audio source (one WS connection or one posted clip) a
    /// distinct `turn` id, so concurrent uploaders never interleave in one
    /// `GET /api/in/audio` listener response.
    pub audio_in_turn: AtomicU64,

    /// Inbound video broadcast — the read side of the vision *input* channel.
    /// `WS /api/in/vision/stream` publishes the camera's WebM chunks here;
    /// `GET /api/in/vision` subscribers play them. Written directly by the ingest
    /// handler, not the binder — it is input data, not the reaction's voice. The
    /// backend never decodes the video; perceiving frames is a future job.
    pub video_in: broadcast::Sender<VideoInEvent>,

    /// Hands each inbound-video source (one camera WS connection) a distinct
    /// `turn` id, keeping a `GET /api/in/vision` observer bound to one camera.
    pub video_in_turn: AtomicU64,

    /// The active inbound-video source, holding its cached WebM init segment so an
    /// observer can join the live stream mid-flight (see [`VideoSource`]). Set on a
    /// camera's first chunk, cleared on close.
    pub video_in_live: Mutex<Option<VideoSource>>,

    /// The in-progress (not-yet-flushed) camera minute — a freshness window for the
    /// agent's `watch` tool, which otherwise sees only persisted minute files up to
    /// ~60s stale. Refreshed as chunks accumulate, cleared on camera close. See
    /// [`PartialMinute`].
    pub video_in_partial: Mutex<Option<PartialMinute>>,

    /// Inbound observer broadcast. Reflexes and GET /api/in/<channel> inspectors
    /// receive recognized inputs from this live, lossy tap.
    pub input_echo: broadcast::Sender<InputEcho>,

    /// Outbound text echo broadcast — the live inspector mirror of the agent's
    /// worded reply. The message itself goes to `transcript`; this broadcast
    /// remains deliberately lossy because it is only observability.
    pub output_echo: broadcast::Sender<OutputEcho>,

    /// Memory substrate — journal. Cloneable handle.
    pub memory: Memory,

    /// Structured visibility into the agent session lifecycle. Served read-only by
    /// the `/api/sessions` endpoints.
    pub observatory: Observatory,

    /// Raw JSON-RPC wire tap — every wire frame, business-logic agnostic. Served
    /// read-only by `GET /api/wire/frames/events` for the raw session inspector.
    pub wire_tap: WireTap,

    /// The model/private boundary. The Responses proxy projects every external
    /// model request; brokered effectors resolve opaque secret references.
    pub privacy: crate::foundation::privacy::PrivacyBoundary,

    /// Where blob media lives. POST /api/in/audio and POST /api/in/vision write
    /// incoming bytes here before journaling the reference.
    pub data_dir: PathBuf,

    /// Owner xiaoyuanzhu sign-in (`Some` only when OIDC is configured). Not a gate.
    /// Retained for a future `sub`-tier account link; no request handler reads it
    /// today — the credential/account routes that did were removed when the config
    /// surface moved to the native tray. `None` ⇒ sign-in unavailable (free tier).
    pub auth: Option<Arc<crate::foundation::auth::AuthState>>,

    /// The tool sink. The `/mcp` handler routes a tool call to the reaction loop
    /// through it; the reaction registers the sink as it stands the loop up. See
    /// [`crate::body::reaction::ToolRegistry`].
    pub tool_registry: ToolRegistry,

    /// Floor state, shared with the reaction. The STT relay reports recognized
    /// speech here ([`crate::body::reaction::Floor::note_speech`]), which is both
    /// what marks the floor theirs and what a barge-in is inferred from; nothing
    /// else on the HTTP side touches it — there is no interrupt endpoint and no
    /// "they stopped" endpoint, because neither would be a thing a client knows.
    pub floor: Floor,

    /// Live-subscriber counts, shared with the reaction. Out-channel handlers hold
    /// a [`crate::body::attachments::Guard`] per connection. One question is asked
    /// of it — is a speaker attached, so speech is worth synthesizing — and nothing
    /// infers from it whether anyone is reading; see
    /// [`crate::body::attachments`].
    pub attachments: crate::body::attachments::Attachments,

    /// Phone-upload grants for the file-upload carrier. A QR encodes `/up/<token>`;
    /// holding a live token is what authorizes the upload. Short TTL, pruned on
    /// access, in-memory (a restart drops outstanding links). See [`files`].
    pub handoffs: Mutex<HashMap<String, files::Handoff>>,

    /// Face-presence state for the presence lane. The presence handler reads and
    /// updates this to decide when an appear/leave event is worth a signal. See
    /// [`FacePresence`] and [`vision::post_presence`].
    pub face_presence: Mutex<FacePresence>,

    /// Who may reach this core: the surface credentials, their exchanged sessions,
    /// and the outstanding pairing codes. Read by the gate on every off-box
    /// request and by the four routes in [`surfaces`]. Not to be confused with
    /// `auth` above, which links an account and gates nothing. See
    /// [`crate::foundation::surfaces`].
    pub surfaces: Arc<crate::foundation::surfaces::Surfaces>,
}

impl AppState {
    /// Append one message the person sent — a typed line, a recognized utterance,
    /// or a handed file — and publish it to the live observer tap.
    ///
    /// `id` and `ts` are the ones the caller already journaled under, so the
    /// message in the conversation and the entry in the log are the same thing
    /// with the same key, and a reload rebuilds the list unchanged. `sender` is the
    /// same one for the same reason: it is the [`crate::types::Sender`] the caller
    /// just wrote to the journal, handed over rather than decided again here, so the
    /// live conversation and the one a restart rebuilds cannot disagree about who
    /// was talking.
    pub fn note_message(
        &self,
        channel: Channel,
        id: String,
        ts: DateTime<Utc>,
        text: &str,
        attachment: Option<Attachment>,
        sender: Option<crate::types::Sender>,
    ) {
        self.transcript.append(Message {
            id,
            ts,
            role: Role::User,
            text: transcript::display_text(text),
            attachment,
            sender,
        });
        let _ = self.input_echo.send(InputEcho {
            channel,
            text: text.to_owned(),
            is_final: true,
            ts,
        });
    }

    /// A rolling recognition partial: a preview of a message, not a message.
    pub fn note_interim(&self, channel: Channel, text: &str) {
        self.transcript.note_interim(text);
        let _ = self.input_echo.send(InputEcho {
            channel,
            text: text.to_owned(),
            is_final: false,
            ts: Utc::now(),
        });
    }

    /// Ask the reaction to warm up now — spawn its subprocess and open its agent
    /// session — triggered when a client opens one of the `/api/out/*` long-polls.
    /// Best-effort and non-blocking: a full queue drops the request, leaving the
    /// cold-start to happen on first use as before. Idempotent on the reaction
    /// side, so repeated GETs are harmless.
    pub fn warm(&self) {
        let _ = self.warm.try_send(());
    }
}

/// Max total multipart body for one handed-file request. Generous enough for a
/// batch of photos/scans/PDFs; the rest of the channels keep axum's small default.
const MAX_UPLOAD: usize = 50 * 1024 * 1024;

/// How far back the boot seed reads. The live window is bounded anyway, so this
/// only has to be long enough that a quiet week still opens on a conversation
/// rather than on nothing; older messages are reached by scrolling.
const SEED_DAYS: i64 = 30;
/// Journal lines the seed will consider. Most of them are not conversation and are
/// filtered out, so this is generous relative to the window it fills.
const SEED_SCAN_MAX: usize = 5000;

pub fn build(
    memory: Memory,
    data_dir: PathBuf,
    observatory: Observatory,
    wire_tap: WireTap,
    privacy: crate::foundation::privacy::PrivacyBoundary,
    tool_registry: ToolRegistry,
    floor: Floor,
    attachments: crate::body::attachments::Attachments,
    auth: Option<Arc<crate::foundation::auth::AuthState>>,
) -> (Router, ServerSeams) {
    // Who may reach this core. Built here rather than handed in: it is state the
    // HTTP front owns end to end — the gate below is the only thing that reads it
    // on the hot path, and `AppState` is how the four routes reach it.
    let surface_reach = Arc::new(crate::foundation::surfaces::Surfaces::new(data_dir.clone()));
    let (inbound_tx, inbound_rx) = mpsc::channel::<Signal>(1024);
    // Duty deliveries. Bounded like every other seam, and dropped rather than awaited
    // when full — the door never holds a listener open (see `server::duty`).
    let (duty_tx, duty_rx) =
        mpsc::channel::<crate::body::reaction::DutyDelivery>(1024);
    // Warm-up requests: a presence GET asks the reaction to stand itself up ahead
    // of the first utterance (see `AppState::warm`).
    let (warm_tx, warm_rx) = mpsc::channel::<()>(1024);
    let transcript = Transcript::new();
    // Fill the conversation from the journal, off the boot path. A restart shows
    // what was already being said instead of an empty room — which is the whole
    // reason the list is durable, and the failure the current-appearance state it
    // replaced accepted by design.
    {
        let transcript = transcript.clone();
        let memory = memory.clone();
        tokio::spawn(async move {
            let since = Utc::now() - chrono::Duration::days(SEED_DAYS);
            match memory.journal.recent(since, SEED_SCAN_MAX).await {
                Ok(entries) => {
                    let messages = transcript::from_journal(entries);
                    tracing::info!(messages = messages.len(), "conversation seeded from the journal");
                    transcript.seed(messages);
                }
                Err(err) => {
                    // The conversation starts empty and fills as it is spoken. A
                    // readable log is not worth failing a boot over.
                    tracing::error!(error = %format!("{err:#}"), "conversation seed failed; starting empty");
                }
            }
        });
    }
    let (audio_tx, _) = broadcast::channel::<AudioEvent>(64);
    // Inbound audio: small, frequent PCM frames, so a larger ring than the others.
    let (audio_in_tx, _) = broadcast::channel::<AudioInEvent>(256);
    let (view_tx, _) = broadcast::channel::<ViewEvent>(64);
    // Retained appearance state, reloaded from disk so the screen survives a
    // restart (see `ViewBus`).
    let view_bus = ViewBus::load(&data_dir);
    // Inbound video: continuous WebM chunks, so a larger ring like inbound audio.
    let (video_in_tx, _) = broadcast::channel::<VideoInEvent>(256);
    // Input echo: live broadcast, lossy ring, no replay (see `InputEcho`).
    let (input_echo_tx, _) = broadcast::channel::<InputEcho>(64);
    // Output text echo: the binder's non-draining mirror (see `OutputEcho`).
    let (output_echo_tx, _) = broadcast::channel::<OutputEcho>(64);

    // The reaction's single transport-free outbound seam. A binder task fans each
    // `OutboundSignal` out to the HTTP-shaped carriers above — folding text and
    // views into appearance state and framing audio spans. The reaction knows
    // none of that.
    let (out_tx, out_rx) = mpsc::channel::<OutboundSignal>(1024);
    tokio::spawn(binder::bind_outbound(
        out_rx,
        transcript.clone(),
        audio_tx.clone(),
        view_bus.clone(),
        view_tx.clone(),
        output_echo_tx.clone(),
    ));

    let state = Arc::new(AppState {
        inbound: inbound_tx,
        duties: duty_tx,
        warm: warm_tx,
        transcript: transcript.clone(),
        audio_out: audio_tx.clone(),
        audio_in: audio_in_tx.clone(),
        audio_in_turn: AtomicU64::new(0),
        views: view_bus,
        view_out: view_tx.clone(),
        video_in: video_in_tx.clone(),
        video_in_turn: AtomicU64::new(0),
        video_in_live: Mutex::new(None),
        video_in_partial: Mutex::new(None),
        input_echo: input_echo_tx.clone(),
        output_echo: output_echo_tx.clone(),
        memory,
        observatory,
        wire_tap,
        privacy,
        data_dir,
        auth: auth.clone(),
        tool_registry,
        floor,
        attachments,
        handoffs: Mutex::new(HashMap::new()),
        face_presence: Mutex::new(FacePresence::default()),
        surfaces: surface_reach.clone(),
    });

    // Channels are namespaced by boundary: `/api/in/*` is the world→agent side
    // (perception), `/api/out/*` is the agent→world side (expression). GETs expose
    // live observer taps; `/api/out/text` and `/api/out/view` additionally serve
    // the backend-owned appearance state. `/api/sessions` is observability, not
    // a channel.
    let router = Router::new()
        // Who may reach this core. `POST /api/session` is open by definition —
        // it is how anything stops being unauthorized; the other three are gated
        // like every other route, so pairing a phone is asked for from a surface
        // that already has access. `/healthz` answers before anything is paired.
        .route("/healthz", get(surfaces::get_healthz))
        .route("/api/session", post(surfaces::post_session))
        .route("/api/pair", post(surfaces::post_pair))
        // This core's address in the community: what it is called, and claiming
        // or renaming it. The id underneath never changes.
        .route(
            "/api/handle",
            get(handle::get_handle)
                .post(handle::post_handle)
                .delete(handle::delete_handle),
        )
        .route("/api/surfaces", get(surfaces::get_surfaces))
        .route("/api/surfaces/{id}", axum::routing::delete(surfaces::delete_surface))
        .route("/api/in/text", post(text::post_text).get(text::get_in_text))
        // Not an input channel: a contentless liveness ping from whatever holds an
        // unsent draft. It reaches the floor and stops there.
        .route("/api/in/text/typing", post(text::post_text_typing))
        .route("/api/out/text", get(text::get_out_text))
        .route("/api/messages", get(text::get_messages))
        .route("/api/media/{*ref}", get(files::get_media))
        .route("/api/in/audio", post(audio::post_audio).get(audio::get_in_audio))
        .route("/api/in/audio/stream", get(audio::get_audio_stream))
        .route("/api/out/audio", get(audio::get_out_audio))
        // The view channel — the retained appearance, served as versioned
        // whole-state snapshots (long-poll on `?since=`).
        .route("/api/out/view", get(view::get_out_view).delete(view::clear_out_view))
        // The inbound half of the view channel: the person went to a view. It is the
        // one thing about the screen that does not come from the agent, and it is read
        // into the next turn rather than driving one — see `view::post_in_view`.
        .route("/api/in/view", post(view::post_in_view))
        // The views a person can go to by name, and the compile that lets one window
        // mount one without taking the stage away from what the agent raised.
        .route("/api/views", get(view::list_views))
        .route("/api/views/open", post(view::open_view))
        .route("/api/views/bookmarks", post(view::bookmark_view))
        // Vision is an input channel that is also observable: the camera streams
        // WebM over the WS, GET plays the live video; POST persists a still frame.
        .route("/api/in/vision", post(vision::post_vision).get(vision::get_vision))
        .route("/api/in/vision/stream", get(vision::get_vision_stream))
        // The presence lane: a cheap local face reflex. The client posts a low-res
        // camera still every couple of seconds; the handler recognizes faces on the
        // local models and emits a perception signal only when who is present
        // changes — real-time "who's here", no remote call. A no-op without the
        // face capability.
        .route("/api/in/vision/presence", post(vision::post_presence))
        // The attention lane: the web face reports its own window coming forward
        // (visibility/focus) — the "they're checking on you" signal for presence.
        // The stage lane: the desktop window reports the frame it is showing, so
        // `review_view` renders a view at the size the person actually has rather
        // than at a constant matching no window. Not `/api/in/*` — a frame size is
        // a rendering parameter, not something the agent perceives.
        .route("/api/stage", post(stage::post_stage))
        // The file channel — handing the agent a file (handed artifact, not a
        // sense). Drag-drop posts to /api/in/file; the phone handoff mints a
        // token (/api/handoff), serves an uploader at /up/<token>, receives at
        // /api/up/<token>, and renders the QR via /api/qr. Uploads get a generous
        // body limit; everything else keeps axum's small default.
        .route("/api/in/file", post(files::post_file).layer(DefaultBodyLimit::max(MAX_UPLOAD)))
        .route("/api/handoff", post(files::post_handoff))
        .route("/up/{token}", get(files::get_up_page))
        .route("/api/up/{token}", post(files::post_up).layer(DefaultBodyLimit::max(MAX_UPLOAD)))
        .route("/api/qr", get(files::get_qr))
        // A standing duty's own inbound door: the agent-provisioned listener keeping a
        // `serving` task alive says something arrived, and the working session holding
        // that duty picks it up. Not a sense, and not the conversation's — see
        // `server::duty`.
        .route("/api/in/duty/{key}", post(duty::post_duty))
        .route("/api/in/touch", post(stubs::post_touch))
        .route("/api/in/smell", post(stubs::post_smell))
        .route("/api/in/taste", post(stubs::post_taste))
        .route("/api/sessions", get(sessions::get_sessions))
        .route("/api/sessions/events", get(sessions::get_sessions_events))
        // The raw wire feed — every JSON-RPC frame, business-logic agnostic.
        // Backs the raw session inspector at `/inspect/sessions`.
        .route("/api/wire/frames/events", get(wire::get_wire_frames_events))
        // Internal external-model boundary. Codex points its Responses provider
        // here and authenticates with a per-boot token; the handler projects the
        // complete serialized request before forwarding it upstream.
        .route(
            "/internal/model/v1/responses",
            post(crate::foundation::privacy::proxy::post_responses),
        )
        // The MCP tool endpoint a session's `mcp_servers` attach connects to. The
        // mind drives output and side-effects by calling tools here; routing is by
        // the X-HI-Role header the attach carries.
        .route("/mcp", post(mcp::post_mcp).get(mcp::get_mcp))
        // Fire a taught quick-action reflex — recognize the current field via the
        // accessibility tree and type the stored value, no model in the loop. The
        // v1 trigger (a later hotkey/gesture would call the same path).
        .route("/api/reflex/invoke", post(reflex::post_invoke))
        // The "认识的人" review surface: list stored people + their clips, serve one
        // crop/clip, and correct identity — name/merge, eject a clip, auto-regroup.
        .route("/api/people", get(people::get_people))
        .route("/api/people/name", post(people::post_name))
        .route("/api/people/eject", post(people::post_eject))
        .route("/api/people/split/preview", post(people::post_split_preview))
        .route("/api/people/split/apply", post(people::post_split_apply))
        .route("/api/people/{subject}/{modality}/{stem}", get(people::get_clip))
        // …and the one crop that stands for a person, which the conversation reads
        // to put a face beside their messages.
        .route("/api/people/{subject}/avatar", get(people::get_avatar))
        // The rest of the review surfaces, same shape as "认识的人": show what the agent
        // has accumulated, and give the person the verb that ends or corrects it. Each is
        // read plus exactly the writes it can honestly honour — no stop verb for workers,
        // no edits at all for tools or drive, because neither is accumulated state a
        // person can fix from a screen.
        .route("/api/tasks", get(tasks::get_tasks))
        .route("/api/tasks/{subject}", patch(tasks::patch_task))
        .route("/api/skills", get(skills::get_skills))
        .route("/api/skills/{*path}", get(skills::get_skill).delete(skills::delete_skill))
        .route("/api/facets", get(facets::get_facets))
        .route("/api/facets/{dimension}/{subject}", get(facets::get_facet).put(facets::put_facet))
        .route("/api/episodes", get(facets::get_episodes))
        .route("/api/workers", get(workers::get_workers))
        // Before `/{id}`: axum matches a literal segment ahead of a capture, but keeping
        // them adjacent and in this order stops a later reader from reading `ended` as an
        // id-shaped route.
        .route("/api/workers/ended", get(workers::get_ended))
        .route("/api/workers/{id}", get(workers::get_worker))
        .route("/api/workers/{id}/frames", get(workers::get_frames))
        .route("/api/workers/{id}/messages", get(workers::get_messages))
        .route("/api/activity", get(activity::get_activity))
        .route("/api/stats", get(stats::get_stats))
        .route("/api/tools", get(tools::get_tools))
        .route("/api/drive", get(drive::get_drive))
        .route("/api/drive/file/{*path}", get(drive::get_drive_file))
        // The device account's energy standing + a signed-in upgrade link. Public,
        // like every route here; the out-of-energy card calls both.
        .route("/api/account/energy", get(account::get_energy))
        .route("/api/account/subscribe", get(account::get_subscribe))
        // The config/energy/mode boundary the Settings UI (native or web) drives as a
        // thin client of the engine — loopback-gated + secret-safe. Reintroduces the
        // HTTP config surface the tray refactor removed. See settings.rs and
        // docs/core-shell-config-api.md.
        .route("/api/settings", get(settings::get_settings))
        .route("/api/settings/appearance", put(settings::put_appearance))
        .route("/api/settings/mode", put(settings::put_mode))
        .route("/api/settings/relay", put(settings::put_relay))
        .route("/api/settings/credentials/{feature}", put(settings::put_feature))
        .route("/api/account/energy/refresh", post(settings::post_energy_refresh))
        // Web→device account link: `start` opens the browser to the site with a
        // loopback callback + CSRF nonce; the site hands a device-ticket back to
        // `callback`, which redeems it at the broker to adopt the signed-in account.
        // Loopback-only (the callback's peer must be 127.0.0.1). See account.rs.
        .route("/account/link/start", get(account::get_link_start))
        .route("/account/link/callback", get(account::get_link_callback))
        // Every channel, observed live as one merged presence stream — the channel
        // inspector's window onto the whole conversation, in and out.
        .route("/api/channels", get(channels::get_channels))
        // The agent's view workshop on disk (under data_dir) — compiled view modules,
        // images, and build-agent artifacts. Served here, not in the appearance
        // router, because that router is embed-only and stateless.
        //
        // Its own `Router` purely so the compressor can be scoped to it. `views_file`
        // reads a whole file and hands back one buffered body, so it compresses like
        // the embedded assets do — but it is the only route on this router that may
        // be wrapped, which is why it cannot simply be a `.layer` on the parent:
        // every `/api/*` neighbour above is a long-poll or SSE body that a
        // compressor would buffer.
        .merge(
            Router::new()
                .route("/views/{*path}", get(generated::views_file))
                .layer(CompressionLayer::new().quality(CompressionLevel::Precise(6))),
        )
        .with_state(state.clone())
        .merge(crate::appearance::router())
        .fallback(not_found);

    // Mount the owner sign-in routes (`/auth/*`) when OIDC is configured. There is
    // no access gate — every route is public; sign-in is an opt-in action that only
    // links the owner's xiaoyuanzhu account for a `sub`-tier upgrade.
    let router = match auth {
        Some(auth) => crate::foundation::auth::mount(router, auth),
        None => router,
    };

    // The gate, outside every route including the appearance router and the
    // owner sign-in mount: an off-box request is answered only with a credential.
    // Loopback passes untouched, which is why `make dev`, the curl journeys, the
    // popover and the codex subprocesses on `/mcp` are unaffected.
    //
    // It goes *inside* the trace layer so a rejected request is still logged, and
    // *outside* everything else so no route can be reached around it. Which
    // acceptor took the request is marked by the listener (see
    // [`crate::foundation::surfaces::Acceptor`]) — this layer only reads it.
    let router = router.layer(axum::middleware::from_fn_with_state(
        surface_reach.clone(),
        crate::foundation::surfaces::gate,
    ));
    let router = router.layer(TraceLayer::new_for_http());

    let seams = ServerSeams {
        inbound_rx,
        duty_rx,
        warm_rx,
        transcript,
        out_tx,
        state,
    };

    (router, seams)
}

/// What `build` hands back to wire the reaction to the HTTP front. `inbound_rx`
/// is the channel POSTs feed; `warm_rx` carries warm-up requests a presence
/// GET raises; `out_tx` is the reaction's single transport-free outbound seam (the
/// binder spawned in `build` carries it to the wire). The `transcript` is
/// exposed only so integration tests can append messages directly
/// without standing up a reaction. `state` is the shared `AppState` (the same
/// `Arc` the router holds), so
/// a non-HTTP producer — the come-and-see-this gesture — can inject inbound
/// signals through the same path as a channel POST.
pub struct ServerSeams {
    pub inbound_rx: mpsc::Receiver<Signal>,
    pub duty_rx: mpsc::Receiver<crate::body::reaction::DutyDelivery>,
    pub warm_rx: mpsc::Receiver<()>,
    pub transcript: Transcript,
    pub out_tx: mpsc::Sender<OutboundSignal>,
    pub state: Arc<AppState>,
}

async fn not_found() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "not found\n")
}
