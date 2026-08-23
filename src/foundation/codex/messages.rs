//! Folding a session's frame log back into the messages it is a record of.
//!
//! Not to be confused with `server::transcript`, which is the person↔agent conversation
//! (`docs/arch/text-transcript.md`). This is one *agent session's* own record — what a rung
//! or a worker did between `thread/start` and the end of its process.
//!
//! The log is written verbatim and stays that way. The rule in
//! `docs/arch/foundation.md#full-frames-not-modelled-events` is about **recording**, and
//! nothing here changes what is stored. This is the other
//! direction: reading it back for a person. Those are not the same job, and the frame log
//! is the wrong shape for the second one. One agent message arrives as an `item/started`,
//! six hundred `item/agentMessage/delta` frames and an `item/completed`; a shell command
//! arrives as a thousand `outputDelta`s. Rendered frame-per-row that is a scrolling wall
//! whose every line is a fragment. Measured over the twelve most recent logs on this
//! machine: **11,891 frames fold to 369 messages**, and the 369 are what happened.
//!
//! So: the record stays whole and uninterpreted on disk, and the *reader* folds. Nothing
//! folded here is stored, and a fold that gets something wrong is a display bug rather than
//! a lost record — which is the whole reason the verbatim log is worth keeping.
//!
//! **An item this build has never heard of still appears.** Codex's item vocabulary keeps
//! growing (`SessionUpdate::activity` says the same), so an unknown `type` becomes a message
//! carrying that type and the item verbatim, not a dropped row. Understanding is required to
//! render an item *well*, never to show that it happened.

use serde::Serialize;
use serde_json::{Map, Value};

/// One message, folded from every frame that belongs to it.
///
/// `seq`..`through` is the span it was folded from, which is what makes this reading and
/// the verbatim one line up: a row here names the frames a reader can go and check.
#[derive(Debug, Clone, Serialize)]
pub struct Message {
    /// What kind of thing happened, in this reader's vocabulary rather than the wire's —
    /// see [`kind_of`]. The wire's own word survives on an unknown item as `type`.
    pub kind: &'static str,
    /// 1-based turn ordinal. Not the protocol's `turnId`: a reader groups by "which turn
    /// of this session", and the opaque uuid answers a question nobody asked.
    pub turn: u64,
    /// The first frame this message was folded from, and the last.
    pub seq: u64,
    pub through: u64,
    /// When the first of those frames was recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<String>,
    /// The item's own status word, verbatim (`inProgress`, `completed`, `failed`). A
    /// message still `inProgress` at the end of a log is a session that stopped mid-item —
    /// the most useful row on the page, so it is not normalised away.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Kind-specific fields, flattened: `text` for a message, `command`/`output`/`exit`
    /// for a shell run, `server`/`tool`/`arguments`/`result` for a tool call.
    #[serde(flatten)]
    pub body: Map<String, Value>,
}

/// One turn of the session, from `turn/started` to `turn/completed`.
///
/// Kept beside the messages rather than as a message, because a turn is not something that
/// happened *in* the conversation — it is the bracket around a stretch of it, and a reader
/// draws it as a rule rather than a row.
#[derive(Debug, Clone, Serialize)]
pub struct Turn {
    pub n: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<String>,
    /// `inProgress` until a `turn/completed` says otherwise. A turn left `inProgress` in a
    /// finished log is where the session died.
    pub status: String,
    /// Whatever the turn failed with, verbatim. Absent on a turn that did not fail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
    /// Total tokens as of the last `thread/tokenUsage/updated` inside this turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
}

/// What [`fold`] read, and what it could not.
#[derive(Debug, Clone, Serialize)]
pub struct Folded {
    pub messages: Vec<Message>,
    pub turns: Vec<Turn>,
    /// Every line in the log, including the ones that folded into nothing.
    pub frames: usize,
    /// Lines that are not JSON, or whose `raw` is not. They stay in the record and are
    /// counted here rather than silently skipped: "this log has 4000 frames and 0 messages"
    /// must be readable as *this reader could not fold it*, not as *nothing happened*.
    pub unreadable: usize,
}

