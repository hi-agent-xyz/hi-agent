//! Replay (§ J, § K): past Reaction turns run again under this build's prompt, or a
//! candidate one, or another model — tools stubbed — and scored by the same audit as the
//! live turns.
//!
//! **Every turn's input is already on disk.** The frame log keeps the `turn/start` Reaction
//! was sent and every call it made — `hi_prepare` now, and the `hi_say` of turns written
//! before it was the one way out — so a turn can be put back in front of a model with
//! its thread's opening window and the turns just before it as history. That is not the
//! live thread — codex's compactions and its own instructions are not reproduced — and it
//! does not need to be: the question replay answers is whether a *change* reads better, and
//! both sides of that comparison are scored the same way.
//!
//! **The set is drawn from what the readers found**: turns the person corrected, turns the
//! audit flagged, and turns it judged good, so a fix cannot overshoot into leaving things
//! out. With no records yet it is the most recent turns that spoke. The set is private
//! conversation, and its report stays in the data directory.

use std::collections::{BTreeMap, HashMap};
use std::io::BufRead as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::StreamExt as _;
use serde::Serialize;
use serde_json::{Value, json};

use crate::mind::memory::{layout, quality};

use super::check::Brief;
use crate::body::legibility::judge::Judge;

mod review;

/// One model round's limit. A replayed turn is a whole generation on a long input.
const ROUND_LIMIT: Duration = Duration::from_secs(240);
/// Tool rounds before a replayed turn is cut off. A live turn is one generation with a few
/// calls; a loop past this is a replay artefact, not speech.
const MAX_ROUNDS: usize = 8;
/// Turns replayed at once.
const PARALLEL: usize = 4;
/// Characters of earlier conversation the audit reads before a turn.
const RECENT_CHARS: usize = 6_000;

pub struct Options {
    pub data_dir: PathBuf,
    /// A whole system prompt to replay under. Unset, this build's own.
    pub prompt: Option<PathBuf>,
    /// The model to replay on. Unset, the one the agent runs on.
    pub model: Option<String>,
    pub limit: usize,
    /// Earlier turns of the same thread that ride along as history.
    pub context: usize,
    pub cases: Option<PathBuf>,
}

/// One `hi_*` call as the frame log recorded it.
#[derive(Clone, Debug, Serialize)]
struct Call {
    tool: String,
    arguments: Value,
    result: String,
    /// What the model reasoned before making it. A thinking upstream refuses a history
    /// whose tool calls arrive without their reasoning.
    reasoning: String,
    steers: Vec<String>,
}

/// One Reaction turn off the frame log.
#[derive(Clone, Debug, Serialize)]
struct Turn {
    file: PathBuf,
    thread: String,
    ts: DateTime<Utc>,
    input: String,
    calls: Vec<Call>,
    typed: String,
    delivered: Vec<(DateTime<Utc>, String)>,
}

impl Turn {
    /// Only actual journaled messages (or a legacy hi_say receipt), never prepared drafts.
    fn said(&self) -> Vec<String> {
        self.said_before(DateTime::<Utc>::MAX_UTC)
    }

    fn said_before(&self, before: DateTime<Utc>) -> Vec<String> {
        if !self.delivered.is_empty() {
            return self.delivered.iter().filter(|(ts, _)| *ts < before).map(|(_, text)| text.clone()).collect();
        }
        let mut said = Vec::new();
        for c in &self.calls {
            match c.tool.as_str() {
                "hi_say" if c.result.starts_with("sent") => {
                    said.extend(c.arguments.get("text").and_then(Value::as_str).map(str::to_string));
                }
                _ => {}
            }
        }
        said
    }

    fn drafted(&self) -> Vec<String> {
        self.calls.iter().filter(|c| c.tool == "hi_prepare" && c.result.starts_with("prepared"))
            .flat_map(|c| finished(&c.arguments).0).chain(self.said()).collect()
    }

    /// What it put on screen, as the judges read it — the same reading.
    fn shown(&self) -> Vec<String> {
        let mut shown = Vec::new();
        for c in &self.calls {
            match c.tool.as_str() {
                "hi_show" => shown.push(shown_line(&c.arguments)),
                "hi_prepare" if c.result.starts_with("prepared") => shown.extend(finished(&c.arguments).1),
                _ => {}
            }
        }
        shown
    }
}

