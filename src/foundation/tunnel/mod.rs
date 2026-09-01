//! The tunnel — one outbound connection, held open, that makes a core behind NAT
//! reachable by name.
//!
//! See [`docs/arch/topology.md`](../../../docs/arch/topology.md)`#core--community---the-tunnel`.
//!
//! **Dialing out is the whole trick.** Anywhere the core can already reach the
//! community, it can be reached back — no port forwarding, no configuration, no
//! public address of its own.
//!
//! ## What rides it, and what does not
//!
//! Only routed traffic. Control — claiming a handle, renaming — is ordinary
//! HTTPS ([`crate::foundation::community`]), because a core that can dial out
//! needs no second request/response protocol to say so.
//!
//! **The connection is the liveness signal, and the only one.** A handle with no
//! live connection is asleep, not lost; there is no heartbeat and nothing to
//! renew.
//!
//! ## The shape
//!
//! One WebSocket to the community, read as a byte stream ([`ws`]), carrying a
//! **yamux** session in which the *community* opens streams and this core
//! accepts them. Multiplexed with per-stream flow control, because a stalled
//! audio stream must not freeze text.
//!
//! Each accepted stream carries plain HTTP/1.1 and is handed to the same axum
//! router this core already serves — marked
//! [`Acceptor::OffBox`](crate::foundation::surfaces::Acceptor), because it is.
//! So a request arriving through the community is gated exactly as one arriving
//! on a public bind, by the same code, and a WebSocket upgrade inside the tunnel
//! passes through as an ordinary `Upgrade`.

pub mod ws;

use std::sync::OnceLock;
use std::time::Duration;

use axum::Router;
use tokio::sync::mpsc;
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tower::ServiceExt as _;

use crate::foundation::community;
use crate::foundation::surfaces::{Acceptor, accepted_on};

/// How long to wait before redialing, and the ceiling it backs off to. A core
/// that cannot reach the community is not broken — it is a laptop on a train —
/// so this retries forever and quietly.
const REDIAL_MIN: Duration = Duration::from_secs(2);
const REDIAL_MAX: Duration = Duration::from_secs(60);

/// Hold a tunnel open for `handle`, redialing forever.
///
/// Returns immediately, handing back the [`tokio::task::AbortHandle`] for the
/// background task so the supervisor can drop this tunnel when the name changes.
/// Nothing else ends it; a core serves its name for as long as it runs.
pub fn spawn(
    data_dir: std::path::PathBuf,
    router: Router,
    handle: String,
) -> tokio::task::AbortHandle {
    let task = tokio::spawn(async move {
        let router = accepted_on(router, Acceptor::OffBox);
        let mut backoff = REDIAL_MIN;
        loop {
            match hold(&data_dir, &router, &handle).await {
                Ok(()) => {
                    tracing::info!(handle = %handle, "the community closed the tunnel; redialing");
                    backoff = REDIAL_MIN;
                }
                Err(e) => {
                    tracing::warn!(handle = %handle, error = %format!("{e:#}"), backoff = ?backoff, "tunnel down");
                    backoff = (backoff * 2).min(REDIAL_MAX);
                }
            }
            tokio::time::sleep(backoff).await;
        }
    });
    task.abort_handle()
}

/// Dial once and serve until the connection ends.
async fn hold(data_dir: &std::path::Path, router: &Router, handle: &str) -> anyhow::Result<()> {
    let token = community::account_token(data_dir).await?;
    let url = tunnel_url(&community::base_url(), handle);

    let mut request = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
        url.as_str(),
    )?;
    request
        .headers_mut()
        .insert(axum::http::header::AUTHORIZATION, format!("Bearer {token}").parse()?);

    let (socket, _) = tokio_tungstenite::connect_async(request).await?;
    tracing::info!(handle = %handle, %url, "tunnel open");

    // The community opens streams; we accept them. That parity is why this side
    // is the yamux server even though it dialed.
    let mut mux = yamux::Connection::new(
        ws::WsByteStream::new(socket),
        yamux::Config::default(),
        yamux::Mode::Server,
    );

    while let Some(stream) = std::future::poll_fn(|cx| mux.poll_next_inbound(cx)).await {
        let stream = stream?;
        let router = router.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_stream(stream, router).await {
                tracing::debug!(error = %format!("{e:#}"), "a routed stream ended badly");
            }
        });
    }
    Ok(())
}

/// Serve one routed request off the tunnel, through the router this core already
/// serves — including upgrades, so a WebSocket inside the tunnel works.
async fn serve_stream(stream: yamux::Stream, router: Router) -> anyhow::Result<()> {
    let io = hyper_util::rt::TokioIo::new(stream.compat());
    // The router is a tower service over axum's body; hyper hands us its own, so
    // the one adaptation is at the body type.
    let service = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
        let router = router.clone();
        async move { router.oneshot(req.map(axum::body::Body::new)).await }
    });
    hyper::server::conn::http1::Builder::new()
        .serve_connection(io, service)
        .with_upgrades()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// The community's tunnel endpoint for `handle`, as a `ws://`/`wss://` URL.
