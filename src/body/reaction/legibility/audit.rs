//! The audit (§ G): the independent read of every spoken turn, and the reading of the
//! person's next message — the one judgment the reader makes rather than a model.
//!
//! **Both are event-driven, never periodic.** A turn that spoke is audited as it ends; the
//! turns spoken since the person's last message are read against their next one when it
//! arrives. Nothing here can touch a turn — what was sent is sent — so everything runs on
//! its own task and every failure costs a data point, never a reply.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;

use crate::foundation::config::tunables;
use crate::mind::memory::quality;

use super::check::{Brief, Ended};
use crate::body::legibility::judge::{Judge, json_object};

/// The `app_settings` keys: `speech_audit` = `off` turns both reads off; the model is its
/// own tunable, since an audit can afford a slower one than the check.
pub(crate) const MODE_KEY: &str = "speech_audit";
pub(crate) const MODEL_KEY: &str = "speech_audit_model";

/// Nobody is waiting on an audit, so it gets room. Two minutes was measured short: on
/// 2026-09-15 half of twelve audits of long report turns on a thinking model timed out.
const AUDIT_LIMIT: Duration = Duration::from_secs(300);

pub fn enabled() -> bool {
    !matches!(tunables::get(MODE_KEY).as_deref().map(str::trim), Some("off"))
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

/// The case an audit reads: the turn's brief, then its messages numbered.
pub(crate) fn audit_case(brief: &Brief, sent: &[String], shown: &[String]) -> String {
    use std::fmt::Write as _;
    let mut numbered = String::from("## The messages it sent, in order\n");
    for (i, m) in sent.iter().enumerate() {
        let _ = writeln!(numbered, "{}. {}", i + 1, m.replace('\n', "\n   "));
    }
    brief.case(&[], shown, &numbered)
}

/// Read one turn. `None` when it could not be judged.
pub(crate) async fn audit(
    judge: &Judge,
    instructions: &str,
    key: &str,
    brief: &Brief,
    sent: &[String],
    shown: &[String],
) -> Option<quality::Audit> {
    let answer = match judge.ask(instructions, &audit_case(brief, sent, shown), AUDIT_LIMIT).await {
        Ok(text) => text,
        Err(err) => {
            tracing::warn!(error = %format!("{err:#}"), "speech audit failed");
            return None;
        }
    };
    let Some(answer) = json_object::<AuditAnswer>(&answer) else {
        tracing::warn!("speech audit answered in a shape it could not be read in");
        return None;
    };
    Some(quality::Audit {
        ts: Utc::now(),
        surface: quality::Surface::Speech,
        turn: key.to_string(),
        model: judge.model().to_string(),
        messages: fold_answers(sent, answer.messages),
        unsaid: clean(answer.unsaid),
        wrong: clean(answer.wrong),
    })
}

/// One entry per message sent, whatever the judge numbered: an entry that names a message
/// that was not sent is dropped, and a message it skipped has no finding.
fn fold_answers(sent: &[String], answers: Vec<AuditedAnswer>) -> Vec<quality::Audited> {
    let mut out: Vec<quality::Audited> = sent
        .iter()
        .map(|text| quality::Audited { text: text.clone(), axis: None, note: None })
        .collect();
    for (i, a) in answers.into_iter().enumerate() {
        let at = a.n.map(|n| n.saturating_sub(1)).unwrap_or(i);
        let Some(slot) = out.get_mut(at) else { continue };
        slot.axis = quality::axis(a.axis.as_deref());
        slot.note = slot
            .axis
            .as_ref()
            .and(a.note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()));
    }
    out
}

