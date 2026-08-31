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

/// How long a pairing code stays valid — pick up the other device and scan.
const PAIRING_TTL: Duration = Duration::from_secs(600);

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

/// What a successful authorization was: which credential, and how it was
/// presented. The presentation matters — a bearer header cannot be sent
/// ambiently by another site, so CSRF only applies to the cookie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Presented {
    Bearer,
    Cookie,
}

struct Session {
    credential_id: String,
    expires: Instant,
}

struct Throttle {
    window_start: Instant,
    failures: u32,
}

/// The live half: exchanged sessions, outstanding pairing codes, and the failure
/// throttle. The durable half is [`store`].
pub struct Surfaces {
    data_dir: PathBuf,
    sessions: Mutex<HashMap<String, Session>>,
    pairing: Mutex<HashMap<String, Instant>>,
    throttle: Mutex<Throttle>,
}

impl Surfaces {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            sessions: Mutex::new(HashMap::new()),
            pairing: Mutex::new(HashMap::new()),
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
        let mut sessions = self.sessions.lock().unwrap();
        let now = Instant::now();
        sessions.retain(|_, s| s.expires > now);
        sessions.insert(session.clone(), Session {
            credential_id: credential_id.clone(),
            expires: now + SESSION_TTL,
        });
        Some((session, credential_id, minted))
    }

    /// Revoke a surface: the credential row, and every session standing on it.
    /// Both halves, or a revoked phone keeps working until its session lapses.
    pub fn revoke(&self, id: &str) -> anyhow::Result<bool> {
        let revoked = store::revoke(&self.data_dir, id)?;
        self.sessions.lock().unwrap().retain(|_, s| s.credential_id != id);
        Ok(revoked)
    }

    /// Whether the request carries a live credential, and how it was presented.
    fn authorize(&self, headers: &HeaderMap) -> Option<Presented> {
        if let Some(token) = bearer(headers) {
            if let Some(id) = self.verify(&token) {
                store::touch(&self.data_dir, &id);
                return Some(Presented::Bearer);
            }
            self.note_failure();
        }
        if let Some(session) = cookie(headers, SESSION_COOKIE) {
            let live = {
                let mut sessions = self.sessions.lock().unwrap();
                let now = Instant::now();
                match sessions.get(&session) {
                    Some(s) if s.expires > now => Some(s.credential_id.clone()),
                    Some(_) => {
                        sessions.remove(&session);
                        None
                    }
                    None => None,
                }
            };
            if let Some(id) = live {
                store::touch(&self.data_dir, &id);
                return Some(Presented::Cookie);
            }
            self.note_failure();
        }
        None
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

/// Paths answered without a session, off-box included.
///
/// `/healthz` and `POST /api/session` are open by definition — one says the
/// process is alive, the other is how anything stops being unauthorized. The
/// upload pair carries its own one-time token, minted by an already-authorized
/// caller, so gating it again would only break the phone handoff.
fn open_path(path: &str, method: &axum::http::Method) -> bool {
    if path == "/healthz" {
        return true;
    }
    if path == "/api/session" && *method == axum::http::Method::POST {
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

    if surfaces.throttled() {
        return (StatusCode::TOO_MANY_REQUESTS, "too many failed attempts\n").into_response();
    }

    let headers = req.headers().clone();
    match surfaces.authorize(&headers) {
        Some(Presented::Cookie) if !Surfaces::csrf_ok(&headers, &method) => (
            StatusCode::FORBIDDEN,
            "a state-changing request needs a non-simple content type or the X-HI-Surface header\n",
        )
            .into_response(),
        Some(_) => next.run(req).await,
        None => unauthorized(&headers, &method),
    }
}

/// What an unauthorized off-box request gets. HTML navigation is answered with a
/// place to enter a pairing code rather than a bare 401 — that page is also how
/// browser-direct onboarding starts.
fn unauthorized(headers: &HeaderMap, method: &axum::http::Method) -> Response {
    let wants_html = *method == axum::http::Method::GET
        && headers
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|a| a.contains("text/html"));
    if wants_html {
        return (StatusCode::UNAUTHORIZED, Html(pairing_page(&base_path(headers))))
            .into_response();
    }
    (
        StatusCode::UNAUTHORIZED,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"unauthorized","detail":"pair this surface, then POST /api/session"}"#,
    )
        .into_response()
}

