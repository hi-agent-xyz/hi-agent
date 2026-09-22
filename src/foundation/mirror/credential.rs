//! Asking the community for the keys: `POST <community>/api/cache/credential`.
//!
//! Presented with the account's token, for the handle this machine serves — the
//! same pair the tunnel dials with, because the prefix a core may write is the
//! name it answers to.

use std::path::Path;
use std::time::Duration;

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::cos::WriteKey;
use super::edge::ReadKey;
use crate::foundation::community;

/// Both halves, for one handle.
#[derive(Debug, Clone)]
pub struct Grant {
    pub handle: String,
    pub bucket: String,
    pub region: String,
    /// `<handle>/` — every key written and every path signed starts with it.
    pub prefix: String,
    pub write: WriteKey,
    pub read: ReadKey,
    /// How long the bucket keeps an object before its lifecycle rule expires it.
    pub lifetime: Duration,
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
struct ReadDto {
    base_url: String,
    param: String,
    key: String,
    valid_secs: u64,
}

#[derive(Deserialize)]
struct GrantDto {
    bucket: String,
    region: String,
    prefix: String,
    write: WriteDto,
    read: ReadDto,
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
        let Ok(g) = serde_json::from_str::<GrantDto>(&body) else {
            return Ok(Answer::Declined("the community answered without a grant".into()));
        };
        return Ok(Answer::Granted(Grant {
            handle: handle.to_string(),
            bucket: g.bucket,
            region: g.region,
            prefix: g.prefix,
            write: WriteKey {
                secret_id: g.write.secret_id,
                secret_key: g.write.secret_key,
                token: g.write.token,
                expires_at: g.write.expires_at,
            },
            read: ReadKey {
                base_url: g.read.base_url.trim_end_matches('/').to_string(),
                param: g.read.param,
                key: g.read.key,
                valid: Duration::from_secs(g.read.valid_secs),
            },
            lifetime: Duration::from_secs(g.lifetime_secs),
        }));
    }

    let e: ErrorDto = serde_json::from_str(&body).unwrap_or_default();
    match e.error.as_str() {
        "cache_not_configured" | "not_your_handle" => Ok(Answer::Declined(e.error)),
        _ => anyhow::bail!("{status} {} {}", e.error, e.message),
    }
}
