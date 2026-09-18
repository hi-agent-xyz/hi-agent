//! The gate on a task's record (`docs/arch/legibility.md` § M): triage in code, one model
//! request, `not recorded — <note>` once, fail open, shadow first.
//!
//! **It reads a line, not a record.** A line is short, the worker writes it holding
//! everything it is about, and the card on the board draws one line of it — so a send-back
//! arrives while the writer can still act on it. *Where it stands* is long, rewritten rarely
//! and waited on by nobody, so it is judged after it lands (§ N), never here.
//!
//! **A refused line lands on the second attempt.** Speech can be dropped; a record cannot.
//! An unrecorded fact is worse than an ugly one, and a gate able to lose facts would be a
//! worse failure than the one it exists for. So the send-back is one per writer per row: the
//! next write from that writer on that row goes through whatever it says.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use chrono::Utc;
use tokio::time::Instant;

use super::judge::Judge;
use super::{Mode, read_verdict};
use crate::foundation::config::tunables;
use crate::mind::memory::quality::{self, Outcome, Scope, Surface};
use crate::mind::memory::tasks::{Note, Task};

/// The `app_settings` key choosing the mode, and the one choosing the model.
pub(crate) const MODE_KEY: &str = "record_check";
pub(crate) const MODEL_KEY: &str = "record_check_model";

/// Past this many characters a line is more than a card can draw, and in scope. A starting
/// value, like speech's (`docs/arch/legibility.md` § Open).
pub(crate) const TRIAGE_CHARS: usize = 120;

/// How long a live check may hold a line. Past it the line is written.
const CHECK_LIMIT: Duration = Duration::from_millis(2_500);

/// How long a shadow check may run and still record a verdict.
const SHADOW_LIMIT: Duration = Duration::from_secs(30);

/// How many of the row's newest lines the judge reads beside the new one — enough to see the
/// same thing being said again, which it cannot see otherwise.
const RECENT_LINES: usize = 8;

/// The record check's mode, from its setting. Shadow unless set otherwise.
pub fn mode() -> Mode {
    Mode::from_setting(tunables::get(MODE_KEY).as_deref())
}

/// What is about to be written on a row.
#[derive(Clone, Copy, Debug)]
pub enum Writing<'a> {
    /// A row being opened: its name and what they asked for.
    Open { title: &'a str, wanted: &'a str },
    /// Something said on a row that exists.
    Note { note: Note, text: &'a str },
}

pub enum Review {
    Pass,
    SendBack(String),
}

/// Whether a write is in the check's scope, and why. Facts only.
fn triage(writing: &Writing) -> Option<Scope> {
    match *writing {
        Writing::Open { .. } | Writing::Note { note: Note::Title, .. } => Some(Scope::Opening),
        Writing::Note { note: Note::Waiting, .. } => Some(Scope::Waiting),
        Writing::Note { note: Note::Stands, .. } => None,
        Writing::Note { text, .. } if text.chars().count() > TRIAGE_CHARS => Some(Scope::Long),
        Writing::Note { .. } => None,
    }
}

/// Writers whose last write on a row was sent back, so their next one goes through.
fn sent_back() -> &'static Mutex<HashSet<(String, String)>> {
    static SENT_BACK: std::sync::OnceLock<Mutex<HashSet<(String, String)>>> = std::sync::OnceLock::new();
    SENT_BACK.get_or_init(Default::default)
}

/// Decide whether `writing` goes on to the record. `writer` is the session writing it,
/// `task` the row as it stands (none when it is being opened).
pub async fn review(
    data_dir: &Path,
    writer: &str,
    subject: &str,
    task: Option<&Task>,
    writing: Writing<'_>,
) -> Review {
    let mode = mode();
    if mode == Mode::Off {
        return Review::Pass;
    }
    let key = (writer.to_owned(), subject.to_owned());
    if sent_back().lock().map(|mut s| s.remove(&key)).unwrap_or(false) {
        return Review::Pass;
    }
    let Some(scope) = triage(&writing) else { return Review::Pass };
    let Some(judge) = Judge::resolve(data_dir, MODEL_KEY) else { return Review::Pass };
    let instructions =
        crate::identity::judge_instructions(data_dir, crate::identity::judges::RECORD).await;
    let reader = crate::mind::memory::snapshot::for_record(data_dir).await;
    let case = case(&reader, task, &writing);
    let checked = Checked {
        data_dir: data_dir.to_path_buf(),
        subject: subject.to_owned(),
        message: describe(&writing),
        scope,
        mode,
        model: judge.model().to_string(),
    };

    match mode {
        Mode::Off => Review::Pass,
        Mode::Shadow => {
            tokio::spawn(async move {
                let started = Instant::now();
                let answer = judge.ask(&instructions, &case, SHADOW_LIMIT).await;
                checked.write(answer, started.elapsed()).await;
            });
            Review::Pass
        }
        Mode::On => {
            let started = Instant::now();
            let answer = match tokio::time::timeout(CHECK_LIMIT, judge.ask(&instructions, &case, CHECK_LIMIT)).await {
                Ok(answer) => answer,
                Err(_) => Err(anyhow::anyhow!("timed out")),
            };
            match checked.write(answer, started.elapsed()).await {
                (Outcome::Revise, Some(note)) => {
                    if let Ok(mut s) = sent_back().lock() {
                        s.insert(key);
                    }
                    Review::SendBack(note)
                }
                _ => Review::Pass,
            }
        }
    }
}

