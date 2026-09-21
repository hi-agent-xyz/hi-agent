//! The pre-send check: host-code triage (§ D), then one System One call (§ E).
//!
//! **It reads a message between `hi_say` accepting it and the floor**, and it can do one of
//! two things: let it through, or answer `not sent — <note>` so Reaction rewrites or drops
//! it inside the same turn. It never writes words — a checker that edits is a second mouth
//! (`docs/arch/arch.md` invariant 1) — and its judge cannot: System One answers typed
//! questions with probabilities, so a send-back's note is the failing axis's own line from
//! the reading standard.
//!
//! **Why System One and not a model writing a verdict.** The model did not answer in time:
//! five days of recorded checks on `deepseek-flash` kept 187 and 162 of them timed out, p50 22 s,
//! because a verdict is ~50 tokens behind ~1,000 of thinking. Asked the same axes as typed
//! questions, System One answered all 167 audited messages on this install in p50 0.53 s,
//! p99 2.2 s. **What it reads well is the literal** — a claim of something on screen with no
//! show this turn, a message that says it repeats — and on the person's own labels of 20 of
//! those messages it ranked their send-backs no better than chance (AUC 0.44–0.55); the model
//! it replaces was never measured against them at all. So this is the judge that produces a
//! number to learn from, taken while the logic above it is still being worked out, not one
//! shown to judge well. **Measured offline only; never watched on a live turn.**
//!
//! Every limit here is about not being able to do harm:
//!
//! - **triage is facts only** — the turn carries a report, the message is not the turn's
//!   first, or it is longer than a short reply — and everything else goes out untouched;
//! - **one send-back per turn** — what Reaction sends after reading one goes out as written;
//! - **a timeout or an error sends the message** — silence is the failure nobody reports,
//!   and a checker must never be able to produce it;
//! - **serial within a turn** — calls queue on one lock, so order holds, and a message that
//!   was already waiting when an earlier one was sent back goes back with it, since it may
//!   lean on the one that did not land;
//! - **one switch** — `speech_check` = `off` reads nothing; otherwise every message in scope is
//!   read, and one that fails is sent back.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde_json::{Map, Value, json};
use tokio::time::Instant;

use crate::body::capabilities::decision;
use crate::foundation::config::tunables;
use crate::mind::memory::quality::{self, Outcome, Scope};

/// Past this many characters a message is more than a short reply, and in scope. A
/// starting value (`docs/arch/legibility.md` § Open), for the replay set to settle.
pub(crate) const TRIAGE_CHARS: usize = 120;

/// How long a live check may hold a message, unless `speech_check_budget_ms` says otherwise.
/// Past it the message goes out.
///
/// **Two and a half seconds, which is what it was first meant to be.** Someone is waiting on
/// this one. It went to ten on 2026-09-20 because a model writing a verdict could not meet
/// two and a half at all — ~90% of its tokens were thinking before the verdict began. System
/// One writes no verdict: measured 2026-09-21 on the full case of every audited message on this
/// install, p50 0.53 s, p99 2.2 s, one of 167 past two and a half.
const CHECK_BUDGET_MS: u64 = 2_500;

/// The `app_settings` keys switching it off, choosing how long a verdict may take, and how little
/// `pass` may carry before a message is sent back.
pub(crate) const SWITCH_KEY: &str = "speech_check";
pub(crate) const BUDGET_KEY: &str = "speech_check_budget_ms";
pub(crate) const PASS_BELOW_KEY: &str = "speech_check_pass_below";

/// A message is sent back when the choice puts less than this on `pass`: more likely to fail
/// an axis than not. A starting value (`docs/arch/legibility.md` § Open) — every answer's
/// whole mass is recorded, so where the cut should sit is read off what the check actually did.
const PASS_BELOW: f64 = 0.5;

/// How long a live check may hold a message, from its setting.
pub(crate) fn budget() -> Duration {
    crate::body::legibility::budget(BUDGET_KEY, CHECK_BUDGET_MS)
}

/// The cut, from its setting.
fn pass_below() -> f64 {
    tunables::get(PASS_BELOW_KEY)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|p| (0.0..=1.0).contains(p))
        .unwrap_or(PASS_BELOW)
}

/// The key the one `choice` is asked under; each axis's `noul` is `fails_<axis>`.
const CHOICE: &str = "axis";

