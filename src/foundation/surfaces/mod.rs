//! Who may reach this core — one mechanism, at the core, identical in every shape.
//!
//! See [`docs/arch/topology.md`](../../../docs/arch/topology.md)`#auth`. This is
//! **not** [`crate::foundation::auth`], which is the owner's optional
//! xiaoyuanzhu sign-in and gates nothing; that one links an account, this one
//! decides whether a request is answered at all.
//!
//! ## Trust is structural
//!
//! A request is off-box or it is not, and that is decided by **which acceptor
//! received it** ([`Acceptor`]) — a property of the socket, not of anything the
//! sender can write. So there is no IP allowlist here, and there never will be:
//! in the relayed shape every request shares the community's source address, so
//! an allowlist would be inert exactly where it was needed.
//!
//! - **loopback** — the loopback listener, `make dev`, curl journeys, the popover,
//!   the codex subprocesses calling `/mcp`. Ungated.
//! - **off-box** — a public bind today, a relayed tunnel stream tomorrow. Gated.
//!
//! [`Acceptor`] is read from the request extensions and **fails closed**: a
//! request that arrived with no acceptor marked is treated as off-box, so
//! forgetting the layer costs access rather than granting it.
//!
//! ## One credential, two presentations
//!
//! A long-lived credential is exchanged once at `POST /api/session` for a short
//! session. Both exist because a header alone cannot carry a browser:
//! `EventSource` cannot set headers, browser `WebSocket` cannot set headers, and
//! neither can plain navigation — and this core serves all three. Apps and `curl`
//! send `Authorization: Bearer`; anything browser-shaped rides the `hi_surface`
//! cookie, which SSE, WebSocket and navigation all send by themselves.
//!
//! Exchanging once also keeps the long-lived secret off the wire, and makes
//! `POST /api/session` the single seam where a stronger proof (a keypair, when
//! core-to-core mail forces one) can be swapped in without touching anything
//! downstream.
//!
//! **The session is durable and rolling**, and lives in [`store`] beside the
//! credential. An app survives a restart by re-exchanging what it keeps in the
//! OS keychain; a browser's only durable secret is the cookie, so a session held
//! in process memory made every restart a walk to the desktop for a fresh code,
//! while the `Set-Cookie` already sent claimed thirty days. Rolling, because the
//! alternative — a second refresh token — would add a credential type to do what
//! extending one row does.
//!
//! ## Three ways a surface is admitted
//!
//! A credential, a one-time pairing code, or **asking**: an unadmitted device
//! states the agent's name, and the person approves it from the `reach` view. The
//! third exists because the first two need a keyboard or a camera pointed at the
//! desktop, and a TV has neither.

pub mod store;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use base64::Engine as _;
use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Cookie carrying an exchanged session. `HttpOnly` so no script can read it,
/// `SameSite=Lax` so the form-POST class of cross-site request cannot use it.
///
/// Named in `hi-wire` because an app sets what a core checks, and the two are
/// separate crates that build for different platforms. See [`Surfaces::csrf_ok`]
/// for what the header is for.
pub use hi_wire::{CSRF_HEADER, SESSION_COOKIE};

/// How long an exchanged session lasts. Long enough that a phone left alone for a
/// weekend does not have to re-pair; short enough that a stolen cookie expires.
const SESSION_TTL: Duration = Duration::from_secs(30 * 24 * 3600);

/// How far into its life a session has to be before a use extends it. Open the
/// address once a month and it keeps working; leave it thirty days and it asks
/// again. Below this, a use costs no write at all.
const RENEW_AFTER: Duration = Duration::from_secs(24 * 3600);

/// How long a pairing code stays valid — pick up the other device and scan. Also
/// how long a device may wait to be let in.
const PAIRING_TTL: Duration = Duration::from_secs(600);

/// How many devices may be waiting to be let in at once, before a ninth is
/// refused.
///
/// **The cap is the flood defence.** `POST /api/access/request` is
/// unauthenticated — there is no throttle to spend and no credential to check —
/// so the bound on how much of it can accumulate is the bound on what a stranger
/// can put on the approver's screen. Eight because a person adds one device at a
/// time, and the list has to stay readable on the screen that answers it.
const MAX_PENDING: usize = 8;

/// Failed credential presentations tolerated per [`THROTTLE_WINDOW`] before the
/// gate stops answering. Process-wide and **not** per source address: in the
/// relayed shape every request shares one address, so a per-IP counter would
/// throttle nothing at all where it matters.
const THROTTLE_MAX: u32 = 20;
const THROTTLE_WINDOW: Duration = Duration::from_secs(60);

/// Which acceptor received a request. Inserted as a request extension by the
/// listener that accepted it — never derived from a header, an address, or a
/// forwarded-for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acceptor {
    /// The loopback listener. Same machine, same user: ungated.
    Loopback,
    /// A public bind, or a stream routed in over the community tunnel. Gated.
    OffBox,
}

/// Mark every request this router serves as having arrived on `acceptor`.
///
/// One call per listener, applied outermost, and it is the *only* thing that
/// distinguishes the two servings of the one router. Named rather than inlined
/// because a caller that stands the router up and forgets it gets a fail-closed
/// 401 on everything — correct, but confusing enough to be worth a name to
/// search for. Integration tests are local callers and say so with
/// `Acceptor::Loopback`.
pub fn accepted_on(router: axum::Router, acceptor: Acceptor) -> axum::Router {
    router.layer(axum::Extension(acceptor))
}

