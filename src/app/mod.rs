//! The app — a surface onto a core.
//!
//! See [`docs/arch/topology.md`](../../docs/arch/topology.md). The app owns the
//! roster, the face, and the OS session; it never holds person-identity and never
//! decides authorization. What it does concretely is one thing:
//!
//! > **The face never knows where the core is.** It talks only to the app's local
//! > proxy; the app routes that to the attached core — loopback or, later, a
//! > tunnel — and attaches the credential upstream.
//!
//! Three things follow, and the third is the point: the webview never holds a
//! credential; switching who you are with is the app repointing its proxy, with
//! no face involvement; and **desktop and mobile run identical face code**, which
//! is what "no architectural difference between them" has to mean concretely.
//!
//! ## It is a module, not yet a process
//!
//! Today this runs inside the same binary as the core it usually renders. That is
//! sequencing, not design: `CLAUDE.md` says not to flip process ownership before
//! the native shell is ready, and the seam this builds — a local proxy in front
//! of a roster — is the same one the shell will hold when it does. Hosting stays
//! a *capability of an app instance* rather than a property of the platform: the
//! core on this machine is simply roster entry #1, and an app that cannot host
//! one has an entry list that starts empty.
//!
//! ## What it does not do
//!
//! It has no opinion about who may reach a core — that is the core's alone
//! ([`crate::foundation::surfaces`]). It supervises only cores it started; a
//! remote core can be observed, never supervised. And it never talks to the
//! community: an address is a base URL, so an app talks only to its cores.

pub mod proxy;
pub mod roster;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// The app's live state: where its roster lives, one HTTP client, and the
/// exchanged session per roster entry.
///
/// Sessions are cached rather than re-exchanged per request because that is what
/// keeps the long-lived credential off the wire — it matters in the relayed
/// shape, where the community terminates TLS and sees every byte we forward.
pub struct App {
    data_dir: PathBuf,
    client: reqwest::Client,
    /// roster entry id → the `hi_surface=…` cookie pair to send upstream.
    sessions: Mutex<HashMap<String, String>>,
}

impl App {
    pub fn new(data_dir: PathBuf) -> anyhow::Result<Self> {
        // No timeout: the core's channels are long-polls and SSE streams that
        // are *supposed* to hang open, and a proxy that gives up on them would
        // break the conversation rather than protect it.
        let client = reqwest::Client::builder()
            .build()
            .map_err(|e| anyhow::anyhow!("building the app's HTTP client: {e}"))?;
        Ok(Self { data_dir, client, sessions: Mutex::new(HashMap::new()) })
    }

    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    /// The session cookie to send to `entry`, exchanging the credential for one
    /// if this is the first request.
    ///
    /// `None` for an entry with no credential — a core on this machine, reached
    /// over loopback, which is not gated.
    async fn session(&self, entry: &roster::Entry) -> Option<String> {
        if entry.credential.is_empty() {
            return None;
        }
        if let Some(c) = self.sessions.lock().unwrap().get(&entry.id) {
            return Some(c.clone());
        }
        let cookie = self.exchange(entry).await?;
        self.sessions.lock().unwrap().insert(entry.id.clone(), cookie.clone());
        Some(cookie)
    }

    /// Trade the credential for a session at the core.
    async fn exchange(&self, entry: &roster::Entry) -> Option<String> {
        let res = self
            .client
            .post(format!("{}/api/session", entry.base_url))
            .bearer_auth(&entry.credential)
            .send()
            .await
            .ok()?;
        if !res.status().is_success() {
            tracing::warn!(
                core = %entry.label,
                status = res.status().as_u16(),
                "the core did not accept this app's credential"
            );
            return None;
        }
        let pair = res
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .map(str::to_string)?;
        tracing::info!(core = %entry.label, "session opened with the core");
        Some(pair)
    }

    /// Forget the cached session for `id`, so the next request exchanges again.
    /// Called when a core answers `401`: the credential may still be good and the
    /// session merely lapsed, and the two look identical from here.
    fn drop_session(&self, id: &str) {
        self.sessions.lock().unwrap().remove(id);
    }

    /// Pair with a core: present `code` — a pairing code, or a credential
    /// already in hand — and keep whatever credential comes back.
    ///
    /// This is the app's half of "pairing a second app". The core mints; we
    /// store. A code that yields no credential means it was a *credential* we
    /// were handed rather than a code, so we keep that instead.
    pub async fn pair(
        &self,
        base_url: &str,
        code: &str,
        label: &str,
    ) -> anyhow::Result<String> {
        let base = base_url.trim_end_matches('/');
        let res = self
            .client
            .post(format!("{base}/api/session"))
            .bearer_auth(code)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(serde_json::json!({ "label": label }).to_string())
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("reaching {base}: {e}"))?;
        anyhow::ensure!(
            res.status().is_success(),
            "{base} did not accept that pairing code ({})",
            res.status()
        );
        let body: serde_json::Value = res.json().await.unwrap_or(serde_json::Value::Null);
        let credential = body
            .get("credential")
            .and_then(|v| v.as_str())
            .unwrap_or(code)
            .to_string();
        roster::add(&self.data_dir, label, base, &credential)
    }
}
