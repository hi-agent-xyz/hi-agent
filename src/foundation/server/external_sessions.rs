//! Short-lived, loopback-only leases for externally hosted one-shot sessions.
//!
//! An external runner must not choose a native hi-agent slug or send a caller-controlled
//! `X-HI-Session-Slug`. This module owns a register → use → release lifecycle:
//! a local caller proves possession of a normal surface credential, hi-agent mints and
//! registers a worker slug owned by the live Cognition rung, and the runner receives one
//! scoped bearer capability for the external MCP endpoint. The raw capability is never
//! stored or logged.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::foundation::mcp::{self, McpReply};
use crate::foundation::registry::{self, RegistrationError, SessionSlug};
use crate::foundation::server::AppState;
use crate::foundation::surfaces::{self, Acceptor};
use crate::identity::{Role, WorkerType};

/// A lease is intentionally shorter than a normal surface session. The runner owns the
/// release call, but expiry is the backstop for crashes and forgotten cleanup.
const LEASE_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_TITLE_CHARS: usize = 72;
const MAX_SUBJECT_CHARS: usize = 160;

struct ExternalRegistration {
    id: SessionSlug,
}

impl Drop for ExternalRegistration {
    fn drop(&mut self) {
        registry::global().unregister(&self.id);
    }
}

struct ExternalLease {
    capability_hash: [u8; 32],
    expires: Instant,
    _registration: ExternalRegistration,
}

/// The live external-session lease table.
pub struct ExternalSessions {
    leases: Mutex<HashMap<SessionSlug, ExternalLease>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReleaseOutcome {
    Released,
    AlreadyReleased,
    InvalidCapability,
}

impl ExternalSessions {
    pub fn new() -> Self {
        Self { leases: Mutex::new(HashMap::new()) }
    }

    /// Register one externally hosted worker against an existing task and live Cognition.
    ///
    /// The caller has already proved a surface credential in the HTTP handler. This method
    /// derives every identity field that matters: role, owner and slug are not inputs.
    pub async fn register(
        &self,
        data_dir: &std::path::Path,
        title: &str,
        subject: &str,
    ) -> anyhow::Result<RegisteredExternalSession> {
        let subject = external_subject(data_dir, subject).await?;

        let owner = registry::global()
            .session_of_role(Role::Cognition)
            .map(|status| status.id)
            .ok_or_else(|| anyhow::anyhow!("the Cognition session is not live"))?;
        let slug = registry::mint(Role::Worker(WorkerType::General), Some(&subject));
        registry::global()
            .register_checked(
                slug.clone(),
                Role::Worker(WorkerType::General),
                Some(owner.clone()),
                title.to_owned(),
                Some(subject),
            )
            .map_err(|error| anyhow::anyhow!("{error}"))?;

        let capability = surfaces::random_token();
        let expires = Instant::now() + LEASE_TTL;
        let expires_at = Utc::now() + chrono::Duration::from_std(LEASE_TTL)?;
        let lease = ExternalLease {
            capability_hash: surfaces::token_hash(&capability),
            expires,
            _registration: ExternalRegistration { id: slug.clone() },
        };

        let duplicate = {
            let mut leases = self.leases.lock().unwrap();
            if leases.contains_key(&slug) {
                true
            } else {
                leases.insert(slug.clone(), lease);
                false
            }
        };
        if duplicate {
            // `mint` is process-unique, but keep the request-boundary invariant explicit if
            // that implementation ever changes.
            registry::global().unregister(&slug);
            return Err(anyhow::anyhow!(
                "{}",
                RegistrationError::Duplicate(slug)
            ));
        }

        Ok(RegisteredExternalSession {
            slug,
            owner,
            mcp_url: "/mcp/external",
            capability,
            expires_at,
        })
    }

    /// Authenticate a capability and return the host-minted sender slug.
    pub fn authenticate(&self, capability: &str) -> Option<SessionSlug> {
        self.prune_expired();
        let want = surfaces::token_hash(capability);
        let leases = self.leases.lock().unwrap();
        leases.iter().find_map(|(slug, lease)| {
            constant_time_eq(&lease.capability_hash, &want).then(|| slug.clone())
        })
    }

