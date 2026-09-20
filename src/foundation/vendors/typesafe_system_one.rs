//! TypeSafe **System One** (`jev`) — typed answers with calibrated probabilities.
//!
//! Endpoint:
//!
//!   POST {base}/v1/systemone        (base default <https://api.typesafe.ai>)
//!   Authorization: Bearer <api_key>
//!
//! One request carries one `state` and a **map of named questions**; the reply carries
//! one answer per question key plus token usage. There is no text to generate and
//! nothing to coerce into a schema — the model answers the question types it knows and
//! reports how sure it is as a number that means the same thing on the next call.
//!
//!   { "state": "…", "model": "jev-latest",
//!     "questions": { "is_urgent": { "type": "noul", "instructions": "…" } } }
//!
//!   { "model": "jev-1.13.0",
//!     "answers": { "is_urgent": { "type": "noul", "noul": 0.95 } },
//!     "usage": { "input_tokens": 296, "output_tokens": 20 } }
//!
//! **The request is passed through, not normalized.** There is exactly one System One
//! vendor, so the wire shape *is* the semantic interface; a vocabulary of our own here
//! would be one no second vendor is asking for. The capability's types
//! ([`crate::body::capabilities::decision`]) mirror the reply for the same reason.
//!
//! The answer is read **self-describingly**: the value lives at the key the answer's
//! own `type` field names (`"type":"noul"` → the `noul` member), so a type we have
//! never heard of keeps its raw object instead of being guessed at. That is the rule
//! songguo's parser follows, for the same reason — the vendor owns this list and grows
//! it without asking us.

use std::time::Duration;

use anyhow::Context;
use serde_json::{Map, Value, json};

use crate::body::capabilities::decision::{Answer, Reply, Usage};

const DEFAULT_API_BASE: &str = "https://api.typesafe.ai";
/// The one route. Appended to the vendor's own base; a configured `base_url` (the
/// managed gateway, whose path differs) is used verbatim instead.
const ENDPOINT_PATH: &str = "/v1/systemone";
/// The rolling alias, so an install tracks the current `jev` without a redeploy.
const DEFAULT_MODEL: &str = "jev-latest";
/// 70–500 ms is the documented range even for a large question map, so a minute is
/// already far past "this is not coming back".
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// How many times a 429/529 is re-sent before the error goes back to the caller. Both
/// mean *come back shortly* rather than *this request is wrong*, and the whole point of
/// the capability is a sweep of hundreds of questions — failing the sweep on one busy
/// moment upstream costs far more than waiting a second.
const MAX_ATTEMPTS: u32 = 3;
/// Doubled after each retry: 0.5 s, then 1 s. A `Retry-After` header beats it.
const FIRST_BACKOFF: Duration = Duration::from_millis(500);

pub struct Config {
    client: reqwest::Client,
    api_key: String,
    endpoint: String,
    model: String,
}

impl Config {
    /// Resolve config from the credential store. `key` is the vendor API key
    /// (required); `base_url`, when set, is the gateway's **full** endpoint and is used
    /// verbatim; with no `base_url` (BYOK) the vendor's own route is used. `model` is
    /// the default for calls that name none — no env.
    pub fn from_store(
        key: Option<&str>,
        base_url: Option<&str>,
        model: Option<&str>,
    ) -> anyhow::Result<Self> {
        let api_key = key
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .ok_or_else(|| anyhow::anyhow!("system one (typesafe) requires an API key"))?
            .to_string();
        let endpoint = match base_url.map(str::trim).filter(|b| !b.is_empty()) {
            Some(base) => base.trim_end_matches('/').to_string(),
            None => format!("{DEFAULT_API_BASE}{ENDPOINT_PATH}"),
        };
        let model = match model.map(str::trim).filter(|m| !m.is_empty()) {
            Some(m) => m.to_string(),
            None => DEFAULT_MODEL.to_string(),
        };

        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .context("building typesafe system one HTTP client")?;

        Ok(Self { client, api_key, endpoint, model })
    }
}

/// Build the request body. Pure (no I/O) so the wire shape is unit-testable without a
/// network call. `model` overrides the configured default for this one call.
fn build_request(
    cfg: &Config,
    state: &Value,
    questions: &Map<String, Value>,
    model: Option<&str>,
) -> Value {
    let model = model.map(str::trim).filter(|m| !m.is_empty()).unwrap_or(&cfg.model);
    json!({
        "state": state,
        "model": model,
        "questions": questions,
    })
}