fn clean(items: Vec<String>) -> Vec<String> {
    items.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// Audit a turn that has just ended, on its own task. Nothing to read in a silent turn.
pub fn after_turn(data_dir: PathBuf, ended: Ended) {
    if ended.sent.is_empty() || !enabled() {
        return;
    }
    tokio::spawn(async move {
        let Some(judge) = Judge::resolve(&data_dir, MODEL_KEY) else { return };
        let instructions =
            crate::identity::judge_instructions(&data_dir, crate::identity::judges::AUDIT).await;
        let Some(record) =
            audit(&judge, &instructions, &ended.key, &ended.brief, &ended.sent, &ended.shown).await
        else {
            return;
        };
        let findings = record.messages.iter().filter(|m| m.axis.is_some()).count();
        tracing::info!(
            messages = record.messages.len(),
            findings,
            unsaid = record.unsaid.len(),
            wrong = record.wrong.len(),
            "speech audit"
        );
        write(&data_dir, quality::Record::Audit(record)).await;
    });
}

#[derive(Deserialize)]
struct ReceptionAnswer {
    #[serde(default)]
    corrects: bool,
    #[serde(default)]
    axis: Option<String>,
    #[serde(default)]
    quote: Option<String>,
}

/// Read the person's message against what was said since their last one, on its own task.
///
/// `said` is every spoken turn since then, by key, with its messages; `theirs` is what they
/// wrote. A reception is only a reading of *how* — what they disagree with is the work's
/// business, not this record's.
pub fn on_reply(data_dir: PathBuf, said: Vec<(String, Vec<String>)>, theirs: String) {
    if said.is_empty() || theirs.trim().is_empty() || !enabled() {
        return;
    }
    tokio::spawn(async move {
        let Some(judge) = Judge::resolve(&data_dir, MODEL_KEY) else { return };
        let standard = crate::identity::reading_standard(&data_dir).await;
        let instructions = format!(
            "{}\n\n{}",
            crate::identity::judges::RECEPTION.trim(),
            crate::identity::reading_axes(&standard)
        );
        if let Some(record) = reception(&judge, &instructions, said, &theirs).await {
            if record.corrects {
                tracing::info!(
                    axis = record.axis.as_deref().unwrap_or(""),
                    quote = record.quote.as_deref().unwrap_or(""),
                    "the person corrected how something was said"
                );
            }
            write(&data_dir, quality::Record::Reception(record)).await;
        }
    });
}

pub(crate) fn reception_case(said: &[(String, Vec<String>)], theirs: &str) -> String {
    use std::fmt::Write as _;
    let mut s = String::from("## What the assistant said since their last message\n");
    for (_, messages) in said {
        for m in messages {
            let _ = writeln!(s, "< {}", m.replace('\n', "\n  "));
        }
    }
    let _ = write!(s, "\n## What they just wrote\n> {}\n", theirs.trim().replace('\n', "\n> "));
    s
}

pub(crate) async fn reception(
    judge: &Judge,
    instructions: &str,
    said: Vec<(String, Vec<String>)>,
    theirs: &str,
) -> Option<quality::Reception> {
    let answer = match judge.ask(instructions, &reception_case(&said, theirs), AUDIT_LIMIT).await {
        Ok(text) => text,
        Err(err) => {
            tracing::warn!(error = %format!("{err:#}"), "reading the reply failed");
            return None;
        }
    };
    let answer = json_object::<ReceptionAnswer>(&answer)?;
    Some(quality::Reception {
        ts: Utc::now(),
        turns: said.into_iter().map(|(key, _)| key).collect(),
        model: judge.model().to_string(),
        corrects: answer.corrects,
        axis: answer.corrects.then(|| quality::axis(answer.axis.as_deref())).flatten(),
        quote: answer
            .corrects
            .then(|| answer.quote.map(|q| q.trim().to_string()).filter(|q| !q.is_empty()))
            .flatten(),
    })
}

async fn write(data_dir: &Path, record: quality::Record) {
    if let Err(err) = quality::append(data_dir, &record).await {
        tracing::warn!(error = %format!("{err:#}"), "could not record a speech judgment");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_sent_gets_one_entry_whatever_the_judge_numbered() {
        let sent = vec!["好".to_string(), "部署了，公网 200，重启 0".to_string()];
        let answers = vec![
            AuditedAnswer { n: Some(2), axis: Some("known".into()), note: Some("「公网 200」".into()) },
            AuditedAnswer { n: Some(9), axis: Some("repeat".into()), note: None },
            AuditedAnswer { n: Some(1), axis: Some("made-up".into()), note: Some("x".into()) },
        ];
        let folded = fold_answers(&sent, answers);
        assert_eq!(folded.len(), 2);
        assert_eq!(folded[0].axis, None, "a coined axis is no finding");
        assert_eq!(folded[0].note, None, "and carries no note");
        assert_eq!(folded[1].axis.as_deref(), Some("known"));
        assert_eq!(folded[1].note.as_deref(), Some("「公网 200」"));
    }

    #[test]
    fn the_reception_case_is_what_was_said_then_what_they_wrote() {
        let said = vec![("t1".to_string(), vec!["部署了".to_string(), "公网 200".to_string()])];
        let case = reception_case(&said, "不用说这么细");
        assert!(case.find("< 公网 200").unwrap() < case.find("> 不用说这么细").unwrap());
    }

    #[test]
    fn the_audit_case_numbers_the_messages_after_the_brief() {
        let brief = Brief { signals: "(from session cognition) 部署好了".into(), ..Brief::default() };
        let case = audit_case(&brief, &["部署了".into(), "第二条\n两行".into()], &[]);
        assert!(case.find("部署好了").unwrap() < case.find("1. 部署了").unwrap());
        assert!(case.contains("2. 第二条\n   两行"));
    }
}