/// A `hi_prepare` call's `finished` branch: its lines, and its shows as lines.
fn finished(arguments: &Value) -> (Vec<String>, Vec<String>) {
    let (mut said, mut shown) = (Vec::new(), Vec::new());
    let branches = arguments.get("branches").and_then(Value::as_array).cloned().unwrap_or_default();
    for b in branches.iter().filter(|b| b.get("when").and_then(Value::as_str) == Some("finished")) {
        for a in b.get("actions").and_then(Value::as_array).into_iter().flatten() {
            match a.get("do").and_then(Value::as_str) {
                Some("say") => said.extend(a.get("text").and_then(Value::as_str).map(str::to_string)),
                Some("show") => shown.push(shown_line(a)),
                _ => {}
            }
        }
    }
    (said, shown)
}

/// One show — a `show` action, or a `hi_show` call on the log — as a line: the op, then the
/// ref or id it named.
fn shown_line(arguments: &Value) -> String {
    let op = arguments.get("op").and_then(Value::as_str).unwrap_or("show");
    let what = ["ref", "id"]
        .iter()
        .find_map(|k| arguments.get(*k).and_then(Value::as_str))
        .unwrap_or("the screen");
    format!("{op} {what}")
}

/// Every Reaction turn in the frame logs, oldest first.
fn read_turns(data_dir: &Path) -> Vec<Turn> {
    let sessions = layout::raw_root(data_dir).join("sessions");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&sessions)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("reaction.jsonl"))
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    let mut turns = Vec::new();
    for file in files {
        turns.extend(read_file(&file));
    }
    turns.sort_by_key(|t| t.ts);
    attach_delivered(data_dir, &mut turns);
    turns
}

fn attach_delivered(data_dir: &Path, turns: &mut [Turn]) {
    use crate::mind::memory::journal;
    use crate::types::{Content, JournalEntry};
    let text_dir = layout::raw_root(data_dir).join("text");
    let mut messages = Vec::new();
    for day in std::fs::read_dir(text_dir).into_iter().flatten().flatten() {
        let Ok(file) = std::fs::File::open(day.path().join("text.jsonl")) else { continue };
        for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
            if let Some(JournalEntry::Message { message, .. }) = journal::classify_line(&line)
                && message.from.is_agent()
                && let Content::Text(text) = message.content {
                messages.push((message.ts, text));
            }
        }
    }
    messages.sort_by_key(|(ts, _)| *ts);
    for (ts, text) in messages {
        if let Some(turn) = turns.iter_mut().rev().find(|t| t.ts <= ts && t.drafted().contains(&text)) {
            turn.delivered.push((ts, text));
        }
    }
}

fn read_file(file: &Path) -> Vec<Turn> {
    let Ok(f) = std::fs::File::open(file) else { return Vec::new() };
    let mut out: Vec<Turn> = Vec::new();
    let mut open: Option<Turn> = None;
    let mut reasoning = String::new();
    let mut steers = Vec::new();
    for line in std::io::BufReader::new(f).lines().map_while(Result::ok) {
        // Most of a frame log is streamed reasoning; skip it before parsing anything.
        let wanted = line.contains("\"turn/start\"")
            || line.contains("\"turn/steer\"")
            || line.contains("\"item/completed\"")
            || line.contains("\"turn/completed\"");
        if !wanted {
            continue;
        }
        let Ok(frame) = serde_json::from_str::<Value>(&line) else { continue };
        let method = frame.get("method").and_then(Value::as_str).unwrap_or("");
        let dir = frame.get("dir").and_then(Value::as_str).unwrap_or("");
        let Some(raw) = frame
            .get("raw")
            .and_then(Value::as_str)
            .and_then(|r| serde_json::from_str::<Value>(r).ok())
        else {
            continue;
        };
        match (dir, method) {
            ("send", "turn/start") => {
                out.extend(open.take());
                reasoning.clear();
                steers.clear();
                let input = raw["params"]["input"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|i| i.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n");
                let ts = frame
                    .get("ts")
                    .and_then(Value::as_str)
                    .and_then(|t| t.parse().ok())
                    .unwrap_or_else(Utc::now);
                open = Some(Turn {
                    file: file.to_path_buf(),
                    thread: frame.get("thread_id").and_then(Value::as_str).unwrap_or("").to_string(),
                    ts,
                    input,
                    calls: Vec::new(),
                    typed: String::new(),
                    delivered: Vec::new(),
                });
            }
            ("send", "turn/steer") => {
                let text = raw["params"]["input"].as_array().into_iter().flatten()
                    .filter_map(|i| i["text"].as_str()).collect::<Vec<_>>().join("\n");
                if !text.is_empty() { steers.push(text); }
            }
            ("recv", "item/completed") => {
                let Some(turn) = open.as_mut() else { continue };
                let item = &raw["params"]["item"];
                match item.get("type").and_then(Value::as_str) {
                    Some("reasoning") => {
                        for part in item["content"].as_array().into_iter().flatten() {
                            if let Some(t) = part.as_str().or_else(|| part.get("text").and_then(Value::as_str)) {
                                reasoning.push_str(t);
                            }
                        }
                    }
                    Some("mcpToolCall") => turn.calls.push(Call {
                        steers: steers.clone(),
                        reasoning: std::mem::take(&mut reasoning),
                        tool: item["tool"].as_str().unwrap_or("").to_string(),
                        arguments: item["arguments"].clone(),
                        result: item["result"]["content"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|c| c.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    }),
                    Some("agentMessage") => {
                        turn.typed = item["text"].as_str().unwrap_or("").to_string();
                    }
                    _ => {}
                }
            }
            ("recv", "turn/completed") => out.extend(open.take()),
            _ => {}
        }
    }
    out.extend(open);
    out
}

/// The `##` section of a window titled `heading`, through to the next `##`.
fn section<'a>(text: &'a str, heading: &str) -> Option<&'a str> {
    let start = text.find(&format!("## {heading}"))?;
    let rest = &text[start..];
    let end = rest[3..].find("\n## ").map(|e| e + 3).unwrap_or(rest.len());
    Some(rest[..end].trim())
}

