//! The pre-send check: host-code triage (§ D), then one model request (§ E).
//!
//! **It reads a message between `hi_say` accepting it and the floor**, and it can do one of
//! two things: let it through, or answer `not sent — <note>` so Reaction rewrites or drops
//! it inside the same turn. It never writes words — a checker that edits is a second mouth
//! (`docs/arch/arch.md` invariant 1).
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
//! - **shadow by default** — the verdict is recorded and nothing is sent back until its
//!   latency and its agreement with the audit are known ([`Mode`]).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::time::Instant;

use crate::foundation::config::tunables;
use crate::mind::memory::quality::{self, Outcome, Scope};

use crate::body::legibility::judge::Judge;
use crate::body::legibility::{Mode, read_verdict};

/// Past this many characters a message is more than a short reply, and in scope. A
/// starting value (`docs/arch/legibility.md` § Open), for the replay set to settle.
pub(crate) const TRIAGE_CHARS: usize = 120;

/// How long a live check may hold a message. Past it the message goes out.
pub(crate) const CHECK_LIMIT: Duration = Duration::from_millis(2_500);

/// How long a shadow check may run and still record a verdict. Longer than the live limit
/// on purpose: what shadow is measuring includes how often the live limit would be missed.
const SHADOW_LIMIT: Duration = Duration::from_secs(30);

/// The `app_settings` key choosing the mode, and the one choosing the model.
pub(crate) const MODE_KEY: &str = "speech_check";
pub(crate) const MODEL_KEY: &str = "speech_check_model";

/// The note a message gets when it was waiting behind one that was sent back.
const LEANS_ON_IT: &str = "a message sent in the same breath was sent back, and this one may \
lean on it — read the note on that one first";

/// The speech check's mode, from its setting.
pub fn mode() -> Mode {
    Mode::from_setting(tunables::get(MODE_KEY).as_deref())
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
    judge: Option<Judge>,
    instructions: Arc<String>,
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
    mode: Mode,
    draft: std::sync::Mutex<Option<Draft>>,
    /// The spoken turns since the person last wrote, waiting to be read against their next
    /// message ([`super::audit::on_reply`]).
    awaiting: std::sync::Mutex<Vec<(String, Vec<String>)>>,
    /// Held from a message's arrival at the mouth to its fate, so messages are read and
    /// sent in the order they were written.
    pub(crate) serial: tokio::sync::Mutex<()>,
}