/// What a successful authorization was: which credential, how it was presented,
/// and whether the presented session just had its life extended.
///
/// The presentation matters — a bearer header cannot be sent ambiently by another
/// site, so CSRF only applies to the cookie.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Authorized {
    presented: Presented,
    credential_id: String,
    /// The gate re-sends the cookie when this is set, so the browser's copy agrees
    /// with the row behind it.
    renewed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Presented {
    Bearer,
    Cookie,
}

/// A device that has asked to be let in and is waiting on a person.
struct AccessRequest {
    label: String,
    /// Shown on **both** screens. Not a secret, and it authorizes nothing — the
    /// secret is the one handed back in the response body. This exists so that when
    /// two requests land at once the approver can tell which one is the device in
    /// their hand.
    code: String,
    secret_hash: String,
    /// For the approver's list. A wall clock, because an [`Instant`] cannot be
    /// rendered on a screen.
    asked_at: String,
    expires: Instant,
    state: Grant,
}

enum Grant {
    Waiting,
    /// The credential minted at approval, held until the asking device claims it.
    Granted(String),
    Refused,
}

/// What the asking device is told when it polls.
pub enum Claim {
    Pending,
    Approved(String),
    Denied,
    /// Also what an unknown secret gets: the two are indistinguishable from
    /// outside, and saying so would confirm a guess.
    Expired,
}

/// One waiting request, as the approver's screen shows it.
#[derive(Debug, Clone, Serialize)]
pub struct Pending {
    pub id: String,
    pub label: String,
    pub code: String,
    pub asked_at: String,
}

struct Throttle {
    window_start: Instant,
    failures: u32,
}

/// The live half: outstanding pairing codes, devices waiting to be let in, and the
/// failure throttle. The durable half — credentials *and sessions* — is [`store`].
///
/// Pairing codes and access requests are in memory on purpose, and sessions are
/// not: a ten-minute grant lost to a restart is a retap on a device somebody is
/// holding, while a session lost to one is a walk to another room.
pub struct Surfaces {
    data_dir: PathBuf,
    pairing: Mutex<HashMap<String, Instant>>,
    requests: Mutex<HashMap<String, AccessRequest>>,
    throttle: Mutex<Throttle>,
}

