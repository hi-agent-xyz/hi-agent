//! Read drafts before the floor holds them; reject with feedback, never rewrite them.
//!
//! With a conversation model, every speech draft and rewrite gets an independent
//! evidence-and-purpose read. Failed or exhausted reviews do not approve a draft.
//! Only an accepted, current-report-backed notice can acquire a delivery obligation.
//! The typed triage/check path remains for installs without that model. Its one-send-back
//! policy is not used by the independent path. Missing source evidence starts a retained
//! background check rather than merely another rewrite.
//! `speech_check=off` disables both paths. See `docs/arch/legibility.md`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde_json::{Map, Value, json};
use tokio::time::Instant;

use crate::body::capabilities::decision;
use crate::foundation::config::tunables;
use crate::mind::memory::quality::{self, Outcome, Scope};
use crate::body::legibility::judge::{self, Judge};

/// Past this many characters a message is more than a short reply, and in scope. A
/// starting value (`docs/arch/legibility.md` § Open), for the replay set to settle.
pub(crate) const TRIAGE_CHARS: usize = 120;

/// Primary check budget, unless `speech_check_budget_ms` says otherwise.
/// Past it the fallback gets its own bounded attempt.
///
/// **Two and a half seconds, which is what it was first meant to be.** Someone is waiting on
/// this one. It went to ten on 2026-09-20 because a model writing a verdict could not meet
/// two and a half at all — ~90% of its tokens were thinking before the verdict began. System
/// One writes no verdict: measured 2026-09-21 on the full case of every audited message on this
/// install, p50 0.53 s, p99 2.2 s, one of 167 past two and a half.
const CHECK_BUDGET_MS: u64 = 2_500;
const FALLBACK_BUDGET_MS: u64 = 45_000;

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

    async fn checked(&self, dir: &std::path::Path, case: String, limit: Duration) -> (anyhow::Result<decision::Reply>, Option<String>) {
        let first = self.ask(case.clone(), limit).await.and_then(|r| {
            anyhow::ensure!(self.verdict(&r, pass_below()).0 != Outcome::Error, "speech check response has no readable axis probabilities");
            Ok(r)
        });
        let error = match first {
            Ok(reply) => return (Ok(reply), None),
            Err(error) => format!("{error:#}"),
        };
        tracing::warn!(error = %error, "speech check unavailable; asking the fallback model");
        let fallback = match Judge::resolve(dir, "speech_check_fallback_model") {
            Some(model) => self.by_model(&model, &case).await,
            None => Err(anyhow::anyhow!("no speech check fallback model configured")),
        };
        (fallback, Some(error))
    }

    async fn by_model(&self, model: &Judge, case: &str) -> anyhow::Result<decision::Reply> {
        use anyhow::Context as _;
        let options = &self.asked[CHOICE]["criteria"];
        let limit = crate::body::legibility::budget("speech_check_fallback_budget_ms", FALLBACK_BUDGET_MS);
        let body = json!({
            "instructions": format!("Read this proposed message against the conversation and material it actually received. \
                Do not rewrite it. Choose pass unless it plainly fails one criterion. \
                A prior assistant assertion is not independent evidence for that assertion. \
                Return JSON only: {{\"axis\":\"pass or one criterion key\",\"reason\":\"one short reason\"}}. \
                Criteria: {options}"),
            "input": [{"role":"user","content":[{"type":"input_text","text":case}]}],
            "text": {"format":{"type":"json_object"}},
            "store": false,
        });
        let raw = model.respond(&body, limit).await?;
        let text = judge::output_text(&raw).context("speech fallback returned no text")?;
        let value: Value = judge::json_object(&text).context("speech fallback returned invalid JSON")?;
        let axis = value["axis"].as_str().context("speech fallback returned no axis")?;
        anyhow::ensure!(options.get(axis).is_some(), "speech fallback returned an unknown axis");
        let cost = judge::cost(&raw);
        // Adapt the categorical verdict for the existing cut; records keep its category,
        // not these internal 0/1 values as if they were calibrated probabilities.
        let mut mass = serde_json::Map::new();
        mass.insert("pass".into(), json!(if axis == "pass" {1.0} else {0.0}));
        mass.insert(axis.to_string(), json!(1.0));
        let answer = decision::Answer::Choice {
            choice: axis.to_string(), probabilities: Some(Value::Object(mass)), confidence: None,
        };
        Ok(decision::Reply { model: model.model().to_string(), answers: [(CHOICE.to_string(), answer)].into(),
            usage: decision::Usage { input_tokens:cost.input, output_tokens:cost.output } })
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
    /// `None` when the rubric lost a section; provider availability is checked when asking
    /// so the model fallback can still run without System One.
    questions: Option<Arc<Questions>>,
    sent: Vec<String>,
    shown: Vec<String>,
    /// When this turn's one send-back was answered.
    sent_back_at: Option<Instant>,
    purpose_reads: usize,
    notices: std::collections::BTreeSet<String>,
    purpose_records: Vec<Value>,
    revision: u64,
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
    Verify { id: String, note: String },
}