fn tunnel_url(base: &str, handle: &str) -> String {
    let base = base.trim_end_matches('/');
    let ws = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    format!("{ws}/api/relay/tunnel?handle={handle}")
}

/// The seam [`serve`] speaks through. Set once, by [`start`].
///
/// A channel rather than a `Router` in `AppState`, because the router owns the
/// state and the state would then own the router. The supervisor holds the only
/// clone and nothing else needs one.
static SERVE: OnceLock<mpsc::UnboundedSender<Option<String>>> = OnceLock::new();

/// Serve `handle` from now on, without a restart.
///
/// **This is what a claim calls.** Claiming a name used to write it to the
/// registry and stop there, so the core dialled nothing until it was next
/// restarted — the community routed the name and answered "asleep" for a core
/// that was running the whole time. A name that does not work until you restart
/// is not an address.
///
/// Quiet when no supervisor is running (a test, or a core built without one):
/// the name is still claimed, and the next start will serve it.
pub fn serve(handle: &str) {
    match SERVE.get() {
        Some(tx) => {
            let _ = tx.send(Some(handle.to_string()));
        }
        None => tracing::debug!(handle, "no tunnel supervisor; this name is served at next start"),
    }
}

/// Stop serving any name — what giving a name up calls.
///
/// The registry frees the name immediately, so a tunnel left open would be this
/// core still answering to something it no longer owns.
pub fn stop() {
    if let Some(tx) = SERVE.get() {
        let _ = tx.send(None);
    }
}

/// `app_settings` key holding which of the account's names *this machine* serves.
pub const KEY_HANDLE: &str = "handle";

/// The name this core serves, as last chosen here. `None` on a core that has
/// never claimed one.
pub fn chosen(data_dir: &std::path::Path) -> Option<String> {
    crate::foundation::credentials::get_setting(data_dir, KEY_HANDLE)
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| !h.is_empty())
}

/// Remember that this machine serves `handle` — what a claim calls, alongside
/// [`serve`].
///
/// **An account owns names; a machine serves one of them.** The registry hands
/// back every name the account holds, oldest first, and reading the served name
/// off the front of that list made a rename look like nothing had happened: the
/// new name was dialled at once, but the next start went back to the oldest one
/// and the page kept printing it. Which name this core answers to is a fact
/// about this machine, so this machine is what records it.
pub fn remember(data_dir: &std::path::Path, handle: &str) {
    if let Err(e) = crate::foundation::credentials::set_setting(data_dir, KEY_HANDLE, handle) {
        tracing::warn!(handle, error = %format!("{e:#}"), "could not record the served name");
    }
}

/// Forget the served name — what giving it up calls, so the next start does not
/// try to dial a name this account no longer owns.
pub fn forget(data_dir: &std::path::Path) {
    let _ = crate::foundation::credentials::set_setting(data_dir, KEY_HANDLE, "");
}

/// Which of `held` this core serves: the remembered one while the account still
/// owns it, and otherwise the oldest — the only answer available to a core that
/// claimed its name before this was recorded, or that has been handed an account
/// whose names it did not choose.
pub fn choose<'a>(data_dir: &std::path::Path, held: &'a [community::Handle]) -> Option<&'a str> {
    let chosen = chosen(data_dir);
    chosen
        .as_deref()
        .and_then(|want| held.iter().find(|h| h.handle == want))
        .or_else(|| held.first())
        .map(|h| h.handle.as_str())
}

/// `app_settings` key holding whether this core dials the community at all.
pub const KEY_RELAY: &str = "relay";

/// Whether this core should be reachable by name. **Absent reads as on**, so an
/// install that has never seen the setting behaves as it always has: claim a
/// name and it works.
///
/// Being reachable and having a name are separate, which is the point of this
/// setting existing. A handle is permanent and owned by an account
/// ([`community`]); serving it is a thing this machine is doing right now, and a
/// person may want to stop doing it — on a train, on someone else's network,
/// or just to be unreachable for an afternoon — without giving up the address
/// they handed out. Turning it off is *deliberately asleep*: the community keeps
/// routing the name and answers with the asleep page, which is a state
/// `topology.md` already has, reached on purpose instead of by accident.
pub fn on(data_dir: &std::path::Path) -> bool {
    !matches!(
        crate::foundation::credentials::get_setting(data_dir, KEY_RELAY)
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("off" | "false" | "0" | "no")
    )
}

