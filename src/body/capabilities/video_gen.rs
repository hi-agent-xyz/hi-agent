//! Video-generation capability — two Hugging Face tasks over any configured
//! provider: [`text_to_video`] (prompt → clip) and [`image_to_video`] (still as
//! first frame → clip).
//!
//! Asynchronous task: submit a request to get a [`VideoHandle`] back, then [`poll`]
//! it until it reaches a terminal [`VideoStatus`]. The split keeps the multi-minute
//! wait honest instead of hiding it behind a single blocking call.
//!
//! **Same shape as [`super::image_gen`], for the same reason**: the caller names a
//! model in [`VideoParams`] and nobody configures a vendor. See that module for why —
//! this one repeats the registry rather than sharing it, because the capabilities are
//! independent by design (no shared-vendor umbrella, no cross-capability references),
//! and the duplication is what buys the ability to change one without touching the
//! other.

use std::sync::OnceLock;
use std::time::Duration;

use bytes::Bytes;

use crate::foundation::vendors::doubao_video_gen;

/// Submit and poll are quick, but one generous timeout covers the slow path.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

// ── the semantic interface ────────────────────────────────────────────────────

/// The knobs a caller may turn, all optional: `None` means "let the model decide".
#[derive(Debug, Clone, Default)]
pub struct VideoParams {
    /// Which model to generate with. `None` → the default of the first configured
    /// provider. See [`models`] for what is reachable.
    pub model: Option<String>,
    /// e.g. `"480p"`, `"720p"`, `"1080p"`.
    pub resolution: Option<String>,
    /// e.g. `"16:9"`, `"9:16"`, `"1:1"`.
    pub ratio: Option<String>,
    /// Clip length in seconds.
    pub duration: Option<u32>,
    pub seed: Option<i64>,
    pub watermark: Option<bool>,
}

/// A reference image for image-to-video (the first frame). Either an already-usable
/// URL passed through untouched, or raw bytes the vendor base64-encodes into a
/// `data:` URL at request time.
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

/// A submitted task: the upstream id plus **which provider holds it**.
///
/// The provider travels with the id because a bare id is only addressable if there is
/// exactly one place it could live, and the registry exists precisely so there can be
/// more than one. Polling the wrong provider for someone else's id returns "unknown
/// task", which reads as a failed generation.
#[derive(Debug, Clone)]
pub struct VideoHandle {
    provider: usize,
    pub id: String,
}

/// A video task as last observed.
#[derive(Debug, Clone)]
pub struct VideoTask {
    pub id: String,
    pub status: VideoStatus,
}

/// One model a provider serves, with the broker's relative hints.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelInfo {
    pub name: String,
    pub quality: i64,
    pub speed: i64,
    pub price: i64,
}

/// One configured provider, as the credential store or the broker describes it.
#[derive(Debug, Clone, Default)]
pub struct ProviderSpec {
    /// Explicit wire id where the source knows one (`doubao`). `None` → inferred.
    pub wire: Option<String>,
    pub base_url: Option<String>,
    pub api_key: String,
    pub default_model: Option<String>,
    /// Empty means BYOK: no menu, so any model name passes through.
    pub models: Vec<ModelInfo>,
}

// ── the registry ──────────────────────────────────────────────────────────────

enum Adapter {
    Doubao(doubao_video_gen::Config),
}

struct Provider {
    adapter: Adapter,
    default_model: Option<String>,
    models: Vec<ModelInfo>,
}

