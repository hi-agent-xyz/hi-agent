//! One model request from the host — no session, no memory, no tools of its own.
//!
//! The check, the audit, the reading of a reply and replay all ask a model the same kind
//! of question: here is a fixed standard, here is a case, answer. None of them needs a
//! thread, so none of them opens one: a codex session would carry a tool surface, a
//! compaction policy and a subprocess for what is a single Responses API call. The
//! endpoint and key are the agent's own ([`AgentConfig`]); the model is a tunable per job,
//! and unset it is the model the agent runs on.

use std::path::Path;
use std::time::Duration;

use anyhow::Context as _;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::foundation::config::{AgentConfig, tunables};

/// Connect budget. The request's own limit is the caller's, because the check and the
/// audit can afford very different waits.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct Judge {
    client: reqwest::Client,
    endpoint: String,
    key: String,
    model: String,
}

impl std::fmt::Debug for Judge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Judge").field("endpoint", &self.endpoint).field("model", &self.model).finish()
    }
}

impl Judge {
    /// The judge for one job, or `None` on an install with no model configured — the job
    /// then does not run, which is how every judge fails: open.
    ///
    /// `model_key` names the `app_settings` tunable that picks the model. Unset, it is the
    /// agent's own model: which model judges is an open question in the design
    /// (`docs/arch/legibility.md` § Open), and the one answer that needs no second
    /// credential is the one already paid for.
    pub fn resolve(data_dir: &Path, model_key: &str) -> Option<Self> {
        let config = AgentConfig::resolve(data_dir);
        if !config.is_configured() {
            return None;
        }
        let model = tunables::get(model_key).or_else(|| config.reaction_model())?;
        Some(Self::new(&config.upstream_base_url, &config.upstream_key, &model))
    }

    pub fn new(base_url: &str, key: &str, model: &str) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            client,
            // The provider base is what codex's `wire_api = "responses"` appends
            // `/responses` to; this is the same URL it reaches.
            endpoint: format!("{}/responses", base_url.trim_end_matches('/')),
            key: key.to_string(),
            model: model.to_string(),
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// The same judge on another model.
    pub fn with_model(&self, model: &str) -> Self {
        Self { model: model.to_string(), ..self.clone() }
    }

    /// Ask once: `instructions` is the fixed part (the standard and the rubric — a stable
    /// prefix, so an upstream that caches prefixes caches it), `input` is the case.
    /// Returns the answer's text.
    pub async fn ask(&self, instructions: &str, input: &str, limit: Duration) -> anyhow::Result<String> {
        let body = json!({
            "model": self.model,
            "instructions": instructions,
            "input": [{
                "role": "user",
                "content": [{ "type": "input_text", "text": input }],
            }],
            "store": false,
        });
        let reply = self.respond(&body, limit).await?;
        output_text(&reply).context("the answer carried no text")
    }

    /// One raw Responses request, for a caller that needs more than text back — replay
    /// declares tools and reads the calls. `body` is sent as given, with the model set.
    pub async fn respond(&self, body: &Value, limit: Duration) -> anyhow::Result<Value> {
        let mut body = body.clone();
        body["model"] = json!(self.model);
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.key)
            .timeout(limit)
            .json(&body)
            .send()
            .await
            .context("sending the judge request")?;
        let status = response.status();
        let text = response.text().await.context("reading the judge response")?;
        if !status.is_success() {
            let head: String = text.chars().take(300).collect();
            anyhow::bail!("upstream answered {status}: {head}");
        }
        serde_json::from_str(&text).context("the judge response was not JSON")
    }
}

/// The assistant text of a Responses reply: every `output_text` part of every output
/// item, joined. `None` when there is none.
pub fn output_text(reply: &Value) -> Option<String> {
    if let Some(t) = reply.get("output_text").and_then(Value::as_str)
        && !t.trim().is_empty()
    {
        return Some(t.trim().to_string());
    }
    let mut acc = String::new();
    for item in reply.get("output").and_then(Value::as_array).into_iter().flatten() {
        for part in item.get("content").and_then(Value::as_array).into_iter().flatten() {
            if part.get("type").and_then(Value::as_str) == Some("output_text")
                && let Some(t) = part.get("text").and_then(Value::as_str)
            {
                acc.push_str(t);
            }
        }
    }
    let acc = acc.trim();
    (!acc.is_empty()).then(|| acc.to_string())
}

/// The JSON object an answer carries, read leniently: a model asked for JSON only still
/// wraps it in a fence or a sentence often enough that insisting would turn a usable
/// verdict into an error, and an error sends the message unread.
pub fn json_object<T: DeserializeOwned>(text: &str) -> Option<T> {
    if let Ok(v) = serde_json::from_str(text.trim()) {
        return Some(v);
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(text.get(start..=end)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_is_read_from_either_shape_of_reply() {
        let flat = json!({ "output_text": " pass " });
        assert_eq!(output_text(&flat).as_deref(), Some("pass"));
        let nested = json!({ "output": [
            { "type": "reasoning", "summary": [] },
            { "type": "message", "content": [
                { "type": "output_text", "text": "{\"verdict\":" },
                { "type": "output_text", "text": "\"pass\"}" },
            ]},
        ]});
        assert_eq!(output_text(&nested).as_deref(), Some("{\"verdict\":\"pass\"}"));
        assert_eq!(output_text(&json!({ "output": [] })), None);
    }

    #[test]
    fn a_verdict_is_found_inside_a_fence_or_a_sentence() {
        #[derive(serde::Deserialize, PartialEq, Debug)]
        struct V {
            verdict: String,
        }
        let want = Some(V { verdict: "revise".into() });
        assert_eq!(json_object::<V>("{\"verdict\":\"revise\"}"), want);
        assert_eq!(json_object::<V>("```json\n{\"verdict\":\"revise\"}\n```"), want);
        assert_eq!(json_object::<V>("Here you go: {\"verdict\":\"revise\"} done"), want);
        assert_eq!(json_object::<V>("no verdict here"), None);
    }

    #[test]
    fn the_endpoint_is_the_one_codex_reaches() {
        let j = Judge::new("https://songguo.example/v1/", "secret-key", "m");
        assert_eq!(j.endpoint, "https://songguo.example/v1/responses");
        assert!(!format!("{j:?}").contains("secret-key"), "the key never reaches a log");
    }
}
