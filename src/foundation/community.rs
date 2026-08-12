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
//! ## The core's own identity
//!
//! A core registers once, anonymously, and keeps `(core_id, secret)` in its
//! config store. That identity is **not** the broker's device account and shares
//! nothing with it — a BYOK install pays us nothing and must still get a name.
//! Reusing the broker's `device_id` here would make the address quietly depend on
//! the account, which is the bug invariant 2 exists to prevent.

use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

use crate::foundation::credentials;

/// Where the community lives. Overridable for a test deployment; there is no
/// per-service address because the four services share one origin today.
pub const ENV_BASE_URL: &str = "HI_AGENT_COMMUNITY";
const DEFAULT_BASE_URL: &str = "https://hi-agent.xyz";

/// `app_settings` keys holding this core's registry identity. Not secrets the
/// agent ever sees; not the broker's account.
const KEY_CORE_ID: &str = "community_core_id";
const KEY_CORE_SECRET: &str = "community_core_secret";

/// What the registry says about this core.
#[derive(Debug, Clone, Deserialize, serde::Serialize, Default)]
pub struct Handle {
    pub core_id: String,
    /// Empty until a handle is claimed. A core with no handle works; it is
    /// simply unreachable from anywhere but this machine.
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub display_name: String,
    /// Where this core is reachable, as the community sees it. The core cannot
    /// work this out from the inside, and it needs it to say where it is — on a
    /// pairing QR, on an upload link.
    #[serde(default)]
    pub base_url: String,
    /// When the lease runs out if nothing checks in. Empty for an unclaimed core.
    #[serde(default)]
    pub expires_at: String,
}

#[derive(Deserialize)]
struct Registration {
    core_id: String,
    secret: String,
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

/// This core's registry identity, registering once if it has none.
///
/// Anonymous: nothing is presented, because registration must not require an
/// account. The credential is `<core_id>.<secret>` and is stored, not derived —
/// a secret that could be recomputed from anything else would be a secret shared
/// with that thing.
pub async fn identity(data_dir: &Path) -> anyhow::Result<String> {
    if let (Some(id), Some(secret)) = (
        credentials::get_setting(data_dir, KEY_CORE_ID),
        credentials::get_setting(data_dir, KEY_CORE_SECRET),
    ) {
        return Ok(format!("{id}.{secret}"));
    }

    let url = format!("{}/api/registry/register", base_url());
    let res = reqwest::Client::new()
        .post(&url)
        .send()
        .await
        .with_context(|| format!("reaching the registry at {url}"))?;
    if !res.status().is_success() {
        anyhow::bail!("the registry refused to register this core ({})", res.status());
    }
    let reg: Registration = res.json().await.context("reading the registration")?;
    credentials::set_setting(data_dir, KEY_CORE_ID, &reg.core_id)?;
    credentials::set_setting(data_dir, KEY_CORE_SECRET, &reg.secret)?;
    tracing::info!(core_id = %reg.core_id, "registered with the community");
    Ok(format!("{}.{}", reg.core_id, reg.secret))
}

/// Claim `handle`, or rename to it. The same call either way — the core id
/// underneath never changes.
pub async fn claim(data_dir: &Path, handle: &str) -> anyhow::Result<Handle> {
    let credential = identity(data_dir).await?;
    let url = format!("{}/api/registry/handle", base_url());
    let res = reqwest::Client::new()
        .post(&url)
        .bearer_auth(&credential)
        .json(&serde_json::json!({ "handle": handle }))
        .send()
        .await
        .with_context(|| format!("reaching the registry at {url}"))?;
    read_handle(res).await
}

/// What this core holds, if anything.
pub async fn current(data_dir: &Path) -> anyhow::Result<Handle> {
    let credential = identity(data_dir).await?;
    let res = reqwest::Client::new()
        .get(format!("{}/api/registry/handle", base_url()))
        .bearer_auth(&credential)
        .send()
        .await
        .context("reaching the registry")?;
    read_handle(res).await
}

/// Check in: tell the registry this core is reachable, which is the only thing
/// that keeps a handle.
///
/// An explicit call today. When the tunnel lands the held connection is what
/// says this, so the lease renews as a side effect of being reachable and there
/// is no second signal to drift out of agreement with the first.
pub async fn check_in(data_dir: &Path) -> anyhow::Result<Handle> {
    let credential = identity(data_dir).await?;
    let res = reqwest::Client::new()
        .post(format!("{}/api/registry/checkin", base_url()))
        .bearer_auth(&credential)
        .send()
        .await
        .context("reaching the registry")?;
    read_handle(res).await
}

/// Read a registry answer, turning its error shape into ours rather than a bare
/// status — "that handle is in use" is the whole content of a 409 and the person
/// asking is entitled to it.
async fn read_handle(res: reqwest::Response) -> anyhow::Result<Handle> {
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    if status.is_success() {
        return serde_json::from_str(&body).context("reading the registry's answer");
    }
    let detail = serde_json::from_str::<ApiError>(&body)
        .ok()
        .map(|e| if e.message.is_empty() { e.error } else { e.message })
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| status.to_string());
    anyhow::bail!("{detail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_community_has_a_default_and_an_override() {
        // Not asserted against the live value: the env is process-wide and this
        // suite runs in parallel. The shape is what matters.
        assert!(DEFAULT_BASE_URL.starts_with("https://"));
        assert!(!DEFAULT_BASE_URL.ends_with('/'), "a base URL joins with a leading-slash path");
    }

    #[test]
    fn an_unclaimed_core_reads_as_having_no_handle() {
        let h: Handle = serde_json::from_str(r#"{"core_id":"abc"}"#).expect("parse");
        assert_eq!(h.core_id, "abc");
        assert!(h.handle.is_empty(), "a core with no name is a normal core");
        assert!(h.base_url.is_empty(), "and it is reachable from nowhere");
    }

    #[test]
    fn a_claim_carries_where_the_core_now_is() {
        let h: Handle = serde_json::from_str(
            r#"{"core_id":"abc","handle":"ana","base_url":"https://hi-agent.xyz/ana",
                "expires_at":"2026-08-19T00:00:00Z"}"#,
        )
        .expect("parse");
        assert_eq!(h.base_url, "https://hi-agent.xyz/ana");
        assert!(!h.expires_at.is_empty(), "a claim without a lease is a deed");
    }

    #[tokio::test]
    async fn a_refusal_carries_the_registry_s_own_words() {
        let res = http_response(409, r#"{"error":"handle_taken","message":"that handle is in use"}"#);
        let err = read_handle(res).await.unwrap_err().to_string();
        assert_eq!(err, "that handle is in use");
    }

    #[tokio::test]
    async fn an_unreadable_refusal_still_says_something() {
        let res = http_response(500, "<html>oh no</html>");
        let err = read_handle(res).await.unwrap_err().to_string();
        assert!(err.contains("500"), "{err}");
    }

    /// A `reqwest::Response` built from parts, so the error mapping can be tested
    /// without a server.
    fn http_response(status: u16, body: &str) -> reqwest::Response {
        let raw = axum::http::Response::builder().status(status).body(body.to_string()).expect("build");
        reqwest::Response::from(raw)
    }
}