struct Registry {
    client: reqwest::Client,
    providers: Vec<Provider>,
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

const DOUBAO_WIRE: &str = "doubao";

/// Register every provider that has a key. Idempotent — the first init wins.
pub fn init(specs: Vec<ProviderSpec>) -> anyhow::Result<()> {
    let providers = build_providers(specs);
    let client = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build()?;
    let _ = REGISTRY.set(Registry { client, providers });
    Ok(())
}

/// The classification half of [`init`], separated so it can be tested without racing
/// other tests for a write-once process global.
///
/// There is one adapter, so the wire id is only ever logged: an unfamiliar spelling
/// from the broker must not cost a boot, and with nothing to choose between there is
/// nothing a stricter reading would buy.
fn build_providers(specs: Vec<ProviderSpec>) -> Vec<Provider> {
    let mut providers = Vec::new();
    for spec in specs {
        if spec.api_key.trim().is_empty() {
            continue;
        }
        if let Some(w) = spec.wire.as_deref().map(str::trim).filter(|w| !w.is_empty())
            && !w.to_ascii_lowercase().contains(DOUBAO_WIRE)
            && !w.to_ascii_lowercase().contains("ark")
        {
            tracing::warn!(wire = w, "no video-gen adapter for this wire; using the ark one");
        }
        providers.push(Provider {
            adapter: Adapter::Doubao(doubao_video_gen::Config::new(
                &spec.api_key,
                spec.base_url.as_deref(),
            )),
            default_model: spec.default_model.clone(),
            models: spec.models.clone(),
        });
    }
    providers
}

fn registry() -> anyhow::Result<&'static Registry> {
    match REGISTRY.get() {
        Some(r) if !r.providers.is_empty() => Ok(r),
        _ => anyhow::bail!("video generation not configured (set a video key in Settings)"),
    }
}

/// Whether any provider is configured.
pub fn available() -> bool {
    registry().is_ok()
}

/// Every model reachable right now, best-quality first.
pub fn models() -> Vec<ModelInfo> {
    registry().map(published_models).unwrap_or_default()
}

fn published_models(reg: &Registry) -> Vec<ModelInfo> {
    let mut all: Vec<ModelInfo> =
        reg.providers.iter().flat_map(|p| p.models.iter().cloned()).collect();
    all.sort_by(|a, b| b.quality.cmp(&a.quality).then_with(|| a.name.cmp(&b.name)));
    // Best-ranked entry per name. `dedup_by` drops only *adjacent* duplicates, and two
    // gateways fronting one model rate it differently, so the copies never are.
    let mut seen = std::collections::HashSet::new();
    all.retain(|m| seen.insert(m.name.clone()));
    all
}

/// The model used when a caller names none.
pub fn default_model() -> Option<String> {
    registry().ok()?.providers.first()?.default_model.clone()
}

/// Resolve `model` to a provider index and the model name to send. Same three rules
/// as [`super::image_gen`]: declared wins, a sole provider takes anything, several
/// providers and no match is named rather than guessed.
fn pick(reg: &Registry, model: Option<&str>) -> anyhow::Result<(usize, String)> {
    let Some(name) = model.map(str::trim).filter(|m| !m.is_empty()) else {
        let Some(default) = reg.providers[0].default_model.clone() else {
            anyhow::bail!(
                "no model named and the configured provider has no default — pass `model` \
                 (reachable: {})",
                model_names(reg)
            )
        };
        return Ok((0, default));
    };
    if let Some(i) = reg.providers.iter().position(|p| p.models.iter().any(|m| m.name == name)) {
        return Ok((i, name.to_string()));
    }
    if reg.providers.len() == 1 {
        return Ok((0, name.to_string()));
    }
    anyhow::bail!("no configured provider serves model {name:?} (reachable: {})", model_names(reg))
}

fn model_names(reg: &Registry) -> String {
    let names: Vec<String> = published_models(reg).into_iter().map(|m| m.name).collect();
    if names.is_empty() { "none published".to_string() } else { names.join(", ") }
}

// ── the tasks ─────────────────────────────────────────────────────────────────

/// Submit a generation request and return the handle to poll. Fast: this only
/// enqueues the work, it does not wait for the clip.
///
/// The shared dispatch behind both tasks. Prefer the task-named entry points
/// ([`text_to_video`], [`image_to_video`]): they make the presence or absence of a
/// first frame — the only thing that distinguishes the two tasks — impossible to get
/// silently wrong.
async fn submit(
    first_frame: Option<&ImageRef>,
    prompt: &str,
    params: &VideoParams,
) -> anyhow::Result<VideoHandle> {
    let reg = registry()?;
    let (index, model) = pick(reg, params.model.as_deref())?;
    let id = match &reg.providers[index].adapter {
        Adapter::Doubao(cfg) => {
            doubao_video_gen::submit(&reg.client, cfg, &model, first_frame, prompt, params).await?
        }
    };
    Ok(VideoHandle { provider: index, id })
}