/// Ask every question in `questions` about one `state`, in one round trip.
pub async fn ask(
    cfg: &Config,
    state: &Value,
    questions: &Map<String, Value>,
    model: Option<&str>,
) -> anyhow::Result<Reply> {
    let body = build_request(cfg, state, questions, model);

    let mut backoff = FIRST_BACKOFF;
    for attempt in 1..=MAX_ATTEMPTS {
        let resp = cfg
            .client
            .post(&cfg.endpoint)
            .bearer_auth(&cfg.api_key)
            .json(&body)
            .send()
            .await
            .context("typesafe system one request failed")?;

        let status = resp.status();
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        let text = resp.text().await.context("reading typesafe system one response")?;

        if status.is_success() {
            return parse_response(&text);
        }
        // 429/529 say *later*; everything else says *this request*. Retrying the second
        // kind only spends the same money on the same answer.
        if !transient(status.as_u16()) || attempt == MAX_ATTEMPTS {
            anyhow::bail!("{}", explain(status.as_u16(), &text));
        }
        tokio::time::sleep(retry_after.unwrap_or(backoff)).await;
        backoff *= 2;
    }
    // Unreachable: the loop either returns or bails on its last attempt.
    anyhow::bail!("typesafe system one: no attempt was made")
}

/// Whether a failing status means *try again shortly* rather than *this request is
/// wrong*. 429 is the rate limit; 529 is the vendor's overload code.
fn transient(status: u16) -> bool {
    matches!(status, 429 | 529)
}

/// The error text, naming the cause rather than reprinting a status number. Each of
/// these has a different thing for the reader to do, and the body carries the vendor's
/// own detail — which for a 422 is the question it could not read.
fn explain(status: u16, body: &str) -> String {
    let detail = body.trim();
    let why = match status {
        401 => "the API key was rejected — check the decision key in Settings",
        422 => "the questions were refused — a `type` it does not know, a `criteria` \
                that does not match the type, or more than 255 options",
        429 => "rate limited — too many calls in too short a window",
        529 => "the service is overloaded",
        _ => "the request failed",
    };
    if detail.is_empty() {
        format!("typesafe system one HTTP {status}: {why}")
    } else {
        format!("typesafe system one HTTP {status}: {why} ({detail})")
    }
}

/// Parse the reply into the capability's types.
///
/// Lenient by construction: a missing `usage` reads as zero and an answer shape we do
/// not recognize is kept whole rather than dropped. The one thing that is an error is a
/// body that is not an object with `answers` — that is not a reply.
fn parse_response(text: &str) -> anyhow::Result<Reply> {
    let root: Value = serde_json::from_str(text)
        .with_context(|| format!("parsing typesafe system one response: {text}"))?;
    let Some(answers) = root.get("answers").and_then(Value::as_object) else {
        anyhow::bail!("typesafe system one returned no `answers`: {text}");
    };
    Ok(Reply {
        model: root.get("model").and_then(Value::as_str).unwrap_or_default().to_string(),
        answers: answers.iter().map(|(k, v)| (k.clone(), parse_answer(v))).collect(),
        usage: Usage {
            input_tokens: usage_field(&root, "input_tokens"),
            output_tokens: usage_field(&root, "output_tokens"),
        },
    })
}

fn usage_field(root: &Value, key: &str) -> u64 {
    root.get("usage").and_then(|u| u.get(key)).and_then(Value::as_u64).unwrap_or(0)
}