impl Surfaces {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            pairing: Mutex::new(HashMap::new()),
            requests: Mutex::new(HashMap::new()),
            throttle: Mutex::new(Throttle { window_start: Instant::now(), failures: 0 }),
        }
    }

    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    /// Verify a long-lived surface bearer without exposing the token to request logs.
    ///
    /// External-session registration is loopback-only, so it cannot rely on the normal
    /// off-box gate. It still has to prove possession of an existing surface credential;
    /// this method is the route-specific, secret-safe check for that boundary.
    pub fn verify_bearer(&self, token: &str) -> bool {
        let Some(id) = self.verify(token) else {
            self.note_failure();
            return false;
        };
        store::touch(&self.data_dir, &id);
        true
    }

    /// Mint the first credential when no surface has ever been paired, and log it
    /// once. Bootstrap only: a core with no screen and no paired app — one in
    /// Docker on a server — otherwise has no way to admit its first surface.
    ///
    /// Returns the token if one was minted, so the caller can decide how loudly to
    /// say it. Nothing is minted once any live credential exists.
    pub fn ensure_first_boot_credential(&self) -> Option<String> {
        match store::count(&self.data_dir) {
            Ok(0) => {}
            Ok(_) => return None,
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), "could not read the surface credentials");
                return None;
            }
        }
        match self.mint("first boot") {
            Ok((_, token)) => Some(token),
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), "could not mint the first-boot credential");
                None
            }
        }
    }

    /// Mint a credential under `label`, returning `(id, token)`. The token exists
    /// exactly once, here — only its hash is stored.
    pub fn mint(&self, label: &str) -> anyhow::Result<(String, String)> {
        let id = uuid::Uuid::now_v7().to_string();
        let token = random_token();
        store::insert(&self.data_dir, &id, label, &hash(&token))?;
        Ok((id, token))
    }

    /// Mint a one-time pairing code. Presented at `POST /api/session` it mints a
    /// real credential; it is spent on first use and expires regardless.
    pub fn mint_pairing_code(&self) -> String {
        let code = random_token();
        let mut map = self.pairing.lock().unwrap();
        let now = Instant::now();
        map.retain(|_, expires| *expires > now);
        map.insert(code.clone(), now + PAIRING_TTL);
        code
    }

    /// Spend a pairing code. `true` exactly once per code.
    fn spend_pairing_code(&self, code: &str) -> bool {
        let mut map = self.pairing.lock().unwrap();
        let now = Instant::now();
        map.retain(|_, expires| *expires > now);
        map.remove(code).is_some()
    }

    /// Resolve a presented credential to its row id, in constant time per row.
    fn verify(&self, token: &str) -> Option<String> {
        let want = hash(token);
        let rows = store::live(&self.data_dir).ok()?;
        let mut found: Option<String> = None;
        for (id, h) in rows {
            if ct_eq(h.as_bytes(), want.as_bytes()) {
                found = Some(id);
            }
        }
        found
    }

    /// Exchange a credential (or a pairing code) for a session. Returns the
    /// session token, the credential id, and — when a pairing code was spent —
    /// the freshly minted credential for the caller to keep.
    pub fn exchange(
        &self,
        presented: &str,
        label: &str,
    ) -> Option<(String, String, Option<String>)> {
        let (credential_id, minted) = if let Some(id) = self.verify(presented) {
            (id, None)
        } else if self.spend_pairing_code(presented) {
            let (id, token) = self.mint(label).ok()?;
            (id, Some(token))
        } else {
            self.note_failure();
            return None;
        };

        store::touch(&self.data_dir, &credential_id);
        let session = random_token();
        store::session_insert(
            &self.data_dir,
            &hash(&session),
            &credential_id,
            Utc::now() + delta(SESSION_TTL),
        )
        .ok()?;
        Some((session, credential_id, minted))
    }

    /// Revoke a surface: the credential row, and every session standing on it.
    /// Both halves, or a revoked phone keeps working until its session lapses.
    pub fn revoke(&self, id: &str) -> anyhow::Result<bool> {
        let revoked = store::revoke(&self.data_dir, id)?;
        store::session_revoke_for(&self.data_dir, id)?;
        Ok(revoked)
    }

    /// Whether the request carries a live credential, how it was presented, and
    /// **which** credential it was — the id is what says whose device this is
    /// ([`store::subject_of`]), so it has to survive the check rather than being
    /// thrown away at it.
    fn authorize(&self, headers: &HeaderMap) -> Option<Authorized> {
        if let Some(token) = bearer(headers) {
            if let Some(id) = self.verify(&token) {
                store::touch(&self.data_dir, &id);
                return Some(Authorized {
                    presented: Presented::Bearer,
                    credential_id: id,
                    renewed: false,
                });
            }
            self.note_failure();
        }
        if let Some(session) = cookie(headers, SESSION_COOKIE) {
            if let Some((id, renewed)) = self.session_of(&session) {
                store::touch(&self.data_dir, &id);
                return Some(Authorized {
                    presented: Presented::Cookie,
                    credential_id: id,
                    renewed,
                });
            }
            self.note_failure();
        }
        None
    }

    /// Resolve a presented session, extending it when it is far enough into its
    /// life. Returns `(credential_id, renewed)`.
    ///
    /// **The same token is re-sent, not rotated.** Rotation would break concurrent
    /// in-flight requests from one page — several fetches start before any of them
    /// returns the new cookie, and whichever lands last wins — for a guarantee
    /// nothing here relies on: the row is what revocation removes, and it is
    /// removed by id either way.
    ///
    /// How far into its life is read off `expires_at` rather than a second column:
    /// the row was written with `expires_at = written + SESSION_TTL`, so the
    /// subtraction says when, and it stays right after a renewal with nothing to
    /// keep in step. A failed renewal write reports `false` rather than re-sending
    /// a cookie that claims more than the row does.
    fn session_of(&self, token: &str) -> Option<(String, bool)> {
        let want = hash(token);
        let rows = store::session_live(&self.data_dir).ok()?;
        let mut found = None;
        for s in rows {
            if ct_eq(s.hash.as_bytes(), want.as_bytes()) {
                found = Some(s);
            }
        }
        let session = found?;

        let now = Utc::now();
        let written = session.expires_at - delta(SESSION_TTL);
        let renewed = now - written > delta(RENEW_AFTER)
            && store::session_renew(&self.data_dir, &session.hash, now + delta(SESSION_TTL))
                .is_ok();
        Some((session.credential_id, renewed))
    }

    /// Ask to be let in under `label`, returning `(id, code, secret)` — or `None`
    /// when [`MAX_PENDING`] devices are already waiting.
    ///
    /// The **secret** is what the asking device polls with and is the only part of
    /// this that authorizes anything; the **code** is for the two people to compare.
    pub fn ask(&self, label: &str) -> Option<(String, String, String)> {
        let mut map = self.requests.lock().unwrap();
        sweep(&mut map);
        if map.values().filter(|r| matches!(r.state, Grant::Waiting)).count() >= MAX_PENDING {
            return None;
        }
        let id = uuid::Uuid::now_v7().to_string();
        let secret = random_token();
        let code = six_digits();
        map.insert(id.clone(), AccessRequest {
            label: label.to_string(),
            code: code.clone(),
            secret_hash: hash(&secret),
            asked_at: Utc::now().to_rfc3339(),
            expires: Instant::now() + PAIRING_TTL,
            state: Grant::Waiting,
        });
        Some((id, code, secret))
    }

    /// Who is waiting, oldest first. Only the `reach` view ever sees this — see
    /// [`docs/arch/topology.md`](../../../docs/arch/topology.md)`#auth` for why the
    /// agent is not told.
    pub fn pending(&self) -> Vec<Pending> {
        let mut map = self.requests.lock().unwrap();
        sweep(&mut map);
        let mut out: Vec<Pending> = map
            .iter()
            .filter(|(_, r)| matches!(r.state, Grant::Waiting))
            .map(|(id, r)| Pending {
                id: id.clone(),
                label: r.label.clone(),
                code: r.code.clone(),
                asked_at: r.asked_at.clone(),
            })
            .collect();
        out.sort_by(|a, b| a.asked_at.cmp(&b.asked_at));
        out
    }

    /// Let a waiting device in: mint a credential under the label it asked with,
    /// and hold it for that device to claim. `false` when there is no such request.
    pub fn approve(&self, id: &str) -> anyhow::Result<bool> {
        let mut map = self.requests.lock().unwrap();
        sweep(&mut map);
        let Some(request) = map.get(id).filter(|r| matches!(r.state, Grant::Waiting)) else {
            return Ok(false);
        };
        let label = request.label.clone();
        let (_, token) = self.mint(&label)?;
        if let Some(request) = map.get_mut(id) {
            request.state = Grant::Granted(token);
        }
        Ok(true)
    }

    /// Refuse one. Told apart from an expiry on the other device, because "nobody
    /// answered" and "somebody said no" are different things to read.
    pub fn deny(&self, id: &str) -> bool {
        let mut map = self.requests.lock().unwrap();
        sweep(&mut map);
        match map.get_mut(id) {
            Some(r) if matches!(r.state, Grant::Waiting) => {
                r.state = Grant::Refused;
                true
            }
            _ => false,
        }
    }

    /// What the asking device gets for its secret. An approved credential comes
    /// back **exactly once** — the row is dropped as it is handed over, so a secret
    /// that leaked after the fact buys nothing.
    ///
    /// **What makes the secret unguessable is the secret**, not a rate limit: it is
    /// [`random_token`], the same 32 bytes a credential is. An unknown one is
    /// counted as a failure like any other, but note what that does and does not
    /// buy — this route is in [`open_path`], and [`gate`] returns on an open path
    /// *before* it consults [`Surfaces::throttled`], so the count is shared
    /// bookkeeping rather than a refusal here. `POST /api/session` is open on the
    /// same terms; making either of them refuse on a spent budget would lock a
    /// device out of the only way it has in.
    pub fn claim(&self, secret: &str) -> Claim {
        let want = hash(secret);
        let mut map = self.requests.lock().unwrap();
        sweep(&mut map);
        let mut found = None;
        for (id, r) in map.iter() {
            if ct_eq(r.secret_hash.as_bytes(), want.as_bytes()) {
                found = Some(id.clone());
            }
        }
        let Some(id) = found else {
            drop(map);
            self.note_failure();
            return Claim::Expired;
        };
        if matches!(map.get(&id).map(|r| &r.state), Some(Grant::Waiting)) {
            return Claim::Pending;
        }
        match map.remove(&id).map(|r| r.state) {
            Some(Grant::Granted(token)) => Claim::Approved(token),
            Some(Grant::Refused) => Claim::Denied,
            _ => Claim::Expired,
        }
    }

    /// Whether a cookie-authenticated state-changing request could have been made
    /// cross-site without a preflight.
    ///
    /// A cross-site *simple* request can only carry `application/x-www-form-
    /// urlencoded`, `multipart/form-data` or `text/plain`; anything else already
    /// forces a preflight this core never answers. So those three are the whole
    /// exposure, and [`CSRF_HEADER`] is the way through for the two routes that
    /// legitimately use them (typed text, an uploaded file).
    fn csrf_ok(headers: &HeaderMap, method: &axum::http::Method) -> bool {
        use axum::http::Method;
        if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
            return true;
        }
        if headers.contains_key(CSRF_HEADER) {
            return true;
        }
        let ct = headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        !matches!(
            ct.as_str(),
            "application/x-www-form-urlencoded" | "multipart/form-data" | "text/plain" | ""
        )
    }

    fn note_failure(&self) {
        let mut t = self.throttle.lock().unwrap();
        if t.window_start.elapsed() > THROTTLE_WINDOW {
            t.window_start = Instant::now();
            t.failures = 0;
        }
        t.failures = t.failures.saturating_add(1);
    }

    fn throttled(&self) -> bool {
        let mut t = self.throttle.lock().unwrap();
        if t.window_start.elapsed() > THROTTLE_WINDOW {
            t.window_start = Instant::now();
            t.failures = 0;
        }
        t.failures > THROTTLE_MAX
    }
}

