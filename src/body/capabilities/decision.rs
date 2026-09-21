//! Decision capability — many typed questions about one state, answered at once with
//! **calibrated probabilities**.
//!
//! The host is already a language model and can judge one thing perfectly well, so
//! judging is not what this is for. What it adds is the two properties a generating
//! model cannot give: **hundreds of judgments in one round trip**, and numbers that
//! **mean the same thing on the next call**. Scoring 200 retrieved candidates, ranking
//! a shortlist, deciding which of 450 pairs match — today that is reading everything
//! into a context window and deciding in prose, slowly, with a self-reported confidence
//! that compares to nothing. These answers compare.
//!
//! **The interface is the wire**, deliberately. [`crate::body::capabilities::image_gen`]
//! has an `ImageParams` because two vendors disagree about how to say the same thing;
//! there is exactly one System One vendor, so a vocabulary of ours here would be one no
//! second vendor is asking for. The types below mirror the reply
//! ([`crate::foundation::vendors::typesafe_system_one`] documents the wire).
//!
//! **Two entrances.** The `hi_system_one` tool, held by the worker and cognition rungs, and
//! the pre-send speech check ([`crate::body::reaction::legibility::check`]), which asks every
//! message in its scope the reading standard's axes as typed questions. Episode boundaries and
//! forgetting are the next obvious internal call sites and are not wired.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::{Map, Value};

use crate::foundation::vendors::typesafe_system_one;

/// One question's answer, discriminated by the type the question asked for.
///
/// A caller branches on this rather than indexing JSON — which is the point of asking a
/// typed question in the first place. `Other` is how a type this build does not know
/// arrives: whole, and visibly not one of the three.
#[derive(Debug, Clone)]
pub enum Answer {
    /// P(yes), 0–1. **No confidence field, and that is not a gap**: the probability
    /// *is* the uncertainty. 0.5 is maximal doubt, not a missing answer.
    Noul { p: f64 },
    /// The winning option, the mass over all of them (sums to 1), and how concentrated
    /// that mass is.
    Choice { choice: String, probabilities: Option<Value>, confidence: Option<f64> },
    /// A weighted position on the ordered levels the question named, so it **lands
    /// between them** — 3.4 on a four-level scale is not a typo for 3.
    Score {
        score: f64,
        legend: Option<Value>,
        probabilities: Option<Value>,
        confidence: Option<f64>,
    },
    /// A type this build has no case for, kept verbatim. The vendor owns the type list
    /// and grows it without asking us; the alternative to carrying one through is
    /// inventing a number for it.
    Other { kind: String, raw: Value },
}

impl Answer {
    /// Back to the wire's own shape, for a caller that hands answers to something that
    /// reads JSON — the `hi_system_one` tool above all, whose reader is a model.
    ///
    /// Lossless in both directions: a known type is rebuilt field for field, and
    /// [`Answer::Other`] was never taken apart. Rendering these as prose instead would
    /// mean inventing a layout for `probabilities`, whose shape is the vendor's.
    pub fn to_json(&self) -> Value {
        let mut o = serde_json::Map::new();
        let mut put = |k: &str, v: Option<Value>| {
            if let Some(v) = v {
                o.insert(k.to_string(), v);
            }
        };
        match self {
            Answer::Noul { p } => {
                put("type", Some(Value::from("noul")));
                put("noul", Some(Value::from(*p)));
            }
            Answer::Choice { choice, probabilities, confidence } => {
                put("type", Some(Value::from("choice")));
                put("choice", Some(Value::from(choice.clone())));
                put("probabilities", probabilities.clone());
                put("confidence", confidence.map(Value::from));
            }
            Answer::Score { score, legend, probabilities, confidence } => {
                put("type", Some(Value::from("score")));
                put("score", Some(Value::from(*score)));
                put("legend", legend.clone());
                put("probabilities", probabilities.clone());
                put("confidence", confidence.map(Value::from));
            }
            Answer::Other { raw, .. } => return raw.clone(),
        }
        Value::Object(o)
    }
}

/// What one call answered, and what it cost.
///
/// `model` is the **resolved** id (`jev-1.13.0`) rather than the alias asked for
/// (`jev-latest`) — a calibrated number is only comparable to another from the same
/// model, so which one answered is part of the answer.
#[derive(Debug, Clone)]
pub struct Reply {
    pub model: String,
    /// Keyed by the caller's own question names. Ordered so a sweep's output reads the
    /// same way twice.
    pub answers: BTreeMap<String, Answer>,
    pub usage: Usage,
}

/// Tokens billed. Output is metered and published at a rate of zero — reported anyway,
/// because a rate that is zero today is still a rate.
#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

enum Backend {
    /// Why nothing is configured, in the words a person could act on.
    Disabled(String),
    TypeSafe(typesafe_system_one::Config),
}

static BACKEND: OnceLock<Backend> = OnceLock::new();

/// The default wire when the source names none — the only System One impl today.
const DEFAULT_WIRE: &str = "typesafe-systemone";

