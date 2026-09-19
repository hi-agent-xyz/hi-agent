//! What is read after it lands (`docs/arch/legibility.md` § N): a task's *Where it stands* each
//! time it is rewritten, and the whole record when a task manager closes it.
//!
//! **Nothing here can touch the record** — what was written is written — so both reads run on
//! their own task and every failure costs a data point, never a write. What they find goes to
//! `memory/quality/` as an audit on the record surface, which is how § H learns from it and § I
//! counts it.
//!
//! **The closing read is the gate's counterweight.** A gate on line length is an incentive to
//! write fewer lines rather than shorter ones, and a line never written is invisible forever. So
//! the close reads the record against what the sessions that did the work reported, and what
//! those reports say changed something for the person and the record never carries is *owed and
//! left unsaid* — the number to watch before `record_check` leaves shadow.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;

use super::judge::Judge;
use crate::foundation::config::tunables;
use crate::mind::memory::quality;
use crate::mind::memory::tasks::{self, Task, TimelineKind};

/// The `app_settings` keys: `record_audit` = `off` turns both reads off; the model is its own.
pub(crate) const MODE_KEY: &str = "record_audit";
pub(crate) const MODEL_KEY: &str = "record_audit_model";

/// Nobody is waiting on an audit, so it gets room — the same as speech's.
const AUDIT_LIMIT: Duration = Duration::from_secs(300);

/// How much of the account a closing read is shown, newest reading first. Past a screenful the
/// panel clamps it too; this is generous so a finding about what is buried can be made.
const ACCOUNT_CHARS: usize = 8_000;

/// How much of what the serving sessions reported a closing read is shown, newest kept.
const REPORT_CHARS: usize = 24_000;

/// How many of the row's newest lines a read of *Where it stands* sees beside it.
const RECENT_LINES: usize = 8;

pub fn enabled() -> bool {
    !matches!(tunables::get(MODE_KEY).as_deref().map(str::trim), Some("off"))
}

/// Read what was just written under *Where it stands*, on its own task.
pub fn after_stands(data_dir: PathBuf, subject: String, text: String) {
    if !enabled() || text.trim().is_empty() {
        return;
    }
    tokio::spawn(async move {
        let Ok(Some(task)) = tasks::read_task(&data_dir, &subject).await else { return };
        let items = vec![text.trim().to_string()];
        let case = written_case(&reader(&data_dir).await, &task, STANDS, &items[0]);
        read_and_keep(&data_dir, &subject, &case, &items).await;
    });
}

/// Read a record whole as it closes, against what its sessions reported, on its own task.
pub fn at_close(data_dir: PathBuf, subject: String) {
    if !enabled() {
        return;
    }
    tokio::spawn(async move {
        let Ok(Some(task)) = tasks::read_task(&data_dir, &subject).await else { return };
        let reports =
            crate::foundation::registry::mail::sent_by_sessions_serving(&data_dir, &subject, REPORT_CHARS)
                .await;
        let (case, items) = closing_case(&reader(&data_dir).await, &task, &reports);
        read_and_keep(&data_dir, &subject, &case, &items).await;
    });
}

async fn reader(data_dir: &Path) -> String {
    crate::mind::memory::snapshot::for_record(data_dir).await
}

async fn read_and_keep(data_dir: &Path, subject: &str, case: &str, items: &[String]) {
    let Some(judge) = Judge::resolve(data_dir, MODEL_KEY) else { return };
    let instructions =
        crate::identity::judge_instructions(data_dir, crate::identity::judges::RECORD_AUDIT).await;
    let answer = match judge.ask(&instructions, case, AUDIT_LIMIT).await {
        Ok(answer) => answer,
        Err(err) => {
            tracing::warn!(error = %format!("{err:#}"), task = %subject, "record audit failed");
            return;
        }
    };
    let Some((messages, unsaid, wrong)) = super::read_audit(&answer, items) else {
        tracing::warn!(task = %subject, "record audit answered in a shape it could not be read in");
        return;
    };
    tracing::info!(
        task = %subject,
        read = messages.len(),
        findings = messages.iter().filter(|m| m.axis.is_some()).count(),
        unsaid = unsaid.len(),
        "record audit"
    );
    let record = quality::Record::Audit(quality::Audit {
        ts: Utc::now(),
        surface: quality::Surface::Record,
        turn: subject.to_string(),
        model: judge.model().to_string(),
        messages,
        unsaid,
        wrong,
    });
    if let Err(err) = quality::append(data_dir, &record).await {
        tracing::warn!(error = %format!("{err:#}"), "could not record a record audit");
    }
}

fn section(s: &mut String, title: &str, body: &str) {
    use std::fmt::Write as _;
    let body = body.trim();
    let _ = write!(s, "## {title}\n{}\n\n", if body.is_empty() { "(nothing)" } else { body });
}

fn the_task(s: &mut String, task: &Task) {
    let asked = task.created().map(|c| c.text.as_str()).unwrap_or_default();
    section(s, "The task", &format!("Title: {}\nWhat they asked for: {asked}", task.title));
}

const STANDS: &str = "Just written under Where it stands — item 1";
const LINE: &str = "Just written on the record — item 1";

