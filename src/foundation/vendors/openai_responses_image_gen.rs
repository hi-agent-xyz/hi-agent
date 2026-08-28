//! OpenAI image generation over the **Responses** API.
//!
//!   POST {base}/responses   (JSON)   prompt → image
//!   Authorization: Bearer <api_key>
//!
//! The second wire for `text-to-image`, beside [`super::openai_image_gen`]. Same
//! capability, same models, different protocol: here the picture is produced by a
//! *mainline* model holding an `image_generation` tool rather than by a call to an
//! image endpoint.
//!
//! **The caller still names the image model, because the tool takes one.** That is the
//! fact this wire turns on — `tools: [{ "type": "image_generation", "model":
//! "gpt-image-2" }]` — and without it this adapter could not exist under a capability
//! whose contract is "you get the model you asked for". Left unset, the mainline model
//! picks the image model itself, which is exactly the outcome the contract forbids.
//!
//! Two things follow from the tool living inside a *turn*, and neither is a defect to
//! fix here:
//!
//! - **A carrier model is required** and is not the image model. It is configuration,
//!   never a guess: a provider that names no carrier is dropped at startup rather than
//!   defaulting to some model we picked, which would go stale and bill silently.
//! - **The carrier bills its own tokens** on top of the image. That is the price of
//!   this wire, and it is why [`super::openai_image_gen`] remains the one to prefer
//!   when a caller just wants a picture.
//!
//! `tool_choice` forces the tool, so a turn cannot come back as chat about the
//! picture it declined to draw.
//!
//! **Editing is not implemented here.** The tool has `action: "edit"` and an
//! `input_image_mask`, so it is reachable in principle; it needs the source image as an
//! `input_image` content part, which is a second wire shape and a second set of
//! refusals. `image_to_image` stays on the images wire, where it is already real.

use anyhow::Context;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::body::capabilities::image_gen::{GeneratedImage, ImageParams, sniff_mime};

const DEFAULT_API_BASE: &str = "https://api.openai.com/v1";

/// One configured Responses-wire provider: the endpoint, the key, and the mainline
/// model that carries the tool.
pub struct Config {
    api_key: String,
    responses: String,
    /// The mainline model the `image_generation` tool rides on. Not the image model —
    /// see the module docs. Never defaulted.
    carrier: String,
}

impl Config {
    /// `base_url`, when set, is the gateway's **full** responses endpoint — the shape
    /// the broker mints for every other wire, so the three stay symmetrical.
    pub fn new(api_key: &str, base_url: Option<&str>, carrier: &str) -> Self {
        let responses = match base_url.map(str::trim).filter(|b| !b.is_empty()) {
            Some(base) => base.trim_end_matches('/').to_string(),
            None => format!("{DEFAULT_API_BASE}/responses"),
        };
        Self {
            api_key: api_key.trim().to_string(),
            responses,
            carrier: carrier.trim().to_string(),
        }
    }
}

/// Build the `responses` body. Pure, so the wire shape is unit-testable.
///
/// The refusals are the ones this *wire* cannot honour, each naming the wire that can.
/// A knob silently dropped here would be recorded as a picture drawn to spec.
fn build_request(
    carrier: &str,
    model: &str,
    prompt: &str,
    params: &ImageParams,
) -> anyhow::Result<Value> {
    super::openai_image_gen::check_size(model, params.size.as_deref())?;
    if params.n.is_some_and(|n| n > 1) {
        anyhow::bail!(
            "the responses wire draws one image per turn — ask for `n` images on the \
             images wire, or make {} calls",
            params.n.unwrap_or(1)
        );
    }
    if params.seed.is_some() {
        anyhow::bail!("`seed` is not a knob on the image_generation tool — use the images wire");
    }
    if params.watermark.is_some() {
        anyhow::bail!(
            "`watermark` is not a knob on the OpenAI wires — name a doubao model to \
             control watermarking"
        );
    }

    let mut tool = json!({ "type": "image_generation", "model": model });
    let obj = tool.as_object_mut().expect("json object");
    if let Some(size) = &params.size {
        obj.insert("size".into(), json!(size));
    }
    if let Some(quality) = &params.quality {
        obj.insert("quality".into(), json!(quality));
    }
    if let Some(background) = &params.background {
        obj.insert("background".into(), json!(background));
    }
    if let Some(output_format) = &params.output_format {
        obj.insert("output_format".into(), json!(output_format));
    }

    Ok(json!({
        "model": carrier,
        "input": prompt,
        "tools": [tool],
        // Forced, not offered: the caller asked for a picture, so a turn that answers
        // in prose has failed rather than declined.
        "tool_choice": { "type": "image_generation" },
    }))
}

