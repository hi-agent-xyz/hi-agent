//! The local/private side of the model boundary.
//!
//! Raw inputs remain available to the trusted host. Before Codex sends any
//! serialized Responses request to an external provider, the loopback proxy in
//! this module projects PII to typed masks and secrets to ordinary drive-file references.

mod filter;
mod store;

use std::path::Path;
use std::sync::Arc;

pub use filter::{PrivacyFinding, Projection, SensitiveDataFilter};
pub use store::{SecretMaterial, SecretStore};

pub mod broker;
pub mod proxy;

pub const ENV_MODEL_PROXY_KEY: &str = "HI_AGENT_MODEL_PROXY_KEY";

#[derive(Clone)]
pub struct PrivacyBoundary {
    inner: Arc<Inner>,
}

struct Inner {
    token: String,
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
                token: format!("proxy_{}", uuid::Uuid::new_v4().simple()),
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

    pub fn accepts_proxy_token(&self, token: &str) -> bool {
        constant_time_eq(self.inner.token.as_bytes(), token.as_bytes())
    }

    pub fn child_env(&self) -> Vec<(String, String)> {
        vec![(ENV_MODEL_PROXY_KEY.to_string(), self.inner.token.clone())]
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}
