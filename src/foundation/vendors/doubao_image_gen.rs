//! Volcengine Ark (Doubao) image generation.
//!
//! Endpoint (per <https://www.volcengine.com/docs/82379/2375486>):
//!
//!   POST {api_base}/images/generations            (synchronous)
//!   Authorization: Bearer <api_key>
//!
//! `api_base` defaults to the **plan** endpoint
//! `https://ark.cn-beijing.volces.com/api/plan/v3` — deliberately *not* the
//! plain `/api/v3`, which the docs warn bills as extra. Override only if you are
//! on a different region or billing arrangement.
//!
//! Generation rides the OpenAI-compatible `images/generations` shape
//! (`prompt` / `size` / `response_format` / `seed` / `watermark`, response is a
//! `data` array of `url` or `b64_json`).
//!
//! **Editing is the same endpoint, plus an `image` field.** Ark publishes no
//! `/images/edits`; `POST {api_base}/images/generations` carries the source picture as
//! an `image` member of the same JSON body and returns the edited one. Confirmed live
//! on 2026-08-31 against the plan endpoint with `doubao-seedream-5.0-lite`: a red
//! circle went in, `change the circle to green` came back green, composition intact.
//! Note `size` here is Ark's own vocabulary — `WIDTHxHEIGHT`, `2k`, `3k`, `4k`; `1K` is
//! rejected — and is passed through rather than validated, since the accepted set is
//! the vendor's to change.

use anyhow::Context;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::body::capabilities::image_gen::{
    GeneratedImage, ImageParams, SourceImage, sniff_mime,
};

/// The plan endpoint. The bare `/api/v3` variant bills as extra (per the docs),
/// so it is intentionally not the default.
const DEFAULT_API_BASE: &str = "https://ark.cn-beijing.volces.com/api/plan/v3";

/// One configured Ark image provider: where to post, and with what key. No model and
/// no HTTP client — the model comes per call (the agent chooses it) and the client is
/// shared by the capability.
pub struct Config {
    api_key: String,
    endpoint: String,
}

impl Config {
    /// `base_url`, when set, is the gateway's **full** image-generations endpoint
    /// (songguo, whose path differs from the vendor's native one) and is used
    /// verbatim; with no `base_url` (BYOK) the vendor's own endpoint is used.
    pub fn new(api_key: &str, base_url: Option<&str>) -> Self {
        let endpoint = match base_url.map(str::trim).filter(|b| !b.is_empty()) {
            Some(base) => base.trim_end_matches('/').to_string(),
            None => format!("{DEFAULT_API_BASE}/images/generations"),
        };
        Self { api_key: api_key.trim().to_string(), endpoint }
    }
}

/// Build the `images/generations` body, or explain which requested knob this wire has
/// no place to put it.
///
/// Pure (no I/O) so the wire shape is unit-testable without a network call. Optional
/// knobs are omitted when unset so the model applies its own defaults.
///
/// **Refusing beats dropping.** An agent that asked for a transparent background and
/// silently got an opaque one has been told the wrong thing about its own work; an
/// agent told "not on this wire, gpt-image-1.5 can" picks again and gets it.
fn build_request(model: &str, prompt: &str, params: &ImageParams) -> anyhow::Result<Value> {
    if params.background.is_some() {
        anyhow::bail!(
            "`background` is not a knob on the doubao wire — name a gpt-image model for a \
             transparent or forced-opaque background"
        );
    }
    if params.quality.is_some() {
        anyhow::bail!(
            "`quality` is not a knob on the doubao wire — pick the model that matches the \
             quality you want, or name a gpt-image model to set it per call"
        );
    }
    if params.output_format.is_some() {
        anyhow::bail!(
            "`output_format` is not a knob on the doubao wire — it returns its own format; \
             name a gpt-image model to choose png/jpeg/webp"
        );
    }
    if params.n.is_some_and(|n| n > 1) {
        anyhow::bail!(
            "`n` above 1 is not wired for doubao — call again for another variant, or name a \
             gpt-image model to batch"
        );
    }

    // Always base64: the alternative is a hosted URL that expires upstream, and every
    // caller here immediately persists the bytes anyway. One round trip, no race.
    let mut body = json!({
        "model": model,
        "prompt": prompt,
        "response_format": "b64_json",
    });
    let obj = body.as_object_mut().expect("json object");
    if let Some(size) = &params.size {
        obj.insert("size".into(), json!(size));
    }
    if let Some(seed) = params.seed {
        obj.insert("seed".into(), json!(seed));
    }
    if let Some(watermark) = params.watermark {
        obj.insert("watermark".into(), json!(watermark));
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
    post(client, cfg, build_request(model, prompt, params)?).await
}

/// Edit `source` under `prompt` — the same call as [`generate`] with the picture added.
///
/// The source always goes up as a data URL, including when the caller had a URL: Ark
/// may well accept a bare link, but that is untested here and a link it cannot reach
/// fails as the model's refusal rather than as a fetch we could have done ourselves.
pub async fn edit(
    client: &reqwest::Client,
    cfg: &Config,
    model: &str,
    source: &SourceImage,
    prompt: &str,
    params: &ImageParams,
) -> anyhow::Result<Vec<GeneratedImage>> {
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
                .with_context(|| format!("reading source image {url}"))?;
            let mime = sniff_mime(&bytes);
            (bytes, mime)
        }
    };
    let data_url = format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    );

    post(client, cfg, build_edit_request(model, prompt, &data_url, params)?).await
}