/// What one message is asked, built once a turn from the rubric and the axis table as
/// installed, with the table's lines kept for the note a send-back carries.
struct Questions {
    asked: Map<String, Value>,
    lines: Vec<(String, String)>,
}

impl Questions {
    /// The rubric's wording around the standard's axes (`src/identity/judges/check.md`). `None`
    /// when a section or the table is missing — then nothing is asked, and the message goes out.
    fn build(rubric: &str, standard: &str) -> Option<Self> {
        let part = |heading: &str| crate::identity::rubric_section(rubric, heading);
        let (frame, each_axis, choice, pass, each_option) =
            (part("Frame")?, part("Each axis")?, part("The one choice")?, part("Pass")?, part("Each option")?);
        // `unsaid` is what the turn has not said yet, and one message cannot show that.
        let lines: Vec<(String, String)> =
            crate::identity::axis_lines(standard).into_iter().filter(|(axis, _)| axis != "unsaid").collect();
        if lines.is_empty() {
            return None;
        }
        let mut asked = Map::new();
        let mut options = Map::new();
        options.insert("pass".into(), Value::from(pass));
        for (axis, line) in &lines {
            let question = each_axis.replace("{line}", line);
            asked.insert(format!("fails_{axis}"), json!({ "type": "noul", "instructions": format!("{frame} {question}") }));
            options.insert(axis.clone(), Value::from(each_option.replace("{line}", line)));
        }
        asked.insert(
            CHOICE.into(),
            json!({ "type": "choice", "instructions": format!("{frame} {choice}"), "criteria": options }),
        );
        Some(Self { asked, lines })
    }

    /// Ask them about one case, inside `limit`.
    async fn ask(&self, case: String, limit: Duration) -> anyhow::Result<decision::Reply> {
        tokio::time::timeout(limit, decision::ask(&Value::String(case), &self.asked, None))
            .await
            .map_err(|_| anyhow::anyhow!("timed out"))?
    }