/// Turn reachability on or off now, and remember it.
///
/// Applies live rather than at next start: a switch that needs a restart to mean
/// anything is not a switch. Turning it on has to look the handle up — a core
/// does not hold its own name, the registry does — and a community that cannot
/// be reached leaves the setting written and the dial to the supervisor's own
/// retry, which is the same path a laptop opening its lid takes.
pub async fn set_on(data_dir: &std::path::Path, on: bool) -> anyhow::Result<()> {
    crate::foundation::credentials::set_setting(
        data_dir,
        KEY_RELAY,
        if on { "on" } else { "off" },
    )?;
    if !on {
        tracing::info!("reachability turned off; closing the tunnel");
        stop();
        return Ok(());
    }
    match community::current(data_dir).await {
        Ok(names) => match choose(data_dir, &names.handles) {
            Some(handle) => serve(handle),
            None => tracing::info!("reachability turned on; no handle claimed yet"),
        },
        Err(e) => tracing::info!(error = %format!("{e:#}"), "reachability turned on; no name to serve yet"),
    }
    Ok(())
}

/// Start the tunnel supervisor, and open one now if this core already has a name.
///
/// Best-effort and quiet: a core with no handle, no account, or no reachable
/// community is a working core that is simply reachable from its own machine.
/// Nothing here is allowed to hold up boot.
pub async fn start(data_dir: &std::path::Path, router: Router) {
    let (tx, mut rx) = mpsc::unbounded_channel::<Option<String>>();
    if SERVE.set(tx).is_err() {
        tracing::warn!("the tunnel supervisor is already running");
        return;
    }

    let dir = data_dir.to_path_buf();
    tokio::spawn(async move {
        // One live core per handle, and one live handle per core: a rename
        // replaces the tunnel rather than adding a second, or the old name would
        // keep answering from the same memory under a name its owner gave up.
        let mut current: Option<(String, tokio::task::AbortHandle)> = None;
        while let Some(wanted) = rx.recv().await {
            if let Some(handle) = wanted.as_ref() {
                if current.as_ref().is_some_and(|(held, _)| held == handle) {
                    continue;
                }
            }
            if let Some((held, abort)) = current.take() {
                match wanted.as_ref() {
                    Some(handle) => {
                        tracing::info!(from = %held, to = %handle, "the name changed; dropping the old tunnel")
                    }
                    None => tracing::info!(handle = %held, "the name was given up; closing the tunnel"),
                }
                abort.abort();
            }
            let Some(handle) = wanted else { continue };
            // The address, not just the name. It was logged once ever — at the
            // moment of claiming — so on every later run the one thing a person
            // wants printed was the one thing that was not.
            tracing::info!(
                handle = %handle,
                address = %format!("{}/{}", community::base_url(), handle),
                "serving a handle"
            );
            let abort = spawn(dir.clone(), router.clone(), handle.clone());
            current = Some((handle, abort));
        }
    });

    if !on(data_dir) {
        tracing::info!(
            "reachability is off; this core is reachable from this machine only. \
             Settings → Reach turns it back on"
        );
        return;
    }

    match community::current(data_dir).await {
        Ok(names) => match choose(data_dir, &names.handles) {
            Some(handle) => serve(handle),
            None => tracing::info!("no handle claimed; reachable from this machine only"),
        },
        Err(e) => tracing::debug!(error = %format!("{e:#}"), "no handle to serve (this core is local-only)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An install that has never seen this setting must behave as it always has.
    ///
    /// The switch is new and every existing data dir predates it, so "absent"
    /// cannot read as off — that would take working installs off the air on
    /// upgrade, silently, with the name still claimed and the address still
    /// handed out. Only an explicit off is off.
    #[test]
    fn reachability_is_on_until_it_is_turned_off() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(on(dir.path()), "a data dir that has never heard of the setting");

        for off in ["off", "false", "0", "no", " OFF "] {
            crate::foundation::credentials::set_setting(dir.path(), KEY_RELAY, off).unwrap();
            assert!(!on(dir.path()), "{off:?} means off");
        }
        for back in ["on", "true", "1", "yes"] {
            crate::foundation::credentials::set_setting(dir.path(), KEY_RELAY, back).unwrap();
            assert!(on(dir.path()), "{back:?} means on");
        }
    }

    /// Turning it off must not give the name up — that is the whole distinction.
    ///
    /// A handle is permanent and owned by an account; being reachable is
    /// something this machine is doing right now. Conflating them would mean
    /// going quiet for an afternoon costs you the address you handed out, which
    /// is exactly what `topology.md` refuses to let a lease do.
    #[tokio::test]
    async fn turning_it_off_keeps_the_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        // No community is reachable from a test, which is the point: the switch
        // is local state and must not need one to be flipped.
        set_on(dir.path(), false).await.expect("off");
        assert!(!on(dir.path()));
        set_on(dir.path(), true).await.expect("on");
        assert!(on(dir.path()));
    }

    #[test]
    fn the_tunnel_url_follows_the_communitys_scheme() {
        assert_eq!(
            tunnel_url("https://hi-agent.xyz", "ana"),
            "wss://hi-agent.xyz/api/relay/tunnel?handle=ana"
        );
        // A local community for testing is plain HTTP, and the tunnel has to
        // follow it rather than insisting on TLS that is not there.
        assert_eq!(
            tunnel_url("http://127.0.0.1:8099/", "ana"),
            "ws://127.0.0.1:8099/api/relay/tunnel?handle=ana"
        );
    }
}