/// The case the judge reads: who the reader is, the row as it stands, and the write last.
fn case(reader: &str, task: Option<&Task>, writing: &Writing) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let section = |s: &mut String, title: &str, body: &str| {
        let body = body.trim();
        let _ = write!(s, "## {title}\n{}\n\n", if body.is_empty() { "(nothing)" } else { body });
    };
    section(&mut s, "Who is reading, and how they want to be told", reader);
    if let Some(task) = task {
        let asked = task.created().map(|c| c.text.as_str()).unwrap_or_default();
        section(&mut s, "The task", &format!("Title: {}\nWhat they asked for: {asked}", task.title));
        let skip = task.timeline.len().saturating_sub(RECENT_LINES);
        let lines = task.timeline[skip..]
            .iter()
            .map(|e| format!("- {} — {}", e.kind.as_str(), e.text))
            .collect::<Vec<_>>()
            .join("\n");
        section(&mut s, "Its newest lines, oldest first", &lines);
    }
    let _ = write!(s, "## What is about to be written\n{}\n", describe(writing));
    s
}

/// The write, as the judge and the record both name it.
fn describe(writing: &Writing) -> String {
    match *writing {
        Writing::Open { title, wanted } => format!("A new row.\nTitle: {title}\nWhat they asked for: {wanted}"),
        Writing::Note { note, text } => format!("{}: {text}", note.as_str()),
    }
}

struct Checked {
    data_dir: std::path::PathBuf,
    subject: String,
    message: String,
    scope: Scope,
    mode: Mode,
    model: String,
}

impl Checked {
    /// Read the answer, record it, and hand back the outcome and note.
    async fn write(self, answer: anyhow::Result<String>, elapsed: Duration) -> (Outcome, Option<String>) {
        let (outcome, axis, note) = read_verdict(answer, elapsed, CHECK_LIMIT, self.mode);
        tracing::info!(
            mode = self.mode.as_str(),
            scope = ?self.scope,
            outcome = ?outcome,
            axis = axis.as_deref().unwrap_or(""),
            latency_ms = elapsed.as_millis() as u64,
            task = %self.subject,
            "record check"
        );
        let record = quality::Record::Check(quality::Check {
            ts: Utc::now(),
            surface: Surface::Record,
            turn: self.subject,
            message: self.message,
            scope: self.scope,
            mode: self.mode.as_str().to_string(),
            outcome,
            axis,
            note: note.clone(),
            latency_ms: elapsed.as_millis() as u64,
            model: self.model,
        });
        if let Err(err) = quality::append(&self.data_dir, &record).await {
            tracing::warn!(error = %format!("{err:#}"), "could not record a record check");
        }
        (outcome, note)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A line that asks the person to act is always read; a short ordinary line never is,
    /// and where it stands is read after it lands, not here.**
    #[test]
    fn triage_reads_facts_only() {
        let long = "字".repeat(TRIAGE_CHARS + 1);
        let longer = "字".repeat(TRIAGE_CHARS * 10);
        let note = |note, text| Writing::Note { note, text };
        assert_eq!(triage(&note(Note::Waiting, "你来登录")), Some(Scope::Waiting));
        assert_eq!(triage(&note(Note::Update, "简历在你盘上了")), None);
        assert_eq!(triage(&note(Note::Update, &long)), Some(Scope::Long));
        assert_eq!(triage(&note(Note::Stands, &longer)), None);
        assert_eq!(triage(&note(Note::Title, "简历")), Some(Scope::Opening));
        assert_eq!(triage(&Writing::Open { title: "简历", wanted: "能改" }), Some(Scope::Opening));
    }

    /// **A writer that was sent back once is let through next** — the record gets the fact
    /// even when the wording lost.
    #[tokio::test]
    async fn a_second_attempt_lands() {
        let dir = tempfile::tempdir().unwrap();
        let key = ("worker-1".to_string(), "resume".to_string());
        sent_back().lock().unwrap().insert(key.clone());
        let writing = Writing::Note { note: Note::Waiting, text: "你来登录" };
        assert!(matches!(review(dir.path(), "worker-1", "resume", None, writing).await, Review::Pass));
        assert!(!sent_back().lock().unwrap().contains(&key), "the pass consumed it");
    }

    /// **The judge sees the row and the reader, and the write last.**
    #[test]
    fn the_case_carries_the_row_and_ends_with_the_write() {
        let mut task = Task::new("导入简历", crate::mind::memory::tasks::TaskStatus::Doing);
        task.timeline.push(crate::mind::memory::tasks::TimelineEntry::new(
            crate::mind::memory::tasks::TimelineKind::Created,
            Utc::now(),
            "要能直接改",
        ));
        let text = case("以后简要汇报", Some(&task), &Writing::Note { note: Note::Delivered, text: "在你盘上了" });
        assert!(text.contains("以后简要汇报") && text.contains("要能直接改"));
        assert!(text.trim_end().ends_with("delivered: 在你盘上了"));
    }
}