/// The "enter your pairing code" page. Self-contained on purpose: it is served to
/// a browser that is not allowed to fetch `/assets/*` yet.
///
/// `base` is where this core is served from ([`base_path`]) and the form posts to
/// `{base}/api/session` — an absolute path, never a relative one. A relative
/// `api/session` resolves against the *directory* of the current URL, so it only
/// reaches the core at `https://hi-agent.xyz/ana/` and posts to the community's
/// own `/api/session` at `https://hi-agent.xyz/ana`. The address a person is given
/// has no trailing slash, so the relative form was broken in exactly the shape
/// this page exists for.
fn pairing_page(base: &str) -> String {
    // A token swap rather than `format!`: the page is mostly CSS and JS braces,
    // and doubling every one of them to satisfy a format string would make it
    // unreadable for one substitution.
    PAIRING_PAGE.replace("__HI_BASE__", base)
}

const PAIRING_PAGE: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Pair this surface</title>
<style>
  :root { color-scheme: light dark }
  body { font: 16px/1.5 -apple-system, system-ui, sans-serif; margin: 0;
         min-height: 100dvh; display: grid; place-items: center; padding: 24px }
  form { width: min(24rem, 100%); display: grid; gap: 12px }
  h1 { font-size: 1.1rem; margin: 0 }
  p { margin: 0; opacity: .7; font-size: .9rem }
  input, button { font: inherit; padding: 10px 12px; border-radius: 10px;
                  border: 1px solid color-mix(in srgb, currentColor 25%, transparent) }
  button { cursor: pointer }
  .bad { color: #c0392b; min-height: 1.5em; font-size: .9rem }
</style></head>
<body><form id="f">
  <h1>Pair this surface</h1>
  <p>On a device that already has access, open Hi Agent and go to
     <b>Reach &rarr; Devices &rarr; Add a device</b>. Paste the code it shows here.
     It is long, and it lasts ten minutes.</p>
  <input id="code" name="code" autocomplete="off" autocapitalize="off" spellcheck="false"
         autofocus placeholder="pairing code">
  <button type="submit">Connect</button>
  <div class="bad" id="err"></div>
</form>
<script>
document.getElementById("f").addEventListener("submit", async (e) => {
  e.preventDefault();
  const err = document.getElementById("err");
  err.textContent = "";
  const code = document.getElementById("code").value.trim();
  if (!code) return;
  const res = await fetch("__HI_BASE__/api/session", {
    method: "POST",
    headers: { "Authorization": "Bearer " + code, "Content-Type": "application/json" },
    body: JSON.stringify({ label: navigator.userAgent.slice(0, 80) }),
  });
  if (res.ok) location.reload();
  else err.textContent = "That code was not accepted. Ask for a fresh one.";
});
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
pub fn session_cookie(session: &str, path: &str, secure: bool) -> String {
    let mut c = format!(
        "{SESSION_COOKIE}={session}; HttpOnly; SameSite=Lax; Path={path}; Max-Age={}",
        SESSION_TTL.as_secs()
    );
    if secure {
        c.push_str("; Secure");
    }
    c
}

/// Where this core is served from, from `X-Forwarded-Prefix` — `""` at its own
/// root, `"/ana"` when the community routes it by subpath. Never trailing.
///
/// **A path prefix, or nothing.** A value that is relative (`ana`), climbing
/// (`/../admin`) or quote-bearing is not a prefix this core will adopt: it
/// arrives from a hop in front and is pasted into URLs and a cookie `Path`, so a
/// nonsense one is dropped rather than repaired.
pub fn base_path(headers: &HeaderMap) -> String {
    let raw = headers
        .get("x-forwarded-prefix")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim()
        .trim_end_matches('/');
    if raw.is_empty() || !raw.starts_with('/') || raw.contains("..") || raw.contains('"') {
        return String::new();
    }
    raw.to_string()
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

    #[test]
    fn a_credential_exchanges_for_a_session_and_a_wrong_one_does_not() {
        let s = surfaces();
        let (id, token) = s.mint("the mac").unwrap();

        let (session, got_id, minted) = s.exchange(&token, "ignored").unwrap();
        assert_eq!(got_id, id);
        assert!(minted.is_none(), "an existing credential mints nothing new");

        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{SESSION_COOKIE}={session}")).unwrap(),
        );
        assert_eq!(s.authorize(&headers), Some(Presented::Cookie));

        assert!(s.exchange("not-a-credential", "x").is_none());
    }

    #[test]
    fn a_bearer_is_accepted_and_stamps_last_seen() {
        let s = surfaces();
        let (id, token) = s.mint("curl").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        assert_eq!(s.authorize(&headers), Some(Presented::Bearer));
        let listed = store::list(s.data_dir()).unwrap();
        let row = listed.iter().find(|r| r.id == id).unwrap();
        assert!(!row.last_seen_at.is_empty());
    }

    #[test]
    fn a_pairing_code_is_spent_once_and_yields_a_credential() {
        let s = surfaces();
        let code = s.mint_pairing_code();
        let (_, id, minted) = s.exchange(&code, "the phone").unwrap();
        let token = minted.expect("pairing mints a credential to keep");
        assert!(s.exchange(&code, "again").is_none(), "a code is one-time");

        // The minted credential is a real one from here on.
        assert_eq!(s.verify(&token).as_deref(), Some(id.as_str()));
        assert_eq!(store::list(s.data_dir()).unwrap()[0].label, "the phone");
    }

    #[test]
    fn the_pairing_page_posts_to_an_absolute_path_under_the_prefix() {
        // At the core's own root there is no prefix, and `/api/session` is right.
        assert!(pairing_page("").contains(r#"fetch("/api/session""#));

        // Relayed, the same page is served at `https://hi-agent.xyz/ana` — with no
        // trailing slash, because that is the address the community hands out. A
        // relative `api/session` would resolve to the *community's* route; only an
        // absolute path under the prefix reaches this core.
        let relayed = pairing_page("/ana");
        assert!(relayed.contains(r#"fetch("/ana/api/session""#));
        assert!(!relayed.contains(r#"fetch("api/session""#));
    }

    #[test]
    fn the_base_path_takes_a_prefix_and_refuses_nonsense() {
        let mut h = HeaderMap::new();
        assert_eq!(base_path(&h), "");

        h.insert("x-forwarded-prefix", HeaderValue::from_static("/ana"));
        assert_eq!(base_path(&h), "/ana");
        h.insert("x-forwarded-prefix", HeaderValue::from_static("/ana/"));
        assert_eq!(base_path(&h), "/ana");

        // Relative, climbing, or able to break out of the attribute it is pasted
        // into: not a prefix, and not repaired into one.
        for bad in ["ana", "/../admin", r#"/a"onload="#] {
            h.insert("x-forwarded-prefix", HeaderValue::from_str(bad).unwrap());
            assert_eq!(base_path(&h), "", "{bad}");
        }
    }

    #[test]
    fn revoking_drops_the_credential_and_its_sessions() {
        let s = surfaces();
        let (id, token) = s.mint("the phone").unwrap();
        let (session, _, _) = s.exchange(&token, "x").unwrap();
        assert!(s.revoke(&id).unwrap());

        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{SESSION_COOKIE}={session}")).unwrap(),
        );
        assert_eq!(s.authorize(&headers), None, "the session goes with the credential");
        assert_eq!(s.verify(&token), None);
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
    fn open_paths_are_the_named_four_and_nothing_else() {
        assert!(open_path("/healthz", &Method::GET));
        assert!(open_path("/api/session", &Method::POST));
        assert!(!open_path("/api/session", &Method::GET));
        assert!(open_path("/up/abc", &Method::GET));
        assert!(open_path("/api/up/abc", &Method::POST));
        assert!(!open_path("/", &Method::GET));
        assert!(!open_path("/api/in/text", &Method::POST));
        assert!(!open_path("/mcp", &Method::POST));
        assert!(!open_path("/api/pair", &Method::POST), "pairing mints access; it is not open");
    }

    #[test]
    fn constant_time_equality_still_answers_the_question() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }

    #[test]
    fn a_session_cookie_only_claims_secure_when_it_is() {
        assert!(session_cookie("s", "/", true).contains("; Secure"));
        assert!(!session_cookie("s", "/", false).contains("; Secure"));
        assert!(session_cookie("s", "/ana", false).contains("Path=/ana"));
    }
}
