//! Exact historical cases through the production checks, with side effects disabled.
//! This is an eligibility replay, not proof of delivery or of background work executing.

use super::*;
use anyhow::Context as _;
use serde::Deserialize;
use super::super::check::{Review, Speech};

#[derive(Clone, Deserialize, Serialize)]
struct Case {
    id: String,
    title: String,
    original: String,
    #[serde(default)]
    user: Vec<String>,
    #[serde(default)]
    context: String,
    #[serde(default)]
    sources: Vec<String>,
    #[serde(default)]
    needle: String,
    #[serde(default)]
    match_input: String,
    #[serde(default)]
    after: Option<DateTime<Utc>>,
}

fn matches(case: &Case, turn: &Turn) -> bool {
    if case.after.is_some_and(|after| turn.ts < after) {
        return false;
    }
    target_call(case,turn).is_some()
}

fn target_call<'a>(case: &Case, turn: &'a Turn) -> Option<&'a Call> {
    turn.calls.iter().find(|call| {
        if call.tool != "hi_prepare" && call.tool != "hi_say" { return false; }
        if !case.match_input.is_empty() {
            return turn.input.contains(&case.match_input) || call.steers.iter().any(|s| s.contains(&case.match_input));
        }
        let needle = if case.needle.is_empty() { &case.original } else { &case.needle };
        if call.tool == "hi_say" { return call.arguments["text"].as_str().is_some_and(|s| s.contains(needle)); }
        finished(&call.arguments).0.iter().any(|s| s.contains(needle))
    })
}

pub(super) async fn run(opts: Options) -> anyhow::Result<()> {
    let path = opts.cases.as_ref().context("review cases missing")?;
    let cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let out = path.parent().context("cases need a parent directory")?.join("model-replay");
    std::fs::create_dir_all(&out)?;
    crate::body::capabilities::init(&crate::foundation::credentials::Credentials::try_load(&opts.data_dir)?)?;
    let model = Judge::resolve(&opts.data_dir, "speech_replay_model").context("no replay model configured")?;
    let model = opts.model.as_ref().map(|m| model.with_model(m)).unwrap_or(model);
    let prompt = match &opts.prompt {
        Some(path) => std::fs::read_to_string(path)?,
        None => crate::identity::reaction_prompt_as_built(),
    };
    eprintln!("Reading historical frame logs; later corrections are not added to model input.");
    let turns = read_turns(&opts.data_dir);
    let results: Vec<Value> = futures::stream::iter(cases.into_iter().take(opts.limit))
        .map(|case| {
            let (turns, model, prompt, opts, out) = (&turns, &model, &prompt, &opts, &out);
            async move {
                eprintln!("replaying {}: {}", case.id, case.title);
                let started = std::time::Instant::now();
                let result = execute(&case, turns, model, prompt, opts, out).await;
                let row = match result {
                    Ok(value) => json!({"case":case,"result":value,"elapsed_ms":started.elapsed().as_millis()}),
                    Err(error) => json!({"case":case,"error":format!("{error:#}"),"elapsed_ms":started.elapsed().as_millis()}),
                };
                std::fs::write(out.join(format!("{}.json",case.id)), serde_json::to_string_pretty(&row).unwrap()).unwrap();
                eprintln!("finished {} in {} s", case.id, started.elapsed().as_secs());
                row
            }
        }).buffer_unordered(3).collect().await;
    let report = json!({"model":model.model(),"generated_at":Utc::now(),"mode":"historical_input_with_production_checks",
        "limits":["No background worker, file operation, screen update or live chat message is executed.",
            "A timing pass means eligible in replay, not delivered to a live user.",
            "Only historical inputs and earlier thread turns are used; no manual rewrites are fed to the model."],
        "cases":results});
    let target = out.join("results.json");
    std::fs::write(&target, serde_json::to_string_pretty(&report)?)?;
    eprintln!("report: {}",target.display());
    Ok(())
}

