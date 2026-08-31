//! Image-generation capability — two Hugging Face tasks over any configured
//! provider: [`text_to_image`] (prompt → still) and [`image_to_image`] (still +
//! instruction → still).
//!
//! Synchronous: one request, one response with the picture(s).
//!
//! **The caller names a model; nobody configures a vendor.** The interface is
//! semantic — a prompt and an [`ImageParams`] whose `model` is just another optional
//! knob — and choosing well is the agent's job, not a setting. What that costs here
//! is a registry instead of a single backend: [`init`] takes every provider the
//! credential store or the broker offers, [`models`] reports what they collectively
//! serve so the tool description can list it, and each call resolves
//! `params.model` → provider → adapter.
//!
//! **The adapter is inferred from the model name, not chosen by anyone.** A wire is
//! plumbing — which HTTP shape to speak — and asking a person to pick one is asking
//! them a question about our code. `gpt-image-*` speaks the OpenAI images wire,
//! `doubao-*`/`seedream*` the Ark one; a provider may still carry an explicit
//! `wire` (the broker does) and that wins where present.
//!
//! **Two wires, and one model may be reachable over both.** A gateway commonly serves
//! the same task from more than one endpoint, and a model can appear on several. Which
//! one a call takes is decided by the *provider* it resolves to, never by the caller:
//! naming a wire is naming our plumbing. Providers arrive ranked best-first and the
//! first to declare the model wins, so the choice is the broker's editorial ranking
//! rather than an accident.
//!
//! **Image generation is the Images API, both halves of it.** `POST /images/generations`
//! and `POST /images/edits` are the two calls OpenAI publishes for `gpt-image-*`, and
//! this capability speaks exactly those. A Responses-API adapter that reached the same
//! model as an `image_generation` tool argument lived here briefly and is deleted: it
//! needed a mainline carrier model whose tokens billed on top of the picture, and it
//! could not edit at all.
//!
//! The capability is a module of free functions over a process-global,
//! once-initialized registry. The registry never appears in a signature. A
//! provider's key arrives the same way for either task and either source — BYOK from
//! the credential store, or the broker-minted bundle — so a task is wired once and
//! works under both.

use std::sync::OnceLock;
use std::time::Duration;

use bytes::Bytes;

use crate::foundation::vendors::{doubao_image_gen, openai_image_gen};

/// Image synthesis is slow (tens of seconds); budget generously. One client is
/// shared by every provider — they differ by endpoint and key, not by transport.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

// ── the semantic interface ────────────────────────────────────────────────────

/// The knobs a caller may turn, all optional: `None` means "let the model decide",
/// so the vendor's own default applies rather than one we invented.
///
/// A **typed superset**, not a pass-through bag of vendor keys. Every field here is
/// a dial an agent has a reason to reach for, and each adapter maps what it can and
/// *refuses* what it cannot with a message naming the alternative. A free-form map
/// would spare us the mapping and make the tool schema unteachable — the agent would
/// have to already know each vendor's parameter names, which is the opposite of
/// choosing from a described menu.
#[derive(Debug, Clone, Default)]
pub struct ImageParams {
    /// Which model to draw with. `None` → the default of the first configured
    /// provider. See [`models`] for what is reachable.
    pub model: Option<String>,
    /// e.g. `"1024x1024"`, `"2K"`, `"adaptive"` — vendor- and model-specific.
    pub size: Option<String>,
    /// The cost/quality dial where the model has one, e.g. `"low"`/`"medium"`/`"high"`.
    pub quality: Option<String>,
    /// How many images to return. `None` → one.
    pub n: Option<u32>,
    /// Fix the seed to make a run repeatable.
    pub seed: Option<i64>,
    /// `"transparent"` for a cutout, `"opaque"`, `"auto"`. Not every model can.
    pub background: Option<String>,
    /// `"png"`, `"jpeg"`, `"webp"`.
    pub output_format: Option<String>,
    pub watermark: Option<bool>,
}

/// The still an [`image_to_image`] call works from: an already-usable URL passed
/// through untouched, or raw bytes the adapter encodes as the wire needs.
///
/// Deliberately its own type rather than one shared with
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