/// The latest copy of a window section this thread had been sent by `upto`.
fn latest_section(thread: &[&Turn], upto: usize, heading: &str) -> String {
    thread[..=upto]
        .iter()
        .rev()
        .find_map(|t| section(&t.input, heading))
        .unwrap_or("")
        .to_string()
}

/// What the audit reads about a turn, rebuilt from the frames: the reader as the thread
/// last had them, the turns just before as a transcript, and this turn's new signals.
fn brief_for(thread: &[&Turn], at: usize, context: usize) -> Brief {
    let reader = [
        latest_section(thread, at, "Working with them"),
        latest_section(thread, at, "What your words have earned"),
    ]
    .join("\n\n");
    let mut recent = String::new();
    for t in &thread[at.saturating_sub(context)..at] {
        if let Some(signals) = section(&t.input, "New signals") {
            recent.push_str(signals.trim_start_matches("## New signals").trim());
            recent.push('\n');
        }
        if let Some(call) = t.calls.last() {
            for steer in &call.steers { recent.push_str(steer); recent.push('\n'); }
        }
        for m in t.said_before(thread[at].ts) {
            recent.push_str(&format!("< {}\n", m.replace('\n', "\n  ")));
        }
    }
    let skip = recent.chars().count().saturating_sub(RECENT_CHARS);
    let recent: String = recent.chars().skip(skip).collect();
    let signals = section(&thread[at].input, "New signals")
        .unwrap_or(&thread[at].input)
        .to_string();
    let carries_report = signals.contains("(from session") || signals.contains("report");
    let screen = latest_section(thread, at, "On screen now");
    Brief { reader: reader.trim().to_string(), recent, signals, screen, carries_report }
}

/// Reconstruct only material available when this message was prepared. Used by the
/// live-check replay, without today's learned memory or later corrections.
#[cfg(test)]
pub(super) fn historical_check_case(data_dir: &Path, needle: &str) -> Option<(Brief, String)> {
    let turns = read_turns(data_dir);
    let turn = turns.iter().find(|t| t.said().iter().any(|s| s.contains(needle)))?;
    let members: Vec<&Turn> = turns.iter().filter(|t| t.file == turn.file && t.thread == turn.thread).collect();
    let at = members.iter().position(|t| std::ptr::eq(*t, turn))?;
    let message = turn.said().into_iter().find(|s| s.contains(needle))?;
    Some((brief_for(&members, at, 6), message))
}

/// The tools a replayed turn may call, spelled the way the runtime spells them.
fn tools() -> Value {
    Value::Array(
        crate::foundation::mcp::tools_for_role(Some("reaction"))
            .into_iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "name": format!("mcp__hi_agent__{}", t["name"].as_str().unwrap_or("")),
                    "description": t["description"],
                    "parameters": t["inputSchema"],
                })
            })
            .collect(),
    )
}

fn user(text: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "input_text", "text": text }] })
}