/// One answer, read at the key its own `type` names.
///
/// **An unrecognised type keeps the raw object**, and so does a recognised one whose
/// value is missing or the wrong shape. Guessing costs a wrong number presented with
/// the same authority as a right one, which is the single thing this capability exists
/// to avoid.
///
/// `noul` carries no confidence, and that is not an omission: the probability *is* the
/// uncertainty. `choice` and `score` carry one because their probability mass is spread
/// over several outcomes.
fn parse_answer(raw: &Value) -> Answer {
    let kind = raw.get("type").and_then(Value::as_str).unwrap_or_default();
    let other = || Answer::Other { kind: kind.to_string(), raw: raw.clone() };
    // The self-describing read: `"type":"noul"` means the value is at `noul`.
    let Some(value) = raw.get(kind) else { return other() };
    let confidence = raw.get("confidence").and_then(Value::as_f64);
    let probabilities = raw.get("probabilities").cloned();
    match kind {
        "noul" => match value.as_f64() {
            Some(p) => Answer::Noul { p },
            None => other(),
        },
        "choice" => match value.as_str() {
            Some(choice) => {
                Answer::Choice { choice: choice.to_string(), probabilities, confidence }
            }
            None => other(),
        },
        "score" => match value.as_f64() {
            Some(score) => Answer::Score {
                score,
                legend: raw.get("legend").cloned(),
                probabilities,
                confidence,
            },
            None => other(),
        },
        _ => other(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            client: reqwest::Client::new(),
            api_key: "test-key".to_string(),
            endpoint: format!("{DEFAULT_API_BASE}{ENDPOINT_PATH}"),
            model: DEFAULT_MODEL.to_string(),
        }
    }

    fn questions() -> Map<String, Value> {
        json!({
            "is_urgent": { "type": "noul", "instructions": "Does this convey urgency?" }
        })
        .as_object()
        .unwrap()
        .clone()
    }

    /// The state and the question map cross untouched — the decision that there is no
    /// interface of ours between the caller and the wire.
    #[test]
    fn the_request_is_the_callers_own_shape() {
        let state = json!("The server is down.");
        let body = build_request(&config(), &state, &questions(), None);

        assert_eq!(body["state"], "The server is down.");
        assert_eq!(body["model"], DEFAULT_MODEL);
        assert_eq!(body["questions"]["is_urgent"]["type"], "noul");
        assert_eq!(
            body["questions"]["is_urgent"]["instructions"],
            "Does this convey urgency?"
        );

        // A structured state is passed as structure, not flattened into prose.
        let structured = json!({ "subject": "outage", "minutes_open": 41 });
        let body = build_request(&config(), &structured, &questions(), None);
        assert_eq!(body["state"]["minutes_open"], 41);

        // A per-call model beats the configured default; a blank one does not.
        assert_eq!(build_request(&config(), &state, &questions(), Some("jev-1.13.0"))["model"], "jev-1.13.0");
        assert_eq!(build_request(&config(), &state, &questions(), Some("  "))["model"], DEFAULT_MODEL);
    }

    #[test]
    fn each_answer_is_read_at_the_key_its_own_type_names() {
        let raw = r#"{
            "model": "jev-1.13.0",
            "answers": {
                "is_urgent": { "type": "noul", "noul": 0.95 },
                "topic": { "type": "choice", "choice": "billing",
                           "probabilities": { "billing": 0.8, "bug": 0.2 }, "confidence": 0.72 },
                "severity": { "type": "score", "score": 3.4,
                              "legend": ["low", "medium", "high", "critical"],
                              "probabilities": [0.0, 0.1, 0.4, 0.5], "confidence": 0.61 }
            },
            "usage": { "input_tokens": 296, "output_tokens": 20 }
        }"#;
        let reply = parse_response(raw).unwrap();

        assert_eq!(reply.model, "jev-1.13.0");
        assert_eq!(reply.usage.input_tokens, 296);
        assert_eq!(reply.usage.output_tokens, 20);

        match &reply.answers["is_urgent"] {
            // No confidence to read: the probability is the uncertainty.
            Answer::Noul { p } => assert!((p - 0.95).abs() < f64::EPSILON),
            other => panic!("expected a noul, got {other:?}"),
        }
        match &reply.answers["topic"] {
            Answer::Choice { choice, probabilities, confidence } => {
                assert_eq!(choice, "billing");
                assert_eq!(probabilities.as_ref().unwrap()["billing"], 0.8);
                assert_eq!(*confidence, Some(0.72));
            }
            other => panic!("expected a choice, got {other:?}"),
        }
        match &reply.answers["severity"] {
            Answer::Score { score, legend, confidence, .. } => {
                // Weighted, so it lands *between* the levels rather than on one.
                assert!((score - 3.4).abs() < f64::EPSILON);
                assert_eq!(legend.as_ref().unwrap()[3], "critical");
                assert_eq!(*confidence, Some(0.61));
            }
            other => panic!("expected a score, got {other:?}"),
        }
    }

    /// A type this build has never heard of is carried through whole. The vendor grows
    /// this list without asking us, and a new question type must cost the caller a
    /// `match` arm it can read — never a number invented to fill a slot.
    #[test]
    fn an_unknown_type_keeps_its_raw_object_rather_than_being_guessed_at() {
        let raw = r#"{ "answers": {
            "when": { "type": "instant", "instant": "2026-09-20T10:00:00Z", "confidence": 0.4 },
            "broken": { "type": "noul" },
            "also_broken": { "type": "choice", "choice": 7 }
        } }"#;
        let reply = parse_response(raw).unwrap();

        match &reply.answers["when"] {
            Answer::Other { kind, raw } => {
                assert_eq!(kind, "instant");
                assert_eq!(raw["instant"], "2026-09-20T10:00:00Z");
            }
            other => panic!("expected the raw object, got {other:?}"),
        }
        // A known type whose value is missing or mis-shaped goes the same way: keeping
        // the object says "look at this", a default would say "the answer is 0".
        assert!(matches!(reply.answers["broken"], Answer::Other { .. }));
        assert!(matches!(reply.answers["also_broken"], Answer::Other { .. }));

        // No usage reported reads as zero, not as a failed parse.
        assert_eq!(reply.usage.input_tokens, 0);
    }

    #[test]
    fn a_body_that_is_not_a_reply_is_an_error() {
        assert!(parse_response(r#"{ "detail": "no key" }"#).is_err());
        assert!(parse_response("not json at all").is_err());
        // An empty answer map is a reply — every question was answered, there were none.
        assert!(parse_response(r#"{ "answers": {} }"#).unwrap().answers.is_empty());
    }

    /// The four documented failures, split by what the reader should do about them.
    #[test]
    fn each_status_names_its_cause_and_only_two_are_worth_retrying() {
        assert!(transient(429) && transient(529));
        assert!(!transient(401) && !transient(422));

        assert!(explain(401, "").contains("key was rejected"));
        assert!(explain(422, r#"{"detail":"unknown type"}"#).contains("does not know"));
        assert!(explain(422, r#"{"detail":"unknown type"}"#).contains("unknown type"), "the vendor's own detail survives");
        assert!(explain(429, "").contains("rate limited"));
        assert!(explain(529, "").contains("overloaded"));
    }
}
