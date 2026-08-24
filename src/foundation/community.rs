//! The community — the always-on shared infrastructure a single core cannot be
//! for itself.
//!
//! See [`docs/arch/topology.md`](../../docs/arch/topology.md). Four services,
//! deliberately independent; this is the client for them, and today it speaks to
//! one:
//!
//! - **registry** — handle ↔ core. Claim a name, keep it by checking in.
//! - *relay*, *broker*, *post* — the relay lands with the tunnel; the broker has
//!   its own client already ([`crate::foundation::broker`]) and shares no key
//!   with this one, which is invariant 2 in the shape of two modules.
//!
//! **Not [`crate::foundation::registry`]**, which is the process-wide session
//! switchboard and has nothing to do with any of this. The name collision is
//! unfortunate and the vocabulary is the doc's, so: `community::registry` is the
//! namespace of handles, `foundation::registry` is the namespace of sessions.
//!
//! ## A handle belongs to an account
//!
//! Not to this core, and not to this machine. A name bound to a core would be
//! lost with the laptop it was minted on — an ordinary event, and a worse
//! outcome than the squatting a lease would have guarded against. So claiming
//! presents the **account's** access token, the same one the broker issues, and
//! a new install signs in and takes its own name back with nothing to migrate
//! off the old disk.
//!
//! The line that survives: the community may require you to be **someone**; it
//! may never require you to be a **customer**. Nothing on this path reads a tier,
//! a balance or a payment.

use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

use crate::foundation::broker;

/// Where the community lives. Overridable for a test deployment; there is no
/// per-service address because the four services share one origin today.
pub const ENV_BASE_URL: &str = "HI_AGENT_COMMUNITY";
const DEFAULT_BASE_URL: &str = "https://hi-agent.xyz";

/// One name this account owns.
#[derive(Debug, Clone, Deserialize, serde::Serialize, Default)]
pub struct Handle {
    pub handle: String,
    #[serde(default)]
    pub display_name: String,
    /// Where the core at this name is reachable, as the community sees it. A
    /// core cannot work this out from the inside and needs it to say where it
    /// is — on a pairing QR, on an upload link.
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub claimed_at: String,
}

/// The names this account owns, and how many it may.
#[derive(Debug, Clone, Deserialize, serde::Serialize, Default)]
pub struct Handles {
    #[serde(default)]
    pub handles: Vec<Handle>,
    #[serde(default)]
    pub limit: usize,
}

#[derive(Deserialize)]
struct ApiError {
    #[serde(default)]
    message: String,
    #[serde(default)]
    error: String,
}

pub fn base_url() -> String {
    std::env::var(ENV_BASE_URL)
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

/// The account's access token — the one thing a claim presents.
///
/// Absent before the broker bootstrap has run, and the error says what to do
/// rather than what failed: a name needs an account, and an anonymous one is not
/// enough (the registry decides that half, and its refusal comes through
/// verbatim).
pub(crate) async fn account_token(data_dir: &Path) -> anyhow::Result<String> {
    broker::account_token(data_dir).await
}

/// Claim `handle`, or claim another one. Permanent: nothing expires and nothing
/// has to be renewed.
pub async fn claim(data_dir: &Path, handle: &str) -> anyhow::Result<Handle> {
    let url = format!("{}/api/registry/handle", base_url());
    let res = reqwest::Client::new()
        .post(&url)
        .bearer_auth(account_token(data_dir).await?)
        .json(&serde_json::json!({ "handle": handle }))
        .send()
        .await
        .with_context(|| format!("reaching the registry at {url}"))?;
    read_json(res).await
}

/// Give a name up — the owner's own call, and the only way one comes free.
pub async fn release(data_dir: &Path, handle: &str) -> anyhow::Result<()> {
    let res = reqwest::Client::new()
        .delete(format!("{}/api/registry/handle", base_url()))
        .bearer_auth(account_token(data_dir).await?)
        .json(&serde_json::json!({ "handle": handle }))
        .send()
        .await
        .context("reaching the registry")?;
    if res.status().is_success() {
        return Ok(());
    }
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    anyhow::bail!("{}", detail(&body, status))
}

/// The names this account owns.
pub async fn current(data_dir: &Path) -> anyhow::Result<Handles> {
    let res = reqwest::Client::new()
        .get(format!("{}/api/registry/handle", base_url()))
        .bearer_auth(account_token(data_dir).await?)
        .send()
        .await
        .context("reaching the registry")?;
    read_json(res).await
}

/// Read a registry answer, turning its error shape into ours rather than a bare
/// status — "that handle is in use" is the whole content of a 409, and the
/// person choosing a name is entitled to it.
async fn read_json<T: serde::de::DeserializeOwned>(res: reqwest::Response) -> anyhow::Result<T> {
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    if status.is_success() {
        return serde_json::from_str(&body).context("reading the registry's answer");
    }
    anyhow::bail!("{}", detail(&body, status))
}

fn detail(body: &str, status: reqwest::StatusCode) -> String {
    serde_json::from_str::<ApiError>(body)
        .ok()
        .map(|e| if e.message.is_empty() { e.error } else { e.message })
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| status.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_community_has_a_default_that_joins_cleanly() {
        assert!(DEFAULT_BASE_URL.starts_with("https://"));
        assert!(!DEFAULT_BASE_URL.ends_with('/'), "a base URL joins with a leading-slash path");
    }

    #[test]
    fn an_account_with_no_names_is_a_normal_account() {
        let h: Handles = serde_json::from_str(r#"{"handles":[],"limit":3}"#).expect("parse");
        assert!(h.handles.is_empty());
        assert_eq!(h.limit, 3);
    }

    #[test]
    fn a_claim_carries_where_the_core_now_is_and_no_expiry() {
        let h: Handle = serde_json::from_str(
            r#"{"handle":"ana","base_url":"https://hi-agent.xyz/ana",
                "claimed_at":"2026-08-13T00:00:00Z"}"#,
        )
        .expect("parse");
        assert_eq!(h.base_url, "https://hi-agent.xyz/ana");
        assert!(!h.claimed_at.is_empty());
    }

    #[test]
    fn a_refusal_carries_the_registry_s_own_words() {
        let taken = r#"{"error":"handle_taken","message":"that handle is in use"}"#;
        assert_eq!(detail(taken, reqwest::StatusCode::CONFLICT), "that handle is in use");
        // And an unreadable one still says something actionable.
        let html = "<html>oh no</html>";
        assert!(detail(html, reqwest::StatusCode::INTERNAL_SERVER_ERROR).contains("500"));
    }

    #[tokio::test]
    async fn claiming_without_an_account_says_what_to_do() {
        let dir = std::env::temp_dir().join(format!("hi-community-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let err = account_token(&dir).await.unwrap_err().to_string();
        assert!(err.contains("sign in"), "{err}");
    }
}