    /// A reply as an outcome, its axis and its note. Sent back when `pass` carries less than
    /// `pass_below`, on the axis the rest of the mass leans to most, with that axis's line as
    /// the note. A reply without the choice, or without `pass` in it, cannot be read — an error,
    /// and an error passes.
    fn verdict(&self, reply: &decision::Reply, pass_below: f64) -> (Outcome, Option<String>, Option<String>) {
        let Some(decision::Answer::Choice { probabilities: Some(Value::Object(mass)), .. }) = reply.answers.get(CHOICE)
        else {
            return (Outcome::Error, None, None);
        };
        let Some(pass) = mass.get("pass").and_then(Value::as_f64) else { return (Outcome::Error, None, None) };
        if pass >= pass_below {
            return (Outcome::Pass, None, None);
        }
        let leaning = mass
            .iter()
            .filter(|(option, _)| option.as_str() != "pass")
            .filter_map(|(option, p)| Some((option, p.as_f64()?)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .and_then(|(option, _)| quality::axis(Some(option.as_str())));
        let Some(axis) = leaning else { return (Outcome::Error, None, None) };
        let note = self.lines.iter().find(|(a, _)| *a == axis).map(|(_, line)| format!("`{axis}` — it {line}"));
        (Outcome::Revise, Some(axis), note)
    }
}

/// The note a message gets when it was waiting behind one that was sent back.
const LEANS_ON_IT: &str = "a message sent in the same breath was sent back, and this one may \
lean on it — read the note on that one first";

/// Whether the speech check reads at all, from its switch.
pub fn enabled() -> bool {
    crate::body::legibility::enabled(SWITCH_KEY)
}

/// What a turn gives its judges to read, gathered when the turn starts.
#[derive(Clone, Debug, Default)]
pub struct Brief {
    /// Who is reading and how they want to be told: the conduct of the people present and
    /// the per-subject read on what the agent's words have earned.
    pub reader: String,
    /// The conversation so far, oldest first.
    pub recent: String,
    /// What reached Reaction this turn.
    pub signals: String,
    /// What was on their screen when the turn began.
    pub screen: String,
    /// Whether that includes a report — mail, or a worker's result.
    pub carries_report: bool,
}

impl Brief {
    /// The case a judge reads: the brief, what this turn already sent and put on screen, and
    /// `then` last.
    pub(crate) fn case(&self, sent: &[String], shown: &[String], then: &str) -> String {
        use std::fmt::Write as _;
        let mut s = String::new();
        let section = |s: &mut String, title: &str, body: &str| {
            let body = body.trim();
            let _ = write!(s, "## {title}\n{}\n\n", if body.is_empty() { "(nothing)" } else { body });
        };
        section(&mut s, "Who is reading, and how they want to be told", &self.reader);
        section(
            &mut s,
            "The conversation so far, oldest first (`>` reached the assistant, `<` is the assistant)",
            &self.recent,
        );
        section(&mut s, "What was on their screen when this turn began", &self.screen);
        section(&mut s, "What reached the assistant this turn", &self.signals);
        let sent = sent.iter().map(|m| format!("- {}", m.replace('\n', "\n  "))).collect::<Vec<_>>();
        section(&mut s, "Already sent this turn", &sent.join("\n"));
        // What went on screen is part of what the turn did: a line saying "it's up" is
        // supported by a show in the same turn and by nothing else.
        section(&mut s, "Put on screen this turn", &shown.join("\n"));
        s.push_str(then.trim());
        s.push('\n');
        s
    }
}

/// The turn a check belongs to, from `begin` to `end`.
struct Draft {
    key: String,
    brief: Brief,
    /// `None` when there is nothing to ask with — System One is not configured, or the rubric
    /// lost a section — and then every message goes out unread.
    questions: Option<Arc<Questions>>,
    sent: Vec<String>,
    shown: Vec<String>,
    /// When this turn's one send-back was answered.
    sent_back_at: Option<Instant>,
}

/// What a turn leaves for the audit when it ends.
pub struct Ended {
    pub key: String,
    pub brief: Brief,
    pub sent: Vec<String>,
    pub shown: Vec<String>,
}

pub enum Review {
    Pass,
    SendBack(String),
}

/// The check's state, shared by the turn loop (which opens and closes a turn) and the
/// mouth (which asks about each message).
pub struct Speech {
    data_dir: PathBuf,
    enabled: bool,
    draft: std::sync::Mutex<Option<Draft>>,
    /// The spoken turns since the person last wrote, waiting to be read against their next
    /// message ([`super::audit::on_reply`]).
    awaiting: std::sync::Mutex<Vec<(String, Vec<String>)>>,
    /// Held from a message's arrival at the mouth to its fate, so messages are read and
    /// sent in the order they were written.
    pub(crate) serial: tokio::sync::Mutex<()>,
}

impl Speech {
    pub fn new(data_dir: PathBuf, enabled: bool) -> Self {
        Self {
            data_dir,
            enabled,
            draft: std::sync::Mutex::new(None),
            awaiting: std::sync::Mutex::new(Vec::new()),
            serial: tokio::sync::Mutex::new(()),
        }
    }

    /// Keep a spoken turn for the reading of the person's next message. Only the latest
    /// few: a reply answers what they just read, not what scrolled away an hour ago.
    pub fn hold_for_reply(&self, ended: &Ended) {
        const HELD: usize = 4;
        if ended.sent.is_empty() {
            return;
        }
        let mut held = self.awaiting.lock().unwrap_or_else(|p| p.into_inner());
        held.push((ended.key.clone(), ended.sent.clone()));
        let over = held.len().saturating_sub(HELD);
        held.drain(..over);
    }

    /// The spoken turns their message answers, taken.
    pub fn take_awaiting(&self) -> Vec<(String, Vec<String>)> {
        std::mem::take(&mut *self.awaiting.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// A check that reads nothing, for a mouth under test.
    pub fn off() -> Self {
        Self::new(PathBuf::new(), false)
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Open a turn. Returns its key, which every record about it carries.
    pub async fn begin(&self, brief: Brief) -> String {
        let key = uuid::Uuid::now_v7().to_string();
        let questions = if !self.enabled || !decision::available() {
            None
        } else {
            let standard = crate::identity::reading_standard(&self.data_dir).await;
            Questions::build(crate::identity::judges::CHECK, &standard).map(Arc::new)
        };
        *self.draft.lock().unwrap_or_else(|p| p.into_inner()) = Some(Draft {
            key: key.clone(),
            brief,
            questions,
            sent: Vec::new(),
            shown: Vec::new(),
            sent_back_at: None,
        });
        key
    }

    /// Close the turn and hand back what the audit needs. `None` outside a turn.
    pub fn end(&self) -> Option<Ended> {
        let draft = self.draft.lock().unwrap_or_else(|p| p.into_inner()).take()?;
        Some(Ended { key: draft.key, brief: draft.brief, sent: draft.sent, shown: draft.shown })
    }

    /// A view went up, came down, or was replaced — `what` as the judges should read it.
    pub fn note_shown(&self, what: &str) {
        if let Some(d) = self.draft.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
            d.shown.push(what.to_string());
        }
    }

    /// A message went out.
    pub fn note_sent(&self, text: &str) {
        if let Some(d) = self.draft.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
            d.sent.push(text.to_string());
        }
    }

    /// Decide whether `text` goes on to the floor. `arrived` is when it reached the mouth,
    /// before it queued behind anything.
    pub async fn review(&self, text: &str, arrived: Instant) -> Review {
        if !self.enabled {
            return Review::Pass;
        }
        let (scope, questions, case, key) = {
            let guard = self.draft.lock().unwrap_or_else(|p| p.into_inner());
            // Outside a turn — the warm-up — nothing reaches anyone anyway.
            let Some(d) = guard.as_ref() else { return Review::Pass };
            if let Some(at) = d.sent_back_at {
                // One send-back a turn. What was already waiting when it was answered goes
                // back with it; what Reaction wrote after reading it goes out.
                return if arrived < at {
                    Review::SendBack(LEANS_ON_IT.to_string())
                } else {
                    Review::Pass
                };
            }
            let Some(scope) = triage(d.brief.carries_report, d.sent.len(), text) else {
                // Not read, and said so — a message triage passes goes out exactly as if a
                // judge had passed it, so it is counted (`legibility::skipped`).
                crate::body::legibility::skipped(
                    &self.data_dir,
                    quality::Surface::Speech,
                    &d.key,
                    text,
                );
                return Review::Pass;
            };
            let Some(questions) = d.questions.clone() else { return Review::Pass };
            let case =
                d.brief.case(&d.sent, &d.shown, &format!("## The message to judge\n{text}"));
            (scope, questions, case, d.key.clone())
        };

        let live = budget();
        let record = CheckRecord {
            data_dir: self.data_dir.clone(),
            key,
            message: text.to_string(),
            scope,
            budget: live,
        };
        let started = Instant::now();
        let answer = questions.ask(case, live.saturating_sub(arrived.elapsed().min(live))).await;
        match record.write(&questions, answer, started.elapsed()).await {
            Some((Outcome::Revise, note)) => {
                if let Some(d) = self.draft.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
                    d.sent_back_at = Some(Instant::now());
                }
                Review::SendBack(note)
            }
            _ => Review::Pass,
        }
    }
}

/// Whether a message is in the check's scope, and why. Facts only (§ D).
fn triage(carries_report: bool, already_sent: usize, text: &str) -> Option<Scope> {
    if carries_report {
        Some(Scope::Report)
    } else if already_sent > 0 {
        Some(Scope::Later)
    } else if text.chars().count() > TRIAGE_CHARS {
        Some(Scope::Long)
    } else {
        None
    }
}

struct CheckRecord {
    data_dir: PathBuf,
    key: String,
    message: String,
    scope: Scope,
    budget: Duration,
}

/// What a check that got no reply records as its model: the capability, since no id came back.
const NO_REPLY_MODEL: &str = "system-one";

impl CheckRecord {
    /// Read the reply, record it with every number it carried, and hand back the verdict and
    /// note. A reply that cannot be read is an error, and an error passes.
    async fn write(
        self,
        questions: &Questions,
        answer: anyhow::Result<decision::Reply>,
        elapsed: Duration,
    ) -> Option<(Outcome, String)> {
        let (outcome, axis, note) = match &answer {
            Ok(reply) => questions.verdict(reply, pass_below()),
            Err(err) if format!("{err:#}").contains("timed out") => (Outcome::Timeout, None, None),
            Err(err) => {
                tracing::debug!(error = %format!("{err:#}"), "a speech check failed; the message goes through");
                (Outcome::Error, None, None)
            }
        };
        let (model, cost, answers) = match &answer {
            Ok(reply) => (
                reply.model.clone(),
                quality::Cost { input: reply.usage.input_tokens, output: reply.usage.output_tokens, ..Default::default() },
                Some(Value::Object(reply.answers.iter().map(|(k, a)| (k.clone(), a.to_json())).collect())),
            ),
            Err(_) => (NO_REPLY_MODEL.to_string(), quality::Cost::default(), None),
        };
        tracing::info!(
            scope = ?self.scope,
            outcome = ?outcome,
            axis = axis.as_deref().unwrap_or(""),
            latency_ms = elapsed.as_millis() as u64,
            budget_ms = self.budget.as_millis() as u64,
            tokens_in = cost.input,
            tokens_out = cost.output,
            model = %model,
            "speech check"
        );
        let record = quality::Record::Check(quality::Check {
            ts: Utc::now(),
            surface: quality::Surface::Speech,
            turn: self.key,
            message: self.message,
            scope: self.scope,
            outcome,
            axis,
            note: note.clone(),
            latency_ms: elapsed.as_millis() as u64,
            model,
            cost,
            budget_ms: self.budget.as_millis() as u64,
            answers,
        });
        let data_dir = self.data_dir;
        tokio::spawn(async move {
            if let Err(err) = quality::append(&data_dir, &record).await {
                tracing::warn!(error = %format!("{err:#}"), "could not record a speech check");
            }
        });
        note.map(|n| (outcome, n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triage_is_facts_and_a_short_first_reply_goes_straight_out() {
        assert_eq!(triage(false, 0, "好，我去查一下"), None);
        assert_eq!(triage(true, 0, "好"), Some(Scope::Report));
        assert_eq!(triage(false, 1, "好"), Some(Scope::Later));
        assert_eq!(triage(false, 0, &"字".repeat(TRIAGE_CHARS + 1)), Some(Scope::Long));
        assert_eq!(triage(false, 0, &"字".repeat(TRIAGE_CHARS)), None);
    }

    #[test]
    fn the_case_puts_the_reader_first_and_the_message_last() {
        let brief = Brief {
            reader: "**赵力** — 简要汇报".into(),
            recent: "> 部署一下".into(),
            signals: "(from session cognition) 部署了".into(),
            screen: "## On screen now\n- ktv/status".into(),
            carries_report: true,
        };
        let case = brief.case(&["好".into()], &["show ktv/deploy".into()], "## The message to judge\n部署了，看过，正常");
        let at = |s: &str| case.find(s).unwrap_or_else(|| panic!("{s} missing from {case}"));
        assert!(at("简要汇报") < at("> 部署一下"));
        assert!(at("> 部署一下") < at("(from session cognition)"));
        assert!(at("- 好") < at("部署了，看过，正常"));
        assert!(at("show ktv/deploy") < at("部署了，看过，正常"));
        assert!(case.trim_end().ends_with("部署了，看过，正常"));
    }

    /// A mouth under test must not reach for a judge: off reads nothing and passes.
    #[tokio::test]
    async fn off_passes_everything_without_a_turn_or_a_model() {
        let speech = Speech::off();
        assert!(matches!(speech.review(&"x".repeat(500), Instant::now()).await, Review::Pass));
        speech.begin(Brief { carries_report: true, ..Brief::default() }).await;
        assert!(matches!(speech.review("x", Instant::now()).await, Review::Pass));
    }

    /// With nobody configured the check cannot run, and a check that cannot run passes.
    #[tokio::test]
    async fn an_unconfigured_install_sends_everything() {
        let dir = tempfile::tempdir().unwrap();
        let speech = Speech::new(dir.path().to_path_buf(), true);
        speech.begin(Brief { carries_report: true, ..Brief::default() }).await;
        assert!(matches!(speech.review("部署了", Instant::now()).await, Review::Pass));
        speech.note_sent("部署了");
        let ended = speech.end().unwrap();
        assert_eq!(ended.sent, vec!["部署了".to_string()]);
        assert!(speech.end().is_none(), "a turn ends once");
    }

    /// One send-back a turn: what was waiting behind it goes back with it, what was written
    /// after reading it goes out.
    #[tokio::test]
    async fn after_one_send_back_only_what_was_already_waiting_goes_back() {
        let dir = tempfile::tempdir().unwrap();
        let speech = Speech::new(dir.path().to_path_buf(), true);
        let before = Instant::now();
        speech.begin(Brief::default()).await;
        speech.draft.lock().unwrap().as_mut().unwrap().sent_back_at = Some(Instant::now());
        assert!(matches!(speech.review("leans on it", before).await, Review::SendBack(n) if n == LEANS_ON_IT));
        assert!(matches!(speech.review("the rewrite", Instant::now()).await, Review::Pass));
    }

    async fn installed_questions(dir: &std::path::Path) -> Questions {
        let standard = crate::identity::reading_standard(dir).await;
        Questions::build(crate::identity::judges::CHECK, &standard).expect("the rubric and the table build")
    }

    /// One `noul` per axis of the table but `unsaid`, and one `choice` over `pass` and those
    /// same axes — every one of them carrying its line, none of them a template left unfilled.
    #[tokio::test]
    async fn every_axis_but_unsaid_is_asked_once_and_offered_once() {
        let dir = tempfile::tempdir().unwrap();
        let q = installed_questions(dir.path()).await;
        let axes = ["known", "machinery", "repeat", "hard", "defensive", "shape", "unsupported", "buried"];
        assert_eq!(q.asked.len(), axes.len() + 1);
        for axis in axes {
            let asked = q.asked[&format!("fails_{axis}")]["instructions"].as_str().unwrap();
            let line = &q.lines.iter().find(|(a, _)| a == axis).unwrap().1;
            assert!(asked.contains(line.as_str()) && asked.starts_with("The last section"), "{axis}: {asked}");
        }
        assert!(!q.asked.contains_key("fails_unsaid"));
        let options = q.asked[CHOICE]["criteria"].as_object().unwrap();
        assert_eq!(options.len(), axes.len() + 1);
        assert!(options.contains_key("pass") && !options.contains_key("unsaid"));
        assert!(!serde_json::to_string(&q.asked).unwrap().contains("{line}"));
    }

    fn reply(mass: Value) -> anyhow::Result<decision::Reply> {
        let mut answers = std::collections::BTreeMap::new();
        answers.insert(
            CHOICE.to_string(),
            decision::Answer::Choice { choice: "x".into(), probabilities: Some(mass), confidence: None },
        );
        answers.insert("fails_known".to_string(), decision::Answer::Noul { p: 0.2 });
        Ok(decision::Reply {
            model: "jev-1.13.0".into(),
            answers,
            usage: decision::Usage { input_tokens: 9000, output_tokens: 230 },
        })
    }

    /// Too little on `pass` is a send-back on the axis the rest leans to, noted with that axis's
    /// line; enough on `pass`, a reply that cannot be read, and no reply at all all pass.
    #[tokio::test]
    async fn too_little_on_pass_is_a_send_back_and_anything_unreadable_passes() {
        let dir = tempfile::tempdir().unwrap();
        let q = installed_questions(dir.path()).await;
        let record = || CheckRecord {
            data_dir: dir.path().to_path_buf(),
            key: "t".into(),
            message: "m".into(),
            scope: Scope::Report,
            budget: budget(),
        };
        let fast = Duration::from_millis(500);
        let leaning = json!({ "pass": 0.08, "unsupported": 0.80, "known": 0.07, "repeat": 0.05 });
        let (outcome, note) = record().write(&q, reply(leaning), fast).await.expect("sent back");
        assert_eq!(outcome, Outcome::Revise);
        assert!(note.starts_with("`unsupported` — it claims more than"), "{note}");

        let fine = json!({ "pass": 0.84, "known": 0.10, "repeat": 0.06 });
        assert_eq!(record().write(&q, reply(fine), fast).await, None);
        let no_pass = json!({ "known": 0.9, "repeat": 0.1 });
        assert_eq!(record().write(&q, reply(no_pass), fast).await, None);
        assert_eq!(record().write(&q, Err(anyhow::anyhow!("timed out")), budget()).await, None);
        assert_eq!(record().write(&q, Err(anyhow::anyhow!("503 no healthy upstream")), fast).await, None);
    }

    /// The budget is a setting, read per call, and unset it is the check's own.
    #[test]
    fn the_budget_comes_from_its_setting() {
        use crate::body::legibility::budget as read_budget;
        assert_eq!(read_budget("nothing-is-set-here", 10_000), Duration::from_secs(10));
        assert_eq!(budget(), Duration::from_millis(CHECK_BUDGET_MS));
    }
}
