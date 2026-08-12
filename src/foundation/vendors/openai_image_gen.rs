//! OpenAI GPT Image generation and editing.
//!
//!   POST {base}/images/generations   (JSON)       prompt → image
//!   POST {base}/images/edits         (multipart)  image + prompt → image
//!   Authorization: Bearer <api_key>
//!
//! `base` defaults to `https://api.openai.com/v1`. Both routes answer with a `data`
//! array of `b64_json`; GPT Image models return base64 unconditionally — there is no
//! `response_format: url` to ask for, which is why nothing here does.
//!
//! **Not the Responses API, deliberately.** `/v1/responses` reaches image generation
//! only through a *mainline* model holding an `image_generation` tool: that model
//! decides whether to draw and picks the image model itself, and it bills its own
//! tokens on top. This capability's contract is the opposite — the caller names the
//! model and gets exactly the picture it asked for — so the Images API is the wire
//! that matches. (`openai-responses` elsewhere in this codebase is the LLM wire and
//! is unrelated.)
//!
//! **Editing lives here and not on the Ark wire**, so
//! [`image_gen::image_to_image`](crate::body::capabilities::image_gen::image_to_image)
//! works whenever a gpt-image model is reachable.

use anyhow::Context;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::body::capabilities::image_gen::{
    GeneratedImage, ImageParams, SourceImage, extension_for, sniff_mime,
};

const DEFAULT_API_BASE: &str = "https://api.openai.com/v1";

/// gpt-image-2's size rules. Checked here rather than left to a 400, because `size`
/// reaches us as free text an agent wrote and "must be a multiple of 16" is
/// actionable in a way that `invalid_request_error` is not.
const GPT_IMAGE_2_MIN_PIXELS: u64 = 655_360;
const GPT_IMAGE_2_MAX_PIXELS: u64 = 8_294_400;
const GPT_IMAGE_2_MAX_EDGE: u64 = 3840;
const GPT_IMAGE_2_MAX_RATIO: f64 = 3.0;

/// One configured OpenAI-images provider: the two endpoints and the key. No model and
/// no HTTP client — the model comes per call, the client is shared.
pub struct Config {
    api_key: String,
    generations: String,
    edits: String,
}

impl Config {
    /// `base_url`, when set, is the gateway's **full** image-generations endpoint
    /// (that is the shape the broker mints and the shape the Ark adapter takes, so
    /// the two stay symmetrical). The edits endpoint is derived from it by swapping
    /// the last path segment, which is the only difference between the two routes.
    pub fn new(api_key: &str, base_url: Option<&str>) -> Self {
        let generations = match base_url.map(str::trim).filter(|b| !b.is_empty()) {
            Some(base) => base.trim_end_matches('/').to_string(),
            None => format!("{DEFAULT_API_BASE}/images/generations"),
        };
        let edits = match generations.rsplit_once('/') {
            Some((head, "generations")) => format!("{head}/edits"),
            _ => format!("{generations}/../edits"),
        };
        Self { api_key: api_key.trim().to_string(), generations, edits }
    }
}

/// Reject a `size` this model cannot serve, with the rule that was broken.
///
/// Only `WxH` is checked — `"auto"` and any other token is left for the API to judge,
/// since a client-side allowlist would reject next month's valid value.
fn check_size(model: &str, size: Option<&str>) -> anyhow::Result<()> {
    let Some(size) = size.map(str::trim).filter(|s| !s.is_empty()) else { return Ok(()) };
    if !model.starts_with("gpt-image-2") {
        return Ok(());
    }
    let Some((w, h)) = size.split_once('x') else { return Ok(()) };
    let (Ok(w), Ok(h)) = (w.trim().parse::<u64>(), h.trim().parse::<u64>()) else {
        return Ok(());
    };

    if w % 16 != 0 || h % 16 != 0 {
        anyhow::bail!("{model} needs both edges to be multiples of 16px; got {size}");
    }
    if w.max(h) > GPT_IMAGE_2_MAX_EDGE {
        anyhow::bail!("{model} caps the long edge at {GPT_IMAGE_2_MAX_EDGE}px; got {size}");
    }
    let pixels = w * h;
    if !(GPT_IMAGE_2_MIN_PIXELS..=GPT_IMAGE_2_MAX_PIXELS).contains(&pixels) {
        anyhow::bail!(
            "{model} needs {GPT_IMAGE_2_MIN_PIXELS}–{GPT_IMAGE_2_MAX_PIXELS} total pixels; \
             {size} is {pixels}"
        );
    }
    let ratio = w.max(h) as f64 / w.min(h) as f64;
    if ratio > GPT_IMAGE_2_MAX_RATIO {
        anyhow::bail!("{model} caps the aspect ratio at 3:1; {size} is {ratio:.1}:1");
    }
    Ok(())
}

