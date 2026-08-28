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
    /// Why nothing is configured, in the words a person could act on.
    Disabled(String),
    Doubao(doubao_vision::Config),
}

static BACKEND: OnceLock<Backend> = OnceLock::new();

/// The default wire when the source names none — the only vision impl today.
const DEFAULT_WIRE: &str = "doubao";

/// One provider offered for this capability, as the credential store or the broker
/// describes it. Its own type, not one shared with the other capabilities: a vendor
/// that happens to back several is configured separately for each.
#[derive(Debug, Clone, Default)]
pub struct ProviderSpec {
    /// Wire id, in the source's own vocabulary. `None`/empty → [`DEFAULT_WIRE`].
    pub wire: Option<String>,
    pub base_url: Option<String>,
    pub api_key: String,
    pub model: Option<String>,
}

/// Resolve the vision backend into the process-global config, from **every provider
/// the source offers, best first**.
///
/// Understanding takes no model argument, so there is nothing for a caller to select
/// on: this holds the first provider whose wire it can actually speak, and the list is
/// what makes a second wire a match arm rather than a refactor.
///
/// A wire we have no impl for is skipped with a warning, never fatal — the broker names
/// wires in its own vocabulary and changes it without asking us, and a menu we half
/// understand must not cost a boot. A provider whose *config* won't build still is
/// fatal: that is a broken setting, not an unfamiliar name. Adding a vendor is a new
/// `Backend` variant plus a match arm here. Idempotent — the first init wins.
pub fn init(providers: Vec<ProviderSpec>) -> anyhow::Result<()> {
    let (chosen, skipped) = select(&providers);
    let backend = match chosen {
        Some(i) => Backend::Doubao(doubao_vision::Config::from_store(
            Some(&providers[i].api_key),
            providers[i].base_url.as_deref(),
            providers[i].model.as_deref(),
        )?),
        None if skipped.is_empty() => Backend::Disabled("set a vision key in Settings".to_string()),
        None => {
            tracing::warn!(wires = ?skipped, "no vision impl for any offered wire; vision is off");
            Backend::Disabled(format!("no impl for the offered wire(s): {}", skipped.join(", ")))
        }
    };
    let _ = BACKEND.set(backend);
    Ok(())
}

/// The provider to use, and the wires passed over on the way to it.
///
/// Split out of [`init`] so the rule can be tested: the backend is a write-once process
/// global, and a test that had to install one could only ever run first, alone.
fn select(providers: &[ProviderSpec]) -> (Option<usize>, Vec<String>) {
    let mut skipped = Vec::new();
    for (i, p) in providers.iter().enumerate() {
        if p.api_key.trim().is_empty() {
            continue;
        }
        match p.wire.as_deref().map(str::trim).filter(|w| !w.is_empty()).unwrap_or(DEFAULT_WIRE) {
            "doubao" => return (Some(i), skipped),
            other => skipped.push(other.to_string()),
        }
    }
    (None, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list is offered best-first, so the first wire we can speak wins — and a wire
    /// we cannot is stepped over rather than taken as the answer. `image-text-to-text`
    /// is exactly the task a gateway is likely to serve over an OpenAI-shaped wire we
    /// have no impl for, beside the doubao one we do.
    #[test]
    fn the_first_speakable_wire_wins_and_the_rest_are_named() {
        let spec = |wire: Option<&str>, key: &str| ProviderSpec {
            wire: wire.map(str::to_owned),
            api_key: key.into(),
            ..Default::default()
        };
        let (chosen, skipped) =
            select(&[spec(Some("openai-responses"), "k"), spec(Some("doubao"), "k")]);
        assert_eq!(chosen, Some(1));
        assert_eq!(skipped, vec!["openai-responses"]);
        assert_eq!(select(&[spec(None, "k")]).0, Some(0), "no wire named → the default");
        assert!(select(&[spec(Some("doubao"), " ")]).1.is_empty(), "no key, no complaint");
    }
}

/// Whether a provider is configured.
pub fn available() -> bool {
    matches!(BACKEND.get(), Some(Backend::Doubao(_)))
}

/// Why the capability is off, for the error text at the point of use.
fn why_off() -> &'static str {
    match BACKEND.get() {
        Some(Backend::Disabled(reason)) => reason.as_str(),
        _ => "set a vision key in Settings",
    }
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
        _ => anyhow::bail!("vision not configured ({})", why_off()),
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