/// A recorded turn as history items: what it was sent, the calls it made and what they
/// answered, and what it typed.
fn history_items(turn: &Turn, n: usize, out: &mut Vec<Value>) {
    out.push(user(&turn.input));
    let mut seen_steers = 0;
    for (i, c) in turn.calls.iter().enumerate() {
        for steer in &c.steers[seen_steers.min(c.steers.len())..] { out.push(user(steer)); }
        seen_steers = c.steers.len();
        let id = format!("h{n}_{i}");
        let reasoning = if c.reasoning.trim().is_empty() { "(not recorded)" } else { c.reasoning.as_str() };
        out.push(json!({
            "type": "reasoning",
            "summary": [],
            "content": [{ "type": "reasoning_text", "text": reasoning }],
        }));
        out.push(json!({
            "type": "function_call",
            "call_id": id,
            "name": format!("mcp__hi_agent__{}", c.tool),
            "arguments": c.arguments.to_string(),
        }));
        out.push(json!({ "type": "function_call_output", "call_id": id, "output": c.result }));
    }
    if !turn.typed.trim().is_empty() {
        out.push(json!({
            "role": "assistant",
            "content": [{ "type": "output_text", "text": turn.typed }],
        }));
    }
}

/// What the host would have answered a replayed call, floor aside: a set it would take is
/// "prepared", as the host says it. A call to a verb that no longer exists — a history turn's
/// `hi_say` is in front of the model — is answered as no tool.
fn stub(name: &str, arguments: &Value) -> String {
    match name.trim_start_matches("mcp__hi_agent__") {
        "hi_prepare" => {
            let (said, _) = finished(arguments);
            if let Some(long) = said.iter().find(|t| t.chars().count() > super::super::tools::SAY_MAX_CHARS) {
                return format!(
                    "branch (\"finished\"), action: too long for one message ({} characters; the most is {}). Nothing was prepared.",
                    long.chars().count(),
                    super::super::tools::SAY_MAX_CHARS
                );
            }
            "prepared — nothing has been said yet: it goes when the room reaches that moment and it still fits".to_string()
        }
        "hi_send_message" => "delivered".to_string(),
        other => format!("no tool named {other}"),
    }
}

/// Run one turn again. Returns what it said and what it put on screen.
async fn replay_turn(
    judge: &Judge,
    prompt: &str,
    thread: &[&Turn],
    at: usize,
    context: usize,
) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    let mut input = Vec::new();
    let from = at.saturating_sub(context);
    // The thread's opening turn carries the whole window; later turns carry only what moved.
    if from > 0 {
        history_items(thread[0], 0, &mut input);
    }
    for (n, t) in thread[from..at].iter().enumerate() {
        history_items(t, n + 1, &mut input);
    }
    input.push(user(&thread[at].input));
    if let Some(call) = thread[at].calls.last() {
        for steer in &call.steers { input.push(user(steer)); }
    }

    let tools = tools();
    let mut said = Vec::new();
    let mut shown = Vec::new();
    for round in 0..MAX_ROUNDS {
        let body = json!({
            "instructions": prompt,
            "input": input,
            "tools": tools,
            "store": false,
        });
        let reply = judge.respond(&body, ROUND_LIMIT).await?;
        let output: Vec<Value> = reply["output"].as_array().cloned().unwrap_or_default();
        let calls: Vec<Value> = output
            .iter()
            .filter(|i| i.get("type").and_then(Value::as_str) == Some("function_call"))
            .cloned()
            .collect();
        if calls.is_empty() {
            break;
        }
        // A thinking model's reasoning goes back with its calls: an upstream in thinking
        // mode refuses the next round without it (measured: 9 of 60 replays failed on
        // exactly that before this). Rebuilt without ids, for the reason calls are.
        input.extend(
            output
                .iter()
                .filter(|i| i.get("type").and_then(Value::as_str) == Some("reasoning"))
                .map(|i| json!({ "type": "reasoning", "summary": [], "content": i["content"] })),
        );
        for (i, call) in calls.iter().enumerate() {
            let name = call["name"].as_str().unwrap_or("").to_string();
            let arguments: Value = call["arguments"]
                .as_str()
                .and_then(|a| serde_json::from_str(a).ok())
                .unwrap_or(Value::Null);
            let id = call["call_id"].as_str().map(str::to_string).unwrap_or(format!("r{round}_{i}"));
            let answer = stub(&name, &arguments);
            if name.ends_with("hi_prepare") && answer.starts_with("prepared") {
                let (lines, shows) = finished(&arguments);
                said.extend(lines);
                shown.extend(shows);
            }
            // Rebuilt without the item's id: with nothing stored upstream, an id is a
            // reference to something that does not exist.
            input.push(json!({
                "type": "function_call",
                "call_id": id,
                "name": name,
                "arguments": call["arguments"].as_str().unwrap_or("{}"),
            }));
            input.push(json!({ "type": "function_call_output", "call_id": id, "output": answer }));
        }
    }
    Ok((said, shown))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum Why {
    Corrected,
    Flagged,
    Good,
    Recent,
}