/// The surface credential a request authenticated with. Present only on off-box
/// requests that passed the gate; absent on loopback, which presents none.
///
/// **This is the boundary saying which device**, not the request claiming one — the
/// same property that makes [`Acceptor`] trustworthy. Nothing a sender can write
/// produces it.
#[derive(Debug, Clone)]
pub struct SurfaceId(pub String);

/// Who the device behind this request is registered to, if anybody. `None` for
/// loopback (no credential), for an unregistered device, and for a revoked one.
///
/// **Addressed channels only.** Typing, handing over a file and opening a view are
/// each one person doing one thing, so the device they did it on says who. A
/// microphone is not: it records whatever was audible, and no property of the device
/// changes that. See [`docs/arch/signal-attribution.md`].
pub fn registered_to(data_dir: &std::path::Path, surface: Option<&SurfaceId>) -> Option<String> {
    store::subject_of(data_dir, &surface?.0)
}

/// Paths answered without a session, off-box included.
///
/// `/healthz` and `POST /api/session` are open by definition — one says the
/// process is alive, the other is how anything stops being unauthorized. The
/// upload pair carries its own one-time token, minted by an already-authorized
/// caller, so gating it again would only break the phone handoff.
///
/// `/api/access/request` joins them for the same reason `POST /api/session` is
/// open: a device with no way in is exactly who calls it. Both halves are open —
/// the `POST` that asks and the `GET` that polls — and the `GET` carries the
/// secret it was handed, which is what keeps one asker from reading another's
/// answer. Note the singular: `/api/access/request**s**` is the approver's side
/// and is gated like everything else.
fn open_path(path: &str, method: &axum::http::Method) -> bool {
    use axum::http::Method;
    if path == "/healthz" {
        return true;
    }
    if path == "/api/session" && *method == Method::POST {
        return true;
    }
    if path == "/api/access/request" && matches!(*method, Method::POST | Method::GET) {
        return true;
    }
    path.starts_with("/up/") || path.starts_with("/api/up/")
}

