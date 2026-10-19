//! Pending speech checks, not another work ledger. A rejected explanation remains owed
//! until an approved answer actually enters the conversation journal.

use std::path::{Path, PathBuf};

use anyhow::{Context, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::foundation::registry::{self, SessionSlug};
use crate::mind::memory::{Journal, layout};
use crate::types::{Content, JournalEntry};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub run: String,
    pub session: SessionSlug,
    pub item_id: String,
    pub quote: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub source: String,
    pub operation: String,
    pub output: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub conclusion: String,
    pub observations: Vec<Observation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub turn: String,
    pub created_at: DateTime<Utc>,
    pub context: String,
    pub question: String,
    pub rejected: String,
    pub report: Option<Report>,
    pub dispatched_run: Option<String>,
    pub approval: Option<Approval>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Approval {
    pub text: String,
    pub at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct Store {
    dir: PathBuf,
}

// Also serializes different handles opened by the MCP handler and recovery path.
static WRITE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

impl Store {
    pub fn new(dir: &Path) -> Self { Self { dir: dir.into() } }

    fn path(&self) -> PathBuf { layout::memory_dir(&self.dir).join("speech-verification.json") }

    async fn load(&self) -> anyhow::Result<Vec<Request>> {
        match tokio::fs::read(self.path()).await {
            Ok(bytes) => serde_json::from_slice(&bytes).context("unreadable pending speech checks"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }

    async fn save(&self, requests: &[Request]) -> anyhow::Result<()> {
        use tokio::io::AsyncWriteExt;
        let path = self.path();
        tokio::fs::create_dir_all(path.parent().unwrap()).await?;
        let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::now_v7()));
        let mut file = tokio::fs::File::create(&temporary).await?;
        file.write_all(&serde_json::to_vec_pretty(requests)?).await?;
        file.sync_all().await?;
        tokio::fs::rename(temporary, path).await?;
        Ok(())
    }

    pub async fn pending(&self) -> anyhow::Result<Vec<Request>> {
        let _guard = WRITE.lock().await;
        self.load().await
    }

    pub async fn request(&self, turn: &str, context: &str, question: &str, rejected: &str) -> anyhow::Result<String> {
        ensure!(!question.trim().is_empty(), "verification needs a specific question");
        let _guard = WRITE.lock().await;
        let mut requests = self.load().await?;
        if let Some(old) = requests.iter().find(|r| r.turn == turn && (r.question == question || r.report.is_none())) {
            return Ok(old.id.clone());
        }
        let id = uuid::Uuid::now_v7().to_string();
        requests.push(Request { id:id.clone(), turn:turn.into(), created_at:Utc::now(),
            context:context.into(), question:question.into(), rejected:rejected.into(),
            report:None, dispatched_run:None, approval:None });
        self.save(&requests).await?;
        Ok(id)
    }

    pub async fn report(&self, id: &str, conclusion: &str, references: &[Reference]) -> anyhow::Result<String> {
        ensure!(!conclusion.trim().is_empty(), "verification result is empty");
        ensure!(references.len() <= 12, "at most 12 source observations per report");
        let mut observations = Vec::new();
        for reference in references {
            observations.push(resolve(&self.dir, reference).await?);
        }
        ensure!(observations.iter().map(|o| o.output.chars().count()).sum::<usize>() <= 96_000,
            "source results exceed the review window; cite focused reads instead");
        let _guard = WRITE.lock().await;
        let mut requests = self.load().await?;
        let request = requests.iter_mut().find(|r| r.id == id).context("no pending speech check with this id")?;
        // An unsupported or failed check can report honestly with no observations. It is
        // still not evidence for the proposed explanation, nor an automatic completion.
        request.report = Some(Report { conclusion:conclusion.into(), observations });
        request.approval = None;
        request.dispatched_run = None;
        let text = render(std::slice::from_ref(request));
        self.save(&requests).await?;
        Ok(text)
    }

    pub async fn approve(&self, text: &str, ids: &[String]) -> anyhow::Result<()> {
        let _guard = WRITE.lock().await;
        let mut requests = self.load().await?;
        for id in ids {
            ensure!(requests.iter().any(|r| &r.id == id && r.report.is_some()), "cannot answer an unchecked request");
        }
        for request in &mut requests {
            if ids.contains(&request.id) {
                request.approval = Some(Approval { text:text.into(), at:Utc::now() });
            }
        }
        self.save(&requests).await
    }

    /// Called only after the complete message has been written to the actual text journal.
    pub async fn delivered(&self, text: &str) -> anyhow::Result<()> {
        let _guard = WRITE.lock().await;
        let mut requests = self.load().await?;
        let before = requests.len();
        requests.retain(|r| !r.approval.as_ref().is_some_and(|a| a.text == text));
        if requests.len() != before { self.save(&requests).await?; }
        Ok(())
    }

    /// Event-driven recovery: once at boot, then when another conversation event arrives.
    /// A mailbox delivery is not an answer. A restart must recover both missing checks
    /// and reports that reached Reaction but were never said.
    pub async fn dispatch(&self) -> anyhow::Result<()> {
        self.dispatch_using(registry::global(), &crate::foundation::run::id().to_string()).await
    }

    async fn dispatch_using(&self, registry: &registry::Registry, run: &str) -> anyhow::Result<()> {
        let _guard = WRITE.lock().await;
        let mut requests = self.load().await?;
        if requests.is_empty() { return Ok(()); }
        if let Some(since) = requests.iter().filter_map(|r| r.approval.as_ref().map(|a| a.at)).min() {
            let entries = Journal::open(self.dir.clone()).await?.recent(since, usize::MAX).await?;
            requests.retain(|r| !r.approval.as_ref().is_some_and(|a| entries.iter().any(|e| matches!(e,
                JournalEntry::Message { message, .. } if message.from.is_agent() && message.ts >= a.at
                    && matches!(&message.content, Content::Text(text) if text == &a.text)))));
        }
        for request in &mut requests {
            if request.dispatched_run.as_deref() == Some(run) { continue; }
            let (role, text) = if request.report.is_some() {
                (crate::identity::Role::Reaction, render(std::slice::from_ref(request)))
            } else {
                (crate::identity::Role::Cognition, instruction(request))
            };
            if let Some(target) = registry.session_of_role(role)
                && registry.post(&target.id, text) == registry::Delivery::Delivered {
                request.dispatched_run = Some(run.into());
            }
        }
        self.save(&requests).await
    }
}

pub fn render(requests: &[Request]) -> String {
    if requests.is_empty() { return String::new(); }
    let pending: Vec<_> = requests.iter().map(|r| serde_json::json!({
        "id":r.id, "question":r.question, "report":r.report,
    })).collect();
    format!("## Pending answers and source observations (host record)\n{}\n\
        These are still owed. Reports are claims, not proof. Only the source observations \
        below can support a mechanism. Preparation, review failure and 'I will check' do not \
        deliver an answer. Reconsider relevance against the person's latest message.\n",
        serde_json::to_string_pretty(&pending).unwrap_or_default())
}

fn instruction(request: &Request) -> String {
    format!("## Verify before answering — {}\n\
        The proposed explanation below was BLOCKED, NOT sent, and is NOT established. \
        Inspect the actual source needed to answer the question; do not rewrite the guess, \
        change a page to match it, or treat a prior assistant statement as evidence. \
        Check supporting AND contradicting material and the scope of any limitation. \
        Reply to reaction using hi_send_message with verification_id='{}'. Attach evidence \
        as [{{run,session,item_id,quote}}] referencing completed successful tool results in \
        memory/raw/sessions/<run>/<session>.jsonl. Use the actual tool call item id and an \
        exact output quote (not your conclusion). Current run: {}. If the source cannot be \
        checked, report that honestly with evidence=[], not a fabricated answer. This is \
        an outstanding answer, not a request to start a duplicate project.\n\n\
        Question: {}\n\nConversation at request:\n{}\n\nRejected draft (unverified):\n{}",
        request.id, request.id, crate::foundation::run::id(), request.question, request.context, request.rejected)
}

pub async fn resolve(dir: &Path, reference: &Reference) -> anyhow::Result<Observation> {
    ensure!(!reference.run.is_empty() && reference.run.len() <= 64
        && reference.run.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'), "invalid source run");
    ensure!(reference.quote.chars().count() >= 6, "source quote is too short");
    let path = layout::session_frames_path(dir, &reference.run, &reference.session);
    let reference = reference.clone();
    tokio::task::spawn_blocking(move || {
        use std::io::BufRead;
        let file = std::fs::File::open(path).context("source session log is unavailable")?;
        for line in std::io::BufReader::new(file).lines() {
            let line = line?;
            if !line.contains("item/completed") { continue; }
            let frame: Value = serde_json::from_str(&line)?;
            if frame["dir"] != "recv" || frame["method"] != "item/completed" { continue; }
            let raw: Value = serde_json::from_str(frame["raw"].as_str().unwrap_or(""))?;
            let item = &raw["params"]["item"];
            if item["id"].as_str() != Some(&reference.item_id) { continue; }
            let (operation, output) = observation(item)?;
            ensure!(output.contains(&reference.quote), "quote is not in the tool result");
            // Keep the surrounding result, not just the reporter's cherry-picked quote.
            ensure!(output.chars().count() <= 48_000, "source result is too large; read a focused source range and cite that result");
            return Ok(Observation { source:format!("{}/{}/{}", reference.run, reference.session, reference.item_id), operation, output });
        }
        anyhow::bail!("no completed tool result with that item id")
    }).await?
}

fn observation(item: &Value) -> anyhow::Result<(String, String)> {
    ensure!(item["status"] == "completed", "source tool did not complete successfully");
    match item["type"].as_str() {
        Some("commandExecution") => {
            ensure!(item["exitCode"] == 0, "source command failed");
            Ok((item["command"].as_str().context("source has no command")?.into(),
                item["aggregatedOutput"].as_str().context("source has no output")?.into()))
        }
        Some("mcpToolCall") => {
            let tool = item["tool"].as_str().context("source has no tool name")?;
            ensure!(!tool.starts_with("hi_") && item["error"].is_null()
                && item["result"]["isError"] != true, "messages and host receipts are not source observations");
            let text = item["result"]["content"].as_array().into_iter().flatten()
                .filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n");
            Ok((format!("{}/{} {}", item["server"], tool, item["arguments"]),
                if text.is_empty() { serde_json::to_string(&item["result"])? } else { text }))
        }
        _ => anyhow::bail!("assistant prose, user messages and file edits are not source observations"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn source(dir: &Path, id: &str, item: Value) -> Reference {
        let session: SessionSlug = "cognition".parse().unwrap();
        let path = layout::session_frames_path(dir, "test", &session);
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        let raw = json!({"params":{"item":item}}).to_string();
        tokio::fs::write(path, json!({"dir":"recv","method":"item/completed","raw":raw}).to_string()+"\n").await.unwrap();
        Reference { run:"test".into(), session, item_id:id.into(), quote:"enumerates assignments".into() }
    }

    #[tokio::test]
    async fn source_is_a_real_result_not_a_verified_label() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = source(dir.path(), "read", json!({"id":"read","type":"commandExecution","status":"completed",
            "exitCode":0,"command":"cat identsearch.py","aggregatedOutput":"The script enumerates assignments; no person clicked lines."})).await;
        assert!(resolve(dir.path(), &r).await.unwrap().output.contains("no person"));
        r.quote = "a person clicked all lines".into();
        assert!(resolve(dir.path(), &r).await.is_err());
        r.run = "../escape".into();
        assert!(resolve(dir.path(), &r).await.is_err());
        for item in [
            json!({"type":"agentMessage","status":"completed","text":"enumerates assignments"}),
            json!({"type":"commandExecution","status":"completed","exitCode":1,"command":"cat x","aggregatedOutput":"enumerates assignments"}),
            json!({"type":"mcpToolCall","status":"completed","tool":"hi_send_message","result":{"text":"enumerates assignments"}}),
        ] { assert!(observation(&item).is_err()); }
    }

    #[tokio::test]
    async fn requests_survive_rewrites_reports_and_restart_until_actual_delivery() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let id = store.request("turn", "who drew the lines?", "How was the mapping obtained?", "a person drew it").await.unwrap();
        assert_eq!(store.request("turn", "same", "How was the mapping obtained?", "a human drew it").await.unwrap(), id);
        assert_eq!(store.request("turn", "same", "Who picked those lines?", "someone picked it").await.unwrap(), id,
            "rephrasing the same turn's missing evidence must not start another check");
        store.delivered("I will check").await.unwrap();
        assert!(store.approve("fixed", std::slice::from_ref(&id)).await.is_err());
        store.report(&id, "could not read the source", &[]).await.unwrap();
        let reopened = Store::new(dir.path());
        assert_eq!(reopened.pending().await.unwrap().len(), 1);
        reopened.approve("I could not establish who drew it.", &[id]).await.unwrap();
        assert_eq!(reopened.pending().await.unwrap().len(), 1, "preparation is not delivery");
        reopened.delivered("I will check").await.unwrap();
        assert_eq!(reopened.pending().await.unwrap().len(), 1);
        reopened.delivered("I could not establish who drew it.").await.unwrap();
        assert!(reopened.pending().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_blocked_first_explanation_runs_a_check_and_recovers_an_unsent_result() {
        use crate::identity::Role;
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let registry = registry::Registry::new();
        let brain: SessionSlug = "cognition".parse().unwrap();
        let mouth: SessionSlug = "reaction".parse().unwrap();
        let id = store.request("first", "Who drew the lines?", "Was this manual or computed?", "a human drew them").await.unwrap();
        store.dispatch_using(&registry, "boot-a").await.unwrap();
        assert!(store.pending().await.unwrap()[0].dispatched_run.is_none(), "unavailable brain does not swallow the check");
        registry.register(brain.clone(), Role::Cognition, None, String::new(), None);
        registry.register(mouth.clone(), Role::Reaction, None, String::new(), None);
        store.dispatch_using(&registry, "boot-a").await.unwrap();
        let mail = registry.take_pending(&brain).unwrap();
        assert_eq!(mail.len(), 1);
        assert!(mail[0].text.contains("BLOCKED, NOT sent"));
        assert!(mail[0].text.contains(&id));
        store.dispatch_using(&registry, "boot-a").await.unwrap();
        assert!(registry.take_pending(&brain).is_none(), "another rewrite does not dispatch duplicate work");

        let source = source(dir.path(), "read-script", serde_json::json!({"id":"read-script","type":"commandExecution",
            "status":"completed","exitCode":0,"command":"cat identsearch.py", "aggregatedOutput":"The script enumerates assignments; no human annotation is required."})).await;
        store.report(&id, "the script computed the assignment", &[source]).await.unwrap();
        store.dispatch_using(&registry, "boot-a").await.unwrap();
        let mail = registry.take_pending(&mouth).unwrap();
        assert!(mail[0].text.contains("cat identsearch.py"));
        assert!(mail[0].text.contains("no human annotation"));
        assert_eq!(store.pending().await.unwrap().len(), 1, "receiving a report is not delivering an answer");
        Store::new(dir.path()).dispatch_using(&registry, "boot-b").await.unwrap();
        assert!(registry.take_pending(&mouth).is_some(), "restart recovers the answer even after its report was read");
        assert!(registry.take_pending(&brain).is_none(), "completed check is not restarted");

        store.approve("The script computed the lines.", &[id]).await.unwrap();
        let journal = Journal::open(dir.path().into()).await.unwrap();
        journal.append(crate::mind::memory::journal::legacy_signal_out(uuid::Uuid::now_v7().to_string(), Utc::now(),
            crate::types::Channel::Text, "The script computed the lines.".into(), None, None)).await.unwrap();
        Store::new(dir.path()).dispatch_using(&registry, "boot-c").await.unwrap();
        assert!(store.pending().await.unwrap().is_empty(), "crash after journal append does not redeliver the answer");
        assert!(registry.take_pending(&mouth).is_none());
    }
}
