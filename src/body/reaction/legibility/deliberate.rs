//! A separate evidence-and-purpose read. It rejects a draft; it never writes the reply.

use std::{path::Path, time::Duration};
use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use serde_json::json;
use super::check::Brief;
use crate::body::legibility::judge::{self, Judge};
use crate::mind::memory::quality;
use super::verification::{self, Request};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Decision {
    pub accept: bool,
    pub axis: String,
    pub reason: String,
    pub instruction: String,
    pub notice: String,
    pub evidence: String,
    pub claims: Vec<Claim>,
    #[serde(default)]
    pub verify: Option<String>,
    #[serde(default)]
    pub answers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Claim {
    #[serde(default)]
    pub id: Option<usize>,
    pub claim: String,
    pub quote: String,
    pub source: String,
}

#[derive(Deserialize)]
struct Inventory { claims: Vec<String> }

pub(super) async fn read(dir: &Path, brief: &Brief, sent: &[String], shown: &[String], text: &str, pending: &[Request])
    -> anyhow::Result<(Decision, String, quality::Cost)>
{
    let model = Judge::resolve(dir, "speech_review_model").context("no deliberate speech reviewer configured")?;
    read_with(&model, brief, sent, shown, text, pending).await
}

async fn read_with(model: &Judge, brief: &Brief, sent: &[String], shown: &[String], text: &str, pending: &[Request])
    -> anyhow::Result<(Decision, String, quality::Cost)>
{
    // Inventory is separate from the purpose verdict: omitting a claim from the verdict
    // must not turn an unsupported explanation into an apparently harmless acknowledgment.
    let inventory_raw = model.respond(&json!({
        "instructions":"Extract every asserted factual claim about this project's actual implementation, causes, test results or limitations. \
            Return JSON {\"claims\":[\"exact span of the draft\"]}. Include claims even in short replies, apologies and corrections. \
            Exclude general knowledge, ordinary acknowledgments (including a simple completion acknowledgment), promises, \
            honest statements that an answer has not been established, and a past claim explicitly being withdrawn. \
            Do not decide whether the claims are true and do not rewrite. An explanation of what software did is not exempt just because it explains a word.",
        "input":[{"role":"user","content":[{"type":"input_text","text":text}]}],
        "reasoning":{"effort":"high"}, "max_output_tokens":4000,
        "text":{"format":{"type":"json_object"}}, "store":false,
    }), Duration::from_secs(180)).await?;
    let inventory: Inventory = judge::json_object(&judge::output_text(&inventory_raw).context("claim inventory returned no text")?)
        .context("invalid claim inventory")?;
    anyhow::ensure!(inventory.claims.iter().all(|claim| !claim.trim().is_empty() && text.contains(claim)), "claim inventory is not grounded in the draft");
    let case = brief.case(sent, shown, &format!("{}\n## Proposed message\n{text}", verification::render(pending)));
    let raw = model.respond(&json!({
        "instructions": include_str!("../../../identity/judges/speech-purpose.md"),
        "input": [{"role":"user","content":[{"type":"input_text","text":format!("{case}\n## Mandatory claim inventory\n{}\nReturn one claims entry for EACH immutable id. Do not split, merge, replace or omit inventory claims. The host supplies the claim text from its id.",
            serde_json::to_string(&inventory.claims.iter().enumerate().map(|(id, claim)| json!({"id":id,"claim":claim})).collect::<Vec<_>>())?)}]}],
        "reasoning":{"effort":"high"},"max_output_tokens":16000,
        "text":{"format":{"type":"json_object"}},"store":false,
    }), Duration::from_secs(180)).await?;
    let mut cost = judge::cost(&raw);
    let inventory_cost = judge::cost(&inventory_raw);
    cost.input += inventory_cost.input; cost.cached += inventory_cost.cached;
    cost.output += inventory_cost.output; cost.thinking += inventory_cost.thinking;
    let response = judge::output_text(&raw).with_context(|| format!(
        "speech purpose reader returned no text ({} output tokens, {} thinking)",cost.output,cost.thinking))?;
    let mut decision: Decision = judge::json_object(&response).context("invalid speech purpose decision")?;
    anyhow::ensure!(["none", "completion", "correction"].contains(&decision.notice.as_str()), "invalid notice kind");
    anyhow::ensure!(decision.accept || quality::axis(Some(&decision.axis)).is_some(), "invalid rejection axis");
    // The independent inventory defines which factual claims need tool provenance.
    // The purpose reader cannot invent a mechanism claim for an ordinary "done" reply.
    for claim in &mut decision.claims {
        if let Some(text) = claim.id.and_then(|id| inventory.claims.get(id)) {
            claim.claim = text.clone();
        }
    }
    decision.claims.retain(|claim| inventory.claims.contains(&claim.claim));
    if decision.claims.iter().any(|claim| source_output(pending, &claim.source)
        .is_some_and(|output| !output.contains(&claim.quote))) {
        // Repair a malformed citation against an already available source, not the reply
        // or its conclusion. Missing sources still start actual verification below.
        let repair_raw = model.respond(&json!({
            "instructions":"Repair source quotations only. Return JSON {\"claims\":[{\"claim\":\"unchanged exact claim\",\"source\":\"unchanged source id\",\"quote\":\"exact contiguous supporting excerpt from that source output\"}]}. \
                Never use ellipses, paraphrases or whitespace changes inside a quote. If that source does not support the claim, use an empty quote. \
                Do not add claims or change conclusions. A correct source id does not prove that it supports the claim.",
            "input":[{"role":"user","content":[{"type":"input_text","text":format!("Claims:\n{}\n{}", serde_json::to_string(&decision.claims)?, verification::render(pending))}]}],
            "reasoning":{"effort":"high"},"max_output_tokens":4000,
            "text":{"format":{"type":"json_object"}},"store":false,
        }), Duration::from_secs(180)).await?;
        #[derive(Deserialize)]
        struct Repair { claims: Vec<Claim> }
        let repair: Repair = judge::json_object(&judge::output_text(&repair_raw).context("citation repair returned no text")?)
            .context("invalid citation repair")?;
        for claim in &mut decision.claims {
            if let Some(fixed) = repair.claims.iter().find(|fixed| fixed.claim == claim.claim && fixed.source == claim.source) {
                claim.quote = fixed.quote.clone();
            }
        }
        let repair_cost = judge::cost(&repair_raw);
        cost.input += repair_cost.input; cost.cached += repair_cost.cached;
        cost.output += repair_cost.output; cost.thinking += repair_cost.thinking;
    }
    if !inventory.claims.iter().all(|claim| decision.claims.iter().any(|c| &c.claim == claim))
        || !claims_supported(&decision.claims, pending) {
        decision.accept = false;
        decision.axis = "unsupported".into();
        decision.reason = "The mechanism claim lacks a quote from an actual source tool result.".into();
        decision.instruction = "Hold this explanation for the source check; do not invent a replacement or assume the opposite is true.".into();
    }
    if !decision.accept && decision.axis == "unsupported" && decision.verify.as_ref().is_none_or(|q| q.trim().is_empty()) {
        decision.verify = Some(format!("Check the source for this proposed explanation, including contrary evidence and scope: {text}"));
    }
    anyhow::ensure!(!decision.accept || decision.verify.is_none(), "a draft needing verification cannot pass");
    anyhow::ensure!(decision.answers.iter().all(|id| pending.iter().any(|r| &r.id == id && r.report.is_some())),
        "review tried to answer an unknown or unchecked question");
    // A model label alone cannot make a draft un-droppable: the current report must
    // actually contain the evidence, and the accepted text must still be the same text.
    if !valid_notice(&decision, brief) {
        decision.notice = "none".into();
    }
    Ok((decision, model.model().to_string(), cost))
}

fn claims_supported(claims: &[Claim], pending: &[Request]) -> bool {
    claims.iter().all(|claim| claim.quote.chars().count() >= 6 && source_output(pending, &claim.source)
        .is_some_and(|output| output.contains(&claim.quote)))
}

fn source_output<'a>(pending: &'a [Request], source: &str) -> Option<&'a str> {
    pending.iter()
        .filter_map(|r| r.report.as_ref()).flat_map(|r| &r.observations)
        .find(|o| o.source == source).map(|o| o.output.as_str())
}

