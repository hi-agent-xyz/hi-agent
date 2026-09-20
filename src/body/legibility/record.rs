//! The gate on a task's record (`docs/arch/legibility.md` § M): triage in code, one model
//! request, `not recorded — <note>` once, fail open, shadow first.
//!
//! **It reads a line, not a record.** A line is short, the worker writes it holding
//! everything it is about, and the card on the board draws one line of it — so a send-back
//! arrives while the writer can still act on it. *Where it stands* is long, rewritten rarely
//! and waited on by nobody, so it is judged after it lands (§ N, [`super::record_audit`]),
//! never here.
//!
//! The ladder, the one send-back and the recording are every gate's ([`super::gate`]); what is
//! the record's own is what it triages on and what the judge is shown.

use std::path::Path;

use super::{Gate, Review, gate};
use crate::mind::memory::quality::{Scope, Surface};
use crate::mind::memory::tasks::{Note, Task};

/// The record's gate: `record_check` chooses its mode, `record_check_model` its model.
pub(crate) const GATE: Gate = Gate {
    surface: Surface::Record,
    mode_key: "record_check",
    model_key: "record_check_model",
    budget_key: "record_check_budget_ms",
    rubric: crate::identity::judges::RECORD,
};

/// Past this many characters a line is more than a card can draw, and in scope. A starting
/// value, like speech's (`docs/arch/legibility.md` § Open).
pub(crate) const TRIAGE_CHARS: usize = 120;

/// How many of the row's newest lines the judge reads beside the new one — enough to see the
/// same thing being said again, which it cannot see otherwise.
const RECENT_LINES: usize = 8;

/// What is about to be written on a row.
#[derive(Clone, Copy, Debug)]
pub enum Writing<'a> {
    /// A row being opened: its name and what they asked for.
    Open { title: &'a str, wanted: &'a str },
    /// Something said on a row that exists.
    Note { note: Note, text: &'a str },
}

/// Whether a write is in the gate's scope, and why. Facts only.
fn triage(writing: &Writing) -> Option<Scope> {
    match *writing {
        Writing::Open { .. } | Writing::Note { note: Note::Title, .. } => Some(Scope::Opening),
        Writing::Note { note: Note::Waiting, .. } => Some(Scope::Waiting),
        Writing::Note { note: Note::Stands, .. } => None,
        Writing::Note { text, .. } if text.chars().count() > TRIAGE_CHARS => Some(Scope::Long),
        Writing::Note { .. } => None,
    }
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
    let Some(scope) = triage(&writing) else {
        // Not read, and said so: what triage passes is a verdict host code made, and it is
        // counted beside the judge's (`super::skipped`).
        super::skipped(data_dir, Surface::Record, subject, &describe(&writing));
        return Review::Pass;
    };
    gate(data_dir, &GATE, writer, subject, scope, case(task, &writing), describe(&writing)).await
}

/// What the judge reads after who the reader is: the row as it stands, and the write last.
fn case(task: Option<&Task>, writing: &Writing) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let section = |s: &mut String, title: &str, body: &str| {
        let body = body.trim();
        let _ = write!(s, "## {title}\n{}\n\n", if body.is_empty() { "(nothing)" } else { body });
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

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

    /// **What triage passes is counted too.** A short ordinary line is never read by a judge,
    /// and until this record existed that made it absent from the denominator rather than
    /// present as a pass — so the share of lines anything had read looked like all of them.
    #[tokio::test]
    async fn a_line_triage_passes_is_recorded_as_unread() {
        use crate::mind::memory::quality;
        let dir = tempfile::tempdir().unwrap();
        let writing = Writing::Note { note: Note::Update, text: "简历在你盘上了" };
        assert!(matches!(review(dir.path(), "worker-1", "resume", None, writing).await, Review::Pass));

        // Written off the write's path on purpose, so wait for it rather than assume it landed.
        let mut records = Vec::new();
        for _ in 0..50 {
            records = quality::read_since(dir.path(), Utc::now() - chrono::Duration::hours(1)).await;
            if !records.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let [quality::Record::Skipped(s)] = records.as_slice() else {
            panic!("one record, saying host code let it through unread: {records:?}")
        };
        assert_eq!(s.surface, Surface::Record);
        assert_eq!(s.turn, "resume");
        assert!(s.message.contains("简历在你盘上了"), "{}", s.message);
    }

    /// **A writer that was sent back once is let through next** — the record gets the fact
    /// even when the wording lost.
    #[tokio::test]
    async fn a_second_attempt_lands() {
        let dir = tempfile::tempdir().unwrap();
        super::super::hold_for_test(Surface::Record, "worker-1", "resume");
        let writing = Writing::Note { note: Note::Waiting, text: "你来登录" };
        assert!(matches!(review(dir.path(), "worker-1", "resume", None, writing).await, Review::Pass));
        assert!(!super::super::held_for_test(Surface::Record, "worker-1", "resume"), "the pass consumed it");
    }

    /// **The judge sees the row, and the write last.**
    #[test]
    fn the_case_carries_the_row_and_ends_with_the_write() {
        let mut task = Task::new("导入简历", crate::mind::memory::tasks::TaskStatus::Doing);
        task.timeline.push(crate::mind::memory::tasks::TimelineEntry::new(
            crate::mind::memory::tasks::TimelineKind::Created,
            Utc::now(),
            "要能直接改",
        ));
        let text = case(Some(&task), &Writing::Note { note: Note::Delivered, text: "在你盘上了" });
        assert!(text.contains("要能直接改"));
        assert!(text.trim_end().ends_with("delivered: 在你盘上了"));
    }
}