#[derive(Serialize)]
struct Replayed {
    ts: DateTime<Utc>,
    why: Why,
    /// The axis the person's correction named, for a corrected turn.
    corrected_axis: Option<String>,
    original: Option<quality::Audit>,
    replay: Option<quality::Audit>,
    replay_error: Option<String>,
}

#[derive(Default, Serialize)]
struct Tally {
    turns: u32,
    messages: u32,
    findings: BTreeMap<String, u32>,
    unsaid: u32,
    wrong: u32,
    /// Corrected turns where the correction's axis came back.
    correction_recurs: u32,
    corrected: u32,
}

impl Tally {
    fn add(&mut self, audit: &quality::Audit, corrected_axis: Option<&str>) {
        self.turns += 1;
        self.messages += audit.messages.len() as u32;
        for m in &audit.messages {
            if let Some(a) = &m.axis {
                *self.findings.entry(a.clone()).or_default() += 1;
                *self.findings.entry("any".into()).or_default() += 1;
            }
        }
        self.unsaid += audit.unsaid.len() as u32;
        self.wrong += audit.wrong.len() as u32;
        if let Some(axis) = corrected_axis {
            self.corrected += 1;
            if audit.messages.iter().any(|m| m.axis.as_deref() == Some(axis)) {
                self.correction_recurs += 1;
            }
        }
    }

    fn per_100(&self, n: u32, of: u32) -> String {
        if of == 0 { "—".into() } else { format!("{:.1}", f64::from(n) * 100.0 / f64::from(of)) }
    }
}

/// Choose the set: corrected first, then flagged, then good, newest first within each.
fn choose(
    turns: &[Turn],
    records: &[quality::Record],
    limit: usize,
) -> Vec<(usize, Why, Option<String>, Option<quality::Audit>)> {
    // A live audit names a turn by key; the frames name it by what it said. Matching on the
    // messages is exact where it matters — a turn that said nothing was never audited.
    let by_said: HashMap<Vec<String>, usize> =
        turns.iter().enumerate().filter(|(_, t)| !t.said().is_empty()).map(|(i, t)| (t.said(), i)).collect();
    let mut audits: HashMap<&str, (&quality::Audit, usize)> = HashMap::new();
    for r in records {
        if let quality::Record::Audit(a) = r {
            let said: Vec<String> = a.messages.iter().map(|m| m.text.clone()).collect();
            if let Some(&i) = by_said.get(&said) {
                audits.insert(a.turn.as_str(), (a, i));
            }
        }
    }
    let mut corrected: BTreeMap<usize, Option<String>> = BTreeMap::new();
    for r in records {
        if let quality::Record::Reception(rec) = r
            && rec.corrects
        {
            for key in &rec.turns {
                if let Some((_, i)) = audits.get(key.as_str()) {
                    corrected.insert(*i, rec.axis.clone());
                }
            }
        }
    }
    let mut chosen: Vec<(usize, Why, Option<String>, Option<quality::Audit>)> = Vec::new();
    let taken = |chosen: &Vec<(usize, Why, Option<String>, Option<quality::Audit>)>, i: usize| {
        chosen.iter().any(|c| c.0 == i)
    };
    let audit_of = |i: usize| audits.values().find(|(_, j)| *j == i).map(|(a, _)| (*a).clone());
    for (&i, axis) in corrected.iter().rev().take(limit.div_ceil(2)) {
        chosen.push((i, Why::Corrected, axis.clone(), audit_of(i)));
    }
    let mut audited: Vec<(&quality::Audit, usize)> = audits.values().copied().collect();
    audited.sort_by_key(|(a, _)| std::cmp::Reverse(a.ts));
    let flagged = |a: &quality::Audit| {
        a.messages.iter().any(|m| m.axis.is_some()) || !a.unsaid.is_empty() || !a.wrong.is_empty()
    };
    for (a, i) in audited.iter().filter(|(a, _)| flagged(a)) {
        if chosen.len() >= limit * 3 / 4 {
            break;
        }
        if !taken(&chosen, *i) {
            chosen.push((*i, Why::Flagged, None, Some((*a).clone())));
        }
    }
    for (a, i) in audited.iter().filter(|(a, _)| !flagged(a)) {
        if chosen.len() >= limit {
            break;
        }
        if !taken(&chosen, *i) {
            chosen.push((*i, Why::Good, None, Some((*a).clone())));
        }
    }
    if chosen.is_empty() {
        for (i, _) in turns.iter().enumerate().rev().filter(|(_, t)| !t.said().is_empty()).take(limit) {
            chosen.push((i, Why::Recent, None, None));
        }
    }
    chosen.truncate(limit);
    chosen
}