/// One generated image, as **bytes** — because that is what a caller wants and
/// everything else is transport.
///
/// Whether the provider handed back base64 or a URL that expires in an hour is the
/// adapter's problem, settled before the value crosses this boundary. The earlier
/// shape (`url: Option<String>` / `b64_json: Option<String>`, exactly one populated)
/// made every caller re-solve it, and a caller that persists the URL instead of the
/// bytes has stored a picture that vanishes.
#[derive(Debug, Clone)]
pub struct GeneratedImage {
    pub bytes: Bytes,
    /// Sniffed from the bytes, not taken on the provider's word.
    pub mime: String,
    /// What the provider says it produced, when it says anything.
    pub size: Option<String>,
}

/// One model a provider serves, with the broker's relative hints. The hints exist to
/// be *shown to the agent* — an agent told only a list of names cannot trade cost
/// against quality, which is the choice we just handed it.
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
    /// Explicit wire id, where the source knows one (`doubao`, `openai-images`).
    /// `None` → inferred from the model name.
    pub wire: Option<String>,
    /// Gateway endpoint; `None` → the vendor's own default.
    pub base_url: Option<String>,
    pub api_key: String,
    /// Used when a caller names no model.
    pub default_model: Option<String>,
    /// What this provider is known to serve. **Empty means BYOK**: we have no menu,
    /// so any model name the agent asks for is passed through and the provider's own
    /// error is the answer.
    pub models: Vec<ModelInfo>,
}

// ── the registry ──────────────────────────────────────────────────────────────

/// One endpoint and key, ready to be spoken to in whichever shape a model needs.
///
/// **Both image-endpoint adapters, because the wire is a property of the call and not
/// of the provider.** A gateway publishes one wire id and serves several model families
/// behind it — songguo says `openai-images` and answers for both gpt-image and
/// seedream — so binding a provider to one adapter at startup sends half its models
/// down the wrong path. That is not theoretical: it sent a seedream edit as OpenAI
/// multipart and got back "could not parse the JSON body".
struct Provider {
    doubao: doubao_image_gen::Config,
    openai: openai_image_gen::Config,
    /// What the source called this endpoint, when we recognised it. Only consulted
    /// for a model whose family we cannot read off its name.
    declared_wire: Option<&'static str>,
    default_model: Option<String>,
    models: Vec<ModelInfo>,
}

impl Provider {
    /// The shape to speak for one model.
    ///
    /// **The model name wins**: it is the most specific thing we know,
    /// and one image endpoint really does serve several families — songguo says
    /// `openai-images` and answers for seedream too. The declared wire is then the
    /// fallback for an unfamiliar name, where a gateway's own word beats a guess.
    fn wire_for(&self, model: &str) -> &'static str {
        family_of(model).or(self.declared_wire).unwrap_or(OPENAI_WIRE)
    }

    /// Whether editing is implemented for `model`'s shape. A fact about *our* code,
    /// not about what the vendor could do: Ark's edit parameter is unconfirmed, so the
    /// doubao arm refuses rather than guessing a field name.
    fn can_edit(&self, model: &str) -> bool {
        self.wire_for(model) == OPENAI_WIRE
    }
}

struct Registry {
    client: reqwest::Client,
    providers: Vec<Provider>,
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

const DOUBAO_WIRE: &str = "doubao";
const OPENAI_WIRE: &str = "openai-images";

/// The family a model name belongs to, or `None` when we don't recognise it.
///
/// Deliberately an `Option` rather than a defaulting guess: "I don't know this
/// model" and "this model is an OpenAI one" are different answers, and only the
/// caller knows whether it has a better source (a declared wire) to fall back on.
fn family_of(model: &str) -> Option<&'static str> {
    let m = model.trim().to_ascii_lowercase();
    if m.starts_with("gpt-image") || m.starts_with("dall-e") {
        Some(OPENAI_WIRE)
    } else if m.starts_with("doubao") || m.starts_with("seedream") {
        Some(DOUBAO_WIRE)
    } else {
        None
    }
}