/// The gate. Loopback passes; off-box needs a credential in one of its two
/// presentations.
pub async fn gate(
    State(surfaces): State<std::sync::Arc<Surfaces>>,
    req: Request,
    next: Next,
) -> Response {
    // Fail closed: no marker means no listener claimed this request, and the safe
    // reading of "I don't know where this came from" is "not from here".
    let acceptor = req.extensions().get::<Acceptor>().copied().unwrap_or(Acceptor::OffBox);
    if acceptor == Acceptor::Loopback {
        return next.run(req).await;
    }

    let path = req.uri().path().to_string();
    let method = req.method().clone();
    if open_path(&path, &method) {
        return next.run(req).await;
    }

    // A published view, and the files that view needs to render. Read before the
    // throttle so a visitor who was never asked for a credential cannot spend one
    // person's failure budget, and only for `GET`: a share is something to look at.
    if method == axum::http::Method::GET
        && crate::foundation::server::view_share::grants(
            surfaces.data_dir(),
            req.uri(),
            req.headers(),
        )
    {
        return next.run(req).await;
    }

    if surfaces.throttled() {
        return (StatusCode::TOO_MANY_REQUESTS, "too many failed attempts\n").into_response();
    }

    let headers = req.headers().clone();
    match surfaces.authorize(&headers) {
        Some(a) if a.presented == Presented::Cookie && !Surfaces::csrf_ok(&headers, &method) => (
            StatusCode::FORBIDDEN,
            "a state-changing request needs a non-simple content type or the X-HI-Surface header\n",
        )
            .into_response(),
        Some(a) => {
            // Which device this came in on, for the handlers that attribute what a
            // person sends. Stamped here because this is the only place that knows:
            // the credential is checked once, and a handler cannot re-derive it
            // without the token. A loopback request never reaches this line and so
            // never carries one — it has no credential to be registered.
            let renewed = a.renewed.then(|| cookie(&headers, SESSION_COOKIE)).flatten();
            let secure = over_tls(&headers);
            let mut req = req;
            req.extensions_mut().insert(SurfaceId(a.credential_id));
            let mut res = next.run(req).await;
            // The row behind this cookie was just extended, so say so to the browser
            // holding it — the same token, with a fresh `Max-Age`. Appended rather
            // than set: a handler may have its own `Set-Cookie` to send.
            if let Some(session) = renewed {
                if let Ok(value) =
                    axum::http::HeaderValue::from_str(&session_cookie(&session, secure))
                {
                    res.headers_mut().append(header::SET_COOKIE, value);
                }
            }
            res
        }
        None => unauthorized(&headers, &method),
    }
}

/// What an unauthorized off-box request gets. HTML navigation is answered with a
/// place to ask for access rather than a bare 401 — that page is also how
/// browser-direct onboarding starts.
fn unauthorized(headers: &HeaderMap, method: &axum::http::Method) -> Response {
    let wants_html = *method == axum::http::Method::GET
        && headers
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|a| a.contains("text/html"));
    if wants_html {
        return (StatusCode::UNAUTHORIZED, Html(PAIRING_PAGE.to_string())).into_response();
    }
    (
        StatusCode::UNAUTHORIZED,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"unauthorized","detail":"ask at /api/access/request, or present a credential at /api/session"}"#,
    )
        .into_response()
}