/// Replay the set and print what changed. The full report is written under
/// `memory/quality/replay/`.
pub async fn run(opts: Options) -> anyhow::Result<()> {
    if opts.cases.is_some() {
        return review::run(opts).await;
    }
    let data_dir = &opts.data_dir;
    let replayer = Judge::resolve(data_dir, "speech_replay_model")
        .ok_or_else(|| anyhow::anyhow!("no model is configured for this data dir"))?;
    let replayer = match &opts.model {
        Some(m) => replayer.with_model(m),
        None => replayer,
    };
    let auditor = Judge::resolve(data_dir, super::audit::MODEL_KEY)
        .ok_or_else(|| anyhow::anyhow!("no model is configured for this data dir"))?;
    let prompt = match &opts.prompt {
        Some(path) => std::fs::read_to_string(path)?,
        None => crate::identity::reaction_prompt_as_built(),
    };
    let audit_instructions =
        crate::identity::judge_instructions(data_dir, crate::identity::judges::AUDIT).await;

    let turns = read_turns(data_dir);
    let records = quality::read_since(data_dir, DateTime::<Utc>::MIN_UTC).await;
    let chosen = choose(&turns, &records, opts.limit);
    anyhow::ensure!(!chosen.is_empty(), "no spoken Reaction turns in the frame logs under {}", data_dir.display());
    eprintln!(
        "replaying {} turns on {} (audit on {}) under {}",
        chosen.len(),
        replayer.model(),
        auditor.model(),
        opts.prompt.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "this build's prompt".into())
    );

    let context = opts.context;
    let threads: HashMap<(PathBuf, String), Vec<usize>> = turns.iter().enumerate().fold(
        HashMap::new(),
        |mut m, (i, t)| {
            m.entry((t.file.clone(), t.thread.clone())).or_default().push(i);
            m
        },
    );

    let results: Vec<Replayed> = futures::stream::iter(chosen)
        .map(|(i, why, corrected_axis, known)| {
            let turns = &turns;
            let threads = &threads;
            let (replayer, auditor, prompt, audit_instructions) =
                (&replayer, &auditor, &prompt, &audit_instructions);
            async move {
                let turn = &turns[i];
                let members: Vec<&Turn> = threads[&(turn.file.clone(), turn.thread.clone())]
                    .iter()
                    .map(|&j| &turns[j])
                    .collect();
                let at = members.iter().position(|t| std::ptr::eq(*t, turn)).unwrap_or(0);
                let brief = brief_for(&members, at, context);
                let original = match known {
                    Some(a) => Some(a),
                    None => {
                        super::audit::audit(
                            auditor,
                            audit_instructions,
                            "original",
                            &brief,
                            &turn.said(),
                            &turn.shown(),
                        )
                        .await
                    }
                };
                let (replay, replay_error) =
                    match replay_turn(replayer, prompt, &members, at, context).await {
                        Ok((said, _)) if said.is_empty() => (
                            Some(quality::Audit {
                                ts: Utc::now(),
                                surface: quality::Surface::Speech,
                                turn: "replay".into(),
                                model: auditor.model().to_string(),
                                messages: vec![],
                                unsaid: vec![],
                                wrong: vec![],
                            }),
                            None,
                        ),
                        Ok((said, shown)) => (
                            super::audit::audit(
                                auditor,
                                audit_instructions,
                                "replay",
                                &brief,
                                &said,
                                &shown,
                            )
                            .await,
                            None,
                        ),
                        Err(err) => (None, Some(format!("{err:#}"))),
                    };
                eprint!(".");
                Replayed { ts: turn.ts, why, corrected_axis, original, replay, replay_error }
            }
        })
        .buffer_unordered(PARALLEL)
        .collect()
        .await;
    eprintln!();

    let (mut before, mut after) = (Tally::default(), Tally::default());
    for r in &results {
        // Only turns both sides could be read on are compared.
        let (Some(o), Some(n)) = (&r.original, &r.replay) else { continue };
        before.add(o, r.corrected_axis.as_deref());
        after.add(n, r.corrected_axis.as_deref());
    }
    print_table(&before, &after, &results);

    let dir = layout::quality_dir(data_dir).join("replay");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", Utc::now().format("%Y-%m-%dT%H-%M-%SZ")));
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&json!({
            "model": replayer.model(),
            "auditor": auditor.model(),
            "prompt": opts.prompt.as_ref().map(|p| p.display().to_string()),
            "context": opts.context,
            "before": before,
            "after": after,
            "turns": results,
        }))?,
    )?;
    println!("\nreport: {}", path.display());
    Ok(())
}

