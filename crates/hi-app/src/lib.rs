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
//! ## Its own crate, because a phone can link it and cannot link a core
//!
//! Everything below builds for `aarch64-apple-ios`, and nothing a core needs —
//! the runtime it spawns, ONNX, ffmpeg — is reachable from here. That is not a
//! packaging detail: on iOS an app *is* this crate plus a webview, because a
//! core cannot be hosted there and the design never asked it to be. Hosting is a
//! capability of an app instance rather than a property of a platform, so the
//! same code either seeds entry #1 with the core on this machine or starts with
//! an empty list and waits to be paired.
//!
//! On a desktop the core's binary links this crate and serves it on a second
//! loopback port; the process split waits for the native shell, per `CLAUDE.md`.
//! Either way the seam is the same one, which is why it is drawn here.
//!
//! ## What it does not do
//!
//! It has no opinion about who may reach a core — that is the core's alone
//! (the core's `foundation::surfaces`). It supervises only cores it started; a
//! remote core can be observed, never supervised. And it never talks to the
//! community: an address is a base URL, so an app talks only to its cores.

pub mod proxy;
pub mod roster;
pub mod screen;

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

    /// Whether a core answers, and how — the one thing a roster has to say
    /// besides which entries exist.
    ///
    /// Asks `/healthz`, which is open by definition in every shape, so this needs
    /// no credential and reports honestly on a core this app is not paired with.
    /// Three answers rather than a bool, because "asleep" is an ordinary state
    /// with an ordinary fix and "unreachable" is not: a claimed handle whose core
    /// is not dialled in gets the community's `503`, and a laptop that is off
    /// gets nothing at all.
    ///
    /// **Observed, never supervised.** A remote core is not this app's to start,
    /// so this reports and offers nothing. The timeout is short because it feeds
    /// a status dot; a slow answer and no answer mean the same thing to a person
    /// deciding who to be with.
    pub async fn reachable(&self, base_url: &str) -> &'static str {
        let res = self
            .client
            .get(format!("{}/healthz", base_url.trim_end_matches('/')))
            .timeout(std::time::Duration::from_secs(4))
            .send()
            .await;
        match res {
            Ok(r) if r.status().is_success() => "here",
            Ok(r) if r.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE => "asleep",
            Ok(_) => "here", // answering at all means something is listening
            Err(_) => "unreachable",
        }
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
        // A `200` is not enough to conclude we reached a core. Every site answers
        // something, and an address that is not a core — a typo, an unclaimed
        // handle falling through to the community's own pages — answers its
        // catch-all. What settles it is the one thing only a core replies with:
        // a session body carrying the credential. Falling back to the code the
        // person typed would store a roster entry that looks paired, can never
        // work, and says so at no point.
        let body: serde_json::Value = res.json().await.unwrap_or(serde_json::Value::Null);
        let credential = body
            .get("credential")
            .and_then(|v| v.as_str())
            .filter(|c| !c.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("{base} answered, but not like a core — check the address"))?
            .to_string();
        roster::add(&self.data_dir, label, base, &credential)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pairing has to reach a *core*, and a status code cannot tell you that.
    ///
    /// Every website answers something. An address that is not a core — a typo, or
    /// an unclaimed handle falling through to the community's own pages — answers
    /// its catch-all with a cheerful `200` and a page. Accepting that stored a
    /// roster entry whose credential was the pairing code the person typed: it
    /// looked paired, could never work, and never said so.
    #[tokio::test]
    async fn a_200_from_something_that_is_not_a_core_is_not_pairing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let app = App::new(dir.path().to_path_buf()).expect("app");

        // A server that answers everything 200 with a page, the way a site does.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let site = axum::Router::new().fallback(axum::routing::any(|| async {
                axum::response::Html("<!doctype html><title>a website</title>")
            }));
            let _ = axum::serve(listener, site).await;
        });

        let err = app
            .pair(&format!("http://{addr}"), "some-code", "not a core")
            .await
            .expect_err("a page is not a session");
        assert!(
            err.to_string().contains("not like a core"),
            "the reason should name the address, got: {err}"
        );
        assert!(
            roster::list(dir.path()).expect("roster").is_empty(),
            "nothing that cannot work belongs on the roster"
        );
    }
}
