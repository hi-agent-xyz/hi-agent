//! Replay (§ J) for task records: a line is replayed at the moment it was written.
//!
//! **What a worker had seen when it wrote is already on disk.** Its frame log keeps the brief
//! each turn opened with and every command, tool call and message after it, so the material a
//! `hi_task_note` call was written from can be put back in front of a model — as a transcript,
//! with `hi_task_note` the only tool — under this build's worker prompt or a candidate one, and
//! the line asked for again. Both lines are read by the record audit. The rest of the turn —
//! its shell, its builds — is not reproduced, and does not need to be: the question is whether
//! a change writes a better line from the same material.
//!
//! **The set is drawn the way speech's is**: lines the gate sent back, lines it passed, and
//! with no records the most recent. It is private work, and its report stays in the data
//! directory, under `memory/quality/replay/`.

use std::collections::HashMap;
use std::io::BufRead as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::StreamExt as _;
use serde::Serialize;
use serde_json::{Value, json};

use super::judge::Judge;
use crate::foundation::registry::SessionSlug;
use crate::mind::memory::{layout, quality};

/// One model round's limit — a whole generation on a long input.
const ROUND_LIMIT: Duration = Duration::from_secs(240);
/// Lines replayed at once.
const PARALLEL: usize = 4;
/// How much of a turn's material a replay is shown, newest kept — the call is written from the
/// end of the turn, and the brief is kept whole at the top regardless.
const MATERIAL_CHARS: usize = 24_000;
/// How much of one command's output or one tool's answer the transcript keeps.
const OUTPUT_CHARS: usize = 1_500;

pub struct Options {
    pub data_dir: PathBuf,
    /// A whole worker system prompt to replay under. Unset, this build's general worker's.
    pub prompt: Option<PathBuf>,
    /// The model to replay on. Unset, the one the agent runs on.
    pub model: Option<String>,
    pub limit: usize,
}

/// One `hi_task_note` call off a frame log, with what its turn had seen before it.
#[derive(Clone, Debug, Serialize)]
struct Written {
    file: PathBuf,
    ts: DateTime<Utc>,
    subject: String,
    brief: String,
    material: String,
    kind: String,
    text: String,
}

impl Written {
    /// The line as the gate's records name it.
    fn message(&self) -> String {
        format!("{}: {}", self.kind, self.text)
    }
}

fn clip(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n {
        s.to_string()
    } else {
        format!("{}… [{} more characters]", s.chars().take(n).collect::<String>(), count - n)
    }
}

fn newest(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n { s.to_string() } else { format!("[…]\n{}", s.chars().skip(count - n).collect::<String>()) }
}

/// `(run, session)` → the task it served, from the session index.
fn served(data_dir: &Path) -> HashMap<(String, SessionSlug), String> {
    let text = std::fs::read_to_string(crate::foundation::registry::index::index_path(data_dir)).unwrap_or_default();
    text.lines()
        .filter(|l| l.contains("\"opened\""))
        .filter_map(|l| serde_json::from_str::<crate::foundation::registry::index::Record>(l).ok())
        .filter_map(|r| match r {
            crate::foundation::registry::index::Record::Opened { run, session, subject: Some(s), .. } => {
                Some(((run, session), s))
            }
            _ => None,
        })
        .collect()
}

/// Every `hi_task_note` call a worker made, oldest first.
fn read_written(data_dir: &Path) -> Vec<Written> {
    let sessions = layout::raw_root(data_dir).join(layout::SESSIONS_DIR);
    let served = served(data_dir);
    let mut out = Vec::new();
    for run in std::fs::read_dir(&sessions).into_iter().flatten().flatten() {
        if !run.path().is_dir() {
            continue;
        }
        let run_id = run.file_name().to_string_lossy().to_string();
        for file in std::fs::read_dir(run.path()).into_iter().flatten().flatten() {
            let path = file.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            if matches!(stem, "reaction" | "cognition" | "reflection") {
                continue;
            }
            let Ok(slug) = stem.parse::<SessionSlug>() else { continue };
            let subject = served.get(&(run_id.clone(), slug)).cloned();
            out.extend(read_file(&path, subject.as_deref()));
        }
    }
    out.sort_by_key(|w| w.ts);
    out
}