/// One provider offered for this capability, as the credential store or the broker
/// describes it. Its own type, not one shared with the other capabilities.
#[derive(Debug, Clone, Default)]
pub struct ProviderSpec {
    /// Wire id, in the source's own vocabulary. `None`/empty → [`DEFAULT_WIRE`].
    pub wire: Option<String>,
    pub base_url: Option<String>,
    pub api_key: String,
    pub model: Option<String>,
}

/// Resolve the decision backend into the process-global config, from **every provider
/// the source offers, best first**.
///
/// Holds the first provider whose wire it can actually speak and logs the rest. A wire
/// we have no impl for is skipped with a warning, never fatal — the broker names wires
/// in its own vocabulary and changes it without asking us. A provider whose *config*
/// won't build still is fatal: that is a broken setting, not an unfamiliar name.
/// Idempotent — the first init wins.
pub fn init(providers: Vec<ProviderSpec>) -> anyhow::Result<()> {
    let (chosen, skipped) = select(&providers);
    let backend = match chosen {
        Some(i) => Backend::TypeSafe(typesafe_system_one::Config::from_store(
            Some(&providers[i].api_key),
            providers[i].base_url.as_deref(),
            providers[i].model.as_deref(),
        )?),
        None if skipped.is_empty() => {
            Backend::Disabled("set a decision key in Settings".to_string())
        }
        None => {
            tracing::warn!(wires = ?skipped, "no decision impl for any offered wire; decision is off");
            Backend::Disabled(format!("no impl for the offered wire(s): {}", skipped.join(", ")))
        }
    };
    let _ = BACKEND.set(backend);
    Ok(())
}

/// Whether an explicit wire id names something [`typesafe_system_one`] can speak.
///
/// Loose on purpose, for the reason spelled out in [`crate::body::capabilities::stt`]:
/// the broker publishes wires in its own vocabulary, and an unfamiliar spelling of a
/// wire we do speak must cost a log line rather than the capability. Both halves of the
/// published id match on their own, so `typesafe-systemone`, `system-one` and a bare
/// `typesafe` all resolve here.
fn speakable(wire: &str) -> bool {
    let w = wire.trim().to_ascii_lowercase();
    w.contains("systemone") || w.contains("system-one") || w.contains("typesafe") || w.contains("jev")
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
        let wire =
            p.wire.as_deref().map(str::trim).filter(|w| !w.is_empty()).unwrap_or(DEFAULT_WIRE);
        if speakable(wire) {
            return (Some(i), skipped);
        }
        skipped.push(wire.to_string());
    }
    (None, skipped)
}

/// Whether a provider is configured.
pub fn available() -> bool {
    matches!(BACKEND.get(), Some(Backend::TypeSafe(_)))
}

/// Why the capability is off, for the error text at the point of use.
fn why_off() -> &'static str {
    match BACKEND.get() {
        Some(Backend::Disabled(reason)) => reason.as_str(),
        _ => "set a decision key in Settings",
    }
}

/// Ask every question in `questions` about one `state`, in one round trip.
///
/// `questions` is a map of caller-chosen names to question objects; the reply is keyed
/// by the same names. `model` overrides the configured default for this call — reach
/// for it to pin a resolved id when a set of numbers has to stay comparable across a
/// long sweep.
pub async fn ask(
    state: &Value,
    questions: &Map<String, Value>,
    model: Option<&str>,
) -> anyhow::Result<Reply> {
    match BACKEND.get() {
        Some(Backend::TypeSafe(cfg)) => {
            typesafe_system_one::ask(cfg, state, questions, model).await
        }
        _ => anyhow::bail!("system one not configured ({})", why_off()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(wire: Option<&str>, key: &str) -> ProviderSpec {
        ProviderSpec { wire: wire.map(str::to_owned), api_key: key.into(), ..Default::default() }
    }

    /// The list is offered best-first, so the first wire we can speak wins — and a wire
    /// we cannot is stepped over rather than taken as the answer.
    #[test]
    fn the_first_speakable_wire_wins_and_the_rest_are_named() {
        let (chosen, skipped) =
            select(&[spec(Some("some-future-judge"), "k"), spec(Some("typesafe-systemone"), "k")]);
        assert_eq!(chosen, Some(1));
        assert_eq!(skipped, vec!["some-future-judge"]);
        assert_eq!(select(&[spec(None, "k")]).0, Some(0), "no wire named → the default");
        assert!(select(&[spec(Some("typesafe-systemone"), " ")]).1.is_empty(), "no key, no complaint");
    }

    /// The name the broker publishes for `system-one`, plus the spellings it might
    /// plausibly publish instead. The broker owns this vocabulary; an unfamiliar
    /// spelling of a wire we do speak is what turned vision off once already.
    #[test]
    fn the_brokers_own_spelling_is_the_wire_we_speak() {
        for wire in ["typesafe-systemone", "typesafe/systemone", "system-one", "typesafe", "jev"] {
            assert_eq!(select(&[spec(Some(wire), "k")]).0, Some(0), "{wire} should be speakable");
        }
    }
}
