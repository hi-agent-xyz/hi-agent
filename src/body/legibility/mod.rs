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

use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use tokio::time::Instant;

use crate::foundation::config::tunables;
use crate::mind::memory::quality::{self, Outcome, Scope, Surface};

pub mod home;
pub mod judge;
pub mod record;
pub mod record_audit;
pub mod record_replay;

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
    answer: &anyhow::Result<judge::Answer>,
    elapsed: Duration,
    live_limit: Duration,
    mode: Mode,
) -> (Outcome, Option<String>, Option<String>) {
    let (outcome, axis, note) = match answer {
        Ok(answer) => match json_object::<Verdict>(&answer.text) {
            Some(v) if v.verdict.trim().eq_ignore_ascii_case("revise") => {
                let note = v.note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
                match note {
                    Some(note) => (Outcome::Revise, quality::axis(v.axis.as_deref()), Some(note)),
                    None => (Outcome::Pass, quality::axis(v.axis.as_deref()), None),
                }
            }
            Some(_) => (Outcome::Pass, None, None),
            None => {
                // **The answer itself, clipped**, because an unreadable verdict is only
                // debuggable from what was actually said: a fenced object, a refusal, and a
                // model that wrote prose all land here identically otherwise.
                tracing::warn!(
                    answer = %answer.text.chars().take(200).collect::<String>(),
                    "a legibility judge answered in a shape it could not be read in"
                );
                (Outcome::Error, None, None)
            }
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

#[derive(Deserialize)]
struct AuditAnswer {
    #[serde(default)]
    messages: Vec<AuditedAnswer>,
    #[serde(default)]
    unsaid: Vec<String>,
    #[serde(default)]
    wrong: Vec<String>,
}

#[derive(Deserialize)]
struct AuditedAnswer {
    #[serde(default)]
    n: Option<usize>,
    #[serde(default)]
    axis: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

/// An audit's answer, read against the numbered `items` it was asked about: one entry per
/// item whatever the judge numbered — an entry naming an item that was not asked about is
/// dropped, and an item it skipped has no finding — then what is owed and left unsaid, and
/// what is wrong. `None` when the answer is not in the shape asked for.
pub(crate) fn read_audit(
    answer: &str,
    items: &[String],
) -> Option<(Vec<quality::Audited>, Vec<String>, Vec<String>)> {
    let answer = json_object::<AuditAnswer>(answer)?;
    let mut out: Vec<quality::Audited> = items
        .iter()
        .map(|text| quality::Audited { text: text.clone(), axis: None, note: None })
        .collect();
    for (i, a) in answer.messages.into_iter().enumerate() {
        let at = a.n.map(|n| n.saturating_sub(1)).unwrap_or(i);
        let Some(slot) = out.get_mut(at) else { continue };
        slot.axis = quality::axis(a.axis.as_deref());
        slot.note = slot.axis.as_ref().and(a.note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()));
    }
    let clean = |items: Vec<String>| -> Vec<String> {
        items.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    };
    Some((out, clean(answer.unsaid), clean(answer.wrong)))
}

/// One gate at one seam that is not speech's (`docs/arch/legibility.md` § M, § *Home*): which
/// surface it judges, the settings that choose its mode and its model, and its rubric. Speech's
/// check reads a turn and lives with Reaction; every other gate reads a write and is this.
pub(crate) struct Gate {
    pub surface: Surface,
    pub mode_key: &'static str,
    pub model_key: &'static str,
    pub budget_key: &'static str,
    pub rubric: &'static str,
}

/// How long a live gate may hold a write, unless its setting says otherwise.
///
/// **Twenty seconds, and it used to be two and a half.** The first number was picked for a
/// person waiting on a reply and then reused here, where nobody is: a worker writing a line
/// is the only thing held. Measured 2026-09-20, the two and a half were unreachable — of 131
/// checks over five days not one answered inside them, because the judge spends 900 to 1,500
/// tokens thinking before ~50 tokens of verdict, and the cheapest real case on the fastest
/// model this install has ran 3.3 seconds. A budget nothing can meet records `timeout` for
/// everything and teaches nothing.
const GATE_BUDGET_MS: u64 = 20_000;

/// A budget from its setting, in milliseconds, or `fallback`. Read per call rather than at
/// startup, so turning it costs no restart.
pub(crate) fn budget(key: &str, fallback: u64) -> Duration {
    let ms = tunables::get(key)
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .unwrap_or(fallback);
    Duration::from_millis(ms)
}

/// How long a shadow run may take and still record a verdict: twice the live budget, so what
/// shadow measures includes how far past the budget the answers that miss it actually land.
/// A ceiling equal to the budget would censor exactly the number being looked for.
pub(crate) fn shadow_limit(budget: Duration) -> Duration {
    budget * 2
}

pub enum Review {
    Pass,
    SendBack(String),
}

impl Gate {
    /// Its mode, from its setting. Shadow unless set otherwise.
    pub fn mode(&self) -> Mode {
        Mode::from_setting(tunables::get(self.mode_key).as_deref())
    }

    /// How long it may hold a write, from its setting.
    pub fn budget(&self) -> Duration {
        budget(self.budget_key, GATE_BUDGET_MS)
    }
}

/// Writers whose last write on a key was sent back, per surface, so their next one goes through.
fn sent_back() -> &'static Mutex<HashSet<(Surface, String, String)>> {
    static SENT_BACK: std::sync::OnceLock<Mutex<HashSet<(Surface, String, String)>>> = std::sync::OnceLock::new();
    SENT_BACK.get_or_init(Default::default)
}

/// Read one write at `gate` and decide whether it goes on. `writer` is the session writing,
/// `key` what it writes on (a task's subject, the home arrangement), `case` what the judge
/// reads after who the reader is, `message` what the record keeps.
///
/// **One send-back per writer per key, then it lands.** Speech can be dropped; a record, an
/// arrangement the person asked for, cannot — a gate able to lose them would be a worse failure
/// than the one it exists for. Fail open: a timeout, an error or an unreadable answer passes.
pub(crate) async fn gate(
    data_dir: &Path,
    gate: &Gate,
    writer: &str,
    key: &str,
    scope: Scope,
    case: String,
    message: String,
) -> Review {
    let mode = gate.mode();
    if mode == Mode::Off {
        return Review::Pass;
    }
    let held = (gate.surface, writer.to_owned(), key.to_owned());
    if sent_back().lock().map(|mut s| s.remove(&held)).unwrap_or(false) {
        return Review::Pass;
    }
    let Some(judge) = judge::Judge::resolve(data_dir, gate.model_key) else { return Review::Pass };
    let instructions = crate::identity::judge_instructions(data_dir, gate.rubric).await;
    let reader = crate::mind::memory::snapshot::for_record(data_dir).await;
    let case = format!(
        "## Who is reading, and how they want to be told\n{}\n\n{}",
        if reader.trim().is_empty() { "(nothing)" } else { reader.trim() },
        case
    );
    let live = gate.budget();
    let checked = Checked {
        data_dir: data_dir.to_path_buf(),
        surface: gate.surface,
        key: key.to_owned(),
        message,
        scope,
        mode,
        model: judge.model().to_string(),
        budget: live,
    };
    match mode {
        Mode::Off => Review::Pass,
        Mode::Shadow => {
            tokio::spawn(async move {
                let started = Instant::now();
                let answer = judge.ask(&instructions, &case, shadow_limit(live)).await;
                checked.write(answer, started.elapsed()).await;
            });
            Review::Pass
        }
        Mode::On => {
            let started = Instant::now();
            let answer = match tokio::time::timeout(live, judge.ask(&instructions, &case, live)).await {
                Ok(answer) => answer,
                Err(_) => Err(anyhow::anyhow!("timed out")),
            };
            match checked.write(answer, started.elapsed()).await {
                (Outcome::Revise, Some(note)) => {
                    if let Ok(mut s) = sent_back().lock() {
                        s.insert(held);
                    }
                    Review::SendBack(note)
                }
                _ => Review::Pass,
            }
        }
    }
}

struct Checked {
    data_dir: std::path::PathBuf,
    surface: Surface,
    key: String,
    message: String,
    scope: Scope,
    mode: Mode,
    model: String,
    budget: Duration,
}

impl Checked {
    /// Read the answer, record it, and hand back the outcome and note.
    async fn write(self, answer: anyhow::Result<judge::Answer>, elapsed: Duration) -> (Outcome, Option<String>) {
        let (outcome, axis, note) = read_verdict(&answer, elapsed, self.budget, self.mode);
        let cost = answer.as_ref().map(|a| a.cost).unwrap_or_default();
        tracing::info!(
            surface = ?self.surface,
            mode = self.mode.as_str(),
            scope = ?self.scope,
            outcome = ?outcome,
            axis = axis.as_deref().unwrap_or(""),
            latency_ms = elapsed.as_millis() as u64,
            budget_ms = self.budget.as_millis() as u64,
            tokens_in = cost.input,
            tokens_cached = cost.cached,
            tokens_out = cost.output,
            tokens_thinking = cost.thinking,
            key = %self.key,
            "legibility gate"
        );
        let record = quality::Record::Check(quality::Check {
            ts: Utc::now(),
            surface: self.surface,
            turn: self.key,
            message: self.message,
            scope: self.scope,
            mode: self.mode.as_str().to_string(),
            outcome,
            axis,
            note: note.clone(),
            latency_ms: elapsed.as_millis() as u64,
            model: self.model,
            cost,
            budget_ms: self.budget.as_millis() as u64,
            answers: None,
        });
        if let Err(err) = quality::append(&self.data_dir, &record).await {
            tracing::warn!(error = %format!("{err:#}"), "could not record a gate's verdict");
        }
        (outcome, note)
    }
}

/// Record that host code let a write through without reading it (`quality::Skipped`).
///
/// **Every seam's triage calls this**, so that what the code passed is countable beside what a
/// judge passed. It is spawned rather than awaited: a write nobody is judging must not wait on
/// a file append, and a record that fails to write costs a data point, never the write.
pub(crate) fn skipped(data_dir: &Path, surface: Surface, key: &str, message: &str) {
    let record = quality::Record::Skipped(quality::Skipped {
        ts: Utc::now(),
        surface,
        turn: key.to_owned(),
        message: message.to_owned(),
    });
    let data_dir = data_dir.to_path_buf();
    tokio::spawn(async move {
        if let Err(err) = quality::append(&data_dir, &record).await {
            tracing::warn!(error = %format!("{err:#}"), "could not record a skipped write");
        }
    });
}

/// A seam's gate is held once per writer per key: a send-back is remembered until that
/// writer's next write on that key, which passes.
#[cfg(test)]
pub(crate) fn hold_for_test(surface: Surface, writer: &str, key: &str) {
    sent_back().lock().unwrap().insert((surface, writer.to_owned(), key.to_owned()));
}

#[cfg(test)]
pub(crate) fn held_for_test(surface: Surface, writer: &str, key: &str) -> bool {
    sent_back().lock().unwrap().contains(&(surface, writer.to_owned(), key.to_owned()))
}