fn read_file(file: &Path, served: Option<&str>) -> Vec<Written> {
    let Ok(f) = std::fs::File::open(file) else { return Vec::new() };
    let mut out = Vec::new();
    let mut brief = String::new();
    let mut material = String::new();
    for line in std::io::BufReader::new(f).lines().map_while(Result::ok) {
        let wanted = line.contains("\"turn/start\"") || line.contains("\"item/completed\"");
        if !wanted {
            continue;
        }
        let Ok(frame) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(raw) = frame.get("raw").and_then(Value::as_str).and_then(|r| serde_json::from_str::<Value>(r).ok())
        else {
            continue;
        };
        let dir = frame.get("dir").and_then(Value::as_str).unwrap_or("");
        let method = frame.get("method").and_then(Value::as_str).unwrap_or("");
        match (dir, method) {
            ("send", "turn/start") => {
                brief = raw["params"]["input"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|i| i.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n");
                material.clear();
            }
            ("recv", "item/completed") => {
                let item = &raw["params"]["item"];
                match item.get("type").and_then(Value::as_str) {
                    Some("commandExecution") => {
                        let command = item["command"].as_str().unwrap_or("");
                        let output = item["aggregatedOutput"].as_str().unwrap_or("");
                        material.push_str(&format!("$ {}\n{}\n\n", clip(command, 600), clip(output, OUTPUT_CHARS)));
                    }
                    Some("fileChange") => {
                        let paths: Vec<&str> = item["changes"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|c| c.get("path").and_then(Value::as_str))
                            .collect();
                        material.push_str(&format!("(edited {})\n\n", paths.join(", ")));
                    }
                    Some("agentMessage") => {
                        material.push_str(&format!("(said) {}\n\n", clip(item["text"].as_str().unwrap_or(""), OUTPUT_CHARS)));
                    }
                    Some("mcpToolCall") => {
                        let tool = item["tool"].as_str().unwrap_or("");
                        let args = &item["arguments"];
                        if tool == "hi_task_note" {
                            let ts = frame
                                .get("ts")
                                .and_then(Value::as_str)
                                .and_then(|t| t.parse().ok())
                                .unwrap_or_else(Utc::now);
                            let subject =
                                args.get("subject").and_then(Value::as_str).or(served).unwrap_or("").to_string();
                            let kind = args["kind"].as_str().unwrap_or("").to_string();
                            let text = args["text"].as_str().unwrap_or("").to_string();
                            if !subject.is_empty() && !kind.is_empty() && !text.trim().is_empty() {
                                out.push(Written {
                                    file: file.to_path_buf(),
                                    ts,
                                    subject,
                                    brief: brief.clone(),
                                    material: newest(&material, MATERIAL_CHARS),
                                    kind: kind.clone(),
                                    text: text.clone(),
                                });
                            }
                            material.push_str(&format!("(wrote on the record) {kind}: {}\n\n", clip(&text, OUTPUT_CHARS)));
                        } else {
                            let answer = item["result"]["content"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|c| c.get("text").and_then(Value::as_str))
                                .collect::<Vec<_>>()
                                .join("\n");
                            material.push_str(&format!(
                                "({tool} {}) → {}\n\n",
                                clip(&args.to_string(), 600),
                                clip(&answer, OUTPUT_CHARS)
                            ));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum Why {
    SentBack,
    Passed,
    Recent,
}

/// Choose the set: sent back first, then passed, newest first within each; with no gate
/// records, the most recent lines.
fn choose(written: &[Written], records: &[quality::Record], limit: usize) -> Vec<(usize, Why)> {
    let mut verdicts: HashMap<(&str, &str), quality::Outcome> = HashMap::new();
    for r in records {
        if let quality::Record::Check(c) = r
            && c.surface == quality::Surface::Record
        {
            verdicts.insert((c.turn.as_str(), c.message.as_str()), c.outcome);
        }
    }
    let messages: Vec<String> = written.iter().map(Written::message).collect();
    let verdict = |i: usize| verdicts.get(&(written[i].subject.as_str(), messages[i].as_str())).copied();
    let mut chosen: Vec<(usize, Why)> = Vec::new();
    for (want, why) in [(quality::Outcome::Revise, Why::SentBack), (quality::Outcome::Pass, Why::Passed)] {
        for i in (0..written.len()).rev() {
            if chosen.len() >= limit {
                break;
            }
            if verdict(i) == Some(want) {
                chosen.push((i, why));
            }
        }
    }
    if chosen.is_empty() {
        chosen = (0..written.len()).rev().take(limit).map(|i| (i, Why::Recent)).collect();
    }
    chosen.truncate(limit);
    chosen
}

fn note_tool() -> Value {
    let t = crate::foundation::mcp::tools_for_role(Some("worker"))
        .into_iter()
        .find(|t| t["name"] == "hi_task_note")
        .unwrap_or_default();
    json!({
        "type": "function",
        "name": "mcp__hi_agent__hi_task_note",
        "description": t["description"],
        "parameters": t["inputSchema"],
    })
}

/// Ask for the line again. `None` when the model wrote nothing on the record.
async fn replay_line(judge: &Judge, prompt: &str, w: &Written) -> anyhow::Result<Option<(String, String)>> {
    let material = format!(
        "{}\n\n## What this turn did before it wrote on the record, oldest first\n{}\n\n## Now\nWrite on this task's record (`{}`) what this turn has to say, with hi_task_note.",
        w.brief.trim(),
        w.material.trim(),
        w.subject
    );
    let body = json!({
        "instructions": prompt,
        "input": [{ "role": "user", "content": [{ "type": "input_text", "text": material }] }],
        "tools": [note_tool()],
        "store": false,
    });
    let reply = judge.respond(&body, ROUND_LIMIT).await?;
    let call = reply["output"].as_array().into_iter().flatten().find(|i| {
        i.get("type").and_then(Value::as_str) == Some("function_call")
            && i["name"].as_str().is_some_and(|n| n.ends_with("hi_task_note"))
    });
    let Some(call) = call else { return Ok(None) };
    let args: Value = call["arguments"].as_str().and_then(|a| serde_json::from_str(a).ok()).unwrap_or(Value::Null);
    let kind = args["kind"].as_str().unwrap_or("").to_string();
    let text = args["text"].as_str().unwrap_or("").to_string();
    Ok((!text.trim().is_empty()).then_some((kind, text)))
}

#[derive(Serialize)]
struct Replayed {
    ts: DateTime<Utc>,
    subject: String,
    why: Why,
    original: String,
    original_audit: Option<quality::Audit>,
    replay: Option<String>,
    replay_audit: Option<quality::Audit>,
    replay_error: Option<String>,
}

#[derive(Default, Serialize)]
struct Tally {
    lines: u32,
    findings: std::collections::BTreeMap<String, u32>,
    chars: usize,
}

impl Tally {
    fn add(&mut self, line: &str, audit: &quality::Audit) {
        self.lines += 1;
        self.chars += line.chars().count();
        for m in &audit.messages {
            if let Some(a) = &m.axis {
                *self.findings.entry(a.clone()).or_default() += 1;
                *self.findings.entry("any".into()).or_default() += 1;
            }
        }
    }
}

/// Replay the set and print what changed. The full report is written under
/// `memory/quality/replay/`.
pub async fn run(opts: Options) -> anyhow::Result<()> {
    let data_dir = &opts.data_dir;
    let replayer = Judge::resolve(data_dir, "record_replay_model")
        .ok_or_else(|| anyhow::anyhow!("no model is configured for this data dir"))?;
    let replayer = match &opts.model {
        Some(m) => replayer.with_model(m),
        None => replayer,
    };
    let auditor = Judge::resolve(data_dir, super::record_audit::MODEL_KEY)
        .ok_or_else(|| anyhow::anyhow!("no model is configured for this data dir"))?;
    let prompt = match &opts.prompt {
        Some(path) => std::fs::read_to_string(path)?,
        None => crate::identity::worker_prompt_as_built(data_dir, crate::identity::WorkerType::General).await,
    };
    let instructions =
        crate::identity::judge_instructions(data_dir, crate::identity::judges::RECORD_AUDIT).await;

    let written = read_written(data_dir);
    let records = quality::read_since(data_dir, DateTime::<Utc>::MIN_UTC).await;
    let chosen = choose(&written, &records, opts.limit);
    anyhow::ensure!(!chosen.is_empty(), "no hi_task_note calls in the frame logs under {}", data_dir.display());
    eprintln!(
        "replaying {} record lines on {} (audit on {}) under {}",
        chosen.len(),
        replayer.model(),
        auditor.model(),
        opts.prompt.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "this build's worker prompt".into())
    );

    let results: Vec<Replayed> = futures::stream::iter(chosen.into_iter().map(|(i, why)| {
        let w = written[i].clone();
        let (replayer, auditor, prompt, instructions) = (&replayer, &auditor, &prompt, &instructions);
        async move {
            let original = w.message();
            let original_audit = super::record_audit::read_line(auditor, instructions, data_dir, &w.subject, &original).await;
            let (replay, replay_error) = match replay_line(replayer, prompt, &w).await {
                Ok(Some((kind, text))) => (Some(format!("{kind}: {text}")), None),
                Ok(None) => (None, Some("wrote nothing on the record".into())),
                Err(err) => (None, Some(format!("{err:#}"))),
            };
            let replay_audit = match &replay {
                Some(line) => super::record_audit::read_line(auditor, instructions, data_dir, &w.subject, line).await,
                None => None,
            };
            Replayed { ts: w.ts, subject: w.subject, why, original, original_audit, replay, replay_audit, replay_error }
        }
    }))
    .buffer_unordered(PARALLEL)
    .collect()
    .await;

    let (mut before, mut after) = (Tally::default(), Tally::default());
    for r in &results {
        if let (Some(a), Some(line), Some(b)) = (&r.original_audit, &r.replay, &r.replay_audit) {
            before.add(&r.original, a);
            after.add(line, b);
        }
    }
    let out_dir = layout::quality_dir(data_dir).join("replay");
    tokio::fs::create_dir_all(&out_dir).await?;
    let out = out_dir.join(format!("records-{}.json", Utc::now().format("%Y%m%dT%H%M%SZ")));
    tokio::fs::write(
        &out,
        serde_json::to_string_pretty(&json!({ "before": before, "after": after, "lines": results }))?,
    )
    .await?;

    println!("record lines compared: {} (of {} replayed)", before.lines, results.len());
    println!("{:<14}{:>10}{:>10}", "axis", "before", "after");
    let axes: std::collections::BTreeSet<&String> = before.findings.keys().chain(after.findings.keys()).collect();
    for axis in axes {
        println!(
            "{:<14}{:>10}{:>10}",
            axis,
            before.findings.get(axis).copied().unwrap_or(0),
            after.findings.get(axis).copied().unwrap_or(0)
        );
    }
    let avg = |t: &Tally| if t.lines == 0 { 0 } else { t.chars / t.lines as usize };
    println!("{:<14}{:>10}{:>10}", "chars/line", avg(&before), avg(&after));
    println!("report: {}", out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(dir: &str, method: &str, raw: Value) -> String {
        json!({ "ts": "2026-09-19T01:47:12Z", "dir": dir, "method": method, "raw": raw.to_string() }).to_string()
    }

    /// **A line is replayed from what its turn had seen before it wrote**: the brief, the
    /// commands and answers after it — and not what came later.
    #[test]
    fn a_note_carries_the_material_written_before_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("general-resume.jsonl");
        let lines = [
            frame("send", "turn/start", json!({ "params": { "input": [{ "text": "导入简历" }] } })),
            frame("recv", "item/completed", json!({ "params": { "item": { "type": "commandExecution", "command": "ls ~/Desktop", "aggregatedOutput": "resume.pdf" } } })),
            frame("recv", "item/completed", json!({ "params": { "item": { "type": "mcpToolCall", "tool": "hi_task_note", "arguments": { "kind": "delivered", "text": "简历在你盘上了" } } } })),
            frame("recv", "item/completed", json!({ "params": { "item": { "type": "commandExecution", "command": "later", "aggregatedOutput": "after" } } })),
        ];
        std::fs::write(&file, lines.join("\n")).unwrap();
        let got = read_file(&file, Some("resume-import"));
        assert_eq!(got.len(), 1);
        let w = &got[0];
        assert_eq!((w.subject.as_str(), w.message().as_str()), ("resume-import", "delivered: 简历在你盘上了"));
        assert_eq!(w.brief, "导入简历");
        assert!(w.material.contains("resume.pdf") && !w.material.contains("after"));
    }

    #[test]
    fn the_set_leads_with_what_the_gate_sent_back() {
        let w = |text: &str| Written {
            file: PathBuf::new(),
            ts: Utc::now(),
            subject: "s".into(),
            brief: String::new(),
            material: String::new(),
            kind: "update".into(),
            text: text.into(),
        };
        let written = vec![w("a"), w("b"), w("c")];
        let check = |message: &str, outcome| {
            quality::Record::Check(quality::Check {
                ts: Utc::now(),
                surface: quality::Surface::Record,
                turn: "s".into(),
                message: message.into(),
                scope: quality::Scope::Long,
                mode: "shadow".into(),
                outcome,
                axis: None,
                note: None,
                latency_ms: 1,
                model: "m".into(),
            })
        };
        let records = vec![check("update: a", quality::Outcome::Pass), check("update: c", quality::Outcome::Revise)];
        assert_eq!(choose(&written, &records, 10), vec![(2, Why::SentBack), (0, Why::Passed)]);
        assert_eq!(choose(&written, &[], 2), vec![(2, Why::Recent), (1, Why::Recent)]);
    }
}