fn print_table(before: &Tally, after: &Tally, results: &[Replayed]) {
    let count = |w: Why| results.iter().filter(|r| r.why == w).count();
    println!(
        "{} turns compared ({} corrected, {} flagged, {} good, {} recent); {} could not be replayed",
        before.turns,
        count(Why::Corrected),
        count(Why::Flagged),
        count(Why::Good),
        count(Why::Recent),
        results.iter().filter(|r| r.replay_error.is_some()).count()
    );
    println!("{:<34}{:>12}{:>12}", "", "original", "replay");
    println!("{:<34}{:>12}{:>12}", "messages", before.messages, after.messages);
    let mut axes: Vec<&String> = before.findings.keys().chain(after.findings.keys()).collect();
    axes.sort();
    axes.dedup();
    for axis in axes {
        let label = if axis == "any" { "findings / 100 messages".to_string() } else { format!("  {axis}") };
        println!(
            "{:<34}{:>12}{:>12}",
            label,
            before.per_100(*before.findings.get(axis).unwrap_or(&0), before.messages),
            after.per_100(*after.findings.get(axis).unwrap_or(&0), after.messages),
        );
    }
    println!(
        "{:<34}{:>12}{:>12}",
        "unsaid / 100 turns",
        before.per_100(before.unsaid, before.turns),
        after.per_100(after.unsaid, after.turns)
    );
    println!("{:<34}{:>12}{:>12}", "wrong", before.wrong, after.wrong);
    if before.corrected > 0 {
        println!(
            "{:<34}{:>12}{:>12}",
            "corrected axis comes back",
            format!("{}/{}", before.correction_recurs, before.corrected),
            format!("{}/{}", after.correction_recurs, after.corrected)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(seq: u32, dir: &str, method: &str, raw: Value) -> String {
        json!({
            "seq": seq, "ts": format!("2026-09-15T09:10:{seq:02}Z"), "dir": dir,
            "method": method, "thread_id": "th", "raw": raw.to_string(),
        })
        .to_string()
    }

    #[test]
    fn a_turn_is_read_back_off_the_frame_log() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("reaction.jsonl");
        let lines = [
            frame(1, "send", "turn/start", json!({ "params": { "input": [{ "type": "text", "text": "## Working with them\n**赵力** — 简要\n\n## New signals\n> 部署一下" }] } })),
            frame(2, "recv", "item/reasoning/textDelta", json!({ "params": {} })),
            frame(3, "recv", "item/completed", json!({ "params": { "item": {
                "type": "mcpToolCall", "tool": "hi_say", "arguments": { "text": "好" },
                "result": { "content": [{ "type": "text", "text": "sent — the message is in the conversation now, and stays there" }] } } } })),
            frame(4, "recv", "item/completed", json!({ "params": { "item": {
                "type": "mcpToolCall", "tool": "hi_say", "arguments": { "text": "被拒的" },
                "result": { "content": [{ "type": "text", "text": "not said — they were still talking" }] } } } })),
            frame(5, "recv", "turn/completed", json!({ "params": {} })),
        ];
        std::fs::write(&file, lines.join("\n")).unwrap();
        let turns = read_file(&file);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].said(), vec!["好".to_string()], "only what came back sent was said");

        // A prepared reply is not speech until the text journal confirms it.
        let prepared = Turn {
            calls: vec![Call {
                tool: "hi_prepare".into(),
                arguments: json!({ "matter": "m", "branches": [
                    { "when": "finished", "actions": [{ "do": "say", "text": "答" }, { "do": "show", "ref": "p/v" }] },
                    { "when": "paused", "actions": [{ "do": "say", "text": "收到" }] }
                ] }),
                result: "prepared \"m\" — finished; paused".into(),
                reasoning: String::new(),
                steers: vec![],
            }],
            ..turns[0].clone()
        };
        assert!(prepared.said().is_empty());
        assert_eq!(prepared.drafted(), vec!["答".to_string()]);
        assert_eq!(prepared.shown(), vec!["show p/v".to_string()]);
        assert_eq!(section(&turns[0].input, "New signals"), Some("## New signals\n> 部署一下"));

        let members: Vec<&Turn> = turns.iter().collect();
        let brief = brief_for(&members, 0, 6);
        assert!(brief.reader.contains("简要"));
        assert!(brief.signals.contains("部署一下"));
    }

    #[test]
    fn a_replayed_prepare_is_answered_the_way_the_host_would() {
        let set = |text: &str| json!({ "matter": "m", "branches": [{ "when": "finished", "actions": [{ "do": "say", "text": text }] }] });
        assert!(stub("mcp__hi_agent__hi_prepare", &set("好")).starts_with("prepared"));
        let long = "字".repeat(super::super::super::tools::SAY_MAX_CHARS + 1);
        assert!(stub("mcp__hi_agent__hi_prepare", &set(&long)).contains("too long"));
        assert_eq!(stub("mcp__hi_agent__hi_say", &json!({ "text": "好" })), "no tool named hi_say");

    }

    #[test]
    fn only_actual_delivery_before_the_next_input_is_user_known() {
        let dir = tempfile::tempdir().unwrap();
        let at: DateTime<Utc> = "2026-10-12T10:00:00Z".parse().unwrap();
        let delivered_at = at + chrono::Duration::seconds(90);
        let make = |text: &str| Turn { file:"reaction.jsonl".into(), thread:"thread".into(), ts:at,
            input:String::new(), typed:String::new(), delivered:Vec::new(), calls:vec![Call {
                tool:"hi_prepare".into(), result:"prepared".into(), reasoning:String::new(), steers:Vec::new(),
                arguments:json!({"branches":[{"when":"finished","actions":[{"do":"say","text":text}]}]}),
            }] };
        let mut turns = vec![make("never sent"), make("delivered later")];
        let entry = crate::mind::memory::journal::legacy_signal_out("message".into(), delivered_at,
            crate::types::Channel::Text, "delivered later".into(), None, None);
        let path = layout::channel_log_path(dir.path(), crate::types::Channel::Text, delivered_at);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_string(&entry).unwrap()+"\n").unwrap();
        attach_delivered(dir.path(), &mut turns);
        assert!(turns[0].said().is_empty());
        assert!(turns[1].said_before(at + chrono::Duration::seconds(60)).is_empty());
        assert_eq!(turns[1].said_before(at + chrono::Duration::seconds(120)), ["delivered later"]);
    }

    #[test]
    fn steering_is_kept_at_the_call_that_had_seen_it_without_future_messages() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("reaction.jsonl");
        let call = |text: &str| json!({"params":{"item":{"type":"mcpToolCall","tool":"hi_say",
            "arguments":{"text":text},"result":{"content":[{"text":"sent"}]}}}});
        let lines = [
            frame(1,"send","turn/start",json!({"params":{"input":[{"text":"initial"}]}})),
            frame(2,"send","turn/steer",json!({"params":{"input":[{"text":"new question"}]}})),
            frame(3,"recv","item/completed",call("first answer")),
            frame(4,"send","turn/steer",json!({"params":{"input":[{"text":"later fact"}]}})),
            frame(5,"recv","item/completed",call("second answer")),
            frame(6,"recv","turn/completed",json!({})),
        ];
        std::fs::write(&file,lines.join("\n")).unwrap();
        let turns = read_file(&file);
        assert_eq!(turns[0].calls[0].steers,vec!["new question"]);
        assert_eq!(turns[0].calls[1].steers,vec!["new question","later fact"]);
        assert_eq!(turns[0].input,"initial");
        let mut history = vec![];
        history_items(&turns[0],0,&mut history);
        let history = serde_json::to_string(&history).unwrap();
        assert!(history.find("new question").unwrap() < history.find("first answer").unwrap());
        assert!(history.find("first answer").unwrap() < history.find("later fact").unwrap());
    }

    #[test]
    fn with_nothing_audited_the_set_is_the_newest_turns_that_spoke() {
        let t = |ts: &str, said: Option<&str>| Turn {
            file: "f".into(),
            thread: "th".into(),
            ts: ts.parse().unwrap(),
            input: String::new(),
            calls: said
                .map(|s| vec![Call {
                    tool: "hi_say".into(),
                    arguments: json!({ "text": s }),
                    result: "sent".into(),
                    reasoning: String::new(),
                    steers: vec![],
                }])
                .unwrap_or_default(),
            typed: String::new(),
            delivered: Vec::new(),
        };
        let turns = vec![
            t("2026-09-15T01:00:00Z", Some("一")),
            t("2026-09-15T02:00:00Z", None),
            t("2026-09-15T03:00:00Z", Some("三")),
        ];
        let chosen = choose(&turns, &[], 5);
        assert_eq!(chosen.iter().map(|c| c.0).collect::<Vec<_>>(), vec![2, 0]);
        assert!(chosen.iter().all(|c| c.1 == Why::Recent));
    }
}
