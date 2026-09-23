//! Asking the community for the keys: `POST <community>/api/cache/credential`.
//!
//! Presented with the account's token, for the handle this machine serves — the
//! same pair the tunnel dials with, because the prefix a core may write is the
//! name it answers to, and the key it signs sessions with is that name's.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::cos::WriteKey;
use crate::foundation::community;

/// Both halves, for one handle.
#[derive(Clone)]
pub struct Grant {
    pub handle: String,
    pub bucket: String,
    pub region: String,
    /// `cache/<handle>/` — every key written starts with it, followed by the
    /// request path the object answers.
    pub prefix: String,
    /// The path prefixes the edge answers from the bucket. Nothing outside them is
    /// uploaded: the edge would never serve it.
    pub paths: Vec<String>,
    pub write: WriteKey,
    /// `K_handle` — what this core signs its session cookies with, and what the
    /// edge derives from the master for this handle's hostname
    /// (`docs/arch/cache.md` § *Keys*).
    pub sign_key: Arc<[u8]>,
    /// How long the bucket keeps an object before its lifecycle rule expires it.
    pub lifetime: Duration,
}

impl std::fmt::Debug for Grant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Grant")
            .field("handle", &self.handle)
            .field("bucket", &self.bucket)
            .field("prefix", &self.prefix)
            .field("paths", &self.paths)
            .field("write", &self.write)
            .field("lifetime", &self.lifetime)
            .finish_non_exhaustive()
    }
}

impl Grant {
    /// What a [`super::record::Row`] was uploaded under.
    pub fn target(&self) -> String {
        format!("{}/{}", self.bucket, self.prefix)
    }

    /// The object key for a request path: the prefix, then the path.
    pub fn key(&self, path: &str) -> String {
        format!("{}{}", self.prefix, path.trim_start_matches('/'))
    }

    /// Whether the edge answers `path` from the bucket at all.
    pub fn fronts(&self, path: &str) -> bool {
        self.paths.iter().any(|p| path.starts_with(p.as_str()))
    }
}

/// What the community said.
pub enum Answer {
    Granted(Grant),
    /// A refusal that will not change by asking again soon: the cache is not
    /// configured there, or the handle is not this account's.
    Declined(String),
}

#[derive(Deserialize)]
struct WriteDto {
    secret_id: String,
    secret_key: String,
    token: String,
    expires_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct GrantDto {
    bucket: String,
    region: String,
    prefix: String,
    paths: Vec<String>,
    write: WriteDto,
    /// Hex.
    sign_key: String,
    lifetime_secs: u64,
}

#[derive(Deserialize, Default)]
struct ErrorDto {
    #[serde(default)]
    error: String,
    #[serde(default)]
    message: String,
}

pub async fn fetch(data_dir: &Path, handle: &str) -> anyhow::Result<Answer> {
    let url = format!("{}/api/cache/credential", community::base_url());
    let token = community::account_token(data_dir).await?;
    let res = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?
        .post(&url)
        .bearer_auth(token)
        .json(&serde_json::json!({ "handle": handle }))
        .send()
        .await
        .with_context(|| format!("reaching {url}"))?;
    let status = res.status();
    let body = res.text().await.unwrap_or_default();

    if status.is_success() {
        // A community that predates the route answers with its site's page, not a
        // 404 — the site is a catch-all. That will not change by asking again.
        let Some((g, sign_key)) = serde_json::from_str::<GrantDto>(&body)
            .ok()
            .and_then(|g| unhex(&g.sign_key).map(|k| (g, k)))
        else {
            return Ok(Answer::Declined("the community answered without a grant".into()));
        };
        return Ok(Answer::Granted(Grant {
            handle: handle.to_string(),
            bucket: g.bucket,
            region: g.region,
            prefix: g.prefix,
            paths: g.paths,
            write: WriteKey {
                secret_id: g.write.secret_id,
                secret_key: g.write.secret_key,
                token: g.write.token,
                expires_at: g.write.expires_at,
            },
            sign_key: sign_key.into(),
            lifetime: Duration::from_secs(g.lifetime_secs),
        }));
    }

    let e: ErrorDto = serde_json::from_str(&body).unwrap_or_default();
    match e.error.as_str() {
        "cache_not_configured" | "not_your_handle" => Ok(Answer::Declined(e.error)),
        _ => anyhow::bail!("{status} {} {}", e.error, e.message),
    }
}

/// A key of at least 16 bytes, from hex; anything shorter or malformed is no key.
fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() < 32 || s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sign_key_is_read_from_hex_and_nothing_short_passes() {
        let k = "8a3131de9ce55e583b3e3a4996dbdb976d3cb452b03b3964a5b7e9bb79f8d7ec";
        assert_eq!(unhex(k).map(|v| v.len()), Some(32));
        assert_eq!(unhex(""), None);
        assert_eq!(unhex("abcd"), None);
        assert_eq!(unhex(&"zz".repeat(16)), None);
    }
}