/// Fold a whole frame log — the file [`crate::mind::memory::layout::session_frames_path`]
/// names — into messages, oldest first.
pub fn fold(text: &str) -> Folded {
    let mut out = Folded { messages: Vec::new(), turns: Vec::new(), frames: 0, unreadable: 0 };
    // Item id → index into `messages`. An item's frames are not adjacent — deltas from one
    // interleave with another's started/completed — so folding is a lookup, not a run.
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    // The open stderr run, if the previous frame was stderr. Consecutive stderr lines are
    // one message: a Rust panic or a codex ERROR arrives as a paragraph of them, and a row
    // per line would bury the transcript under a stack trace.
    let mut stderr_run: Option<usize> = None;
    let mut turn: u64 = 0;

    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        out.frames += 1;
        let Ok(envelope) = serde_json::from_str::<Value>(line) else {
            out.unreadable += 1;
            continue;
        };
        let seq = envelope.get("seq").and_then(Value::as_u64).unwrap_or(0);
        let ts = envelope.get("ts").and_then(Value::as_str).map(str::to_string);
        let raw = envelope.get("raw").and_then(Value::as_str).unwrap_or_default();

        // stderr is not JSON-RPC — it is whatever the subprocess printed, and when a run
        // ends badly it is the only thing that says why.
        if envelope.get("dir").and_then(Value::as_str) == Some("stderr") {
            let clean = strip_ansi(raw);
            match stderr_run {
                Some(i) => {
                    let text = out.messages[i].body.entry("text").or_insert_with(|| "".into());
                    if let Some(s) = text.as_str() {
                        *text = format!("{s}\n{clean}").into();
                    }
                    out.messages[i].through = seq;
                }
                None => {
                    let mut body = Map::new();
                    body.insert("text".into(), clean.into());
                    out.messages.push(Message {
                        kind: "stderr",
                        turn,
                        seq,
                        through: seq,
                        ts,
                        status: None,
                        body,
                    });
                    stderr_run = Some(out.messages.len() - 1);
                }
            }
            continue;
        }
        stderr_run = None;

        let Ok(body) = serde_json::from_str::<Value>(raw) else {
            out.unreadable += 1;
            continue;
        };
        let params = body.get("params");
        let method = body.get("method").and_then(Value::as_str).unwrap_or_default();

        match method {
            "turn/started" => {
                turn += 1;
                out.turns.push(Turn {
                    n: turn,
                    started: ts,
                    ended: None,
                    status: "inProgress".into(),
                    error: None,
                    tokens: None,
                });
            }
            "turn/completed" | "turn/failed" => {
                // A `completed` for a turn we never saw start — the log begins mid-turn,
                // which a tail always can — still closes something rather than nothing.
                if out.turns.is_empty() {
                    turn += 1;
                    out.turns.push(Turn {
                        n: turn,
                        started: None,
                        ended: None,
                        status: "inProgress".into(),
                        error: None,
                        tokens: None,
                    });
                }
                let t = out.turns.last_mut().expect("just ensured non-empty");
                let obj = params.and_then(|p| p.get("turn"));
                t.ended = ts;
                t.status = obj
                    .and_then(|o| o.get("status"))
                    .and_then(Value::as_str)
                    .unwrap_or(if method == "turn/failed" { "failed" } else { "completed" })
                    .to_string();
                t.error = obj
                    .and_then(|o| o.get("error"))
                    .filter(|e| !e.is_null())
                    .or_else(|| params.and_then(|p| p.get("error")).filter(|e| !e.is_null()))
                    .cloned();
            }
            "thread/tokenUsage/updated" => {
                if let (Some(t), Some(n)) = (
                    out.turns.last_mut(),
                    params
                        .and_then(|p| p.get("tokenUsage"))
                        .and_then(|u| u.get("total"))
                        .and_then(|t| t.get("totalTokens"))
                        .and_then(Value::as_u64),
                ) {
                    t.tokens = Some(n);
                }
            }
            // Said by the agent's own runtime rather than by the agent — a permission
            // profile it could not honour, a capability it does not have. It changes what
            // the session could do, so it belongs in the reading of what it did.
            "warning" => {
                let text = params
                    .and_then(|p| p.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let mut b = Map::new();
                b.insert("text".into(), text.into());
                out.messages.push(Message {
                    kind: "warning",
                    turn,
                    seq,
                    through: seq,
                    ts,
                    status: None,
                    body: b,
                });
            }
            "item/started" | "item/updated" | "item/completed" => {
                let Some(item) = params.and_then(|p| p.get("item")) else { continue };
                // An item with no id cannot be folded onto later, so it is keyed by the
                // frame it arrived on — one message, never merged, rather than dropped.
                let key = item
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("seq:{seq}"));
                let at = match index.get(&key) {
                    Some(&i) => i,
                    None => {
                        out.messages.push(Message {
                            kind: kind_of(item),
                            turn,
                            seq,
                            through: seq,
                            ts,
                            status: None,
                            body: Map::new(),
                        });
                        index.insert(key, out.messages.len() - 1);
                        out.messages.len() - 1
                    }
                };
                absorb(&mut out.messages[at], item, seq);
            }
            // A delta is a fragment of an item that is still open. Once `item/completed`
            // has landed the item carries the whole text, so appending would double it —
            // hence `only_while_open`.
            "item/agentMessage/delta" => {
                append_delta(&mut out.messages, &index, params, "text");
            }
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                append_delta(&mut out.messages, &index, params, "text");
            }
            "item/commandExecution/outputDelta" => {
                append_delta(&mut out.messages, &index, params, "output");
            }
            // Everything else is protocol housekeeping — handshakes, rate limits, MCP
            // startup status, the responses to our own requests. It is in the log; it is
            // not something that happened in the conversation.
            _ => {}
        }
    }

    // A message with no text is not something a reader can read, and both kinds that arrive
    // that way arrive that way *constantly*:
    //
    // - **Reasoning**, always. Every reasoning item in the logs on this machine — 121 of 121
    //   — carries an empty `summary` and `content`, so it says only "it thought here".
    // - **Empty `final_answer` agent messages**, 95 of 342 in one session. Real, and not a
    //   fault: a rung that answers by calling `hi_say` produces a turn whose text answer is
    //   genuinely empty, `item/started` and `item/completed` both carrying `""`.
    //
    // Keeping them would have made two of every five rows blank. Dropping them loses
    // nothing, because a session that died *mid*-sentence is not this case — the deltas that
    // did arrive are already folded in, so that message has text and stays.
    out.messages.retain(|m| match m.kind {
        "thinking" | "agent" | "user" => m.body.contains_key("text"),
        _ => true,
    });
    out
}