pub async fn generate(
    client: &reqwest::Client,
    cfg: &Config,
    model: &str,
    prompt: &str,
    params: &ImageParams,
) -> anyhow::Result<Vec<GeneratedImage>> {
    let body = build_request(&cfg.carrier, model, prompt, params)?;
    let resp = client
        .post(&cfg.responses)
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send()
        .await
        .context("openai responses image-gen request failed")?;

    let status = resp.status();
    let text = resp.text().await.context("reading openai responses image-gen response")?;
    if !status.is_success() {
        anyhow::bail!("openai responses image-gen HTTP {status}: {text}");
    }
    read_images(&text)
}

/// Pull every finished picture out of a Responses turn.
///
/// The output array is heterogeneous — reasoning, messages, tool calls — and only
/// `image_generation_call` items carry a picture, in `result`, as bare base64 with no
/// `data:` prefix. An item still `in_progress`/`generating` has no result yet and is
/// not an error; a `failed` one is, and says so with whatever the turn said.
fn read_images(text: &str) -> anyhow::Result<Vec<GeneratedImage>> {
    let parsed: ResponsesTurn = serde_json::from_str(text)
        .with_context(|| format!("parsing openai responses image-gen response: {text}"))?;

    let mut out = Vec::new();
    let mut failed = false;
    for item in &parsed.output {
        if item.kind.as_deref() != Some("image_generation_call") {
            continue;
        }
        match item.result.as_deref().filter(|s| !s.is_empty()) {
            Some(b64) => {
                let bytes = bytes::Bytes::from(
                    base64::engine::general_purpose::STANDARD
                        .decode(b64)
                        .context("decoding openai responses image base64")?,
                );
                let mime = sniff_mime(&bytes);
                out.push(GeneratedImage { bytes, mime, size: None });
            }
            None => failed |= item.status.as_deref() == Some("failed"),
        }
    }

    if out.is_empty() {
        // The turn came back without a picture. Say which of the two ways it did:
        // the tool ran and failed, or the model answered in prose despite the forced
        // tool_choice. They point at different fixes, and "no images" points at neither.
        if failed {
            anyhow::bail!("openai responses image-gen: the image_generation call failed");
        }
        anyhow::bail!(
            "openai responses image-gen returned no image (the turn answered without \
             drawing): {}",
            parsed.answer_text().unwrap_or_else(|| "no text either".to_string())
        );
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct ResponsesTurn {
    #[serde(default)]
    output: Vec<OutputItem>,
}

impl ResponsesTurn {
    /// Whatever the turn said instead of drawing, for the error above.
    fn answer_text(&self) -> Option<String> {
        let text: String = self
            .output
            .iter()
            .filter(|i| i.kind.as_deref() == Some("message"))
            .flat_map(|i| i.content.iter())
            .filter_map(|c| c.text.as_deref())
            .collect::<Vec<_>>()
            .join(" ");
        let text = text.trim();
        (!text.is_empty()).then(|| text.chars().take(300).collect())
    }
}

#[derive(Debug, Deserialize)]
struct OutputItem {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    status: Option<String>,
    /// Base64 image on an `image_generation_call`.
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    content: Vec<ContentPart>,
}

#[derive(Debug, Deserialize)]
struct ContentPart {
    #[serde(default)]
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The whole reason this wire can exist under this capability.** The image model
    /// goes in the tool and the carrier goes on top; swapping them sends `gpt-image-2`
    /// somewhere it is not a model, and omitting the tool's model lets the carrier pick
    /// — which is the one outcome the capability's contract forbids.
    #[test]
    fn the_named_model_rides_in_the_tool_and_the_carrier_on_top() {
        let params = ImageParams {
            size: Some("1024x1024".into()),
            quality: Some("high".into()),
            output_format: Some("png".into()),
            ..Default::default()
        };
        let body = build_request("gpt-5.4", "gpt-image-2", "a red bicycle", &params).unwrap();

        assert_eq!(body["model"], "gpt-5.4", "the carrier hosts the turn");
        assert_eq!(body["input"], "a red bicycle");
        let tool = &body["tools"][0];
        assert_eq!(tool["type"], "image_generation");
        assert_eq!(tool["model"], "gpt-image-2", "the caller's model, not the carrier's pick");
        assert_eq!(tool["size"], "1024x1024");
        assert_eq!(tool["quality"], "high");
        assert_eq!(tool["output_format"], "png");
        // Forced, or a turn may answer in prose and call that success.
        assert_eq!(body["tool_choice"]["type"], "image_generation");
    }

    /// An unset knob must not appear at all, so the API's own default applies rather
    /// than one we invented by serializing a `None`.
    #[test]
    fn unset_knobs_are_absent_not_null() {
        let body = build_request("gpt-5.4", "gpt-image-2", "a cat", &ImageParams::default()).unwrap();
        let tool = body["tools"][0].as_object().unwrap();
        for k in ["size", "quality", "background", "output_format"] {
            assert!(!tool.contains_key(k), "{k} was sent unasked");
        }
    }

    /// Each refusal names the wire that *can* do it. A knob dropped in silence would be
    /// recorded as a picture drawn to spec.
    #[test]
    fn a_knob_this_wire_lacks_is_refused_by_name() {
        let n = ImageParams { n: Some(3), ..Default::default() };
        let err = build_request("gpt-5.4", "gpt-image-2", "x", &n).unwrap_err().to_string();
        assert!(err.contains("images wire"), "{err}");

        let seed = ImageParams { seed: Some(7), ..Default::default() };
        let err = build_request("gpt-5.4", "gpt-image-2", "x", &seed).unwrap_err().to_string();
        assert!(err.contains("seed"), "{err}");

        let wm = ImageParams { watermark: Some(true), ..Default::default() };
        let err = build_request("gpt-5.4", "gpt-image-2", "x", &wm).unwrap_err().to_string();
        assert!(err.contains("doubao"), "{err}");

        // n = 1 is what everyone means by "one picture" and must not trip the refusal.
        let one = ImageParams { n: Some(1), ..Default::default() };
        assert!(build_request("gpt-5.4", "gpt-image-2", "x", &one).is_ok());

        // The model's own size rules are the images wire's, and they still apply.
        let bad = ImageParams { size: Some("1000x1000".into()), ..Default::default() };
        let err = build_request("gpt-5.4", "gpt-image-2", "x", &bad).unwrap_err().to_string();
        assert!(err.contains("multiples of 16"), "{err}");
    }

    /// The picture is one item among many in a heterogeneous output array — reasoning
    /// and messages ride alongside it, and reading position instead of type would pick
    /// up whichever the model happened to emit first.
    #[test]
    fn the_image_is_found_among_the_other_output_items() {
        let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\n");
        let body = format!(
            r#"{{"output":[
                {{"type":"reasoning","summary":[]}},
                {{"type":"image_generation_call","id":"ig_1","status":"completed","result":"{png}"}},
                {{"type":"message","content":[{{"type":"output_text","text":"here you go"}}]}}
            ]}}"#
        );
        let images = read_images(&body).unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].mime, "image/png", "the mime comes from the bytes");
    }

    /// "The tool failed" and "the model answered instead of drawing" have different
    /// fixes, so they must not share an error. The prose is quoted because it is
    /// usually a refusal worth reading.
    #[test]
    fn a_turn_with_no_picture_says_which_way_it_went() {
        let failed = r#"{"output":[{"type":"image_generation_call","id":"i","status":"failed"}]}"#;
        let err = read_images(failed).unwrap_err().to_string();
        assert!(err.contains("failed"), "{err}");

        let chatted = r#"{"output":[{"type":"message","content":[{"text":"I can't draw that"}]}]}"#;
        let err = read_images(chatted).unwrap_err().to_string();
        assert!(err.contains("answered without"), "{err}");
        assert!(err.contains("I can't draw that"), "the refusal itself is the useful part: {err}");
    }
}