/// The adapter an explicit wire id selects, or `None` if we have none for it.
///
/// Spelling is deliberately loose. The broker names wires in its own vocabulary and
/// has changed it before (`openai-images` vs the gateway's `openai/images`), and this
/// string is machine-generated plumbing — since nobody picks a wire in Settings any
/// more, matching it strictly buys no typo protection and costs a boot.
fn wire_named(wire: &str) -> Option<&'static str> {
    let w = wire.trim().to_ascii_lowercase();
    if w.contains("doubao") || w.contains("ark") || w.contains("volc") {
        Some(DOUBAO_WIRE)
    } else if w.contains("openai") || w.contains("image") {
        Some(OPENAI_WIRE)
    } else {
        None
    }
}

/// Register every provider that has a key. Idempotent — the first init wins.
///
/// A spec with a blank key is skipped: "no image provider" is an ordinary state (BYOK
/// with nothing pasted yet), not a misconfiguration. A wire we don't recognise is
/// logged and the model name decides instead — see [`wire_named`] for why an
/// unfamiliar string must not be fatal here.
pub fn init(specs: Vec<ProviderSpec>) -> anyhow::Result<()> {
    let providers = build_providers(specs);
    let client = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build()?;
    let _ = REGISTRY.set(Registry { client, providers });
    Ok(())
}

/// The classification half of [`init`], separated so it can be tested: the registry
/// is a write-once process global, and a test that had to install one could only ever
/// run first, alone.
fn build_providers(specs: Vec<ProviderSpec>) -> Vec<Provider> {
    let mut providers = Vec::new();
    for spec in specs {
        if spec.api_key.trim().is_empty() {
            continue;
        }
        let declared = spec.wire.as_deref().map(str::trim).filter(|w| !w.is_empty());
        let declared_wire = declared.and_then(wire_named);
        if let (Some(unknown), None) = (declared, declared_wire) {
            tracing::warn!(
                wire = unknown,
                "no image-gen adapter for this wire; model names will decide instead"
            );
        }
        providers.push(Provider {
            doubao: doubao_image_gen::Config::new(&spec.api_key, spec.base_url.as_deref()),
            openai: openai_image_gen::Config::new(&spec.api_key, spec.base_url.as_deref()),
            declared_wire,
            default_model: spec.default_model.clone(),
            models: spec.models.clone(),
        });
    }
    providers
}

/// The live registry, or the one error an operator can act on.
fn registry() -> anyhow::Result<&'static Registry> {
    match REGISTRY.get() {
        Some(r) if !r.providers.is_empty() => Ok(r),
        _ => anyhow::bail!("image generation not configured (set an image key in Settings)"),
    }
}

/// Whether any provider is configured.
pub fn available() -> bool {
    registry().is_ok()
}

/// Every model reachable right now, best-quality first — the menu the tool
/// description is built from. Empty *with* a provider configured means BYOK: no menu
/// was published, and any name the agent gives is passed through.
pub fn models() -> Vec<ModelInfo> {
    registry().map(published_models).unwrap_or_default()
}

fn published_models(reg: &Registry) -> Vec<ModelInfo> {
    let mut all: Vec<ModelInfo> =
        reg.providers.iter().flat_map(|p| p.models.iter().cloned()).collect();
    all.sort_by(|a, b| b.quality.cmp(&a.quality).then_with(|| a.name.cmp(&b.name)));
    // Keep the best-ranked entry per name. `dedup_by` would not do: it only drops
    // *adjacent* duplicates, and two gateways fronting one model rate it differently,
    // so the copies are never adjacent — which is the only case this exists for.
    let mut seen = std::collections::HashSet::new();
    all.retain(|m| seen.insert(m.name.clone()));
    all
}

/// The model used when a caller names none: the first configured provider's default.
pub fn default_model() -> Option<String> {
    registry().ok()?.providers.first()?.default_model.clone()
}