/// The page a browser with no way in gets. Self-contained on purpose: it is served
/// to a browser that is not allowed to fetch `/assets/*` yet.
///
/// **Two ways in, and asking leads.** A code means somebody is standing at the
/// desktop reading one out; asking means they only have to say yes, which is the
/// case this page is usually reached in — a browser whose session lapsed, on a
/// machine that is not the one the agent runs on.
///
/// Every fetch names an absolute path, never a relative one. A relative
/// `api/session` resolves against the *directory* of the current URL, so it would
/// miss on every address that is not a directory, which is most of them.
const PAIRING_PAGE: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Ask to be let in</title>
<style>
  :root { color-scheme: light dark }
  body { font: 16px/1.5 -apple-system, system-ui, sans-serif; margin: 0;
         min-height: 100dvh; display: grid; place-items: center; padding: 24px }
  main { width: min(24rem, 100%); display: grid; gap: 14px }
  form { display: grid; gap: 10px }
  h1 { font-size: 1.1rem; margin: 0 }
  p { margin: 0; opacity: .7; font-size: .9rem }
  input, button { font: inherit; padding: 10px 12px; border-radius: 10px;
                  border: 1px solid color-mix(in srgb, currentColor 25%, transparent) }
  button { cursor: pointer }
  .bad { color: #c0392b; min-height: 1.5em; font-size: .9rem }
  .or { display: flex; align-items: center; gap: 10px; opacity: .45; font-size: .8rem }
  .or::before, .or::after { content: ""; flex: 1; height: 1px;
                            background: color-mix(in srgb, currentColor 30%, transparent) }
  .code { font: 700 2.6rem/1.1 ui-monospace, SFMono-Regular, Menlo, monospace;
          letter-spacing: .12em; text-align: center; margin: 6px 0 }
  [hidden] { display: none !important }
</style></head>
<body><main>
  <div id="ask">
    <h1>Ask to be let in</h1>
    <p>Say which agent this is, and approve it there. Nothing to type across the room.</p>
    <form id="askForm">
      <input id="label" autocomplete="off" autocapitalize="off" spellcheck="false" autofocus
             placeholder="what to call this browser">
      <button type="submit">Let me in</button>
    </form>
    <div class="or">or, if you have a code</div>
    <form id="codeForm">
      <input id="code" autocomplete="off" autocapitalize="off" spellcheck="false"
             placeholder="one-time code">
      <button type="submit">Connect</button>
    </form>
    <div class="bad" id="err"></div>
  </div>

  <div id="waiting" hidden>
    <h1>Waiting to be let in</h1>
    <div class="code" id="shown"></div>
    <p id="waitingWhy">Open <b>Reach</b> on your agent and approve this code. It lasts ten
       minutes.</p>
    <button id="cancel" type="button">Cancel</button>
  </div>
</main>
<script>
const $ = (id) => document.getElementById(id);
const err = $("err");
let polling = null;

async function open_(presented) {
  const res = await fetch("/api/session", {
    method: "POST",
    headers: { "Authorization": "Bearer " + presented, "Content-Type": "application/json" },
    body: JSON.stringify({ label: navigator.userAgent.slice(0, 80) }),
  });
  return res.ok;
}

$("codeForm").addEventListener("submit", async (e) => {
  e.preventDefault();
  err.textContent = "";
  const code = $("code").value.trim();
  if (!code) return;
  if (await open_(code)) location.reload();
  else err.textContent = "That code was not accepted. Ask for a fresh one.";
});

$("askForm").addEventListener("submit", async (e) => {
  e.preventDefault();
  err.textContent = "";
  const label = $("label").value.trim() || navigator.userAgent.slice(0, 80);
  const res = await fetch("/api/access/request", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ label }),
  });
  if (!res.ok) {
    err.textContent = res.status === 429
      ? "Too many devices are already waiting. Try again in a few minutes."
      : "Could not ask this agent to let you in.";
    return;
  }
  const { code, secret } = await res.json();
  $("shown").textContent = code;
  $("ask").hidden = true;
  $("waiting").hidden = false;
  polling = setInterval(() => poll(secret), 2000);
});

$("cancel").addEventListener("click", () => {
  clearInterval(polling);
  $("waiting").hidden = true;
  $("ask").hidden = false;
});

async function poll(secret) {
  const res = await fetch("/api/access/request", {
    headers: { "Authorization": "Bearer " + secret },
  });
  if (!res.ok) return;
  const { state, credential } = await res.json();
  if (state === "approved" && credential) {
    clearInterval(polling);
    // The credential is spent for a cookie and never kept: this page is a browser,
    // and a browser holding the long-lived secret is the one thing the design says
    // it must not.
    if (await open_(credential)) location.reload();
  } else if (state === "denied" || state === "expired") {
    clearInterval(polling);
    $("waiting").hidden = true;
    $("ask").hidden = false;
    err.textContent = state === "denied" ? "That was turned down." : "Nobody answered in time.";
  }
}
</script>
</body></html>
"##;