/// Append a streaming fragment to the open item it belongs to.
fn append_delta(
    messages: &mut [Message],
    index: &std::collections::HashMap<String, usize>,
    params: Option<&Value>,
    field: &str,
) {
    let Some(params) = params else { return };
    let Some(id) = params.get("itemId").and_then(Value::as_str) else { return };
    let Some(delta) = params.get("delta").and_then(Value::as_str) else { return };
    let Some(&at) = index.get(id) else { return };
    let m = &mut messages[at];
    // The completed item already carries the whole of this field; a delta arriving after
    // it (or replayed by a re-read) must not be appended twice.
    if m.status.as_deref() == Some("completed") {
        return;
    }
    match m.body.get(field).and_then(Value::as_str) {
        Some(existing) => {
            let joined = format!("{existing}{delta}");
            m.body.insert(field.into(), joined.into());
        }
        None => {
            m.body.insert(field.into(), delta.into());
        }
    }
}

/// This reader's word for an item, from the wire's `type`.
///
/// Translated rather than passed through, because the wire names a payload and a reader
/// names an act: `commandExecution` is *it ran something*, `mcpToolCall` is *it used a
/// tool*. An unknown type keeps the wire's own word (see [`absorb`]).
///
/// Shared with the stats scan ([`crate::foundation::server::stats`]), which counts the
/// same items without folding their text. Two copies of this vocabulary would be free to
/// disagree about what happened.
pub fn kind_of(item: &Value) -> &'static str {
    match item.get("type").and_then(Value::as_str).unwrap_or_default() {
        "userMessage" => "user",
        "agentMessage" => "agent",
        "reasoning" => "thinking",
        "commandExecution" => "command",
        "fileChange" | "patchApply" => "edit",
        "mcpToolCall" => "tool",
        "webSearch" => "search",
        "todoList" => "todo",
        // Not the agent doing something — the runtime dropping the earlier part of the
        // conversation to fit the window. It explains a session that suddenly forgets, so
        // it is named rather than left as a bare `contextCompaction` row.
        "contextCompaction" => "compaction",
        _ => "item",
    }
}

