//! quality — what the judges found in what was sent, and the numbers read off it
//! (`docs/arch/legibility.md` § G, § I).
//!
//! **One JSON line per judgment, a file per day, append-only**, under
//! [`layout::quality_dir`]. Three kinds, each written by the moment that has the answer:
//!
//! - a **check** — the pre-send read of one message, in shadow or live, with what it cost;
//! - an **audit** — the independent read of a whole spoken turn after it ended;
//! - a **reception** — what the person's next message said about *how* the last turns were
//!   put, which is the one judgment the reader makes rather than a model.
//!
//! **Nothing here decides anything.** Reflection reads these files to learn grain and
//! conduct (§ H), the numbers are served to whoever asks (§ I), and replay draws its set
//! from them (§ J). A record that fails to write costs a data point, never a turn.
//!
//! The axes are the reading standard's (`src/identity/craft/reading.md`), and a judge that
//! answers with anything else has its answer kept as `None` rather than coined into a tenth.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::layout;

/// The axes a line fails on, as the reading standard names them.
pub const AXES: [&str; 9] = [
    "known",
    "machinery",
    "repeat",
    "hard",
    "defensive",
    "shape",
    "unsupported",
    "buried",
    "unsaid",
];

/// An axis a judge wrote, if it is one of [`AXES`]. Case and surrounding backticks are
/// forgiven; a coined axis is not.
pub fn axis(raw: Option<&str>) -> Option<String> {
    let a = raw?.trim().trim_matches('`').to_ascii_lowercase();
    AXES.contains(&a.as_str()).then_some(a)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Record {
    Check(Check),
    Audit(Audit),
    Reception(Reception),
}

impl Record {
    pub fn ts(&self) -> DateTime<Utc> {
        match self {
            Record::Check(c) => c.ts,
            Record::Audit(a) => a.ts,
            Record::Reception(r) => r.ts,
        }
    }
}

/// Why a message was in the check's scope (§ D). The first that applies, in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// The turn carries a report — mail or a worker's result.
    Report,
    /// Not the first message of the turn.
    Later,
    /// Longer than a short reply.
    Long,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pass,
    Revise,
    /// No answer inside the check's time limit. The message went out.
    Timeout,
    /// The request failed or the answer could not be read. The message went out.
    Error,
}

/// One pre-send read of one message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub ts: DateTime<Utc>,
    /// The turn it belongs to — the key audits and receptions share.
    pub turn: String,
    pub message: String,
    pub scope: Scope,
    /// `shadow` (judged, never sent back) or `on`.
    pub mode: String,
    pub outcome: Outcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// From the moment the message reached the mouth to the verdict. In shadow this is
    /// what the check *would* have cost.
    pub latency_ms: u64,
    pub model: String,
}

/// One message of an audited turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Audited {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The independent read of one spoken turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Audit {
    pub ts: DateTime<Utc>,
    pub turn: String,
    pub model: String,
    pub messages: Vec<Audited>,
    /// Owed and left unsaid: a question unanswered, a result they were waiting on.
    #[serde(default)]
    pub unsaid: Vec<String>,
    /// Said and not true to what reached the turn.
    #[serde(default)]
    pub wrong: Vec<String>,
}

/// What the person's next message said about how the turns before it were put.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reception {
    pub ts: DateTime<Utc>,
    /// The spoken turns since their previous message, which this reads.
    pub turns: Vec<String>,
    pub model: String,
    /// Whether they corrected *how* something was said — too much, too fine, not wanted,
    /// unclear, not in their words. Disagreeing with *what* was said is not this.
    pub corrects: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<String>,
    /// Their words, as said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
}

fn day_file(data_dir: &Path, day: NaiveDate) -> PathBuf {
    layout::quality_dir(data_dir).join(format!("{day}.jsonl"))
}

/// Append one record to its day's file.
///
/// Serialised through one process-wide lock: checks, audits and receptions finish on
/// separate tasks, and two appends interleaving inside one line would cost both.
pub async fn append(data_dir: &Path, record: &Record) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt as _;
    static WRITE: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    let _held = WRITE.get_or_init(Default::default).lock().await;

    let path = day_file(data_dir, record.ts().date_naive());
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut line = serde_json::to_string(record)?;
    line.push('\n');
    let mut file =
        tokio::fs::OpenOptions::new().create(true).append(true).open(&path).await?;
    file.write_all(line.as_bytes()).await?;
    Ok(())
}

