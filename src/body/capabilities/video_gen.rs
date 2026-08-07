//! Video-generation capability — two Hugging Face tasks over one provider:
//! [`text_to_video`] (prompt → clip) and [`image_to_video`] (still as first frame
//! → clip).
//!
//! Asynchronous task: submit a request to get a task id back, then [`poll`]
//! until it reaches a terminal [`VideoStatus`]. The split keeps the multi-minute
//! wait honest instead of hiding it behind a single blocking call.
//!
//! The capability is a module of free functions over a process-global,
//! once-initialized config: [`init`] resolves the vendor from the credential
//! store, [`available`] reports whether a provider is configured, and the task
//! functions dispatch to it via [`submit`]. The config never appears in a
//! signature. The provider's key arrives the same way for either task — BYOK from
//! the credential store, or the broker-minted bundle — so a task is wired once and
//! works under both.
//!
//! **No caller wires this in yet.** The module is built and unit-tested
//! standalone so a later *emission* path can call it as a purely additive
//! change.

use std::sync::OnceLock;

use bytes::Bytes;

use crate::foundation::vendors::doubao_video_gen;

/// A reference image for image-to-video (e.g. the first frame). Either an
/// already-usable URL passed through untouched, or raw bytes the vendor
/// base64-encodes into a `data:` URL at request time.
#[derive(Debug, Clone)]
pub enum ImageRef {
    Url(String),
    Bytes { bytes: Bytes, mime: String },
}

impl ImageRef {
    pub fn url(url: impl Into<String>) -> Self {
        ImageRef::Url(url.into())
    }

    pub fn bytes(bytes: Bytes, mime: impl Into<String>) -> Self {
        ImageRef::Bytes { bytes, mime: mime.into() }
    }
}

/// A request for one short video clip. Only `prompt` is required; the optional
/// `first_frame` turns this into image-to-video, and the remaining knobs map to
/// the vendor's generation parameters when set.
#[derive(Debug, Clone, Default)]
pub struct VideoRequest {
    pub prompt: String,
    /// First-frame reference for image-to-video. `None` → text-to-video.
    pub first_frame: Option<ImageRef>,
    /// e.g. `"480p"`, `"720p"`, `"1080p"`.
    pub resolution: Option<String>,
    /// e.g. `"16:9"`, `"9:16"`, `"1:1"`.
    pub ratio: Option<String>,
    /// Clip length in seconds.
    pub duration: Option<u32>,
    pub watermark: Option<bool>,
    pub seed: Option<i64>,
}

impl VideoRequest {
    /// A bare text-to-video prompt with all knobs left at the vendor default.
    pub fn new(prompt: impl Into<String>) -> Self {
        Self { prompt: prompt.into(), ..Default::default() }
    }
}

/// Where an async video task is in its lifecycle. The non-terminal states
/// (`Queued`, `Running`) mean "poll again later"; the rest are terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoStatus {
    Queued,
    Running,
    Succeeded { video_url: String, last_frame_url: Option<String> },
    Failed { message: String },
    Cancelled,
    Expired,
}

impl VideoStatus {
    /// True once the task has stopped changing — a poll loop exits here.
    pub fn is_terminal(&self) -> bool {
        !matches!(self, VideoStatus::Queued | VideoStatus::Running)
    }
}

/// A video task as last observed: its upstream id plus current [`VideoStatus`].
#[derive(Debug, Clone)]
pub struct VideoTask {
    pub id: String,
    pub status: VideoStatus,
}

enum Backend {
    Disabled,
    Doubao(doubao_video_gen::Config),
}

static BACKEND: OnceLock<Backend> = OnceLock::new();

/// The default wire when the store selects none — the only video-gen impl today.
const DEFAULT_WIRE: &str = "doubao";

