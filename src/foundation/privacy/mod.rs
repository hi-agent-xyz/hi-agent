//! Keys a person typed, kept out of the model by accident rather than by force.
//!
//! **What this is.** Somebody pastes an API key into the conversation without
//! thinking about it — that is the unconscious moment this exists for. The key is
//! written to an ordinary file under `drive/accounts/secrets/`, and every prompt
//! that enters a model session gets the file's path in its place. The agent can
//! still use the credential (`"$(cat drive/accounts/secrets/x.txt)"`), so nothing
//! it could do before becomes impossible.
//!
//! **What this is not.** It is not a vault, and no prompt may offer one. The
//! secret is transparent to the host, to the drive, and to the person. It is
//! transparent to the agent too, the moment the agent decides to go and read the
//! file — which is allowed, and deliberately unguarded: an agent reading a
//! credential it was pointed at is a decision, not an accident, and this module
//! has no opinion about decisions. Only two seams exist:
//!
//! 1. **Detection**, once, on inbound human text ([`SensitiveDataFilter::file_secrets`]
//!    from `POST /api/in/text`). Nothing else is ever scanned.
//! 2. **Substitution**, by exact match, in [`AgentSession::prompt`] — the one
//!    function every model turn passes through, which is what makes the journal
//!    snapshot replaying an old message safe on the twentieth turn as well as the
//!    first.
//!
//! Tool results, agent-to-agent mail, the system prompt, and codex's own shell
//! are all untouched. The journal and the conversation keep exactly what was
//! typed.

mod filter;
mod store;

use std::path::Path;
use std::sync::Arc;

pub use filter::{PrivacyFinding, SensitiveDataFilter};
pub use store::{SecretMaterial, SecretStore};

pub mod broker;

#[derive(Clone)]
pub struct PrivacyBoundary {
    inner: Arc<Inner>,
}

struct Inner {
    filter: SensitiveDataFilter,
    store: SecretStore,
    http: reqwest::Client,
}

impl PrivacyBoundary {
    pub fn open(data_dir: &Path) -> anyhow::Result<Self> {
        let store = SecretStore::open(data_dir)?;
        let filter = SensitiveDataFilter::new(store.clone());
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            inner: Arc::new(Inner {
                filter,
                store,
                http,
            }),
        })
    }

    pub fn filter(&self) -> &SensitiveDataFilter {
        &self.inner.filter
    }

    pub fn store(&self) -> &SecretStore {
        &self.inner.store
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.inner.http
    }
}