async fn execute(case: &Case, turns: &[Turn], model: &Judge, prompt: &str, opts: &Options, out: &Path) -> anyhow::Result<Value> {
    let turn = turns.iter().find(|t| matches(case,t)).context("no matching historical turn")?;
    let thread: Vec<&Turn> = turns.iter().filter(|t| t.file == turn.file && t.thread == turn.thread).collect();
    let at = thread.iter().position(|t| std::ptr::eq(*t,turn)).unwrap();
    let from = at.saturating_sub(opts.context);
    let mut input = Vec::new();
    if from > 0 { history_items(thread[0],0,&mut input); }
    for (n,t) in thread[from..at].iter().enumerate() { history_items(t,n+1,&mut input); }
    input.push(user(&turn.input));
    let target = target_call(case,turn).unwrap();
    for steer in &target.steers { input.push(user(steer)); }
    let mut brief = brief_for(&thread,at,opts.context);
    for steer in &target.steers { brief.signals.push_str(&format!("\n\n{steer}")); }
    let conversation = brief.case(&[],&[],"");
    let records = out.join(&case.id);
    let speech = Speech::for_replay(opts.data_dir.clone(),records.clone());
    let turn_key = speech.begin(brief).await;
    let mut attempts = Vec::new();
    let mut accepted = Vec::new();
    let mut requests = Vec::new();
    let mut calls = Vec::new();
    let mut cost = quality::Cost::default();
    let mut generation_error = None;
    let tools = tools();
    for round in 0..MAX_ROUNDS {
        let body = json!({"instructions":prompt,"input":input,"tools":tools,"store":false});
        let raw = match model.respond(&body,ROUND_LIMIT).await {
            Ok(raw) => raw,
            Err(error) => { generation_error = Some(format!("{error:#}")); break; }
        };
        std::fs::create_dir_all(&records)?;
        std::fs::write(records.join(format!("round-{round}.json")),serde_json::to_string_pretty(&raw)?)?;
        let used = crate::body::legibility::judge::cost(&raw);
        cost.input += used.input; cost.output += used.output; cost.thinking += used.thinking;
        let output = raw["output"].as_array().cloned().unwrap_or_default();
        let functions: Vec<_> = output.iter().filter(|i| i["type"] == "function_call").cloned().collect();
        if functions.is_empty() { break; }
        // Keep parallel calls in one assistant turn. Interleaving their results creates
        // a second thinking-mode assistant turn without the reasoning that produced it.
        input.extend(continuation_items(&output));
        for (n,call) in functions.iter().enumerate() {
            let name = call["name"].as_str().unwrap_or_default();
            let args: Value = serde_json::from_str(call["arguments"].as_str().unwrap_or("{}"))?;
            let mut answer = "Replay only: no side effect was executed.".to_string();
            if name.ends_with("hi_prepare") {
                let (lines,_) = finished(&args);
                let mut rejected = Vec::new();
                for line in lines {
                    if line.chars().count() > super::super::super::tools::SAY_MAX_CHARS {
                        rejected.push("too long for one message".to_string());
                        attempts.push(json!({"text":line,"check":"too_long"}));
                        continue;
                    }
                    match speech.review(&line,tokio::time::Instant::now()).await {
                        Review::SendBack(note) => {
                            rejected.push(note.clone());
                            attempts.push(json!({"text":line,"check":"revise","note":note}));
                        }
                        Review::Verify { id, note } => {
                            rejected.push(format!("source verification needed: {note}. Do not rewrite the guess; no background check runs in this replay."));
                            requests.push(json!({"do":"verify","id":id,"note":note,"executed":false}));
                            attempts.push(json!({"text":line,"check":"needs_verification","note":note}));
                        }
                        Review::Pass => {
                            let speech_only = args["branches"].as_array().into_iter().flatten()
                                .filter(|branch| branch["when"] == "finished")
                                .flat_map(|branch| branch["actions"].as_array().into_iter().flatten())
                                .all(|action| action["do"] == "say");
                            let timing = super::super::super::prepared::replay_fit(&opts.data_dir,&conversation,&line,speech_only && speech.required_notice(&line)).await;
                            let eligible = timing["eligible"] == true;
                            attempts.push(json!({"text":line,"check":"pass_or_policy_bypass","timing":timing}));
                            if eligible { speech.note_sent(&line); accepted.push(line); }
                        }
                    }
                }
                for branch in args["branches"].as_array().into_iter().flatten() {
                    for action in branch["actions"].as_array().into_iter().flatten() {
                        if action["do"] == "send_message" { requests.push(action.clone()); }
                    }
                }
                answer = if rejected.is_empty() {
                    "prepared for replay; nothing was delivered and no background action was executed".into()
                } else { format!("not prepared — {}",rejected.join("; ")) };
            } else if name.ends_with("hi_send_message") {
                requests.push(args.clone());
                answer = "Replay: request recorded, but no worker ran and no lookup result is available. Do not claim it was checked or completed.".into();
            }
            calls.push(json!({"tool":name,"arguments":args,"result":answer}));
            let id = call["call_id"].as_str().map(str::to_string).unwrap_or(format!("r{round}_{n}"));
            input.push(json!({"type":"function_call_output","call_id":id,"output":answer}));
        }
        if !accepted.is_empty() { break; }
    }
    let checks: Vec<_> = quality::read_since(&records,DateTime::<Utc>::MIN_UTC).await.into_iter().filter(|record| {
        match record {
            quality::Record::Check(check) => check.turn == turn_key,
            quality::Record::Skipped(skip) => skip.turn == turn_key,
            _ => false,
        }
    }).collect();
    let row = json!({"turn_key":turn_key,"historical_ts":turn.ts,"frame_file":turn.file,"candidate_messages":accepted,"attempts":attempts,
        "requests_not_executed":requests,"calls":calls,"checks":checks,"generation_cost":cost,
        "historical_input":turn.input,"steers_before_reply":target.steers,"judge_context":conversation,
        "generation_error":generation_error,"purpose_reviews":speech.purpose_records()});
    speech.end();
    Ok(row)
}

fn continuation_items(output: &[Value]) -> Vec<Value> {
    output.iter().filter(|item| item["type"] == "reasoning" || item["type"] == "function_call")
        .map(|item| {
            let mut item = item.clone();
            if let Some(object) = item.as_object_mut() {
                object.remove("id");
                object.remove("status");
                // Some gateways return an opaque placeholder alongside real thinking.
                // Prefer the actual reasoning instead of sending conflicting forms.
                if object.get("content").and_then(Value::as_array).is_some_and(|c| !c.is_empty()) {
                    object.remove("encrypted_content");
                }
            }
            item
        }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parallel_calls_remain_in_the_same_thinking_turn() {
        let output = vec![
            json!({"type":"reasoning","id":"r","status":"completed","encrypted_content":"placeholder",
                "summary":[],"content":[{"type":"reasoning_text","text":"the actual thinking"}]}),
            json!({"type":"function_call","id":"a","call_id":"a","name":"prepare","arguments":"{}"}),
            json!({"type":"function_call","id":"b","call_id":"b","name":"check","arguments":"{}"}),
        ];
        let items = continuation_items(&output);
        assert_eq!(items.len(),3);
        assert_eq!(items[0]["content"][0]["text"],"the actual thinking");
        assert!(items[0].get("encrypted_content").is_none());
        assert!(items.iter().all(|i| i.get("id").is_none()));
        assert_eq!(items[1]["call_id"],"a");
        assert_eq!(items[2]["call_id"],"b");
    }
}
