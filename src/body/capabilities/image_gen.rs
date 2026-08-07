//! Image-generation capability — two Hugging Face tasks over one provider:
//! [`text_to_image`] (prompt → still) and [`image_to_image`] (still + instruction
//! → still).
//!
//! Synchronous: one request, one response with the picture(s).
//!
//! The capability is a module of free functions over a process-global,
//! once-initialized config: [`init`] resolves the vendor from the config store,
//! [`available`] reports whether a provider is configured, and the task functions
//! dispatch to it. The config never appears in a signature. The provider's key
//! arrives the same way for either task and either source — BYOK from the
//! credential store, or the broker-minted bundle — so a task is wired once and
//! works under both.
//!
//! **No caller wires this in yet.** The module is built and unit-tested
//! standalone so a later *emission* path can call it as a purely additive
//! change.
//!
//! [`image_to_image`] is declared and unimplemented: the shape is settled here so
//! that adding it is one vendor arm, not an interface decision made under time
//! pressure at the call site.

use std::sync::OnceLock;

use bytes::Bytes;

use crate::foundation::vendors::doubao_image_gen;

/// How the vendor should return a generated image: a hosted `url` (the default;
/// cheaper on the wire but expires upstream, so a caller persists it promptly)
/// or inline base64 (`b64_json`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ImageFormat {
    #[default]
    Url,
    B64Json,
}

/// A request for one or more still images. Only `prompt` is required; every
/// other field is `None` → "let the model decide", so the vendor's own defaults
/// apply (size, watermark, …) rather than us hard-coding them.
#[derive(Debug, Clone, Default)]
pub struct ImageRequest {
    pub prompt: String,
    /// e.g. `"1024x1024"`, `"2K"`, or `"adaptive"`; vendor-specific.
    pub size: Option<String>,
    pub seed: Option<i64>,
    pub watermark: Option<bool>,
    pub response_format: ImageFormat,
}

impl ImageRequest {
    /// A bare prompt with all knobs left at the vendor default.
    pub fn new(prompt: impl Into<String>) -> Self {
        Self { prompt: prompt.into(), ..Default::default() }
    }
}

/// The still an [`ImageEditRequest`] works from: an already-usable URL passed
/// through untouched, or raw bytes the vendor base64-encodes into a `data:` URL
/// at request time.
///
/// Deliberately its own type rather than a shared one with
/// [`super::video_gen::ImageRef`]: the capabilities are independent by design (see
/// the module docs on `capabilities`), so a vendor that happens to back both is
/// configured — and typed — separately for each. The duplication is the price of
/// being able to change one without touching the other.
#[derive(Debug, Clone)]
pub enum SourceImage {
    Url(String),
    Bytes { bytes: Bytes, mime: String },
}

impl SourceImage {
    pub fn url(url: impl Into<String>) -> Self {
        SourceImage::Url(url.into())
    }

    pub fn bytes(bytes: Bytes, mime: impl Into<String>) -> Self {
        SourceImage::Bytes { bytes, mime: mime.into() }
    }
}

/// A request to edit one existing still. `source` is the image to work from and
/// `prompt` says what to change; the remaining knobs mirror [`ImageRequest`] so a
/// caller moving between the two tasks doesn't meet a different vocabulary.
#[derive(Debug, Clone)]
pub struct ImageEditRequest {
    pub source: SourceImage,
    pub prompt: String,
    /// e.g. `"1024x1024"`, `"2K"`, or `"adaptive"`; vendor-specific.
    pub size: Option<String>,
    pub seed: Option<i64>,
    pub watermark: Option<bool>,
    pub response_format: ImageFormat,
}

impl ImageEditRequest {
    /// A source image and an instruction, with all knobs left at the vendor default.
    pub fn new(source: SourceImage, prompt: impl Into<String>) -> Self {
        Self {
            source,
            prompt: prompt.into(),
            size: None,
            seed: None,
            watermark: None,
            response_format: ImageFormat::default(),
        }
    }
}

/// One generated image. Exactly one of `url` / `b64_json` is populated,
/// matching the requested [`ImageFormat`].
#[derive(Debug, Clone)]
pub struct GeneratedImage {
    pub url: Option<String>,
    pub b64_json: Option<String>,
    pub size: Option<String>,
}

enum Backend {
    Disabled,
    Doubao(doubao_image_gen::Config),
}

static BACKEND: OnceLock<Backend> = OnceLock::new();

/// The default wire when the store selects none — the only image-gen impl today.
const DEFAULT_WIRE: &str = "doubao";

/// Resolve the image-gen backend into the process-global config from the credential
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
            "doubao" => Backend::Doubao(doubao_image_gen::Config::from_store(store_key, base_url, model)?),
            other => anyhow::bail!("unknown image-gen wire: {other}"),
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

/// `text-to-image` — generate image(s) for `req` and return them. Synchronous:
/// the future resolves once the picture(s) are ready.
pub async fn text_to_image(req: &ImageRequest) -> anyhow::Result<Vec<GeneratedImage>> {
    match BACKEND.get() {
        Some(Backend::Doubao(cfg)) => doubao_image_gen::generate(cfg, req).await,
        _ => anyhow::bail!("image generation not configured (set an image key in Settings)"),
    }
}

/// `image-to-image` — edit an existing still under an instruction, returning a new
/// image and leaving the source untouched.
///
/// **Declared, not implemented.** The provider config it would use is already
/// resolved (same key, same wire as [`text_to_image`]); what is missing is the
/// vendor arm. Adding it is a `doubao_image_gen::edit` plus one match arm here —
/// deliberately the same shape as every other task, so the work is filling in a
/// blank rather than designing an interface.
///
/// The error distinguishes "no provider configured" from "this vendor cannot do
/// it yet", because those are different problems with different fixes and a
/// caller that conflates them reports the wrong one.
pub async fn image_to_image(req: &ImageEditRequest) -> anyhow::Result<Vec<GeneratedImage>> {
    let _ = req;
    match BACKEND.get() {
        Some(Backend::Doubao(_)) => {
            anyhow::bail!("image-to-image is not implemented for the doubao wire yet")
        }
        _ => anyhow::bail!("image generation not configured (set an image key in Settings)"),
    }
}