    /// Release a named lease only when its capability matches. The registration is dropped
    /// after the table lock is released, so registry bookkeeping cannot block or re-enter
    /// the lease table.
    fn release(&self, slug: &SessionSlug, capability: &str) -> ReleaseOutcome {
        self.prune_expired();
        let want = surfaces::token_hash(capability);
        let removed = {
            let mut leases = self.leases.lock().unwrap();
            match leases.get(slug) {
                Some(lease) if constant_time_eq(&lease.capability_hash, &want) => {
                    leases.remove(slug)
                }
                Some(_) => return ReleaseOutcome::InvalidCapability,
                None => return ReleaseOutcome::AlreadyReleased,
            }
        };
        if removed.is_some() {
            ReleaseOutcome::Released
        } else {
            // The entry can only disappear between the lookup and remove if another
            // release raced us; DELETE remains idempotent in that case.
            ReleaseOutcome::AlreadyReleased
        }
    }

    fn prune_expired(&self) {
        let now = Instant::now();
        let expired = {
            let mut leases = self.leases.lock().unwrap();
            let ids: Vec<_> = leases
                .iter()
                .filter(|(_, lease)| lease.expires <= now)
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter().filter_map(|id| leases.remove(&id)).collect::<Vec<_>>()
        };
        // Drop outside the mutex: each registration's Drop unregisters from the global
        // switchboard and may write the session index.
        drop(expired);
    }

    #[cfg(test)]
    fn live_count(&self) -> usize {
        self.prune_expired();
        self.leases.lock().unwrap().len()
    }
}

impl Default for ExternalSessions {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ExternalSessions {
    fn drop(&mut self) {
        let leases = std::mem::take(self.leases.get_mut().unwrap());
        drop(leases);
    }
}

#[derive(Debug, Serialize)]
pub struct RegisteredExternalSession {
    pub slug: SessionSlug,
    pub owner: SessionSlug,
    pub mcp_url: &'static str,
    pub capability: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterRequest {
    pub title: String,
    pub subject: String,
}

/// `POST /api/external-sessions`.
pub async fn post_register(
    State(state): State<std::sync::Arc<AppState>>,
    acceptor: Option<Extension<Acceptor>>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<RegisterRequest>,
) -> Response {
    if acceptor.map(|Extension(a)| a) != Some(Acceptor::Loopback) {
        return (StatusCode::FORBIDDEN, "external sessions are loopback-only\n").into_response();
    }
    let Some(surface_token) = bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, "present a surface bearer credential\n").into_response();
    };
    if !state.surfaces.verify_bearer(&surface_token) {
        return (StatusCode::UNAUTHORIZED, "that surface credential was not accepted\n")
            .into_response();
    }
    if request.title.trim().is_empty()
        || request.title.chars().count() > MAX_TITLE_CHARS
        || request.subject.trim().is_empty()
        || request.subject.chars().count() > MAX_SUBJECT_CHARS
    {
        return (
            StatusCode::BAD_REQUEST,
            format!(
                "title must be 1-{MAX_TITLE_CHARS} characters and subject 1-{MAX_SUBJECT_CHARS} characters\n"
            ),
        )
            .into_response();
    }

    match state
        .external_sessions
        .register(&state.data_dir, request.title.trim(), request.subject.trim())
        .await
    {
        Ok(session) => (StatusCode::CREATED, axum::Json(session)).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, format!("{error}\n")).into_response(),
    }
}

/// `DELETE /api/external-sessions/{slug}`.
pub async fn delete_release(
    State(state): State<std::sync::Arc<AppState>>,
    acceptor: Option<Extension<Acceptor>>,
    Path(raw_slug): Path<String>,
    headers: HeaderMap,
) -> Response {
    if acceptor.map(|Extension(a)| a) != Some(Acceptor::Loopback) {
        return (StatusCode::FORBIDDEN, "external sessions are loopback-only\n").into_response();
    }
    let Some(capability) = bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, "present the external-session capability\n")
            .into_response();
    };
    let Ok(slug) = raw_slug.parse::<SessionSlug>() else {
        return (StatusCode::BAD_REQUEST, "invalid session slug\n").into_response();
    };
    match state.external_sessions.release(&slug, &capability) {
        ReleaseOutcome::Released | ReleaseOutcome::AlreadyReleased => {
            StatusCode::NO_CONTENT.into_response()
        }
        ReleaseOutcome::InvalidCapability => {
            (StatusCode::UNAUTHORIZED, "that capability was not accepted\n").into_response()
        }
    }
}