/// Every readable record from `since` on, oldest first. An unreadable line is skipped —
/// a record is a data point, and one torn line must not cost the rest of the day.
pub async fn read_since(data_dir: &Path, since: DateTime<Utc>) -> Vec<Record> {
    let dir = layout::quality_dir(data_dir);
    let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
        return Vec::new();
    };
    let mut days: Vec<(NaiveDate, PathBuf)> = Vec::new();
    while let Ok(Some(ent)) = rd.next_entry().await {
        let name = ent.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".jsonl") else { continue };
        let Ok(day) = stem.parse::<NaiveDate>() else { continue };
        if day >= since.date_naive() {
            days.push((day, ent.path()));
        }
    }
    days.sort();
    let mut out = Vec::new();
    for (_, path) in days {
        let Ok(text) = tokio::fs::read_to_string(&path).await else { continue };
        out.extend(
            text.lines()
                .filter_map(|l| serde_json::from_str::<Record>(l).ok())
                .filter(|r| r.ts() >= since),
        );
    }
    out.sort_by_key(Record::ts);
    out
}

/// The numbers (§ I). Server-side, beside the logs; no card in the face.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Numbers {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    /// **The primary.** How often the person corrected how something was said, by day.
    /// The reader is the only authority on the reader's bar; the target is zero.
    pub corrections_per_day: BTreeMap<NaiveDate, u32>,
    pub corrections: u32,
    pub receptions: u32,
    /// Audit findings per 100 audited messages, by axis, and `any` for all of them.
    pub findings_per_100_messages: BTreeMap<String, f64>,
    pub messages_audited: u32,
    pub turns_audited: u32,
    /// Owed and left unsaid, per 100 audited turns — so shorter never passes for better.
    pub unsaid_per_100_turns: f64,
    pub check: CheckNumbers,
    /// Whether the check deserves its place: its verdicts against the audit's, message by
    /// message, for every message both read.
    pub agreement: Agreement,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CheckNumbers {
    pub checked: u32,
    pub revise: u32,
    pub timeout: u32,
    pub error: u32,
    pub revise_rate: Option<f64>,
    pub latency_p50_ms: Option<u64>,
    pub latency_p95_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Agreement {
    pub compared: u32,
    pub both: u32,
    pub check_only: u32,
    pub audit_only: u32,
    /// Of the messages the check would send back, the share the audit also flagged.
    pub precision: Option<f64>,
    /// Of the messages the audit flagged, the share the check would have sent back.
    pub recall: Option<f64>,
}

fn ratio(n: u32, d: u32) -> Option<f64> {
    (d > 0).then(|| f64::from(n) / f64::from(d))
}

fn percentile(sorted: &[u64], p: f64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = ((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    Some(sorted[rank - 1])
}

pub fn numbers(records: &[Record]) -> Numbers {
    let mut n = Numbers {
        from: records.first().map(Record::ts),
        to: records.last().map(Record::ts),
        ..Numbers::default()
    };
    let mut latencies = Vec::new();
    // (turn, message) → whether the check would send it back. The last verdict wins: a
    // rewrite that follows a send-back is a different message and keys separately.
    let mut checked: BTreeMap<(&str, &str), bool> = BTreeMap::new();
    let mut findings: BTreeMap<String, u32> = BTreeMap::new();
    let mut unsaid = 0u32;

    for record in records {
        match record {
            Record::Reception(r) => {
                n.receptions += 1;
                if r.corrects {
                    n.corrections += 1;
                    *n.corrections_per_day.entry(r.ts.date_naive()).or_default() += 1;
                }
            }
            Record::Check(c) => {
                n.check.checked += 1;
                match c.outcome {
                    Outcome::Revise => n.check.revise += 1,
                    Outcome::Timeout => n.check.timeout += 1,
                    Outcome::Error => n.check.error += 1,
                    Outcome::Pass => {}
                }
                latencies.push(c.latency_ms);
                if matches!(c.outcome, Outcome::Pass | Outcome::Revise) {
                    checked.insert((&c.turn, &c.message), c.outcome == Outcome::Revise);
                }
            }
            Record::Audit(a) => {
                n.turns_audited += 1;
                unsaid += a.unsaid.len() as u32;
                for m in &a.messages {
                    n.messages_audited += 1;
                    if let Some(axis) = &m.axis {
                        *findings.entry(axis.clone()).or_default() += 1;
                        *findings.entry("any".into()).or_default() += 1;
                    }
                }
            }
        }
    }
    for record in records {
        let Record::Audit(a) = record else { continue };
        for m in &a.messages {
            let Some(&sent_back) = checked.get(&(a.turn.as_str(), m.text.as_str())) else {
                continue;
            };
            let flagged = m.axis.is_some();
            n.agreement.compared += 1;
            match (sent_back, flagged) {
                (true, true) => n.agreement.both += 1,
                (true, false) => n.agreement.check_only += 1,
                (false, true) => n.agreement.audit_only += 1,
                (false, false) => {}
            }
        }
    }
    let g = &mut n.agreement;
    g.precision = ratio(g.both, g.both + g.check_only);
    g.recall = ratio(g.both, g.both + g.audit_only);

    let per_100 = |count: u32, of: u32| if of == 0 { 0.0 } else { f64::from(count) * 100.0 / f64::from(of) };
    n.findings_per_100_messages =
        findings.into_iter().map(|(k, v)| (k, per_100(v, n.messages_audited))).collect();
    n.unsaid_per_100_turns = per_100(unsaid, n.turns_audited);
    n.check.revise_rate = ratio(n.check.revise, n.check.checked);
    latencies.sort_unstable();
    n.check.latency_p50_ms = percentile(&latencies, 0.50);
    n.check.latency_p95_ms = percentile(&latencies, 0.95);
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    fn check(turn: &str, message: &str, outcome: Outcome, latency_ms: u64) -> Record {
        Record::Check(Check {
            ts: at("2026-09-15T01:00:00Z"),
            turn: turn.into(),
            message: message.into(),
            scope: Scope::Report,
            mode: "shadow".into(),
            outcome,
            axis: None,
            note: None,
            latency_ms,
            model: "m".into(),
        })
    }

    #[test]
    fn an_axis_is_one_of_the_standards_or_none() {
        assert_eq!(axis(Some(" `Machinery` ")), Some("machinery".into()));
        assert_eq!(axis(Some("verbosity")), None, "a coined axis is not kept");
        assert_eq!(axis(Some("")), None);
        assert_eq!(axis(None), None);
    }

    #[tokio::test]
    async fn records_land_in_their_days_file_and_read_back_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let late = Record::Reception(Reception {
            ts: at("2026-09-15T02:00:00Z"),
            turns: vec!["t1".into()],
            model: "m".into(),
            corrects: true,
            axis: Some("machinery".into()),
            quote: Some("不用说这么细".into()),
        });
        let early = check("t1", "部署了", Outcome::Pass, 900);
        append(dir.path(), &late).await.unwrap();
        append(dir.path(), &early).await.unwrap();
        assert!(layout::quality_dir(dir.path()).join("2026-09-15.jsonl").exists());

        let back = read_since(dir.path(), at("2026-09-14T00:00:00Z")).await;
        assert_eq!(back, vec![early, late]);
        assert!(read_since(dir.path(), at("2026-09-15T03:00:00Z")).await.is_empty());
    }

    /// The primary number is the person's corrections, and the check is measured against
    /// the audit rather than against itself.
    #[test]
    fn the_numbers_put_corrections_first_and_score_the_check_against_the_audit() {
        let records = vec![
            check("t1", "a", Outcome::Revise, 1_000),
            check("t1", "b", Outcome::Pass, 2_000),
            check("t1", "c", Outcome::Revise, 3_000),
            check("t2", "d", Outcome::Timeout, 2_500),
            Record::Audit(Audit {
                ts: at("2026-09-15T01:01:00Z"),
                turn: "t1".into(),
                model: "m".into(),
                messages: vec![
                    Audited { text: "a".into(), axis: Some("known".into()), note: None },
                    Audited { text: "b".into(), axis: Some("machinery".into()), note: None },
                    Audited { text: "c".into(), axis: None, note: None },
                    Audited { text: "z".into(), axis: None, note: None },
                ],
                unsaid: vec!["the second question".into()],
                wrong: vec![],
            }),
            Record::Reception(Reception {
                ts: at("2026-09-15T01:05:00Z"),
                turns: vec!["t1".into()],
                model: "m".into(),
                corrects: true,
                axis: Some("machinery".into()),
                quote: Some("不用说这么细".into()),
            }),
        ];
        let n = numbers(&records);
        assert_eq!(n.corrections, 1);
        assert_eq!(n.corrections_per_day.get(&"2026-09-15".parse().unwrap()), Some(&1));
        assert_eq!(n.messages_audited, 4);
        assert_eq!(n.findings_per_100_messages.get("any"), Some(&50.0));
        assert_eq!(n.findings_per_100_messages.get("known"), Some(&25.0));
        assert_eq!(n.unsaid_per_100_turns, 100.0);
        assert_eq!(n.check.checked, 4);
        assert_eq!(n.check.timeout, 1);
        assert_eq!(n.check.revise_rate, Some(0.5));
        assert_eq!(n.check.latency_p50_ms, Some(2_000));
        assert_eq!(n.check.latency_p95_ms, Some(3_000));
        // a: both; b: audit only; c: check only; z was never checked.
        assert_eq!(n.agreement.compared, 3);
        assert_eq!(n.agreement.precision, Some(0.5));
        assert_eq!(n.agreement.recall, Some(0.5));
    }
}