/// The edit body: the generation body with the source picture added. Pure, so the one
/// thing that distinguishes an edit from a draw on this wire is unit-testable.
fn build_edit_request(
    model: &str,
    prompt: &str,
    image: &str,
    params: &ImageParams,
) -> anyhow::Result<Value> {
    let mut body = build_request(model, prompt, params)?;
    body.as_object_mut().expect("json object").insert("image".into(), json!(image));
    Ok(body)
}

/// The one request both calls make: same endpoint, same key, same response shape.
async fn post(
    client: &reqwest::Client,
    cfg: &Config,
    body: Value,
) -> anyhow::Result<Vec<GeneratedImage>> {
    let resp = client
        .post(&cfg.endpoint)
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send()
        .await
        .context("doubao image-gen request failed")?;

    let status = resp.status();
    let text = resp.text().await.context("reading doubao image-gen response")?;
    if !status.is_success() {
        anyhow::bail!("doubao image-gen HTTP {status}: {text}");
    }

    let parsed: ImageResponse = serde_json::from_str(&text)
        .with_context(|| format!("parsing doubao image-gen response: {text}"))?;
    if parsed.data.is_empty() {
        anyhow::bail!("doubao image-gen returned no images");
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
    url: Option<String>,
    #[serde(default)]
    b64_json: Option<String>,
    #[serde(default)]
    size: Option<String>,
}

impl ImageDatum {
    /// Bytes, whichever way they arrived. We ask for base64, but a gateway is free to
    /// answer with a URL and the caller must not have to care — that is the whole
    /// reason `GeneratedImage` carries bytes.
    async fn into_image(self, client: &reqwest::Client) -> anyhow::Result<GeneratedImage> {
        let bytes = if let Some(b64) = self.b64_json.as_deref().filter(|s| !s.is_empty()) {
            bytes::Bytes::from(
                base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .context("decoding doubao image-gen base64")?,
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
            anyhow::bail!("doubao image-gen returned a datum with neither bytes nor a url");
        };
        let mime = sniff_mime(&bytes);
        Ok(GeneratedImage { bytes, mime, size: self.size })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_base_is_the_plan_endpoint_not_plain_v3() {
        // The docs warn that /api/v3 bills as extra; the default must be plan/v3.
        assert!(DEFAULT_API_BASE.contains("/api/plan/v3"));
        assert!(!DEFAULT_API_BASE.contains("/api/v3/"));
        let cfg = Config::new("k", None);
        assert!(cfg.endpoint.ends_with("/api/plan/v3/images/generations"), "{}", cfg.endpoint);
    }

    /// A gateway's endpoint is a full URL with its own path; appending ours to it
    /// would post to a route that does not exist.
    #[test]
    fn a_gateway_endpoint_is_used_verbatim() {
        let cfg = Config::new("k", Some("https://gw.example/v1/images/generations/"));
        assert_eq!(cfg.endpoint, "https://gw.example/v1/images/generations");
    }

    #[test]
    fn the_model_comes_per_call_and_unset_knobs_are_omitted() {
        let body =
            build_request("doubao-seedream-5.0-lite", "a red bicycle", &ImageParams::default())
                .unwrap();
        assert_eq!(body["model"], "doubao-seedream-5.0-lite");
        assert_eq!(body["prompt"], "a red bicycle");
        // Bytes, always — a URL that expires is not a picture anybody kept.
        assert_eq!(body["response_format"], "b64_json");
        let obj = body.as_object().unwrap();
        assert!(!obj.contains_key("size"));
        assert!(!obj.contains_key("seed"));
        assert!(!obj.contains_key("watermark"));
    }

    #[test]
    fn set_knobs_ride_along() {
        let params = ImageParams {
            size: Some("2K".into()),
            seed: Some(42),
            watermark: Some(false),
            ..Default::default()
        };
        let body = build_request("doubao-seedream-5.0-lite", "a sunset", &params).unwrap();
        assert_eq!(body["size"], "2K");
        assert_eq!(body["seed"], 42);
        assert_eq!(body["watermark"], false);
    }

    /// The refusal is the contract: a knob this wire cannot honour must come back as
    /// an error naming what can, never as a quietly different picture.
    #[test]
    fn a_knob_this_wire_lacks_is_refused_by_name() {
        for params in [
            ImageParams { background: Some("transparent".into()), ..Default::default() },
            ImageParams { quality: Some("high".into()), ..Default::default() },
            ImageParams { output_format: Some("webp".into()), ..Default::default() },
            ImageParams { n: Some(4), ..Default::default() },
        ] {
            let err = build_request("doubao-seedream-5.0-lite", "a cat", &params)
                .unwrap_err()
                .to_string();
            assert!(err.contains("gpt-image"), "must name a model that can: {err}");
        }
        // n=1 is what everyone means by "one image" — not a refusal.
        let params = ImageParams { n: Some(1), ..Default::default() };
        assert!(build_request("doubao-seedream-5.0-lite", "a cat", &params).is_ok());
    }

    /// **Editing on this wire is the generation call with one more field.** Ark
    /// publishes no `/images/edits`; the source rides the same JSON body as `image`,
    /// and everything else about the request is unchanged. Verified live on
    /// 2026-08-31 against the plan endpoint: a red circle in, a green one back.
    #[test]
    fn an_edit_is_the_generation_body_plus_the_source_image() {
        let params = ImageParams { size: Some("2k".into()), ..Default::default() };
        let body =
            build_edit_request("doubao-seedream-5.0-lite", "make it green", "data:image/png;base64,AAA", &params)
                .unwrap();
        assert_eq!(body["model"], "doubao-seedream-5.0-lite");
        assert_eq!(body["prompt"], "make it green");
        assert_eq!(body["image"], "data:image/png;base64,AAA");
        assert_eq!(body["size"], "2k", "Ark's own size vocabulary, passed through");
        assert_eq!(body["response_format"], "b64_json");
        // The endpoint is the generations one — there is no second URL to get wrong.
        let cfg = Config::new("k", None);
        assert!(cfg.endpoint.ends_with("/images/generations"), "{}", cfg.endpoint);
    }

    #[test]
    fn parses_a_response_carrying_either_form() {
        let raw = r#"{
            "data": [
                { "url": "https://example.com/a.png", "size": "1024x1024" },
                { "b64_json": "AAAA" }
            ]
        }"#;
        let parsed: ImageResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.data[0].url.as_deref(), Some("https://example.com/a.png"));
        assert_eq!(parsed.data[0].size.as_deref(), Some("1024x1024"));
        assert_eq!(parsed.data[1].b64_json.as_deref(), Some("AAAA"));
        assert!(parsed.data[1].url.is_none());
    }

    #[tokio::test]
    async fn base64_becomes_bytes_with_a_sniffed_type() {
        let png = b"\x89PNG\r\n\x1a\n\x00rest";
        let datum = ImageDatum {
            url: None,
            b64_json: Some(base64::engine::general_purpose::STANDARD.encode(png)),
            size: Some("1024x1024".into()),
        };
        let img = datum.into_image(&reqwest::Client::new()).await.unwrap();
        assert_eq!(img.bytes.as_ref(), png);
        assert_eq!(img.mime, "image/png");
        assert_eq!(img.size.as_deref(), Some("1024x1024"));
    }
}