/// Reject a knob this *model* cannot honour (as opposed to this wire).
///
/// gpt-image-2 has no transparent background — that is gpt-image-1.5 — and no
/// fidelity dial, because its image inputs are always high fidelity. Both are cases
/// where the API would accept the call and quietly do something else.
fn check_model_knobs(model: &str, params: &ImageParams) -> anyhow::Result<()> {
    let transparent = params.background.as_deref().map(str::trim) == Some("transparent");
    if transparent && model.starts_with("gpt-image-2") {
        anyhow::bail!(
            "{model} has no transparent background — name gpt-image-1.5 with \
             output_format=png for a true cutout"
        );
    }
    Ok(())
}

/// The shared knobs both routes send. `model` and `prompt` are the caller's; the rest
/// ride only when set, so the API's own defaults apply.
fn common_fields(params: &ImageParams) -> Vec<(&'static str, Value)> {
    let mut fields = Vec::new();
    if let Some(size) = &params.size {
        fields.push(("size", json!(size)));
    }
    if let Some(quality) = &params.quality {
        fields.push(("quality", json!(quality)));
    }
    if let Some(n) = params.n {
        fields.push(("n", json!(n)));
    }
    if let Some(background) = &params.background {
        fields.push(("background", json!(background)));
    }
    if let Some(output_format) = &params.output_format {
        fields.push(("output_format", json!(output_format)));
    }
    fields
}

/// Build the `images/generations` body. Pure, so the wire shape is unit-testable.
fn build_generate_request(
    model: &str,
    prompt: &str,
    params: &ImageParams,
) -> anyhow::Result<Value> {
    check_size(model, params.size.as_deref())?;
    check_model_knobs(model, params)?;
    if params.watermark.is_some() {
        anyhow::bail!(
            "`watermark` is not a knob on the OpenAI images wire — name a doubao model to \
             control watermarking"
        );
    }

    let mut body = json!({ "model": model, "prompt": prompt });
    let obj = body.as_object_mut().expect("json object");
    for (k, v) in common_fields(params) {
        obj.insert(k.into(), v);
    }
    // `seed` is accepted on the Images API where the model supports it; omitted
    // otherwise so an unsupported knob never becomes a silent default.
    if let Some(seed) = params.seed {
        obj.insert("seed".into(), json!(seed));
    }
    Ok(body)
}

pub async fn generate(
    client: &reqwest::Client,
    cfg: &Config,
    model: &str,
    prompt: &str,
    params: &ImageParams,
) -> anyhow::Result<Vec<GeneratedImage>> {
    let body = build_generate_request(model, prompt, params)?;
    let resp = client
        .post(&cfg.generations)
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send()
        .await
        .context("openai image-gen request failed")?;
    read_images(client, resp, "image-gen").await
}

