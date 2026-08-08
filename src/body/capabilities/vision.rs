//! Vision capability — image and video understanding (visual input → text).
//!
//! The returned text is the whole point: cognition here is text-only, so vision
//! is the perception path that lets visual signal enter the same symbol stream
//! as everything else.
//!
//! The capability is a module of free functions over a process-global,
//! once-initialized config: [`init`] resolves the vendor from the config store,
//! [`available`] reports whether a provider is configured, and [`understand`]
//! dispatches to it. The config never appears in a signature.
//!
//! **No caller wires this in yet.** `POST /api/in/vision` still only persists
//! frames. This module is the capability a future, deliberately-triggered
//! perception path will call; wiring it in later is purely additive.

use std::sync::OnceLock;

use bytes::Bytes;

use crate::foundation::vendors::doubao_vision;

/// A piece of visual input to understand. The two variants map to the two
/// content-part kinds the upstream distinguishes (`image_url` vs `video_url`);
/// each carries its bytes inline or points at a URL via [`MediaSource`].
#[derive(Debug, Clone)]
pub enum VisualMedia {
    Image(MediaSource),
    Video(MediaSource),
}

impl VisualMedia {
    /// Image from raw bytes + IANA mime (e.g. `image/jpeg`). Encoded to a base64
    /// `data:` URL by the vendor — the common case for a captured frame.
    pub fn image_bytes(bytes: Bytes, mime: impl Into<String>) -> Self {
        VisualMedia::Image(MediaSource::Bytes { bytes, mime: mime.into() })
    }

    /// Image referenced by a remote URL or a pre-built `data:` URL.
    pub fn image_url(url: impl Into<String>) -> Self {
        VisualMedia::Image(MediaSource::Url(url.into()))
    }

    /// Video from raw bytes + IANA mime (e.g. `video/mp4`). Encoded to a base64
    /// `data:` URL by the vendor. Large clips are better passed as a URL.
    pub fn video_bytes(bytes: Bytes, mime: impl Into<String>) -> Self {
        VisualMedia::Video(MediaSource::Bytes { bytes, mime: mime.into() })
    }

    /// Video referenced by a remote URL or a pre-built `data:` URL.
    pub fn video_url(url: impl Into<String>) -> Self {
        VisualMedia::Video(MediaSource::Url(url.into()))
    }
}

/// Where a piece of media's bytes come from: an already-usable URL (remote or
/// `data:`) passed through untouched, or raw bytes the vendor base64-encodes
/// into a `data:` URL at request time.
#[derive(Debug, Clone)]
pub enum MediaSource {
    Url(String),
    Bytes { bytes: Bytes, mime: String },
}

enum Backend {
    Disabled,
    Doubao(doubao_vision::Config),
}

static BACKEND: OnceLock<Backend> = OnceLock::new();

/// The default wire when the store selects none — the only vision impl today.
const DEFAULT_WIRE: &str = "doubao";

/// Resolve the vision backend into the process-global config from the credential
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
            "doubao" => Backend::Doubao(doubao_vision::Config::from_store(store_key, base_url, model)?),
            other => anyhow::bail!("unknown vision wire: {other}"),
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

/// Understand `media` under the instruction `prompt` (e.g. "Describe what you
/// see") and return the model's natural-language answer.
///
/// The shared vendor dispatch behind both understanding tasks. Prefer the
/// task-named entry points ([`image_text_to_text`], [`video_text_to_text`]) when
/// you hold bytes; reach for this directly only when the input is a URL.
pub async fn understand(media: VisualMedia, prompt: &str) -> anyhow::Result<String> {
    match BACKEND.get() {
        Some(Backend::Doubao(cfg)) => doubao_vision::understand(cfg, media, prompt).await,
        _ => anyhow::bail!("vision not configured (set a vision key in Settings)"),
    }
}

/// `image-text-to-text` — an image plus an instruction in, text out.
///
/// One name for one task, all the way down: the MCP tool, this function, and the
/// vendor's model card all say `image-text-to-text`. Naming the entry point after
/// the task (rather than letting one `understand` serve two of them) is what stops
/// a caller from picking the wrong [`VisualMedia`] variant — the image/video choice
/// is made by which function you call, not by an argument you can get wrong.
pub async fn image_text_to_text(
    bytes: Bytes,
    mime: impl Into<String>,
    prompt: &str,
) -> anyhow::Result<String> {
    understand(VisualMedia::image_bytes(bytes, mime), prompt).await
}

/// `video-text-to-text` — a clip plus an instruction in, text out.
///
/// Always a vendor call: no model reached through the agent wire takes video, so
/// unlike [`image_text_to_text`] this one has no native path to fall back to.
pub async fn video_text_to_text(
    bytes: Bytes,
    mime: impl Into<String>,
    prompt: &str,
) -> anyhow::Result<String> {
    understand(VisualMedia::video_bytes(bytes, mime), prompt).await
}