/// Read one line as the record audit reads it, without keeping the answer — what record replay
/// scores both sides of a comparison with (`docs/arch/legibility.md` § J). The row is read as
/// it stands now; a row that is gone is read as its subject alone.
pub(crate) async fn read_line(
    judge: &Judge,
    instructions: &str,
    data_dir: &Path,
    subject: &str,
    line: &str,
) -> Option<quality::Audit> {
    let task = match tasks::read_task(data_dir, subject).await {
        Ok(Some(task)) => task,
        _ => Task::new(subject, tasks::TaskStatus::Doing),
    };
    let items = vec![line.trim().to_string()];
    let case = written_case(&reader(data_dir).await, &task, LINE, &items[0]);
    let answer = judge.ask(instructions, &case, AUDIT_LIMIT).await.ok()?;
    let (messages, unsaid, wrong) = super::read_audit(&answer, &items)?;
    Some(quality::Audit {
        ts: Utc::now(),
        surface: quality::Surface::Record,
        turn: subject.to_string(),
        model: judge.model().to_string(),
        messages,
        unsaid,
        wrong,
    })
}

/// A read of one thing just written: the reader, the row, and the text numbered as item 1.
fn written_case(reader: &str, task: &Task, heading: &str, text: &str) -> String {
    let mut s = String::new();
    section(&mut s, "Who is reading, and how they want to be told", reader);
    the_task(&mut s, task);
    let skip = task.timeline.len().saturating_sub(RECENT_LINES);
    let lines = task.timeline[skip..]
        .iter()
        .map(|e| format!("- {} — {}", e.kind.as_str(), e.text))
        .collect::<Vec<_>>()
        .join("\n");
    section(&mut s, "Its newest lines, oldest first", &lines);
    section(&mut s, heading, &format!("1. {}", text.replace('\n', "\n   ")));
    s
}

/// A closing read: the reader, the row, the account and every line a mind wrote, numbered, then
/// what the sessions that served it reported. The items are what is judged; the reports are what
/// the record is judged against.
fn closing_case(
    reader: &str,
    task: &Task,
    reports: &[(chrono::DateTime<Utc>, String)],
) -> (String, Vec<String>) {
    use std::fmt::Write as _;
    let mut items: Vec<String> = Vec::new();
    let account = task.body.trim();
    if !account.is_empty() {
        items.push(account.chars().take(ACCOUNT_CHARS).collect());
    }
    items.extend(
        task.timeline
            .iter()
            .filter(|e| !matches!(e.kind, TimelineKind::Moved | TimelineKind::Made))
            .map(|e| format!("{} — {}", e.kind.as_str(), e.text)),
    );
    let mut s = String::new();
    section(&mut s, "Who is reading, and how they want to be told", reader);
    the_task(&mut s, task);
    let mut numbered = String::new();
    for (i, item) in items.iter().enumerate() {
        let label = if i == 0 && !account.is_empty() { " (Where it stands, newest reading first)" } else { "" };
        let _ = writeln!(numbered, "{}.{label} {}", i + 1, item.replace('\n', "\n   "));
    }
    section(&mut s, "The record, numbered — what you judge", &numbered);
    let reported = reports
        .iter()
        .map(|(at, text)| format!("[{}] {}", at.format("%m-%d %H:%M"), text.replace('\n', "\n  ")))
        .collect::<Vec<_>>()
        .join("\n");
    section(&mut s, "What the sessions that did the work reported, oldest first — what you judge it against", &reported);
    (s, items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mind::memory::tasks::{TaskStatus, TimelineEntry};

    fn task() -> Task {
        let mut task = Task::new("导入简历", TaskStatus::Done);
        task.body = "交付了，等你看。".into();
        let at = Utc::now();
        task.timeline.push(TimelineEntry::new(TimelineKind::Created, at, "要能直接改"));
        task.timeline.push(TimelineEntry::new(TimelineKind::Delivered, at, "在你盘上了"));
        task.timeline.push(TimelineEntry::new(TimelineKind::Moved, at, "doing → done"));
        task
    }

    /// **The close judges what a mind wrote, against what the work reported.** The account comes
    /// first and says so; the store's own lines are not judged; the reports come after, as what
    /// the record is read against.
    #[test]
    fn a_closing_read_numbers_the_record_and_ends_with_the_reports() {
        let reports = vec![(Utc::now(), "简历导进来了，另外发现 PDF 里有两页扫描件没法编辑".to_string())];
        let (case, items) = closing_case("以后简要汇报", &task(), &reports);
        assert_eq!(items, vec!["交付了，等你看。", "created — 要能直接改", "delivered — 在你盘上了"]);
        assert!(case.contains("1. (Where it stands, newest reading first) 交付了"));
        assert!(!case.contains("doing → done"), "the store's own lines are not judged");
        assert!(case.find("## The record").unwrap() < case.find("扫描件").unwrap());
    }

    #[test]
    fn a_stands_read_numbers_the_new_prose_as_its_one_item() {
        let case = written_case("以后简要汇报", &task(), STANDS, "交付了，等你看。\n第二段。");
        assert!(case.trim_end().ends_with("1. 交付了，等你看。\n   第二段。"));
        assert!(case.contains("要能直接改"));
    }
}