/// `images/edits` — multipart, because the source image is a file part. A URL source
/// is fetched first: this route takes bytes, not a link, and the fetch is ours to do
/// rather than something to fail the call over.
pub async fn edit(
    client: &reqwest::Client,
    cfg: &Config,
    model: &str,
    source: &SourceImage,
    prompt: &str,
    params: &ImageParams,
) -> anyhow::Result<Vec<GeneratedImage>> {
    check_size(model, params.size.as_deref())?;
    check_model_knobs(model, params)?;

    let (bytes, mime) = match source {
        SourceImage::Bytes { bytes, mime } => (bytes.clone(), mime.clone()),
        SourceImage::Url(url) => {
            let bytes = client
                .get(url)
                .send()
                .await
                .with_context(|| format!("fetching source image {url}"))?
                .error_for_status()?
                .bytes()
                .await
                .context("reading source image body")?;
            let mime = sniff_mime(&bytes);
            (bytes, mime)
        }
    };

    let part = reqwest::multipart::Part::bytes(bytes.to_vec())
        .file_name(format!("source.{}", extension_for(&mime)))
        .mime_str(&mime)
        .context("source image has an unusable content type")?;

    let mut form = reqwest::multipart::Form::new()
        .text("model", model.to_string())
        .text("prompt", prompt.to_string())
        .part("image", part);
    for (k, v) in common_fields(params) {
        // Multipart carries text, so a JSON scalar goes as its bare rendering — an
        // integer must not arrive quoted-with-braces.
        let text = match v {
            Value::String(s) => s,
            other => other.to_string(),
        };
        form = form.text(k, text);
    }

    let resp = client
        .post(&cfg.edits)
        .bearer_auth(&cfg.api_key)
        .multipart(form)
        .send()
        .await
        .context("openai image-edit request failed")?;
    read_images(client, resp, "image-edit").await
}