/// Resolve `model` to the provider that should serve it and the model name to send.
///
/// 1. A model somebody **declares** goes to that provider.
/// 2. A model nobody declares goes to the sole provider, if there is exactly one —
///    BYOK has no menu, and the provider's own catalog is the authority.
/// 3. Otherwise it is a mistake worth naming, listing what *is* reachable. Silently
///    substituting a model that happens to be configured would be recorded as the
///    model that was asked for.
fn pick<'a>(reg: &'a Registry, model: Option<&str>) -> anyhow::Result<(&'a Provider, String)> {
    let Some(name) = model.map(str::trim).filter(|m| !m.is_empty()) else {
        let p = &reg.providers[0];
        let Some(default) = p.default_model.clone() else {
            anyhow::bail!(
                "no model named and the configured provider has no default — \
                 pass `model` (reachable: {})",
                model_names(reg)
            )
        };
        return Ok((p, default));
    };

    if let Some(p) = reg.providers.iter().find(|p| p.models.iter().any(|m| m.name == name)) {
        return Ok((p, name.to_string()));
    }
    if reg.providers.len() == 1 {
        return Ok((&reg.providers[0], name.to_string()));
    }
    anyhow::bail!("no configured provider serves model {name:?} (reachable: {})", model_names(reg))
}

fn model_names(reg: &Registry) -> String {
    let names: Vec<String> = published_models(reg).into_iter().map(|m| m.name).collect();
    if names.is_empty() { "none published".to_string() } else { names.join(", ") }
}

// ── the tasks ─────────────────────────────────────────────────────────────────

/// `text-to-image` — draw `prompt`. Synchronous: the future resolves once the
/// picture(s) are ready.
pub async fn text_to_image(
    prompt: &str,
    params: &ImageParams,
) -> anyhow::Result<Vec<GeneratedImage>> {
    if prompt.trim().is_empty() {
        anyhow::bail!("text-to-image needs a prompt");
    }
    let reg = registry()?;
    let (provider, model) = pick(reg, params.model.as_deref())?;
    let client = &reg.client;
    match provider.wire_for(&model) {
        DOUBAO_WIRE => {
            doubao_image_gen::generate(client, &provider.doubao, &model, prompt, params).await
        }
        _ => openai_image_gen::generate(client, &provider.openai, &model, prompt, params).await,
    }
}

/// `image-to-image` — edit `source` under `prompt`, returning a new image and
/// leaving the original untouched.
///
/// A provider whose adapter has no edit implementation says so and names what can,
/// rather than reporting "not configured": those are different problems with
/// different fixes, and a caller that conflates them sends the user to Settings for a
/// setting that would not have helped.
pub async fn image_to_image(
    source: &SourceImage,
    prompt: &str,
    params: &ImageParams,
) -> anyhow::Result<Vec<GeneratedImage>> {
    if prompt.trim().is_empty() {
        anyhow::bail!("image-to-image needs a prompt saying what to change");
    }
    let reg = registry()?;
    let (provider, model) = pick(reg, params.model.as_deref())?;
    if !provider.can_edit(&model) {
        anyhow::bail!(
            "editing is not implemented for {model} (the {} wire) — name a gpt-image model \
             instead (reachable: {})",
            provider.wire_for(&model),
            model_names(reg)
        );
    }
    openai_image_gen::edit(&reg.client, &provider.openai, &model, source, prompt, params).await
}

// ── shared adapter helpers ────────────────────────────────────────────────────

/// The image type the bytes actually are, by magic number.
///
/// Sniffed rather than taken from `output_format`, because that parameter is a
/// *request* and the answer is whatever arrived. Labelling a JPEG `image/png`
/// because we asked for PNG is the kind of small lie that surfaces three layers later
/// as a picture nothing will open.
pub(crate) fn sniff_mime(bytes: &[u8]) -> String {
    let t = match bytes {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        _ => "application/octet-stream",
    };
    t.to_string()
}