impl Speech {
    pub fn new(data_dir: PathBuf, mode: Mode) -> Self {
        Self {
            data_dir,
            mode,
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
        Self::new(PathBuf::new(), Mode::Off)
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Open a turn. Returns its key, which every record about it carries.
    pub async fn begin(&self, brief: Brief) -> String {
        let key = uuid::Uuid::now_v7().to_string();
        let (judge, instructions) = if self.mode == Mode::Off {
            (None, String::new())
        } else {
            (
                Judge::resolve(&self.data_dir, MODEL_KEY),
                crate::identity::judge_instructions(&self.data_dir, crate::identity::judges::CHECK)
                    .await,
            )
        };
        *self.draft.lock().unwrap_or_else(|p| p.into_inner()) = Some(Draft {
            key: key.clone(),
            brief,
            judge,
            instructions: Arc::new(instructions),
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
        if self.mode == Mode::Off {
            return Review::Pass;
        }
        let (scope, judge, instructions, case, key) = {
            let guard = self.draft.lock().unwrap_or_else(|p| p.into_inner());
            // Outside a turn — the warm-up — nothing reaches anyone anyway.
            let Some(d) = guard.as_ref() else { return Review::Pass };
            if let Some(at) = d.sent_back_at {
                // One send-back a turn. What was already waiting when it was answered goes
                // back with it; what Reaction wrote after reading it goes out.
                return if arrived < at && self.mode == Mode::On {
                    Review::SendBack(LEANS_ON_IT.to_string())
                } else {
                    Review::Pass
                };
            }
            let Some(scope) = triage(d.brief.carries_report, d.sent.len(), text) else {
                return Review::Pass;
            };
            let Some(judge) = d.judge.clone() else { return Review::Pass };
            let case =
                d.brief.case(&d.sent, &d.shown, &format!("## The message to judge\n{text}"));
            (scope, judge, d.instructions.clone(), case, d.key.clone())
        };

        let record = CheckRecord {
            data_dir: self.data_dir.clone(),
            key,
            message: text.to_string(),
            scope,
            mode: self.mode,
            model: judge.model().to_string(),
        };
        match self.mode {
            Mode::Off => Review::Pass,
            Mode::Shadow => {
                tokio::spawn(async move {
                    let started = Instant::now();
                    let answer = judge.ask(&instructions, &case, SHADOW_LIMIT).await;
                    record.write(answer, started.elapsed()).await;
                });
                Review::Pass
            }
            Mode::On => {
                let started = Instant::now();
                let answer = match tokio::time::timeout(
                    CHECK_LIMIT.saturating_sub(arrived.elapsed().min(CHECK_LIMIT)),
                    judge.ask(&instructions, &case, CHECK_LIMIT),
                )
                .await
                {
                    Ok(answer) => answer,
                    Err(_) => Err(anyhow::anyhow!("timed out")),
                };
                let elapsed = started.elapsed();
                let verdict = record.write(answer, elapsed).await;
                match verdict {
                    Some((Outcome::Revise, note)) => {
                        if let Some(d) =
                            self.draft.lock().unwrap_or_else(|p| p.into_inner()).as_mut()
                        {
                            d.sent_back_at = Some(Instant::now());
                        }
                        Review::SendBack(note)
                    }
                    _ => Review::Pass,
                }
            }
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
    mode: Mode,
    model: String,
}

impl CheckRecord {
    /// Read the answer, record it, and hand back the verdict and note. An answer that
    /// cannot be read is an error, and an error passes.
    async fn write(
        self,
        answer: anyhow::Result<String>,
        elapsed: Duration,
    ) -> Option<(Outcome, String)> {
        let (outcome, axis, note) = read_verdict(answer, elapsed, CHECK_LIMIT, self.mode);
        tracing::info!(
            mode = self.mode.as_str(),
            scope = ?self.scope,
            outcome = ?outcome,
            axis = axis.as_deref().unwrap_or(""),
            latency_ms = elapsed.as_millis() as u64,
            "speech check"
        );
        let record = quality::Record::Check(quality::Check {
            ts: Utc::now(),
            surface: quality::Surface::Speech,
            turn: self.key,
            message: self.message,
            scope: self.scope,
            mode: self.mode.as_str().to_string(),
            outcome,
            axis,
            note: note.clone(),
            latency_ms: elapsed.as_millis() as u64,
            model: self.model,
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
    fn the_mode_is_shadow_unless_it_is_set() {
        assert_eq!(Mode::from_setting(None), Mode::Shadow);
        assert_eq!(Mode::from_setting(Some(" ON ")), Mode::On);
        assert_eq!(Mode::from_setting(Some("off")), Mode::Off);
        assert_eq!(Mode::from_setting(Some("maybe")), Mode::Shadow);
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

    /// A mouth under test must not reach for a model: off reads nothing and passes.
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
        let speech = Speech::new(dir.path().to_path_buf(), Mode::On);
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
        let speech = Speech::new(dir.path().to_path_buf(), Mode::On);
        let before = Instant::now();
        speech.begin(Brief::default()).await;
        speech.draft.lock().unwrap().as_mut().unwrap().sent_back_at = Some(Instant::now());
        assert!(matches!(speech.review("leans on it", before).await, Review::SendBack(n) if n == LEANS_ON_IT));
        assert!(matches!(speech.review("the rewrite", Instant::now()).await, Review::Pass));
    }

    #[tokio::test]
    async fn a_revise_with_a_note_is_a_send_back_and_anything_unreadable_passes() {
        let dir = tempfile::tempdir().unwrap();
        let record = |mode| CheckRecord {
            data_dir: dir.path().to_path_buf(),
            key: "t".into(),
            message: "m".into(),
            scope: Scope::Report,
            mode,
            model: "judge".into(),
        };
        let revise = r#"{"verdict":"revise","axis":"known","note":"「还没读完」是读的人默认的"}"#;
        assert_eq!(
            record(Mode::On).write(Ok(revise.into()), Duration::from_millis(900)).await,
            Some((Outcome::Revise, "「还没读完」是读的人默认的".into()))
        );
        let bare = r#"{"verdict":"revise","axis":"known","note":""}"#;
        assert_eq!(record(Mode::On).write(Ok(bare.into()), Duration::from_millis(900)).await, None);
        assert_eq!(record(Mode::On).write(Ok("not json".into()), Duration::ZERO).await, None);
        assert_eq!(
            record(Mode::On).write(Err(anyhow::anyhow!("timed out")), CHECK_LIMIT).await,
            None
        );
    }
}