fn valid_notice(decision: &Decision, brief: &Brief) -> bool {
    decision.accept && brief.carries_report && decision.notice != "none"
        && decision.evidence.chars().count() >= 6
        && brief.signals.contains(&decision.evidence)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "uses the real reviewer; make eval-first-replies"]
    async fn first_reply_source_checks_live() {
        let dir = std::path::PathBuf::from(std::env::var("HI_AGENT_EVAL_DATA").expect("DATA is required"));
        let out = std::path::PathBuf::from(std::env::var("HI_AGENT_EVAL_OUT").expect("OUT is required"));
        crate::foundation::config::tunables::init(&dir);
        let mut probes = Vec::new();
        for id in ["02", "18", "20", "21"] {
            let record: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join(format!(
                "output/reply-replay-20261015/model-replay/case-{id}.json"))).unwrap()).unwrap();
            let brief = Brief { recent:record["result"]["judge_context"].as_str().unwrap().into(), ..Brief::default() };
            probes.push((format!("historical-{id}"), brief, record["case"]["original"].as_str().unwrap().to_string(), Vec::new(), false));
        }
        probes.push(("completion".into(), Brief { signals:"请求的页面布局修改已经完成并核对。".into(), carries_report:true,
            recent:"> 把左栏和视频的位置调好。".into(), ..Brief::default() }, "改好了。".into(), Vec::new(), true));

        let reference = super::super::verification::Reference { run:"69f1ebe6deac".into(), session:"cognition".parse().unwrap(),
            item_id:"call_00_ItM5GtdF6DRP6Q41HdzC1074".into(), quote:"for a1, a2 in itertools.permutations(XS, 2):".into() };
        let observation = super::super::verification::resolve(&dir, &reference).await.unwrap();
        let pending = vec![Request { id:"supported-check".into(), turn:"probe".into(), created_at:chrono::Utc::now(),
            context:String::new(), question:"identsearch.py 是否枚举线条归属？".into(), rejected:String::new(),
            dispatched_run:None, approval:None, report:Some(super::super::verification::Report {
                conclusion:"脚本枚举线条归属".into(), observations:vec![observation] }) }];
        probes.push(("supported-but-challenged".into(), Brief { recent:"< identsearch.py 会枚举线条的归属。".into(),
            signals:"> 不对吧？这应该是人标的，你确定吗？".into(), ..Brief::default() },
            "我确认的是这个脚本怎么选归属：它逐个尝试组合、打分，再把结果排序。最初四条线怎么来的，这次没查。".into(), pending, true));

        let probes: Vec<_> = (1..=2).flat_map(|round| probes.iter().cloned().map(move |(id, brief, text, pending, expected)|
            (format!("{id}-round-{round}"), brief, text, pending, expected))).collect();

        use futures::{StreamExt, stream};
        let results: Vec<_> = stream::iter(probes).map(|(id, brief, text, pending, expected)| {
            let dir = dir.clone();
            async move {
                let started = std::time::Instant::now();
                let result = read(&dir, &brief, &[], &[], &text, &pending).await;
                match result {
                    Ok((decision, model, cost)) => json!({"id":id,"draft":text,"expected_accept":expected,"matched":decision.accept == expected,
                        "decision":decision,"model":model,"cost":cost,"elapsed_ms":started.elapsed().as_millis()}),
                    Err(error) => json!({"id":id,"draft":text,"expected_accept":expected,"matched":false,"error":format!("{error:#}")}),
                }
            }
        }).buffer_unordered(3).collect().await;
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("results.json"), serde_json::to_vec_pretty(&results).unwrap()).unwrap();
        for result in &results { eprintln!("{}", result); }
        assert!(results.iter().all(|r| r["matched"] == true), "first-reply check failed; inspect results.json, not just rejection counts");
    }

    async fn model(replies: Vec<serde_json::Value>) -> (Judge, tokio::task::JoinHandle<()>) {
        use std::{collections::VecDeque, sync::Arc};
        let replies = Arc::new(tokio::sync::Mutex::new(VecDeque::from(replies)));
        let app = axum::Router::new().route("/responses", axum::routing::post({
            let replies = replies.clone();
            move || {
                let replies = replies.clone();
                async move {
                    let answer = replies.lock().await.pop_front().expect("unexpected model request");
                    axum::Json(json!({"output":[{"type":"message","content":[{"type":"output_text","text":answer.to_string()}]}]}))
                }
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let judge = Judge::new(&format!("http://{}", listener.local_addr().unwrap()), "test", "test");
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        (judge, task)
    }

    fn pass() -> serde_json::Value {
        json!({"accept":true,"axis":"pass","reason":"answer","instruction":"","notice":"none","evidence":"","claims":[],"verify":null,"answers":[]})
    }

    #[tokio::test]
    async fn an_empty_claim_list_cannot_hide_a_first_reply_explanation() {
        let draft = "A person clicked the court lines.";
        let (judge, task) = model(vec![json!({"claims":[draft]}), pass()]).await;
        let (decision, _, _) = read_with(&judge, &Brief::default(), &[], &[], draft, &[]).await.unwrap();
        task.abort();
        assert!(!decision.accept);
        assert_eq!(decision.axis, "unsupported");
        assert!(decision.verify.unwrap().contains(draft));
    }

    #[tokio::test]
    async fn a_confident_background_assertion_is_not_a_source_observation() {
        let draft = "A person clicked the court lines.";
        let mut verdict = pass();
        verdict["claims"] = json!([{"claim":draft,"source":"background","quote":"I checked first hand: a person clicked the court lines."}]);
        let (judge, task) = model(vec![json!({"claims":[draft]}), verdict]).await;
        let brief = Brief { signals:"I checked first hand: a person clicked the court lines.".into(), carries_report:true, ..Brief::default() };
        let (decision, _, _) = read_with(&judge, &brief, &[], &[], draft, &[]).await.unwrap();
        task.abort();
        assert!(!decision.accept);
        assert!(decision.verify.is_some());
    }

    #[tokio::test]
    async fn a_supported_answer_is_not_reversed_just_because_it_is_challenged() {
        let draft = "The script enumerates assignments.";
        let pending = vec![Request { id:"q".into(), turn:"t".into(), created_at:chrono::Utc::now(), context:String::new(),
            question:"Was this manual?".into(), rejected:String::new(), dispatched_run:None, approval:None,
            report:Some(super::super::verification::Report { conclusion:draft.into(), observations:vec![
                super::super::verification::Observation { source:"run/cognition/read".into(), operation:"cat identsearch.py".into(), output:draft.into() } ] }) }];
        let mut verdict = pass();
        verdict["claims"] = json!([{"id":0,"claim":"shortened by reviewer","source":"run/cognition/read","quote":draft}]);
        verdict["answers"] = json!(["q"]);
        let (judge, task) = model(vec![json!({"claims":[draft]}), verdict]).await;
        let brief = Brief { signals:"Are you sure? It must be manual!".into(), ..Brief::default() };
        let (decision, _, _) = read_with(&judge, &brief, &[], &[], draft, &pending).await.unwrap();
        task.abort();
        assert!(decision.accept);
        assert!(decision.verify.is_none());
        assert_eq!(decision.answers, ["q"]);
    }

    #[tokio::test]
    async fn completion_does_not_require_inventing_a_technical_explanation() {
        let mut verdict = pass();
        verdict["notice"] = json!("completion");
        verdict["evidence"] = json!("requested page change is complete");
        let (judge, task) = model(vec![json!({"claims":[]}), verdict]).await;
        let brief = Brief { carries_report:true, signals:"requested page change is complete".into(), ..Brief::default() };
        let (decision, _, _) = read_with(&judge, &brief, &[], &[], "改好了。", &[]).await.unwrap();
        task.abort();
        assert!(decision.accept);
        assert_eq!(decision.notice, "completion");
    }

    #[tokio::test]
    async fn a_broken_claim_inventory_is_not_an_approval() {
        let (judge, task) = model(vec![json!({"missing":"claims"})]).await;
        assert!(read_with(&judge, &Brief::default(), &[], &[], "a person clicked it", &[]).await.is_err());
        task.abort();
    }
    #[test]
    fn a_report_assertion_or_invented_quote_cannot_establish_a_mechanism() {
        let mut claim = Claim { id:None, claim:"the score is used".into(), quote:"candidate_score() is called before linking".into(), source:"run/cognition/read".into() };
        assert!(!claims_supported(std::slice::from_ref(&claim), &[]), "a quote from prose is not source evidence");
        let pending = vec![Request { id:"q".into(), turn:"t".into(), created_at:chrono::Utc::now(), context:String::new(),
            question:String::new(), rejected:String::new(), dispatched_run:None, approval:None,
            report:Some(super::super::verification::Report { conclusion:"the score is used".into(), observations:vec![
                super::super::verification::Observation { source:claim.source.clone(), operation:"cat scoring.py".into(), output:claim.quote.clone() } ] }) }];
        assert!(claims_supported(std::slice::from_ref(&claim), &pending));
        claim.source="invented/read".into();
        assert!(!claims_supported(std::slice::from_ref(&claim), &pending));
        claim.source="run/cognition/read".into(); claim.quote="the code never uses scoring".into();
        assert!(!claims_supported(std::slice::from_ref(&claim), &pending));
        claim.quote.clear();
        assert!(!claims_supported(&[claim], &pending));
        assert!(claims_supported(&[], &pending));
    }
    #[test]
    fn only_an_accepted_notice_grounded_in_the_current_report_is_required() {
        let brief = Brief { carries_report:true, signals:"requested page change is complete and verified".into(), ..Default::default() };
        let mut d = Decision { accept:true, axis:"pass".into(), reason:String::new(), instruction:String::new(), notice:"completion".into(), evidence:"page change is complete and verified".into(), claims:vec![], verify:None, answers:vec![] };
        assert!(valid_notice(&d, &brief));
        d.accept=false; assert!(!valid_notice(&d,&brief));
        d.accept=true; d.evidence="a previous assistant said it was finished".into(); assert!(!valid_notice(&d,&brief));
        d.evidence=brief.signals.clone(); assert!(!valid_notice(&d,&Brief { carries_report:false, ..brief }));
    }
}
