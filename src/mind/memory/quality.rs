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

/// Which kind of thing a person reads a judgment was about (`docs/arch/legibility.md` §
/// *The surfaces*). **One file for all of them**, because a correction about how the person is
/// told things is a fact about the person, and a store split by surface could not carry it
/// from one to the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// A spoken message. Every record written before surfaces existed is one, which is what
    /// the default reads it back as.
    #[default]
    Speech,
    /// A task's record: a line, a title, where it stands.
    Record,
    /// A view put on the person's screen, as its reviewer judged it.
    View,
    /// A group's label or note on the home screen.
    Home,
}

impl Surface {
    pub const ALL: [Surface; 4] = [Surface::Speech, Surface::Record, Surface::View, Surface::Home];

    /// What one of the things judged on this surface is called, plural, for a reader.
    pub fn things(self) -> &'static str {
        match self {
            Surface::Speech => "spoken messages",
            Surface::Record => "task-record lines",
            Surface::View => "views",
            Surface::Home => "home labels",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Record {
    Check(Check),
    Skipped(Skipped),
    Audit(Audit),
    Reception(Reception),
    Bypass(Bypass),
}

impl Record {
    pub fn ts(&self) -> DateTime<Utc> {
        match self {
            Record::Check(c) => c.ts,
            Record::Skipped(s) => s.ts,
            Record::Audit(a) => a.ts,
            Record::Reception(r) => r.ts,
            Record::Bypass(b) => b.ts,
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
    /// Longer than a short reply, or than a line a card can draw.
    Long,
    /// A record line asking the person to do something — the one whose burial costs most.
    Waiting,
    /// A row being opened, or its name corrected: the title every card draws.
    Opening,
    /// A reviewer's verdict on something built for the person — a view.
    Review,
    /// A label or note about to go on the home screen.
    Label,
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

/// One pre-send read of one message — or, on a record, one line before it is written.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub ts: DateTime<Utc>,
    #[serde(default)]
    pub surface: Surface,
    /// What it belongs to: the turn, for speech — the key audits and receptions share — and
    /// the task's subject, for a record.
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
    /// What the request cost upstream, which is the only thing that explains the latency
    /// beside it.
    #[serde(default, skip_serializing_if = "Cost::is_unknown")]
    pub cost: Cost,
    /// The budget this verdict was measured against, so a row stamped `timeout` can still be
    /// read a month later when the budget has moved.
    #[serde(default)]
    pub budget_ms: u64,
}

/// What one judge request cost upstream, as the reply reported it.
///
/// **Kept because latency here is almost entirely output tokens, and nothing else
/// distinguishes a slow judge from a slow network.** Measured 2026-09-20 against
/// `deepseek-flash`: a record check answered in 4.7s, of which 914 of 1,012 output tokens
/// were the model thinking before it wrote ~50 tokens of verdict; the same request with a
/// one-line case answered in 1.6s, and a bare prompt in 0.7s. Without these four numbers
/// that is indistinguishable from an upstream having a bad day.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cost {
    pub input: u64,
    /// Of `input`, what the upstream served from its prefix cache. The standard rides in
    /// front of every case so that this number stays high (`docs/arch/legibility.md` § A).
    pub cached: u64,
    pub output: u64,
    /// Of `output`, what was spent before the answer began.
    pub thinking: u64,
}

impl Cost {
    /// Nothing was reported — an older record, or an upstream that sends no usage.
    pub fn is_unknown(&self) -> bool {
        *self == Cost::default()
    }
}

/// A write that reached a seam and went through unread, because host code decided it was out
/// of scope (`docs/arch/legibility.md` § D).
///
/// **Kept because "not read" is a verdict.** Triage judges no wording — it routes on facts a
/// person never sees, like a length or a position in a turn — but a message it does not route
/// goes out exactly as if a judge had passed it. Left unrecorded, those writes are missing
/// from the denominator of every number here, which makes the checked share read as the whole.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Skipped {
    pub ts: DateTime<Utc>,
    pub surface: Surface,
    /// The turn, or the task's subject.
    pub turn: String,
    pub message: String,
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

/// The independent read of one spoken turn — or of one record, after it was written.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Audit {
    pub ts: DateTime<Utc>,
    #[serde(default)]
    pub surface: Surface,
    /// The turn, or the task's subject.
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

/// A surface written some other way than through its seam — a record the host did not write
/// the last version of. The seam is held by detection rather than by a wall
/// (`docs/arch/legibility.md` § L), so this is what makes a walk-around countable.
///
/// **Countable was not enough.** The first shape of this record carried the instant, the
/// surface and the subject, and nothing else. On 2026-09-20, asked which session had written
/// twenty-one lines around the verbs on one row, the record could not answer and the wire log
/// took half an hour to half-answer. So it now carries who was on the row and how much moved:
/// enough to name a suspect and to tell one hand-written line from a rewritten record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bypass {
    pub ts: DateTime<Utc>,
    pub surface: Surface,
    pub subject: String,
    /// The sessions opened against this subject, as the session index knows them. Not proof —
    /// anything running unsandboxed can write any file — but it is where to look first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub serving: Vec<String>,
    /// The record's size when the host last wrote it, and as it was found.
    #[serde(default)]
    pub was: Size,
    #[serde(default)]
    pub found: Size,
}

/// How big a record is, in the two units a reader of one cares about.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub chars: u64,
    /// Lines on its timeline — the count that says whether entries were added by hand.
    pub entries: u64,
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

/// The numbers (§ I), one set per surface. Server-side, beside the logs; no card in the face.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Legibility {
    pub speech: Numbers,
    pub record: Numbers,
    pub view: Numbers,
    pub home: Numbers,
}

pub fn legibility(records: &[Record]) -> Legibility {
    Legibility {
        speech: numbers(records, Surface::Speech),
        record: numbers(records, Surface::Record),
        view: numbers(records, Surface::View),
        home: numbers(records, Surface::Home),
    }
}

/// The numbers for one surface.
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
    /// Writes that went around the seam. Zero on speech, which has no other way out.
    pub bypasses: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CheckNumbers {
    pub checked: u32,
    /// Writes host code let through without reading. **The denominator's other half**: a
    /// `revise_rate` over `checked` alone describes the sample, not the surface.
    pub skipped: u32,
    pub revise: u32,
    pub timeout: u32,
    pub error: u32,
    pub revise_rate: Option<f64>,
    /// Of everything written to this surface, the share a judge read at all.
    pub read_rate: Option<f64>,
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

/// Whether a record is about `surface`. A reception reads the person's reply to what was
/// *said*, so it belongs to speech.
pub fn on(record: &Record, surface: Surface) -> bool {
    match record {
        Record::Check(c) => c.surface == surface,
        Record::Skipped(s) => s.surface == surface,
        Record::Audit(a) => a.surface == surface,
        Record::Reception(_) => surface == Surface::Speech,
        Record::Bypass(b) => b.surface == surface,
    }
}

pub fn numbers(records: &[Record], surface: Surface) -> Numbers {
    let records: Vec<&Record> = records.iter().filter(|r| on(r, surface)).collect();
    let mut n = Numbers {
        from: records.first().map(|r| r.ts()),
        to: records.last().map(|r| r.ts()),
        ..Numbers::default()
    };
    let mut latencies = Vec::new();
    // (turn, message) → whether the check would send it back. The last verdict wins: a
    // rewrite that follows a send-back is a different message and keys separately.
    let mut checked: BTreeMap<(&str, &str), bool> = BTreeMap::new();
    let mut findings: BTreeMap<String, u32> = BTreeMap::new();
    let mut unsaid = 0u32;

    for record in &records {
        match record {
            Record::Bypass(_) => n.bypasses += 1,
            Record::Reception(r) => {
                n.receptions += 1;
                if r.corrects {
                    n.corrections += 1;
                    *n.corrections_per_day.entry(r.ts.date_naive()).or_default() += 1;
                }
            }
            Record::Skipped(_) => n.check.skipped += 1,
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
    for record in &records {
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
    n.check.read_rate = ratio(n.check.checked, n.check.checked + n.check.skipped);
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
            surface: Surface::Speech,
            turn: turn.into(),
            message: message.into(),
            scope: Scope::Report,
            mode: "shadow".into(),
            outcome,
            axis: None,
            note: None,
            latency_ms,
            model: "m".into(),
            cost: Default::default(),
            budget_ms: 0,
        })
    }

    /// **One file, one set of numbers per surface.** A record's check never counts as
    /// speech's, a reception is always speech's, and a bypass is counted where it happened.
    #[test]
    fn each_surface_is_counted_apart_from_one_file() {
        let mut on_record = check("resume", "简历在你盘上了", Outcome::Revise, 800);
        if let Record::Check(c) = &mut on_record {
            c.surface = Surface::Record;
        }
        let records = vec![
            check("t1", "a", Outcome::Pass, 1_000),
            on_record,
            Record::Bypass(Bypass {
                serving: Vec::new(),
                was: Size::default(),
                found: Size::default(),
                ts: at("2026-09-15T01:30:00Z"),
                surface: Surface::Record,
                subject: "resume".into(),
            }),
        ];
        let both = legibility(&records);
        assert_eq!((both.speech.check.checked, both.speech.bypasses), (1, 0));
        assert_eq!((both.record.check.checked, both.record.check.revise, both.record.bypasses), (1, 1, 1));

        // Written before surfaces existed, read back as speech.
        let old = r#"{"kind":"check","ts":"2026-09-15T01:00:00Z","turn":"t","message":"m","scope":"long","mode":"shadow","outcome":"pass","latency_ms":5,"model":"m"}"#;
        let Record::Check(c) = serde_json::from_str::<Record>(old).unwrap() else { panic!() };
        assert_eq!(c.surface, Surface::Speech);
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
                surface: Surface::Speech,
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
        let n = numbers(&records, Surface::Speech);
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