/// Both routes answer alike: `{"data": [{"b64_json": …}]}`.
async fn read_images(
    client: &reqwest::Client,
    resp: reqwest::Response,
    what: &str,
) -> anyhow::Result<Vec<GeneratedImage>> {
    let status = resp.status();
    let text = resp.text().await.with_context(|| format!("reading openai {what} response"))?;
    if !status.is_success() {
        anyhow::bail!("openai {what} HTTP {status}: {text}");
    }
    let parsed: ImageResponse = serde_json::from_str(&text)
        .with_context(|| format!("parsing openai {what} response: {text}"))?;
    if parsed.data.is_empty() {
        anyhow::bail!("openai {what} returned no images");
    }
    let mut out = Vec::with_capacity(parsed.data.len());
    for datum in parsed.data {
        out.push(datum.into_image(client).await?);
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct ImageResponse {
    #[serde(default)]
    data: Vec<ImageDatum>,
}

#[derive(Debug, Deserialize)]
struct ImageDatum {
    #[serde(default)]
    b64_json: Option<String>,
    /// GPT Image never sends one, but a gateway in front of another model may.
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    size: Option<String>,
}

impl ImageDatum {
    async fn into_image(self, client: &reqwest::Client) -> anyhow::Result<GeneratedImage> {
        let bytes = if let Some(b64) = self.b64_json.as_deref().filter(|s| !s.is_empty()) {
            bytes::Bytes::from(
                base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .context("decoding openai image base64")?,
            )
        } else if let Some(url) = self.url.as_deref().filter(|s| !s.is_empty()) {
            client
                .get(url)
                .send()
                .await
                .with_context(|| format!("fetching generated image {url}"))?
                .error_for_status()?
                .bytes()
                .await
                .context("reading generated image body")?
        } else {
            anyhow::bail!("openai images returned a datum with neither bytes nor a url");
        };
        let mime = sniff_mime(&bytes);
        Ok(GeneratedImage { bytes, mime, size: self.size })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_routes_are_derived_from_one_base() {
        let cfg = Config::new("k", None);
        assert_eq!(cfg.generations, "https://api.openai.com/v1/images/generations");
        assert_eq!(cfg.edits, "https://api.openai.com/v1/images/edits");

        // A gateway hands us the full generations URL; edits is its sibling.
        let cfg = Config::new("k", Some("https://gw.example/v1/images/generations/"));
        assert_eq!(cfg.generations, "https://gw.example/v1/images/generations");
        assert_eq!(cfg.edits, "https://gw.example/v1/images/edits");
    }

    #[test]
    fn the_model_comes_per_call_and_unset_knobs_are_omitted() {
        let body = build_generate_request("gpt-image-2", "a red bicycle", &ImageParams::default())
            .unwrap();
        assert_eq!(body["model"], "gpt-image-2");
        assert_eq!(body["prompt"], "a red bicycle");
        let obj = body.as_object().unwrap();
        for k in ["size", "quality", "n", "background", "output_format", "seed"] {
            assert!(!obj.contains_key(k), "{k} should be omitted when unset");
        }
        // No response_format: GPT Image returns base64 and takes no such argument.
        assert!(!obj.contains_key("response_format"));
    }

    #[test]
    fn set_knobs_ride_along() {
        let params = ImageParams {
            size: Some("1024x1024".into()),
            quality: Some("high".into()),
            n: Some(2),
            output_format: Some("webp".into()),
            seed: Some(7),
            ..Default::default()
        };
        let body = build_generate_request("gpt-image-2", "a sunset", &params).unwrap();
        assert_eq!(body["size"], "1024x1024");
        assert_eq!(body["quality"], "high");
        assert_eq!(body["n"], 2);
        assert_eq!(body["output_format"], "webp");
        assert_eq!(body["seed"], 7);
    }

    /// `size` is free text an agent wrote. Each rule caught here is a 400 the agent
    /// would otherwise have to interpret, and every message says what to change.
    #[test]
    fn a_size_gpt_image_2_cannot_serve_is_refused_with_the_rule() {
        let bad = |size: &str| {
            let params = ImageParams { size: Some(size.into()), ..Default::default() };
            build_generate_request("gpt-image-2", "a cat", &params).unwrap_err().to_string()
        };
        assert!(bad("1000x1000").contains("multiples of 16"), "{}", bad("1000x1000"));
        assert!(bad("4096x1024").contains("long edge"), "{}", bad("4096x1024"));
        assert!(bad("512x512").contains("total pixels"), "{}", bad("512x512"));
        assert!(bad("3072x512").contains("3:1"), "{}", bad("3072x512"));

        // Valid, and the non-WxH tokens are the API's to judge, not ours.
        for ok in ["1024x1024", "1536x1024", "auto", ""] {
            let params = ImageParams { size: Some(ok.into()), ..Default::default() };
            assert!(build_generate_request("gpt-image-2", "a cat", &params).is_ok(), "{ok}");
        }
        // Another model's rules are not gpt-image-2's.
        let params = ImageParams { size: Some("1000x1000".into()), ..Default::default() };
        assert!(build_generate_request("gpt-image-1.5", "a cat", &params).is_ok());
    }

    /// The API accepts `background: transparent` on gpt-image-2 and does not deliver
    /// it. An agent that asked for a cutout must find out here, not from the picture.
    #[test]
    fn transparency_is_refused_on_the_model_that_cannot_do_it() {
        let params = ImageParams { background: Some("transparent".into()), ..Default::default() };
        let err = build_generate_request("gpt-image-2", "a logo", &params).unwrap_err().to_string();
        assert!(err.contains("gpt-image-1.5"), "{err}");
        assert!(build_generate_request("gpt-image-1.5", "a logo", &params).is_ok());
    }

    #[test]
    fn a_knob_this_wire_lacks_is_refused_by_name() {
        let params = ImageParams { watermark: Some(true), ..Default::default() };
        let err = build_generate_request("gpt-image-2", "a cat", &params).unwrap_err().to_string();
        assert!(err.contains("doubao"), "{err}");
    }

    #[tokio::test]
    async fn base64_becomes_bytes_with_a_sniffed_type() {
        let jpg = b"\xff\xd8\xff\xe0\x00\x10JFIF";
        let datum = ImageDatum {
            b64_json: Some(base64::engine::general_purpose::STANDARD.encode(jpg)),
            url: None,
            size: None,
        };
        let img = datum.into_image(&reqwest::Client::new()).await.unwrap();
        assert_eq!(img.bytes.as_ref(), jpg);
        assert_eq!(img.mime, "image/jpeg");
    }
}