/// The filename extension for a sniffed mime — what [`crate::mind::memory::media`]
/// needs to name the artifact on disk.
pub fn extension_for(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the reshape: nobody configures a vendor, so the model name
    /// has to carry that information. A wrong guess here sends an OpenAI key to Ark.
    #[test]
    fn a_model_name_names_its_family_or_admits_it_cannot() {
        assert_eq!(family_of("gpt-image-2"), Some(OPENAI_WIRE));
        assert_eq!(family_of("GPT-Image-1.5"), Some(OPENAI_WIRE));
        assert_eq!(family_of("doubao-seedream-5.0-lite"), Some(DOUBAO_WIRE));
        assert_eq!(family_of("seedream-4.0"), Some(DOUBAO_WIRE));
        // Not a guess — the caller has a declared wire to fall back on and needs to
        // know when to use it.
        assert_eq!(family_of("some-new-thing"), None);
    }

    /// **The regression a live run caught.** A gateway publishes one wire id and
    /// serves several families behind it; songguo says `openai-images` and answers for
    /// seedream too. Binding the adapter to the provider sent a seedream edit as
    /// OpenAI multipart, and the gateway answered "could not parse the JSON body".
    #[test]
    fn a_gateways_wire_does_not_override_the_model_it_is_serving() {
        let r = reg(vec![ProviderSpec {
            wire: Some("openai-images".into()),
            base_url: Some("https://gw.example/v1/images/generations".into()),
            api_key: "k".into(),
            default_model: Some("doubao-seedream-5.0-lite".into()),
            models: vec![
                ModelInfo { name: "doubao-seedream-5.0-lite".into(), ..Default::default() },
                ModelInfo { name: "gpt-image-2".into(), ..Default::default() },
            ],
        }]);
        let p = &r.providers[0];

        assert_eq!(p.wire_for("doubao-seedream-5.0-lite"), DOUBAO_WIRE, "the model wins");
        assert_eq!(p.wire_for("gpt-image-2"), OPENAI_WIRE);
        // One credential, both families — which is the case the gateway exists for.
        assert!(!p.can_edit("doubao-seedream-5.0-lite"));
        assert!(p.can_edit("gpt-image-2"));

        // A name we don't know falls back to what the gateway called itself.
        assert_eq!(p.wire_for("something-new"), OPENAI_WIRE);
    }

    /// Bytes are the interface, so their type must come from the bytes. A provider
    /// that ignores `output_format: png` and returns JPEG must not be believed.
    #[test]
    fn the_mime_comes_from_the_bytes() {
        assert_eq!(sniff_mime(b"\x89PNG\r\n\x1a\n"), "image/png");
        assert_eq!(sniff_mime(b"\xff\xd8\xff\xe0junk"), "image/jpeg");
        assert_eq!(sniff_mime(b"RIFF\x00\x00\x00\x00WEBPVP8 "), "image/webp");
        assert_eq!(sniff_mime(b"not an image"), "application/octet-stream");
        assert_eq!(extension_for(&sniff_mime(b"\x89PNG\r\n\x1a\n")), "png");
    }

    /// The caller's mistake is not the operator's, and must not be reported as one —
    /// this fires before any provider lookup, so it holds configured or not.
    #[tokio::test]
    async fn an_empty_prompt_is_the_caller_s_mistake() {
        let err = text_to_image("   ", &ImageParams::default()).await.unwrap_err().to_string();
        assert!(err.contains("needs a prompt"), "{err}");

        let src = SourceImage::url("https://example.invalid/a.png");
        let err =
            image_to_image(&src, "", &ImageParams::default()).await.unwrap_err().to_string();
        assert!(err.contains("what to change"), "{err}");
    }

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

    /// A blank key is "nothing pasted yet" — an ordinary state, not something to
    /// refuse to start over.
    #[test]
    fn a_blank_key_never_becomes_a_provider() {
        let built = build_providers(vec![
            spec("  ", Some("gpt-image-2"), &[]),
            spec("k", Some("gpt-image-2"), &[]),
        ]);
        assert_eq!(built.len(), 1);
    }

    /// **Two wires for one task, which is now the ordinary case.** A gateway serving
    /// `text-to-image` over both an images endpoint and something else hands us both;
    /// each keeps its own endpoint and menu, and a named model reaches the one that
    /// declares it. Nothing here merges them.
    #[test]
    fn two_wires_for_one_task_are_both_reachable() {
        let r = reg(vec![
            ProviderSpec {
                wire: Some("openai-images".into()),
                base_url: Some("https://gw.example/v1/images/generations".into()),
                api_key: "k".into(),
                    default_model: Some("gpt-image-2".into()),
                models: vec![ModelInfo { name: "gpt-image-2".into(), quality: 96, ..Default::default() }],
            },
            ProviderSpec {
                wire: Some("ark/images".into()),
                base_url: Some("https://ark.example/api/v3/images/generations".into()),
                api_key: "k2".into(),
                    default_model: Some("doubao-seedream-5.0-lite".into()),
                models: vec![ModelInfo {
                    name: "doubao-seedream-5.0-lite".into(),
                    quality: 75,
                    ..Default::default()
                }],
            },
        ]);
        assert_eq!(r.providers.len(), 2);

        let (p, model) = pick(&r, Some("doubao-seedream-5.0-lite")).unwrap();
        assert_eq!(p.wire_for(&model), DOUBAO_WIRE);
        assert_eq!(p.default_model.as_deref(), Some("doubao-seedream-5.0-lite"), "the ark one");

        // The published menu is the union, so the agent is shown both.
        let names: Vec<String> = published_models(&r).into_iter().map(|m| m.name).collect();
        assert_eq!(names, vec!["gpt-image-2", "doubao-seedream-5.0-lite"]);

        // No model named → the first provider's default. The composition root hands
        // them over best-first, so "first" is the best on offer.
        let (_, model) = pick(&r, None).unwrap();
        assert_eq!(model, "gpt-image-2");
    }

    /// A Responses endpoint speaks exactly one body shape, so there the declared wire
    /// outranks the model name — the reverse of every other case. `gpt-image-2` behind
    /// `/v1/responses` is a tool argument, not the model of the call, and reading its
    /// name as "OpenAI images" would POST `prompt`/`size` to an endpoint whose schema is

    /// **The carrier is configuration, never a guess.** Without one there is no model to
    /// host the tool, and any we chose would bill the user for a decision nobody made —

    /// One model, two wires, one menu entry per wire — the case the whole multi-wire
    /// shape exists for. Which one a call takes is the broker's ranking (best first),
    /// One model, two wires, one menu entry — the case the multi-wire shape exists
    /// for. Which one a call takes is the broker's ranking (best first), not the
    /// caller's, because naming a wire is naming our plumbing. The model is
    /// deliberately one whose family we cannot read off its name, so the declared wire
    /// is what decides and the ranking is visible.
    #[test]
    fn one_model_served_over_two_wires_resolves_to_the_better_ranked_one() {
        let wire = |w: &str, url: &str| ProviderSpec {
            wire: Some(w.into()),
            base_url: Some(url.into()),
            api_key: "k".into(),
            default_model: Some("acme-diffusion-1".into()),
            models: vec![ModelInfo {
                name: "acme-diffusion-1".into(),
                quality: 96,
                ..Default::default()
            }],
        };
        let images = wire("openai-images", "https://gw.example/v1/images/generations");
        let ark = wire("ark/images", "https://ark.example/api/v3/images/generations");

        let r = reg(vec![ark.clone(), images.clone()]);
        let (p, model) = pick(&r, Some("acme-diffusion-1")).unwrap();
        assert_eq!(p.wire_for(&model), DOUBAO_WIRE, "first to declare it wins");

        // Reverse the ranking and the same request takes the images wire instead —
        // and regains editing, which only that wire implements.
        let r = reg(vec![images, ark]);
        let (p, model) = pick(&r, Some("acme-diffusion-1")).unwrap();
        assert_eq!(p.wire_for(&model), OPENAI_WIRE);
        assert!(p.can_edit(&model));

        // Either way the menu shows the model once, not twice.
        let names: Vec<String> = published_models(&r).into_iter().map(|m| m.name).collect();
        assert_eq!(names, vec!["acme-diffusion-1"]);
    }

    /// The broker names wires in its own vocabulary and has changed it before. An
    /// unfamiliar spelling must degrade to the model name, never to a failed boot.
    #[test]
    fn a_wire_id_is_matched_loosely_and_never_fatally() {
        assert_eq!(wire_named("openai/images"), Some(OPENAI_WIRE));
        assert_eq!(wire_named("openai-images"), Some(OPENAI_WIRE));
        assert_eq!(wire_named("ark/images"), Some(DOUBAO_WIRE));
        assert_eq!(wire_named("teapot"), None);

        let bogus = ProviderSpec {
            wire: Some("teapot".into()),
            api_key: "k".into(),
            default_model: Some("doubao-seedream-5.0-lite".into()),
            ..Default::default()
        };
        let built = build_providers(vec![bogus]);
        assert_eq!(built.len(), 1, "an unknown wire must not cost us the provider");
        assert!(built[0].declared_wire.is_none());
        assert_eq!(
            built[0].wire_for("doubao-seedream-5.0-lite"),
            DOUBAO_WIRE,
            "the model name decided instead"
        );
    }

    /// The routing rule, which is the whole registry: a declared model goes to whoever
    /// declares it, even when another provider is listed first.
    #[test]
    fn a_named_model_reaches_the_provider_that_declares_it() {
        let r = reg(vec![
            spec("ark-key", Some("doubao-seedream-5.0-lite"), &["doubao-seedream-5.0-lite"]),
            spec("oai-key", Some("gpt-image-2"), &["gpt-image-2", "gpt-image-1.5"]),
        ]);

        let (p, model) = pick(&r, Some("gpt-image-2")).unwrap();
        assert_eq!(model, "gpt-image-2");
        assert_eq!(p.wire_for(&model), OPENAI_WIRE);
        assert_eq!(p.default_model.as_deref(), Some("gpt-image-2"), "the second provider");

        let (p, model) = pick(&r, Some("doubao-seedream-5.0-lite")).unwrap();
        assert_eq!(model, "doubao-seedream-5.0-lite");
        assert_eq!(p.wire_for(&model), DOUBAO_WIRE);

        // No model named → the first provider's default, not a search.
        let (p, model) = pick(&r, None).unwrap();
        assert_eq!(model, "doubao-seedream-5.0-lite");
        assert_eq!(p.wire_for(&model), DOUBAO_WIRE);
    }

    /// BYOK publishes no menu, so an unlisted name must reach the one provider we
    /// have — its catalog is the authority, not ours.
    #[test]
    fn an_unlisted_model_passes_through_to_a_sole_provider() {
        let r = reg(vec![spec("oai-key", Some("gpt-image-2"), &[])]);
        let (_, model) = pick(&r, Some("gpt-image-3-unreleased")).unwrap();
        assert_eq!(model, "gpt-image-3-unreleased");
    }

    /// With several providers there is no "the" provider to guess at, and guessing
    /// would be recorded as the model that was asked for. Say what is reachable instead.
    #[test]
    fn an_unlisted_model_with_several_providers_is_named_not_substituted() {
        let r = reg(vec![
            spec("ark-key", Some("doubao-seedream-5.0-lite"), &["doubao-seedream-5.0-lite"]),
            spec("oai-key", Some("gpt-image-2"), &["gpt-image-2"]),
        ]);
        let err = pick(&r, Some("midjourney")).err().expect("must not substitute").to_string();
        assert!(err.contains("midjourney"), "{err}");
        assert!(err.contains("gpt-image-2") && err.contains("doubao-seedream"), "{err}");
    }

    /// The menu the tool description is built from: best quality first, no duplicates
    /// when two gateways front the same model.
    #[test]
    fn the_published_menu_is_ranked_and_deduplicated() {
        let r = reg(vec![
            spec("a", Some("gpt-image-1.5"), &["gpt-image-1.5"]),
            spec("b", Some("gpt-image-2"), &["gpt-image-2", "gpt-image-1.5"]),
        ]);
        let names: Vec<String> = published_models(&r).into_iter().map(|m| m.name).collect();
        assert_eq!(names, vec!["gpt-image-1.5", "gpt-image-2"]);
    }

    /// "This vendor can't" and "nothing is configured" are different problems with
    /// different fixes, and only one of them is answered by opening Settings.
    #[tokio::test]
    async fn editing_on_a_wire_that_cannot_says_so_and_names_one_that_can() {
        let r = reg(vec![
            spec("ark-key", Some("doubao-seedream-5.0-lite"), &["doubao-seedream-5.0-lite"]),
            spec("oai-key", Some("gpt-image-2"), &["gpt-image-2"]),
        ]);
        let (p, model) = pick(&r, Some("doubao-seedream-5.0-lite")).unwrap();
        assert!(!p.can_edit(&model));
        let (p2, m2) = pick(&r, Some("gpt-image-2")).unwrap();
        assert!(p2.can_edit(&m2));
        assert_eq!(model, "doubao-seedream-5.0-lite");
        assert!(model_names(&r).contains("gpt-image-2"));
    }
}