/// SHA-256, hex. **Not argon2id, deliberately** — a slow KDF exists to frustrate
/// guessing of low-entropy *passwords*, and a 32-byte random credential is not
/// guessable, so argon2 would buy nothing and cost latency on every attach. The
/// broker's argon2id use is correct because those are human passwords. Do not
/// "fix" this to match it.
pub(crate) fn token_hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn hash(token: &str) -> String {
    let digest = token_hash(token);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// 32 bytes of CSPRNG, base64url. Built from two v4 UUIDs rather than a new `rand`
/// dependency — v4 is 122 bits from the OS CSPRNG, so this is 244 bits of entropy
/// in the 32 bytes the design asks for.
pub(crate) fn random_token() -> String {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// A [`Duration`] as the chrono delta the durable timestamps are dated with.
fn delta(d: Duration) -> chrono::TimeDelta {
    chrono::TimeDelta::seconds(d.as_secs() as i64)
}

/// Drop the requests nobody answered in time, so every count and list in this
/// module reads the same map.
fn sweep(map: &mut HashMap<String, AccessRequest>) {
    let now = Instant::now();
    map.retain(|_, r| r.expires > now);
}

/// A 6-digit code for two people to compare across a room. Six because it is read
/// aloud and typed by nobody — it authorizes nothing, so entropy is not what it is
/// for; telling two simultaneous requests apart is.
fn six_digits() -> String {
    format!("{:06}", (uuid::Uuid::new_v4().as_u128() % 1_000_000) as u32)
}

/// Constant-time equality. Compares every byte regardless of where the first
/// difference is, so the time taken says nothing about how close a guess was.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let rest = v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer "))?;
    let rest = rest.trim();
    (!rest.is_empty()).then(|| rest.to_string())
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    for header in headers.get_all(header::COOKIE) {
        let Ok(raw) = header.to_str() else { continue };
        for pair in raw.split(';') {
            let pair = pair.trim();
            if let Some(v) = pair.strip_prefix(name).and_then(|r| r.strip_prefix('=')) {
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// Render the `Set-Cookie` for an exchanged session.
///
/// `Secure` is set only when the request actually arrived over TLS. Always
/// setting it would mean a plain-HTTP off-box test silently fails to keep any
/// session at all — a browser drops a `Secure` cookie on an insecure origin
/// without saying so — which is a worse failure than the one it prevents on a
/// deployment that has no TLS to protect in the first place.
/// `Path=/` and no `Domain=`: a core is the whole of its own origin, and a
/// host-only cookie reaches no other one (`topology.md` § *Addressing*).
pub fn session_cookie(session: &str, secure: bool) -> String {
    let mut c = format!(
        "{SESSION_COOKIE}={session}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}",
        SESSION_TTL.as_secs()
    );
    if secure {
        c.push_str("; Secure");
    }
    c
}

/// Whether the request reached us over TLS. `X-Forwarded-Proto` is the community's
/// word for it in the relayed shape; it is only ever read to decide whether to
/// *add* a cookie attribute, never to decide access.
pub fn over_tls(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|p| p.eq_ignore_ascii_case("https"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, Method};

    fn surfaces() -> Surfaces {
        let p = std::env::temp_dir().join(format!("hi-surfaces-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        Surfaces::new(p)
    }

    fn cookie_headers(session: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{SESSION_COOKIE}={session}")).unwrap(),
        );
        headers
    }

    /// The page is served to a browser that may be anywhere on this core's address,
    /// so every request it makes names an absolute path rather than a relative one.
    #[test]
    fn the_unauthorized_page_fetches_absolute_paths() {
        assert!(PAIRING_PAGE.contains(r#"fetch("/api/session""#));
        assert!(PAIRING_PAGE.contains(r#"fetch("/api/access/request""#));
        assert!(!PAIRING_PAGE.contains(r#"fetch("api/"#));
    }

    #[test]
    fn revoking_drops_the_credential_and_its_sessions() {
        let s = surfaces();
        let (id, token) = s.mint("the phone").unwrap();
        let (session, _, _) = s.exchange(&token, "x").unwrap();
        assert!(s.revoke(&id).unwrap());

        assert_eq!(
            s.authorize(&cookie_headers(&session)),
            None,
            "the session goes with the credential"
        );
        assert_eq!(s.verify(&token), None);
    }

    /// The regression this whole change exists for: the process that minted the
    /// session is gone, and the browser holding its cookie still gets in.
    #[test]
    fn a_session_survives_the_process_that_issued_it() {
        let dir = std::env::temp_dir().join(format!("hi-surfaces-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let session = {
            let before = Surfaces::new(dir.clone());
            let (_, token) = before.mint("the browser").unwrap();
            before.exchange(&token, "x").unwrap().0
        };

        // A second `Surfaces` over the same data dir shares nothing in memory.
        let after = Surfaces::new(dir);
        let authorized = after.authorize(&cookie_headers(&session)).expect("still admitted");
        assert_eq!(authorized.presented, Presented::Cookie);
        assert!(!authorized.renewed, "a session used the same minute is not rewritten");
    }

    /// Renewal is a use far enough into the session's life, and it moves the row.
    /// A use inside [`RENEW_AFTER`] costs no write at all.
    #[test]
    fn a_session_rolls_only_once_it_is_a_day_old() {
        let s = surfaces();
        let (id, token) = s.mint("the browser").unwrap();
        let (session, _, _) = s.exchange(&token, "x").unwrap();

        let (_, renewed) = s.session_of(&session).expect("live");
        assert!(!renewed, "fresh sessions are left alone");

        // Backdate the row to two days into its life by pulling its expiry in.
        let aged = Utc::now() + delta(SESSION_TTL) - delta(Duration::from_secs(2 * 24 * 3600));
        store::session_renew(s.data_dir(), &hash(&session), aged).unwrap();

        let (who, renewed) = s.session_of(&session).expect("still live");
        assert_eq!(who, id);
        assert!(renewed, "and a use two days in extends it");

        let rows = store::session_live(s.data_dir()).unwrap();
        assert_eq!(rows.len(), 1, "extended, not replaced");
        assert!(rows[0].expires_at > aged);
        assert!(!s.session_of(&session).unwrap().1, "and it settles down again");
    }

    #[test]
    fn the_ninth_device_waiting_to_be_let_in_is_refused() {
        let s = surfaces();
        for i in 0..MAX_PENDING {
            assert!(s.ask(&format!("device {i}")).is_some());
        }
        assert!(s.ask("one too many").is_none());
        assert_eq!(s.pending().len(), MAX_PENDING);

        // Answering one makes room: the cap counts who is *waiting*, which is what
        // the approver's screen has to hold.
        let first = s.pending()[0].id.clone();
        assert!(s.deny(&first));
        assert!(s.ask("now there is room").is_some());
    }

    #[test]
    fn an_approved_request_yields_its_credential_exactly_once() {
        let s = surfaces();
        let (id, code, secret) = s.ask("the TV").unwrap();
        assert_eq!(s.pending()[0].code, code, "both screens show the same code");
        assert!(matches!(s.claim(&secret), Claim::Pending));

        assert!(s.approve(&id).unwrap());
        let Claim::Approved(credential) = s.claim(&secret) else {
            panic!("approved");
        };
        assert_eq!(s.verify(&credential).is_some(), true, "and it is a real credential");
        assert!(matches!(s.claim(&secret), Claim::Expired), "the row is gone with the answer");
        assert!(s.pending().is_empty());
        assert!(!s.approve(&id).unwrap(), "and it cannot be approved a second time");
    }

    #[test]
    fn a_denial_reads_as_a_denial_and_a_wrong_secret_costs_a_failure() {
        let s = surfaces();
        let (id, _, secret) = s.ask("the TV").unwrap();
        assert!(s.deny(&id));
        assert!(matches!(s.claim(&secret), Claim::Denied), "not the same as nobody answering");
        assert!(matches!(s.claim(&secret), Claim::Expired));

        let before = s.throttle.lock().unwrap().failures;
        assert!(matches!(s.claim("not a secret anybody was given"), Claim::Expired));
        assert!(
            s.throttle.lock().unwrap().failures > before,
            "guessing spends the budget a wrong credential does"
        );
    }

    #[test]
    fn the_first_boot_credential_is_minted_once() {
        let s = surfaces();
        assert!(s.ensure_first_boot_credential().is_some());
        assert!(s.ensure_first_boot_credential().is_none(), "only when there are none");
    }

    #[test]
    fn csrf_lets_through_exactly_what_a_simple_request_cannot_be() {
        let mut simple = HeaderMap::new();
        simple.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
        assert!(!Surfaces::csrf_ok(&simple, &Method::POST));
        assert!(Surfaces::csrf_ok(&simple, &Method::GET), "reads are not state-changing");

        let mut json = HeaderMap::new();
        json.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
        assert!(Surfaces::csrf_ok(&json, &Method::POST));

        let mut flagged = HeaderMap::new();
        flagged.insert(header::CONTENT_TYPE, HeaderValue::from_static("multipart/form-data"));
        flagged.insert(CSRF_HEADER, HeaderValue::from_static("1"));
        assert!(Surfaces::csrf_ok(&flagged, &Method::POST));

        // A DELETE with no body carries no content type, and a cross-site form
        // cannot issue one — but it also cannot be told apart from `text/plain`
        // here, so it takes the header like the rest.
        assert!(!Surfaces::csrf_ok(&HeaderMap::new(), &Method::DELETE));
    }

    #[test]
    fn open_paths_are_the_named_ones_and_nothing_else() {
        assert!(open_path("/healthz", &Method::GET));
        assert!(open_path("/api/session", &Method::POST));
        assert!(!open_path("/api/session", &Method::GET));
        assert!(open_path("/up/abc", &Method::GET));
        assert!(open_path("/api/up/abc", &Method::POST));
        assert!(!open_path("/", &Method::GET));
        assert!(!open_path("/api/in/text", &Method::POST));
        assert!(!open_path("/mcp", &Method::POST));
        assert!(!open_path("/api/pair", &Method::POST), "pairing mints access; it is not open");

        // Asking to be let in, and polling the answer — a device with no way in is
        // exactly who calls these.
        assert!(open_path("/api/access/request", &Method::POST));
        assert!(open_path("/api/access/request", &Method::GET));
        assert!(!open_path("/api/access/request", &Method::DELETE));
        // The approver's side is not open, and the plural is the only thing between
        // them. A device that could read this list could read every waiting code.
        assert!(!open_path("/api/access/requests", &Method::GET));
        assert!(!open_path("/api/access/requests/abc/approve", &Method::POST));
        assert!(!open_path("/api/access/requests/abc", &Method::DELETE));
    }

    #[test]
    fn constant_time_equality_still_answers_the_question() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }

    #[test]
    fn a_session_cookie_only_claims_secure_when_it_is() {
        assert!(session_cookie("s", true).contains("; Secure"));
        assert!(!session_cookie("s", false).contains("; Secure"));
        // Host-only and whole-origin: the two properties per-core origins depend on.
        assert!(session_cookie("s", false).contains("Path=/"));
        assert!(!session_cookie("s", false).contains("Domain="));
    }
}