/// `text-to-video` — a prompt in, a clip out.
///
/// Takes no first frame at all, rather than accepting one and ignoring it: the two
/// tasks are priced and prompted differently, and a silent upgrade is the kind of
/// substitution that gets recorded as the thing that was asked for.
pub async fn text_to_video(prompt: &str, params: &VideoParams) -> anyhow::Result<VideoHandle> {
    if prompt.trim().is_empty() {
        anyhow::bail!("text-to-video needs a prompt");
    }
    submit(None, prompt, params).await
}

/// `image-to-video` — a still as first frame plus an optional prompt in, a clip out.
pub async fn image_to_video(
    first_frame: &ImageRef,
    prompt: &str,
    params: &VideoParams,
) -> anyhow::Result<VideoHandle> {
    submit(Some(first_frame), prompt, params).await
}

/// Download a finished clip. Lives here rather than at the call site so it reuses the
/// capability's client, and because "the bytes behind a `video_url`" is this
/// capability's vocabulary — the URL is only ever produced by [`poll`].
///
/// Callers should not sit on the URL: the vendor expires it roughly a day out.
pub async fn fetch(url: &str) -> anyhow::Result<Bytes> {
    let reg = registry()?;
    Ok(reg.client.get(url).send().await?.error_for_status()?.bytes().await?)
}

/// Fetch the current state of a previously-submitted task. Callers poll this until
/// [`VideoStatus::is_terminal`] on their own cadence.
pub async fn poll(handle: &VideoHandle) -> anyhow::Result<VideoTask> {
    let reg = registry()?;
    let Some(provider) = reg.providers.get(handle.provider) else {
        anyhow::bail!("the provider that holds task {} is no longer configured", handle.id)
    };
    match &provider.adapter {
        Adapter::Doubao(cfg) => doubao_video_gen::poll(&reg.client, cfg, &handle.id).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(key: &str, model: Option<&str>, models: &[&str]) -> ProviderSpec {
        ProviderSpec {
            wire: None,
            base_url: None,
            api_key: key.into(),
            default_model: model.map(str::to_owned),
            models: models
                .iter()
                .enumerate()
                .map(|(i, n)| ModelInfo {
                    name: (*n).into(),
                    quality: 100 - i as i64,
                    ..Default::default()
                })
                .collect(),
        }
    }

    fn reg(specs: Vec<ProviderSpec>) -> Registry {
        Registry { client: reqwest::Client::new(), providers: build_providers(specs) }
    }

    #[test]
    fn a_named_model_reaches_the_provider_that_declares_it() {
        let r = reg(vec![
            spec("a", Some("doubao-seedance-2.0"), &["doubao-seedance-2.0"]),
            spec("b", Some("other-video-1"), &["other-video-1"]),
        ]);
        assert_eq!(pick(&r, Some("other-video-1")).unwrap(), (1, "other-video-1".into()));
        assert_eq!(pick(&r, None).unwrap(), (0, "doubao-seedance-2.0".into()));
        let err = pick(&r, Some("nope")).unwrap_err().to_string();
        assert!(err.contains("nope") && err.contains("doubao-seedance-2.0"), "{err}");
    }

    /// A task id is only addressable next to the provider holding it. Losing that
    /// pairing turns a running generation into "unknown task".
    #[test]
    fn a_handle_carries_the_provider_that_holds_it() {
        let h = VideoHandle { provider: 1, id: "task-abc".into() };
        assert_eq!(h.provider, 1);
        assert_eq!(h.id, "task-abc");
    }

    /// The caller's mistake is not the operator's, and fires before any lookup.
    #[tokio::test]
    async fn an_empty_prompt_is_the_caller_s_mistake() {
        let err = text_to_video("  ", &VideoParams::default()).await.unwrap_err().to_string();
        assert!(err.contains("needs a prompt"), "{err}");
    }
}