/// The check's state, shared by the turn loop (which opens and closes a turn) and the
/// mouth (which asks about each message).
pub struct Speech {
    data_dir: PathBuf,
    record_dir: PathBuf,
    enabled: bool,
    dispatch_checks: bool,
    pub(crate) verification: super::verification::Store,
    draft: std::sync::Mutex<Option<Draft>>,
    /// The spoken turns since the person last wrote, waiting to be read against their next
    /// message ([`super::audit::on_reply`]).
    awaiting: std::sync::Mutex<Vec<(String, Vec<String>)>>,
}

impl Speech {
    pub fn new(data_dir: PathBuf, enabled: bool) -> Self {
        let verification = super::verification::Store::new(&data_dir);
        Self {
            record_dir: data_dir.clone(),
            data_dir,
            enabled,
            dispatch_checks: true,
            verification,
            draft: std::sync::Mutex::new(None),
            awaiting: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub(super) fn for_replay(data_dir: PathBuf, record_dir: PathBuf) -> Self {
        let mut speech = Self::new(data_dir, true);
        speech.dispatch_checks = false;
        speech.verification = super::verification::Store::new(&record_dir);
        speech.record_dir = record_dir;
        speech
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

    pub(crate) async fn recover_checks(&self) {
        if self.enabled && self.dispatch_checks {
            if let Err(error) = self.verification.dispatch().await {
                tracing::error!(error = %error, "pending speech checks could not be recovered");
            }
        }
    }

    pub(crate) async fn pending_checks(&self) -> String {
        if !self.enabled { return String::new(); }
        match self.verification.pending().await {
            Ok(pending) => super::verification::render(&pending),
            Err(error) => format!("Pending speech checks are unreadable ({error}); do not treat this as no answer owed."),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Open a turn. Returns its key, which every record about it carries.
    pub async fn begin(&self, brief: Brief) -> String {
        self.recover_checks().await;
        let key = uuid::Uuid::now_v7().to_string();
        let questions = if !self.enabled {
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
            purpose_reads: 0,
            notices: Default::default(),
            purpose_records: Vec::new(),
            revision: 0,
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

    /// A message went out. The floor releases lines when the room reaches their moment, which
    /// may be after the turn that wrote them has closed: then it is kept with the spoken turns
    /// their next message answers, so what a reply is read against — and where a prepared
    /// matter was left — still has it. The per-turn audit of that closed turn does not.
    pub fn note_sent(&self, text: &str) {
        if let Some(d) = self.draft.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
            d.sent.push(text.to_string());
            return;
        }
        const HELD: usize = 4;
        let mut held = self.awaiting.lock().unwrap_or_else(|p| p.into_inner());
        held.push((uuid::Uuid::now_v7().to_string(), vec![text.to_string()]));
        let over = held.len().saturating_sub(HELD);
        held.drain(..over);
    }

    /// Everything the agent has said since the person last wrote, oldest first: the spoken
    /// turns held for their reply, then this turn's. What a branch prepared now will be
    /// read against, because it is what their next message answers ([`super::super::prepared`]).
    pub fn said_since_their_last(&self) -> Vec<String> {
        let mut said: Vec<String> = self
            .awaiting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .flat_map(|(_, sent)| sent.iter().cloned())
            .collect();
        if let Some(d) = self.draft.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            said.extend(d.sent.iter().cloned());
        }
        said
    }

    /// Decide whether `text` goes on to the floor. `arrived` is when it reached the mouth,
    /// before it queued behind anything.
    pub async fn review(&self, text: &str, arrived: Instant) -> Review {
        if !self.enabled {
            return Review::Pass;
        }
        if Judge::resolve(&self.data_dir, "speech_review_model").is_some() {
            if let Some(review) = self.deliberate(text).await {
                return review;
            }
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
                    &self.record_dir,
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
            data_dir: self.record_dir.clone(),
            key,
            message: text.to_string(),
            scope,
            budget: live,
        };
        let started = Instant::now();
        let (answer, primary_error) = questions.checked(&self.data_dir, case, live.saturating_sub(arrived.elapsed().min(live))).await;
        match record.write(&questions, answer, started.elapsed(), primary_error).await {
            Some((Outcome::Revise, note)) => {
                if let Some(d) = self.draft.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
                    d.sent_back_at = Some(Instant::now());
                }
                Review::SendBack(note)
            }
            _ => Review::Pass,
        }
    }

    pub(crate) fn required_notice(&self, text: &str) -> bool {
        self.draft.lock().unwrap_or_else(|p| p.into_inner()).as_ref()
            .is_some_and(|draft| draft.notices.contains(text))
    }

    pub(crate) fn note_incoming(&self, signal: &str) {
        if let Some(d) = self.draft.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
            d.brief.signals.push_str(&format!("\n\n{signal}"));
            d.revision += 1;
            d.purpose_reads = 0;
            d.notices.clear();
        }
    }

    pub(super) fn purpose_records(&self) -> Vec<Value> {
        self.draft.lock().unwrap_or_else(|p| p.into_inner()).as_ref()
            .map(|draft| draft.purpose_records.clone()).unwrap_or_default()
    }

    async fn deliberate(&self, text: &str) -> Option<Review> {
        let (key, revision, brief, sent, shown, exhausted) = {
            let mut guard = self.draft.lock().unwrap_or_else(|p| p.into_inner());
            let d = guard.as_mut()?;
            // Never treat the next rewrite as approved just because a prior draft failed.
            // Bound paid reviews instead of silently accepting an unreviewed rewrite.
            let exhausted = d.purpose_reads >= 6;
            if !exhausted { d.purpose_reads += 1; }
            (d.key.clone(), d.revision, d.brief.clone(), d.sent.clone(), d.shown.clone(), exhausted)
        };
        if exhausted {
            return Some(self.retain_check(&key, &brief,
                "No reply passed review. Recover the actual answer owed to the person, verify any project-specific explanation against its source, and return only the necessary result.",
                text, "Review budget exhausted. The answer remains owed; another unreviewed rewrite cannot pass.").await);
        }
        let started = Instant::now();
        let pending = match self.verification.pending().await {
            Ok(pending) => pending,
            Err(error) => return Some(Review::SendBack(format!("Pending verification is unreadable: {error}; nothing is approved."))),
        };
        let result = super::deliberate::read(&self.data_dir, &brief, &sent, &shown, text, &pending).await;
        let (decision, model, cost) = match result {
            Ok(value) => value,
            Err(error) => {
                let diagnostic = json!({"message":text,"error":format!("{error:#}")});
                if let Some(d) = self.draft.lock().unwrap_or_else(|p| p.into_inner()).as_mut().filter(|d| d.key == key) {
                    d.purpose_records.push(diagnostic);
                }
                tracing::warn!(error = %format!("{error:#}"), "purpose review failed; draft remains unapproved");
                return Some(self.retain_check(&key, &brief,
                    "The reply review failed. Determine the actual answer owed from the original conversation and inspect the relevant source before proposing an explanation. The blocked draft is not evidence.",
                    text, "The review was unavailable; the answer remains owed, not approved.").await);
            }
        };
        {
            let mut guard = self.draft.lock().unwrap_or_else(|p| p.into_inner());
            let Some(d) = guard.as_mut().filter(|d| d.key == key) else {
                return Some(Review::SendBack("The turn ended during review; this approval cannot be used for another turn.".into()));
            };
            if d.revision != revision {
                return Some(Review::SendBack("New input arrived during review. Read it and prepare the reply again; this review is stale.".into()));
            }
            if decision.accept && decision.notice != "none" { d.notices.insert(text.to_string()); }
            d.purpose_records.push(json!({"message":text,"decision":decision}));
        }
        let record = quality::Record::Check(quality::Check {
            ts:Utc::now(), surface:quality::Surface::Speech, turn:key.clone(), message:text.into(),
            scope:if brief.carries_report { Scope::Report } else { Scope::Later },
            outcome:if decision.accept { Outcome::Pass } else { Outcome::Revise },
            axis:if decision.accept { None } else { Some(decision.axis.clone()) },
            note:Some(format!("{} {}", decision.reason, decision.instruction)),
            latency_ms:started.elapsed().as_millis() as u64, model, cost, budget_ms:540_000,
            answers:Some(json!({"purpose":decision})),
        });
        if let Err(error) = quality::append(&self.record_dir,&record).await {
            tracing::warn!(error = %error,"could not record purpose review");
        }
        if decision.accept && !decision.answers.is_empty() {
            if let Err(error) = self.verification.approve(text, &decision.answers).await {
                return Some(Review::SendBack(format!("The answer could not be retained for delivery: {error}")));
            }
        }
        if !decision.accept && let Some(question) = &decision.verify {
            return Some(self.retain_check(&key, &brief, question, text, &format!("{} {}", decision.reason, decision.instruction)).await);
        }
        Some(if decision.accept { Review::Pass } else {
            Review::SendBack(format!("{} — {} {}", decision.axis, decision.reason, decision.instruction))
        })
    }

    async fn retain_check(&self, key: &str, brief: &Brief, question: &str, text: &str, note: &str) -> Review {
        let context = format!("{}\n\n{}", brief.recent, brief.signals);
        match self.verification.request(key, &context, question, text).await {
            Ok(id) => {
                self.recover_checks().await;
                Review::Verify { id, note:note.into() }
            }
            Err(error) => Review::SendBack(format!("Verification could not be retained: {error}. Do not send this explanation.")),
        }
    }
}

impl Speech {
    /// Read a line prepared for the person's next message (`hi_prepare`), before it is held.
    ///
    /// **The same questions, asked about the moment the line is for.** The case is this
    /// turn's — the conversation, what went out, what is on screen — with the condition that
    /// would release the line stated above it, since that is when it would be read.
    ///
    /// Two things differ from [`review`](Self::review), and both follow from nobody waiting on
    /// it. **It does not take the mouth's serial lock**: a line that may never be said must not
    /// put seconds in front of one somebody is waiting for. **It does not spend the turn's one
    /// send-back**: that limit bounds how long a person waits, and a prepared line holds up
    /// nobody. A send-back here drops the branch's actions — its condition stays in the set —
    /// and the caller says so in the tool's answer, so Reaction can prepare again.
    ///
    /// Recorded under [`Scope::Later`], which is true — a prepared line is written after the
    /// turn's first message — though it does not tell a prepared line from a spoken one; the
    /// message text says which.
    pub async fn review_prepared(&self, condition: &str, text: &str) -> Review {
        if !self.enabled {
            return Review::Pass;
        }
        if Judge::resolve(&self.data_dir, "speech_review_model").is_some()
            && let Some(review) = self.deliberate(text).await {
            return review;
        }
        let (questions, case, key) = {
            let guard = self.draft.lock().unwrap_or_else(|p| p.into_inner());
            let Some(d) = guard.as_ref() else { return Review::Pass };
            let Some(questions) = d.questions.clone() else { return Review::Pass };
            let then = format!(
                "## Prepared for the person's next message — said only if it is: {condition}\n\n\
                 ## The message to judge\n{text}"
            );
            (questions, d.brief.case(&d.sent, &d.shown, &then), d.key.clone())
        };
        let live = budget();
        let record = CheckRecord {
            data_dir: self.record_dir.clone(),
            key,
            message: text.to_string(),
            scope: Scope::Later,
            budget: live,
        };
        let started = Instant::now();
        let (answer, primary_error) = questions.checked(&self.data_dir, case, live).await;
        match record.write(&questions, answer, started.elapsed(), primary_error).await {
            Some((Outcome::Revise, note)) => Review::SendBack(note),
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
        primary_error: Option<String>,
    ) -> Option<(Outcome, String)> {
        let allowed = if primary_error.is_some() {
            self.budget + crate::body::legibility::budget("speech_check_fallback_budget_ms", FALLBACK_BUDGET_MS)
        } else { self.budget };
        let (outcome, axis, note) = match &answer {
            Ok(reply) => questions.verdict(reply, pass_below()),
            Err(err) if format!("{err:#}").contains("timed out") => (Outcome::Timeout, None, None),
            Err(err) => {
                tracing::debug!(error = %format!("{err:#}"), "a speech check failed; the message goes through");
                (Outcome::Error, None, None)
            }
        };
        let diagnostic = match (&primary_error, &answer) {
            (Some(first), Err(last)) => Some(format!("primary: {first}; fallback: {last:#}")),
            (Some(first), Ok(_)) => Some(format!("fallback used; primary: {first}")),
            (None, Err(error)) => Some(format!("{error:#}")),
            _ => None,
        };
        let (model, cost, answers) = match &answer {
            Ok(reply) => (
                reply.model.clone(),
                quality::Cost { input: reply.usage.input_tokens, output: reply.usage.output_tokens, ..Default::default() },
                Some(if primary_error.is_some() {
                    json!({ "fallback": { "axis": axis.as_deref().unwrap_or("pass"), "kind": "categorical" } })
                } else {
                    Value::Object(reply.answers.iter().map(|(k, a)| (k.clone(), a.to_json())).collect())
                }),
            ),
            Err(_) => (NO_REPLY_MODEL.to_string(), quality::Cost::default(), None),
        };
        tracing::info!(
            scope = ?self.scope,
            outcome = ?outcome,
            axis = axis.as_deref().unwrap_or(""),
            latency_ms = elapsed.as_millis() as u64,
            budget_ms = allowed.as_millis() as u64,
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
            note: match (&note, diagnostic) {
                (Some(note), Some(error)) => Some(format!("{note}; {error}")),
                (_, Some(error)) => Some(error),
                _ => note.clone(),
            },
            latency_ms: elapsed.as_millis() as u64,
            model,
            cost,
            budget_ms: allowed.as_millis() as u64,
            answers,
        });
        let data_dir = self.data_dir;
        if let Err(err) = quality::append(&data_dir, &record).await {
            tracing::warn!(error = %format!("{err:#}"), "could not record a speech check");
        }
        note.map(|n| (outcome, n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "uses the configured providers and spends model calls; make eval-checks"]
    async fn speech_checks_live() {
        let dir = PathBuf::from(std::env::var("HI_AGENT_EVAL_DATA").expect("DATA is required"));
        tunables::init(&dir);
        crate::body::capabilities::init(&crate::foundation::credentials::Credentials::try_load(&dir).unwrap()).unwrap();
        let q = installed_questions(&dir).await;
        let cases = [
            "## The conversation\n> 这段结果是怎么来的？\n## The message to judge\n我还没查到生成它的记录。",
            "## The conversation\n> 请把白色标记换成红色。\n## The message to judge\n你要把白色标记换成红色，因为白色看不清，我知道了。我会认真去检查怎么修改。",
        ];
        for case in cases {
            let started = Instant::now();
            match q.ask(case.to_string(), Duration::from_secs(15)).await {
                Ok(reply) => eprintln!("primary {} ms: {:?}", started.elapsed().as_millis(), q.verdict(&reply, pass_below())),
                Err(error) => eprintln!("primary {} ms: {error:#}", started.elapsed().as_millis()),
            }
        }
        // Force the primary deadline to exercise recovery even while its provider is healthy.
        let (reply, failure) = q.checked(&dir,cases[1].into(),Duration::ZERO).await;
        assert!(failure.is_some());
        let reply = reply.expect("fallback must answer this live probe");
        eprintln!("forced primary outage -> {}: {:?}",reply.model,q.verdict(&reply,pass_below()));
        assert_ne!(q.verdict(&reply,pass_below()).0,Outcome::Error);
        if let Some((brief, message)) = super::super::replay::historical_check_case(&dir, "是我这边早先手工点的一次") {
            let case = brief.case(&[], &[], &format!("## The message to judge\n{message}"));
            eprintln!("historical case: {} characters", case.chars().count());
            match q.ask(case, Duration::from_secs(15)).await {
                Ok(reply) => eprintln!("historical primary: {:?}", q.verdict(&reply, pass_below())),
                Err(error) => eprintln!("historical primary: {error:#}"),
            }
        }
    }

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

    #[tokio::test]
    async fn replay_triage_records_do_not_enter_the_live_quality_log() {
        let source = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let speech = Speech::for_replay(source.path().to_path_buf(), output.path().to_path_buf());
        let key = speech.begin(Brief::default()).await;
        assert!(matches!(speech.review("好，我去查一下", Instant::now()).await, Review::Pass));
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let records = quality::read_since(output.path(), chrono::DateTime::<Utc>::MIN_UTC).await;
                if records.iter().any(|r| matches!(r, quality::Record::Skipped(s) if s.turn == key)) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("the replay skip record must be written");
        assert!(quality::read_since(source.path(), chrono::DateTime::<Utc>::MIN_UTC).await.is_empty());
    }

    #[tokio::test]
    async fn review_exhaustion_retains_the_answer_instead_of_approving_or_forgetting_it() {
        let source = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let speech = Speech::for_replay(source.path().into(), output.path().into());
        speech.begin(Brief { signals:"Where did these lines come from?".into(), ..Brief::default() }).await;
        speech.draft.lock().unwrap().as_mut().unwrap().purpose_reads = 6;
        assert!(matches!(speech.deliberate("a human clicked them").await, Some(Review::Verify { .. })));
        assert!(speech.end().unwrap().sent.is_empty());
        let retained = super::super::verification::Store::new(output.path()).pending().await.unwrap();
        assert_eq!(retained.len(), 1);
        assert!(retained[0].context.contains("Where did these lines come from?"));
        assert!(!source.path().join("memory/speech-verification.json").exists(), "replay must not mutate live pending answers");
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

    #[tokio::test]
    async fn fallback_accepts_only_a_known_axis_and_keeps_the_primary_failure() {
        use axum::{Router, routing::post};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route("/responses", post(|| async {
            axum::Json(json!({"output":[{"type":"message","content":[{"type":"output_text","text":"{\"axis\":\"repeat\",\"reason\":\"same information twice\"}"}]}]}))
        }));
        let server = tokio::spawn(async move { axum::serve(listener,app).await.unwrap(); });
        let dir = tempfile::tempdir().unwrap();
        let q = installed_questions(dir.path()).await;
        let reply = q.by_model(&Judge::new(&format!("http://{address}"),"test-key","test-model"),"case").await.unwrap();
        assert_eq!(q.verdict(&reply,pass_below()).0,Outcome::Revise);
        let record = CheckRecord { data_dir:dir.path().to_path_buf(),key:"fallback-test".into(),message:"m".into(),scope:Scope::Report,budget:budget() };
        assert!(record.write(&q,Ok(reply),Duration::ZERO,Some("primary request failed".into())).await.is_some());
        let records = quality::read_since(dir.path(),chrono::DateTime::<Utc>::MIN_UTC).await;
        let quality::Record::Check(saved) = &records[0] else { panic!("missing check") };
        assert_eq!(saved.model,"test-model");
        assert!(saved.note.as_ref().unwrap().contains("primary request failed"));
        assert_eq!(saved.answers.as_ref().unwrap()["fallback"]["kind"], "categorical");
        assert_eq!(saved.answers.as_ref().unwrap()["fallback"]["axis"], "repeat");
        assert!(saved.answers.as_ref().unwrap().get(CHOICE).is_none());
        server.abort();
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
        let (outcome, note) = record().write(&q, reply(leaning), fast, None).await.expect("sent back");
        assert_eq!(outcome, Outcome::Revise);
        assert!(note.starts_with("`unsupported` — it claims more than"), "{note}");

        let fine = json!({ "pass": 0.84, "known": 0.10, "repeat": 0.06 });
        assert_eq!(record().write(&q, reply(fine), fast, None).await, None);
        let no_pass = json!({ "known": 0.9, "repeat": 0.1 });
        assert_eq!(record().write(&q, reply(no_pass), fast, None).await, None);
        assert_eq!(record().write(&q, Err(anyhow::anyhow!("timed out")), budget(), None).await, None);
        assert_eq!(record().write(&q, Err(anyhow::anyhow!("503 no healthy upstream")), fast, None).await, None);
    }

    /// The budget is a setting, read per call, and unset it is the check's own.
    #[test]
    fn the_budget_comes_from_its_setting() {
        use crate::body::legibility::budget as read_budget;
        assert_eq!(read_budget("nothing-is-set-here", 10_000), Duration::from_secs(10));
        assert_eq!(budget(), Duration::from_millis(CHECK_BUDGET_MS));
    }
}
