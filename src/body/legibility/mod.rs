//! Legibility at every seam — the half of `docs/arch/legibility.md` that is not any one
//! surface's.
//!
//! The standard is bound to what a person reads, never to a verb, so the machinery that
//! holds it cannot live inside one rung. [`judge`] is one model request with the reading
//! standard as its prefix; [`Mode`] is the ladder every gate climbs (off, shadow, on);
//! [`read_verdict`] is how any judge's answer becomes an outcome. What stays with a surface
//! is only what reads that surface's own facts: speech's triage reads a turn and lives with
//! Reaction ([`crate::body::reaction::legibility`]); a record's reads a line and lives in
//! [`record`].

use std::time::Duration;

use serde::Deserialize;

use crate::mind::memory::quality::{self, Outcome};

pub mod judge;
pub mod record;

use judge::json_object;

/// How far a gate is allowed to go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Nothing is read.
    Off,
    /// Read and recorded; nothing is sent back. The default until the numbers say otherwise.
    Shadow,
    /// Read, recorded, and sent back when it fails.
    On,
}

impl Mode {
    pub fn from_setting(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("off") => Mode::Off,
            Some("on") => Mode::On,
            _ => Mode::Shadow,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Shadow => "shadow",
            Mode::On => "on",
        }
    }
}

#[derive(Deserialize)]
struct Verdict {
    #[serde(default)]
    verdict: String,
    #[serde(default)]
    axis: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

/// A judge's answer as an outcome, its axis and its note. An answer that cannot be read is an
/// error, and an error passes; a send-back with no note is not one, because there is nothing
/// for the writer to act on. A shadow verdict slower than `live_limit` is recorded as the
/// timeout live would have been.
pub(crate) fn read_verdict(
    answer: anyhow::Result<String>,
    elapsed: Duration,
    live_limit: Duration,
    mode: Mode,
) -> (Outcome, Option<String>, Option<String>) {
    let (outcome, axis, note) = match answer {
        Ok(text) => match json_object::<Verdict>(&text) {
            Some(v) if v.verdict.trim().eq_ignore_ascii_case("revise") => {
                let note = v.note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
                match note {
                    Some(note) => (Outcome::Revise, quality::axis(v.axis.as_deref()), Some(note)),
                    None => (Outcome::Pass, quality::axis(v.axis.as_deref()), None),
                }
            }
            Some(_) => (Outcome::Pass, None, None),
            None => (Outcome::Error, None, None),
        },
        Err(err) if format!("{err:#}").contains("timed out") => (Outcome::Timeout, None, None),
        Err(err) => {
            tracing::debug!(error = %format!("{err:#}"), "a legibility check failed; the text goes through");
            (Outcome::Error, None, None)
        }
    };
    let outcome = if outcome != Outcome::Timeout && elapsed > live_limit && mode == Mode::Shadow {
        Outcome::Timeout
    } else {
        outcome
    };
    (outcome, axis, note)
}