/// Resolve the video-gen backend into the process-global config from the credential
/// store. A non-empty `store_key` (BYOK or broker-managed) enables the capability
/// on the configured `wire` (`None` → [`DEFAULT_WIRE`]); no key → disabled. An
/// unknown wire is an error. Adding a vendor is a new `Backend` variant plus a
/// match arm here. Idempotent — the first init wins.
pub fn init(
    store_key: Option<&str>,
    base_url: Option<&str>,
    model: Option<&str>,
    wire: Option<&str>,
) -> anyhow::Result<()> {
    let backend = if store_key.map(|k| !k.trim().is_empty()).unwrap_or(false) {
        match wire.unwrap_or(DEFAULT_WIRE) {
            "doubao" => Backend::Doubao(doubao_video_gen::Config::from_store(store_key, base_url, model)?),
            other => anyhow::bail!("unknown video-gen wire: {other}"),
        }
    } else {
        Backend::Disabled
    };
    let _ = BACKEND.set(backend);
    Ok(())
}

/// Whether a provider is configured.
pub fn available() -> bool {
    matches!(BACKEND.get(), Some(Backend::Doubao(_)))
}

/// Submit a generation request and return the task id to poll. Fast: this only
/// enqueues the work, it does not wait for the clip.
///
/// The shared vendor dispatch behind both generation tasks. Prefer the task-named
/// entry points ([`text_to_video`], [`image_to_video`]): they make the presence or
/// absence of a first frame — the only thing that distinguishes the two tasks —
/// impossible to get silently wrong.
pub async fn submit(req: &VideoRequest) -> anyhow::Result<String> {
    match BACKEND.get() {
        Some(Backend::Doubao(cfg)) => doubao_video_gen::submit(cfg, req).await,
        _ => anyhow::bail!("video generation not configured (set a video key in Settings)"),
    }
}

/// `text-to-video` — a prompt in, a clip out. Returns the task id to [`poll`].
///
/// Rejects a request carrying a `first_frame` rather than quietly performing
/// image-to-video under a text-to-video name: the two are different tasks, priced
/// and prompted differently, and a silent upgrade is the kind of substitution that
/// gets recorded as the thing that was asked for.
pub async fn text_to_video(req: &VideoRequest) -> anyhow::Result<String> {
    if req.first_frame.is_some() {
        anyhow::bail!("text-to-video was given a first frame — call `image_to_video` for that");
    }
    submit(req).await
}

/// `image-to-video` — a still as first frame plus an optional prompt in, a clip
/// out. Returns the task id to [`poll`].
///
/// Requires `first_frame`; without it this is [`text_to_video`] and says so rather
/// than generating something unrelated to the picture the caller meant to animate.
pub async fn image_to_video(req: &VideoRequest) -> anyhow::Result<String> {
    if req.first_frame.is_none() {
        anyhow::bail!("image-to-video needs a `first_frame` — call `text_to_video` without one");
    }
    submit(req).await
}

/// Fetch the current state of a previously-submitted task. Callers poll this
/// until [`VideoStatus::is_terminal`] on their own cadence.
pub async fn poll(task_id: &str) -> anyhow::Result<VideoTask> {
    match BACKEND.get() {
        Some(Backend::Doubao(cfg)) => doubao_video_gen::poll(cfg, task_id).await,
        _ => anyhow::bail!("video generation not configured (set a video key in Settings)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The only thing separating the two generation tasks is whether a first frame
    /// is present, which makes a mixed-up call silently plausible: text-to-video
    /// handed a frame would generate *something*, and it would be recorded as the
    /// animation of a picture nobody animated. Both directions are refused by name.
    #[tokio::test]
    async fn the_two_video_tasks_refuse_each_other_s_request() {
        let mut with_frame = VideoRequest::new("a cat stretching");
        with_frame.first_frame = Some(ImageRef::url("https://example.invalid/cat.png"));

        let err = text_to_video(&with_frame).await.unwrap_err().to_string();
        assert!(err.contains("image_to_video"), "{err}");

        let err = image_to_video(&VideoRequest::new("a cat stretching")).await.unwrap_err().to_string();
        assert!(err.contains("text_to_video"), "{err}");
    }

    /// The guard is the caller's mistake, not the operator's — so it must not be
    /// reported as a missing provider. With no backend configured, a *correct*
    /// request still falls through to the configuration error, and that is the one
    /// the operator can act on.
    #[tokio::test]
    async fn a_correct_request_reports_configuration_not_shape() {
        let err = text_to_video(&VideoRequest::new("a cat")).await.unwrap_err().to_string();
        assert!(err.contains("not configured"), "{err}");
    }
}