/// Strip ANSI escapes — colour, cursor moves — from text meant to be read as text.
///
/// codex writes its own tracing output to stderr with colour on, so a real error line
/// arrives as `\x1b[2m2026-…\x1b[0m \x1b[31mERROR\x1b[0m …`. In a terminal that is red; in
/// the panel it is line noise around the message. The escapes stay in the frame log, which
/// is the record; this is the reading.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // CSI (`ESC [` … final byte @-~) covers colour and cursor control; anything else
        // after ESC is a two-character sequence, and dropping the pair is right for both.
        if chars.next() == Some('[') {
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        }
    }
    out
}

/// Fold one item frame into its message. Called for `started`, `updated` and `completed`,
/// each of which carries the whole item as known at that moment — so the last one wins and
/// a session that died mid-item keeps what it had.
fn absorb(m: &mut Message, item: &Value, seq: u64) {
    m.through = seq;
    let get = |k: &str| item.get(k).and_then(Value::as_str).map(str::to_string);
    if let Some(s) = get("status") {
        m.status = Some(s);
    }
    let mut set = |k: &str, v: Value| {
        if !v.is_null() {
            m.body.insert(k.into(), v);
        }
    };

    match item.get("type").and_then(Value::as_str).unwrap_or_default() {
        "userMessage" => set("text", content_text(item).into()),
        "agentMessage" => {
            if let Some(t) = get("text").filter(|t| !t.is_empty()) {
                set("text", t.into());
            }
            // `commentary` is the agent thinking out loud mid-turn; `final_answer` is what
            // it came back with. A reader that flattens them loses the difference between
            // an aside and an answer.
            set("phase", item.get("phase").cloned().unwrap_or(Value::Null));
        }
        "reasoning" => {
            let text = summary_text(item);
            if !text.is_empty() {
                set("text", text.into());
            }
        }
        "commandExecution" => {
            set("command", item.get("command").cloned().unwrap_or(Value::Null));
            set("cwd", item.get("cwd").cloned().unwrap_or(Value::Null));
            set("exit", item.get("exitCode").cloned().unwrap_or(Value::Null));
            set("ms", item.get("durationMs").cloned().unwrap_or(Value::Null));
            // The completed item carries the whole output; while it runs, the deltas are
            // all there is, so this must not clobber what they have accumulated.
            if let Some(o) = get("aggregatedOutput").filter(|o| !o.is_empty()) {
                set("output", o.into());
            }
        }
        "fileChange" | "patchApply" => {
            let changes = item.get("changes").and_then(Value::as_array);
            let paths: Vec<Value> = changes
                .map(|cs| cs.iter().filter_map(|c| c.get("path").cloned()).collect())
                .unwrap_or_default();
            set("paths", Value::Array(paths));
            let diff = changes
                .map(|cs| {
                    cs.iter()
                        .filter_map(|c| c.get("diff").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            if !diff.is_empty() {
                set("diff", diff.into());
            }
        }
        "mcpToolCall" => {
            set("server", item.get("server").cloned().unwrap_or(Value::Null));
            set("tool", item.get("tool").cloned().unwrap_or(Value::Null));
            set("arguments", item.get("arguments").cloned().unwrap_or(Value::Null));
            set("result", item.get("result").cloned().unwrap_or(Value::Null));
            set("error", item.get("error").cloned().unwrap_or(Value::Null));
            set("ms", item.get("durationMs").cloned().unwrap_or(Value::Null));
        }
        "webSearch" => set("query", item.get("query").cloned().unwrap_or(Value::Null)),
        // Not understood, so nothing is claimed about it: the wire's own word for what it
        // is, and the item exactly as it arrived.
        other => {
            set("type", other.into());
            set("item", item.clone());
        }
    }
}

/// `content: [{type: "text", text: …}]` joined — the shape a user message arrives in.
fn content_text(item: &Value) -> String {
    item.get("content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// A reasoning item's text, from either half it can arrive in. Both are arrays that hold
/// bare strings in some builds and `{text}` objects in others, so both are accepted.
fn summary_text(item: &Value) -> String {
    ["summary", "content"]
        .iter()
        .filter_map(|k| item.get(*k).and_then(Value::as_array))
        .flatten()
        .filter_map(|v| {
            v.as_str().map(str::to_string).or_else(|| {
                v.get("text").and_then(Value::as_str).map(str::to_string)
            })
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A frame log line as the tap writes it: an envelope whose `raw` is the wire line.
    ///
    /// A string body is written through as itself rather than as JSON, because that is what
    /// the tap does with stderr — it is not a JSON-RPC line, it is whatever the subprocess
    /// printed, and a helper that quoted it would be testing a frame the tap never writes.
    fn frame(seq: u64, dir: &str, body: Value) -> String {
        let raw = match body.as_str() {
            Some(s) => s.to_string(),
            None => body.to_string(),
        };
        json!({
            "seq": seq,
            "ts": "2026-08-12T05:36:42.990579Z",
            "conn": 0,
            "agent_session": 3,
            "role": "cognition",
            "dir": dir,
            "method": body.get("method").cloned(),
            "raw": raw,
        })
        .to_string()
    }

    fn log(lines: &[String]) -> String {
        format!("{}\n", lines.join("\n"))
    }

    /// The shape this whole module exists for: one message arriving as a stream of
    /// fragments comes back as one message, not as its fragments.
    #[test]
    fn a_streamed_message_folds_into_one() {
        let text = log(&[
            frame(1, "recv", json!({"method": "turn/started", "params": {"turn": {"id": "t1"}}})),
            frame(
                2,
                "recv",
                json!({"method": "item/started", "params": {"item": {
                    "type": "agentMessage", "id": "msg_1", "text": "", "phase": "final_answer"
                }}}),
            ),
            frame(
                3,
                "recv",
                json!({"method": "item/agentMessage/delta",
                       "params": {"itemId": "msg_1", "delta": "Bal"}}),
            ),
            frame(
                4,
                "recv",
                json!({"method": "item/agentMessage/delta",
                       "params": {"itemId": "msg_1", "delta": "ance is fine."}}),
            ),
            frame(
                5,
                "recv",
                json!({"method": "item/completed", "params": {"item": {
                    "type": "agentMessage", "id": "msg_1",
                    "text": "Balance is fine.", "phase": "final_answer", "status": "completed"
                }}}),
            ),
        ]);

        let out = fold(&text);
        assert_eq!(out.frames, 5);
        assert_eq!(out.messages.len(), 1, "five frames, one thing said");
        let m = &out.messages[0];
        assert_eq!(m.kind, "agent");
        assert_eq!(m.body["text"], "Balance is fine.");
        assert_eq!(m.body["phase"], "final_answer");
        assert_eq!(m.status.as_deref(), Some("completed"));
        // The span is how a reader gets from this row back to the verbatim frames.
        assert_eq!((m.seq, m.through), (2, 5));
        assert_eq!(m.turn, 1);
    }

    /// The completed item carries the whole text, so a delta that arrives (or is re-read)
    /// after it must not be appended a second time.
    #[test]
    fn a_delta_after_completion_does_not_double_the_text() {
        let text = log(&[
            frame(
                1,
                "recv",
                json!({"method": "item/started",
                       "params": {"item": {"type": "agentMessage", "id": "m", "text": ""}}}),
            ),
            frame(
                2,
                "recv",
                json!({"method": "item/completed", "params": {"item": {
                    "type": "agentMessage", "id": "m", "text": "done", "status": "completed"
                }}}),
            ),
            frame(
                3,
                "recv",
                json!({"method": "item/agentMessage/delta",
                       "params": {"itemId": "m", "delta": "done"}}),
            ),
        ]);
        assert_eq!(fold(&text).messages[0].body["text"], "done");
    }

    /// A command that is still running has no `aggregatedOutput` — the deltas are the only
    /// output there is, which is exactly the case a live reader is watching.
    #[test]
    fn a_running_command_keeps_the_output_its_deltas_carried() {
        let text = log(&[
            frame(
                1,
                "recv",
                json!({"method": "item/started", "params": {"item": {
                    "type": "commandExecution", "id": "exec-1",
                    "command": "cargo test", "cwd": "/w", "status": "inProgress"
                }}}),
            ),
            frame(
                2,
                "recv",
                json!({"method": "item/commandExecution/outputDelta",
                       "params": {"itemId": "exec-1", "delta": "running 9 tests\n"}}),
            ),
            frame(
                3,
                "recv",
                json!({"method": "item/commandExecution/outputDelta",
                       "params": {"itemId": "exec-1", "delta": "ok"}}),
            ),
        ]);
        let out = fold(&text);
        let m = &out.messages[0];
        assert_eq!(m.kind, "command");
        assert_eq!(m.body["command"], "cargo test");
        assert_eq!(m.body["output"], "running 9 tests\nok");
        assert_eq!(m.status.as_deref(), Some("inProgress"), "still running, and says so");
    }

    /// The tool call is what verification reads, so arguments and result survive whole.
    #[test]
    fn a_tool_call_keeps_its_arguments_and_result() {
        let text = log(&[frame(
            7,
            "recv",
            json!({"method": "item/completed", "params": {"item": {
                "type": "mcpToolCall", "id": "exec-9", "server": "hi-agent", "tool": "hi_send_message",
                "status": "completed", "arguments": {"to": "2", "message": "done"},
                "result": {"ok": true}, "durationMs": 12
            }}}),
        )]);
        let m = &fold(&text).messages[0];
        assert_eq!(m.kind, "tool");
        assert_eq!(m.body["server"], "hi-agent");
        assert_eq!(m.body["tool"], "hi_send_message");
        assert_eq!(m.body["arguments"]["message"], "done");
        assert_eq!(m.body["result"]["ok"], true);
    }

    /// An item type from a newer codex than this build still renders as something that
    /// happened. Understanding is for rendering it *well*, never for showing it at all.
    #[test]
    fn an_unknown_item_type_still_becomes_a_message() {
        let text = log(&[frame(
            3,
            "recv",
            json!({"method": "item/completed", "params": {"item": {
                "type": "somethingNewEntirely", "id": "x1", "status": "completed", "payload": 42
            }}}),
        )]);
        let m = &fold(&text).messages[0];
        assert_eq!(m.kind, "item");
        assert_eq!(m.body["type"], "somethingNewEntirely", "the wire's own word for it");
        assert_eq!(m.body["item"]["payload"], 42, "and the item, verbatim");
    }

    /// Protocol housekeeping is in the log and is not part of the conversation.
    #[test]
    fn handshakes_and_rate_limits_are_not_messages() {
        let text = log(&[
            frame(1, "send", json!({"id": 1, "method": "initialize", "params": {}})),
            frame(2, "recv", json!({"id": 1, "result": {"userAgent": "x"}})),
            frame(3, "recv", json!({"method": "account/rateLimits/updated", "params": {}})),
            frame(4, "recv", json!({"method": "mcpServer/startupStatus/updated", "params": {}})),
        ]);
        let out = fold(&text);
        assert_eq!(out.frames, 4);
        assert!(out.messages.is_empty());
        assert_eq!(out.unreadable, 0, "understood and skipped is not unreadable");
    }

    /// A turn that never completed is where the session died, and the reader has to be able
    /// to say so — same reason an `opened` with no `closed` is reported as lost.
    #[test]
    fn an_unfinished_turn_stays_in_progress_and_a_finished_one_carries_its_tokens() {
        let text = log(&[
            frame(1, "recv", json!({"method": "turn/started", "params": {"turn": {}}})),
            frame(
                2,
                "recv",
                json!({"method": "thread/tokenUsage/updated",
                       "params": {"tokenUsage": {"total": {"totalTokens": 22391}}}}),
            ),
            frame(
                3,
                "recv",
                json!({"method": "turn/completed",
                       "params": {"turn": {"status": "completed", "error": null}}}),
            ),
            frame(4, "recv", json!({"method": "turn/started", "params": {"turn": {}}})),
        ]);
        let out = fold(&text);
        assert_eq!(out.turns.len(), 2);
        assert_eq!(out.turns[0].status, "completed");
        assert_eq!(out.turns[0].tokens, Some(22391));
        assert_eq!(out.turns[1].status, "inProgress", "the one it died inside");
        assert!(out.turns[1].ended.is_none());
    }

    /// A failed turn keeps what it failed with. Nothing else in the log says why.
    #[test]
    fn a_failed_turn_keeps_its_error() {
        let text = log(&[
            frame(1, "recv", json!({"method": "turn/started", "params": {"turn": {}}})),
            frame(
                2,
                "recv",
                json!({"method": "turn/completed", "params": {"turn": {
                    "status": "failed", "error": {"message": "stream disconnected"}
                }}}),
            ),
        ]);
        let out = fold(&text);
        assert_eq!(out.turns[0].status, "failed");
        assert_eq!(out.turns[0].error.as_ref().unwrap()["message"], "stream disconnected");
    }

    /// A crash arrives as a paragraph of stderr lines. One message, not a row per line.
    #[test]
    fn consecutive_stderr_lines_are_one_message() {
        let text = log(&[
            frame(1, "stderr", json!("ERROR codex_core: thread panicked")),
            frame(2, "stderr", json!("  stack backtrace:")),
            frame(
                3,
                "recv",
                json!({"method": "item/completed",
                       "params": {"item": {"type": "agentMessage", "id": "m", "text": "hi"}}}),
            ),
            frame(4, "stderr", json!("and again, later")),
        ]);
        let out = fold(&text);
        let kinds: Vec<_> = out.messages.iter().map(|m| m.kind).collect();
        assert_eq!(kinds, vec!["stderr", "agent", "stderr"], "a run, then a break, then a run");
        assert_eq!((out.messages[0].seq, out.messages[0].through), (1, 2));
    }

    /// stderr is not JSON and the tap writes it as a bare string in `raw`. Folding it must
    /// not go through the JSON-RPC path — that is how it used to vanish.
    #[test]
    fn stderr_text_survives_verbatim() {
        let text = log(&[frame(1, "stderr", json!("not json at all"))]);
        let out = fold(&text);
        assert_eq!(out.messages[0].body["text"], "not json at all");
        assert_eq!(out.unreadable, 0);
    }

    /// codex colours its own tracing, so every real stderr line arrives wrapped in escapes.
    /// Read as text they are noise around the message; the record keeps them.
    #[test]
    fn stderr_colour_is_stripped_for_reading() {
        let text = log(&[frame(
            1,
            "stderr",
            json!("\u{1b}[2m2026-08-10T13:35:02Z\u{1b}[0m \u{1b}[31mERROR\u{1b}[0m router: boom"),
        )]);
        assert_eq!(fold(&text).messages[0].body["text"], "2026-08-10T13:35:02Z ERROR router: boom");
    }

    /// Compaction is the runtime dropping the earlier conversation to fit the window. It is
    /// the explanation for a session that suddenly forgets, so it is named.
    #[test]
    fn context_compaction_is_named_rather_than_left_as_an_unknown_item() {
        let text = log(&[frame(
            1,
            "recv",
            json!({"method": "item/completed",
                   "params": {"item": {"type": "contextCompaction", "id": "c1"}}}),
        )]);
        assert_eq!(fold(&text).messages[0].kind, "compaction");
    }

    /// A line this build cannot parse is counted, never quietly dropped: a log that folds
    /// to nothing must be distinguishable from a session where nothing happened.
    #[test]
    fn unreadable_lines_are_counted_rather_than_hidden() {
        let text = "not json at all\n\n{\"seq\":2,\"dir\":\"recv\",\"raw\":\"also not json\"}\n";
        let out = fold(text);
        assert_eq!(out.frames, 2, "blank lines are not frames");
        assert_eq!(out.unreadable, 2);
        assert!(out.messages.is_empty());
    }

    /// A rung that answers by calling `hi_say` completes its turn with an agent message whose
    /// text is genuinely `""`. Ninety-five of one session's three hundred messages were
    /// these, and a blank row per turn is worse than no row.
    #[test]
    fn an_empty_agent_message_is_not_a_row() {
        let text = log(&[
            frame(
                1,
                "recv",
                json!({"method": "item/completed", "params": {"item": {
                    "type": "agentMessage", "id": "m1", "text": "", "phase": "final_answer"
                }}}),
            ),
            frame(
                2,
                "recv",
                json!({"method": "item/completed", "params": {"item": {
                    "type": "mcpToolCall", "id": "e1", "server": "hi-agent", "tool": "hi_say"
                }}}),
            ),
        ]);
        let out = fold(&text);
        assert_eq!(out.messages.len(), 1, "the call it answered with, not a blank line");
        assert_eq!(out.messages[0].kind, "tool");
    }

    /// A session that died mid-sentence is *not* that case: the deltas that arrived are
    /// already folded in, so the message has text and stays.
    #[test]
    fn a_message_cut_off_mid_sentence_keeps_what_it_had() {
        let text = log(&[
            frame(
                1,
                "recv",
                json!({"method": "item/started",
                       "params": {"item": {"type": "agentMessage", "id": "m", "text": ""}}}),
            ),
            frame(
                2,
                "recv",
                json!({"method": "item/agentMessage/delta",
                       "params": {"itemId": "m", "delta": "I was saying"}}),
            ),
        ]);
        let out = fold(&text);
        assert_eq!(out.messages.len(), 1);
        assert_eq!(out.messages[0].body["text"], "I was saying");
    }

    /// An empty reasoning item says only "it thought here" — every reasoning item in the
    /// logs on this machine is empty, and a third of the transcript would have been blank
    /// rows. A summary that *does* arrive is kept.
    #[test]
    fn empty_thinking_is_dropped_and_thinking_with_a_summary_is_kept() {
        let empty = log(&[frame(
            1,
            "recv",
            json!({"method": "item/completed",
                   "params": {"item": {
                       "type": "reasoning", "id": "r1", "summary": [], "content": []
                   }}}),
        )]);
        assert!(fold(&empty).messages.is_empty());

        let spoken = log(&[frame(
            1,
            "recv",
            json!({"method": "item/completed", "params": {"item": {
                "type": "reasoning", "id": "r2", "summary": ["Checking the ledger first."]
            }}}),
        )]);
        let m = &fold(&spoken).messages[0];
        assert_eq!(m.kind, "thinking");
        assert_eq!(m.body["text"], "Checking the ledger first.");
    }

    /// The prompt is a message like any other, and it is the one a reader looks for first.
    #[test]
    fn the_prompt_is_the_first_message() {
        let text = log(&[frame(
            1,
            "recv",
            json!({"method": "item/completed", "params": {"item": {
                "type": "userMessage", "id": "u1",
                "content": [
                    {"type": "text", "text": "# Active tasks\n"},
                    {"type": "text", "text": "- [doing] x"}
                ]
            }}}),
        )]);
        let m = &fold(&text).messages[0];
        assert_eq!(m.kind, "user");
        assert_eq!(m.body["text"], "# Active tasks\n- [doing] x");
    }

    /// An edit names the files it touched without making a reader open the diff.
    #[test]
    fn a_file_change_names_its_paths_and_keeps_the_diff() {
        let text = log(&[frame(
            1,
            "recv",
            json!({"method": "item/completed", "params": {"item": {
                "type": "fileChange", "id": "f1", "status": "completed",
                "changes": [
                    {"path": "/w/a.md", "kind": {"type": "update"}, "diff": "@@ -1 +1 @@\n-a\n+b"},
                    {"path": "/w/b.md", "kind": {"type": "add"}, "diff": "@@ -0,0 +1 @@\n+new"}
                ]
            }}}),
        )]);
        let m = &fold(&text).messages[0];
        assert_eq!(m.kind, "edit");
        assert_eq!(m.body["paths"][0], "/w/a.md");
        assert_eq!(m.body["paths"][1], "/w/b.md");
        assert!(m.body["diff"].as_str().unwrap().contains("+new"));
    }

    /// Messages come back in wire order across turns, and each carries the turn it happened
    /// in — that is what lets a reader draw the turn as a rule instead of a row.
    #[test]
    fn messages_keep_wire_order_and_carry_their_turn() {
        let say = |seq: u64, id: &str, text: &str| {
            frame(
                seq,
                "recv",
                json!({"method": "item/completed", "params": {"item": {
                    "type": "agentMessage", "id": id, "text": text, "status": "completed"
                }}}),
            )
        };
        let text = log(&[
            frame(1, "recv", json!({"method": "turn/started", "params": {"turn": {}}})),
            say(2, "a", "first"),
            frame(3, "recv", json!({"method": "turn/completed", "params": {"turn": {}}})),
            frame(4, "recv", json!({"method": "turn/started", "params": {"turn": {}}})),
            say(5, "b", "second"),
        ]);
        let out = fold(&text);
        assert_eq!(out.messages.len(), 2);
        assert_eq!(out.messages[0].body["text"], "first");
        assert_eq!(out.messages[0].turn, 1);
        assert_eq!(out.messages[1].body["text"], "second");
        assert_eq!(out.messages[1].turn, 2);
    }
}