/// `POST /mcp/external`.
pub async fn post_mcp(
    State(state): State<std::sync::Arc<AppState>>,
    acceptor: Option<Extension<Acceptor>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if acceptor.map(|Extension(a)| a) != Some(Acceptor::Loopback) {
        return (StatusCode::FORBIDDEN, "external sessions are loopback-only\n").into_response();
    }
    let Some(capability) = bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, "present the external-session capability\n")
            .into_response();
    };
    let Some(slug) = state.external_sessions.authenticate(&capability) else {
        return (StatusCode::UNAUTHORIZED, "that external-session capability was not accepted\n")
            .into_response();
    };
    let msg: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(error) => {
            return (StatusCode::BAD_REQUEST, format!("invalid JSON-RPC body: {error}"))
                .into_response();
        }
    };
    match mcp::handle_external(
        &state.tool_registry,
        &state.data_dir,
        &state.privacy,
        &state.video_in_partial,
        &state.observatory,
        slug,
        &msg,
    )
    .await
    {
        McpReply::Json(value) => axum::Json(value).into_response(),
        McpReply::Accepted => StatusCode::ACCEPTED.into_response(),
    }
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?
        .trim();
    (!token.is_empty()).then(|| token.to_owned())
}

fn constant_time_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut diff = 0u8;
    for (left, right) in a.iter().zip(b) {
        diff |= left ^ right;
    }
    diff == 0
}

/// Resolve the metadata subject carried by an external lease.
///
/// Auto-run definitions are durable automation objects under `data/agents`, while ordinary
/// work is filed under `memory/facets/tasks`. The weekly-report runner supplies the former's
/// stable name (`weekly-report-friday`), not the dated implementation task that happened to
/// staff this change. Accept either canonical object, but never accept an arbitrary string.
async fn external_subject(data_dir: &std::path::Path, requested: &str) -> anyhow::Result<String> {
    let subject = crate::mind::memory::facets::slug(requested);
    if subject.is_empty() {
        anyhow::bail!("external session subject must contain a usable character");
    }
    let task = crate::mind::memory::tasks::named(data_dir, &subject).await?;
    if matches!(task, crate::mind::memory::tasks::Named::Row(_)) {
        return Ok(subject);
    }

    let definition = data_dir
        .join("agents")
        .join(&subject)
        .join(format!("{subject}.md"));
    if tokio::fs::try_exists(&definition).await.unwrap_or(false) {
        return Ok(subject);
    }

    let open = match task {
        crate::mind::memory::tasks::Named::Missing { open } => open,
        crate::mind::memory::tasks::Named::Row(_) => unreachable!("row checked above"),
    };
    anyhow::bail!("no existing task or agent definition named `{subject}`; open tasks:\n{open}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_compare_without_early_exit() {
        assert!(constant_time_eq(&[7; 32], &[7; 32]));
        assert!(!constant_time_eq(&[7; 32], &[8; 32]));
    }

    #[test]
    fn empty_lease_table_drops_cleanly() {
        let leases = ExternalSessions::new();
        assert_eq!(leases.live_count(), 0);
    }

    #[test]
    fn expired_leases_are_removed_before_authentication() {
        let leases = ExternalSessions::new();
        let slug = "expired-external".parse().expect("slug");
        leases.leases.lock().unwrap().insert(
            slug,
            ExternalLease {
                capability_hash: surfaces::token_hash("expired-capability"),
                expires: Instant::now() - Duration::from_secs(1),
                _registration: ExternalRegistration { id: "expired-external".parse().unwrap() },
            },
        );
        assert!(leases.authenticate("expired-capability").is_none());
        assert_eq!(leases.live_count(), 0);
    }
}
