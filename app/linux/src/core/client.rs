//! Everything this app says to a core, and the only place an address is parsed.
//!
//! The wire is `docs/api/client.md`: `POST /api/session` exchanges a pairing
//! code or a long-lived credential for a short session cookie, and
//! `GET /healthz` says whether the process answers. Nothing here is
//! Linux-specific except which HTTP stack does the sending — the Kotlin, Swift
//! and C# files of the same name do the same things in the same order.

use std::net::IpAddr;
use std::str::FromStr;

use glib::translate::IntoGlib;
use soup::prelude::*;

use super::models::{CoreError, HealthState, SessionExchange, cookie_name, cookie_value};

/// Two sessions for the whole shell, and no more: libsoup's timeout is a
/// property of the session rather than of a message, so "wait 20s for a pairing
/// exchange" and "give up on a health probe after 4s" cannot share one.
///
/// No cookie jar is attached to either, deliberately. The session belongs to
/// WebKit's cookie manager, and a second copy here would be a second place for
/// it to be stale. iOS makes the same choice with an ephemeral `URLSession`,
/// Android with `CookieJar.NO_COOKIES`, Windows with `UseCookies = false`.
fn session() -> soup::Session {
    thread_local! {
        static SESSION: soup::Session = soup::Session::builder().timeout(20).build();
    }
    SESSION.with(|s| s.clone())
}

/// The health poll runs every ten seconds for as long as the window is open, so
/// this is reused rather than rebuilt — a session per probe would throw away the
/// connection and its TLS handshake each time.
fn probe_session() -> soup::Session {
    thread_local! {
        static PROBE: soup::Session = soup::Session::builder().timeout(4).build();
    }
    PROBE.with(|s| s.clone())
}

/// Parse and canonicalise an address the person typed, and decide whether we
/// are allowed to dial it at all.
///
/// Linux imposes no App Transport Security, so unlike iOS and Android nothing
/// here is forced. The rule is kept anyway, and identically: plain `http://`
/// reaches a core on this network and never a public host. A desktop is not the
/// place to be the lax client, and the rule is a client contract in
/// `docs/api/client.md` terms rather than a platform accommodation.
///
/// The consequence worth knowing is the same one the phones have: a LAN address
/// like `http://192.168.1.5:12358` is not a secure context, so the face gets no
/// microphone and no camera there. `http://127.0.0.1` does.
///
/// Deliberately no DNS: a name is judged by its shape, never by resolving it.
/// Resolution would block the main loop, and a name that resolves inside the LAN
/// today is not a promise about tomorrow.
pub fn normalize_base_url(raw: &str) -> Result<String, CoreError> {
    let value = raw.trim();
    let invalid = || {
        CoreError::InvalidAddress(
            "Enter a core address beginning with http:// or https://.".into(),
        )
    };

    let uri = glib::Uri::parse(value, glib::UriFlags::NONE).map_err(|_| invalid())?;

    let scheme = uri.scheme().to_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(invalid());
    }
    if uri.userinfo().is_some_and(|info| !info.is_empty()) {
        return Err(CoreError::InvalidAddress(
            "A core address cannot carry a username or password.".into(),
        ));
    }
    let host = uri.host().ok_or_else(invalid)?;
    if host.is_empty() {
        return Err(invalid());
    }
    if scheme == "http" && !is_local_host(&host) {
        return Err(CoreError::InvalidAddress(format!(
            "Plain http:// only works for a core on this network. Use https:// to reach {host}."
        )));
    }

    // Query and fragment are dropped and the path reduced to canonical form, so
    // `https://hi-agent.xyz/ana` and `https://hi-agent.xyz/ana/?x=1` are one
    // roster entry rather than two.
    Ok(glib::Uri::join(
        glib::UriFlags::NONE,
        Some(&scheme),
        None,
        Some(&host),
        uri.port(),
        &normalized_path(uri.path().as_str()),
        None,
        None,
    )
    .to_string())
}

/// Whether `http://` to this host is the local-network case.
pub fn is_local_host(host: &str) -> bool {
    let bare = host.trim().trim_matches(['[', ']']).to_lowercase();
    if bare.is_empty() {
        return false;
    }
    if bare == "localhost" || bare.ends_with(".local") || bare.ends_with(".localhost") {
        return true;
    }

    if let Some(literal) = parse_literal(&bare) {
        return match literal {
            IpAddr::V4(v4) => {
                v4.is_loopback() || v4.is_private() || v4.is_link_local()
            }
            // Link-local `fe80::/10` and unique-local `fc00::/7`.
            IpAddr::V6(v6) => {
                let first = v6.octets()[0];
                v6.is_loopback()
                    || (first == 0xfe && (v6.octets()[1] & 0xc0) == 0x80)
                    || (first & 0xfe) == 0xfc
            }
        };
    }

    // A single-label name — `desktop-7f3`, `hi-core` — is only resolvable on the
    // local network, which is exactly the unqualified-hostname case.
    !bare.contains('.')
}

/// Parse a host as an address literal without ever resolving a name. The shape
/// is checked first so a hostname never reaches a resolver.
fn parse_literal(host: &str) -> Option<IpAddr> {
    let looks_v4 = {
        let parts: Vec<&str> = host.split('.').collect();
        parts.len() == 4
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()))
    };
    if !looks_v4 && !host.contains(':') {
        return None;
    }
    IpAddr::from_str(host).ok()
}

fn normalized_path(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        "/".into()
    } else {
        format!("/{trimmed}")
    }
}

/// Append a path to the core's base, keeping any subpath the base carries — a
/// core lives at `https://hi-agent.xyz/ana`, so its session endpoint is
/// `/ana/api/session` and not `/api/session`.
pub fn endpoint(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    format!("{base}/{}", path.trim_matches('/'))
}

/// `POST /api/session`. Presents a pairing code the first time and the stored
/// credential every time after; the core tells the two apart, not us.
pub async fn exchange(
    base_url: &str,
    presented: &str,
    label: &str,
) -> Result<(SessionExchange, soup::Cookie), CoreError> {
    let message = soup::Message::new("POST", &endpoint(base_url, "api/session"))
        .map_err(|e| CoreError::InvalidAddress(e.to_string()))?;

    let body = serde_json::json!({ "label": label.trim() }).to_string();
    message.set_request_body_from_bytes(
        Some("application/json"),
        Some(&glib::Bytes::from_owned(body.into_bytes())),
    );
    message
        .request_headers()
        .expect("a message built by soup always has request headers")
        .append("Authorization", &format!("Bearer {presented}"));

    let bytes = session()
        .send_and_read_future(&message, glib::Priority::DEFAULT)
        .await
        .map_err(|e| {
            let detail = e.to_string();
            CoreError::RequestFailed(if detail.is_empty() {
                "The core could not be reached.".into()
            } else {
                detail
            })
        })?;

    let text = String::from_utf8_lossy(&bytes).into_owned();
    let status = message.status().into_glib() as u32;
    if !(200..300).contains(&status) {
        return Err(CoreError::Rejected {
            status,
            detail: text.trim().to_string(),
        });
    }

    let parsed: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| CoreError::RequestFailed("The core returned an unexpected session response.".into()))?;
    let id = parsed
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            CoreError::RequestFailed("The core returned an unexpected session response.".into())
        })?
        .to_string();
    let credential = parsed
        .get("credential")
        .and_then(|v| v.as_str())
        .filter(|c| !c.is_empty())
        .map(str::to_string);

    // The cookie survives verbatim, which it does not on Windows: libsoup parses
    // the `Set-Cookie` line and WebKit takes the resulting `SoupCookie`, so the
    // core keeps ownership of `Path`, `Max-Age` and `SameSite` exactly as it does
    // on the phones. `CoreWebView.BuildCookie` on Windows has to carry every
    // attribute across by hand and silently drops any the core adds later.
    let cookie = soup::cookies_from_response(&message)
        .into_iter()
        .find(|cookie| {
            cookie_name(cookie).as_deref() == Some(hi_wire::SESSION_COOKIE)
                && cookie_value(cookie).is_some_and(|value| !value.is_empty())
        })
        .ok_or(CoreError::MissingSessionCookie)?;

    Ok((SessionExchange { id, credential }, cookie))
}

/// `GET /healthz` — open, and the only thing the roster polls.
pub async fn health(base_url: &str) -> HealthState {
    let Ok(message) = soup::Message::new("GET", &endpoint(base_url, "healthz")) else {
        return HealthState::Unreachable;
    };
    if probe_session()
        .send_future(&message, glib::Priority::DEFAULT)
        .await
        .is_err()
    {
        return HealthState::Unreachable;
    }
    match message.status().into_glib() {
        200 => HealthState::Here,
        503 => HealthState::Asleep,
        _ => HealthState::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_http_is_confined_to_this_network() {
        assert!(is_local_host("localhost"));
        assert!(is_local_host("127.0.0.1"));
        assert!(is_local_host("192.168.1.5"));
        assert!(is_local_host("10.0.0.2"));
        assert!(is_local_host("172.16.0.1"));
        assert!(is_local_host("169.254.4.4"));
        assert!(is_local_host("desktop-7f3"));
        assert!(is_local_host("core.local"));
        assert!(is_local_host("::1"));
        assert!(is_local_host("fe80::1"));
        assert!(is_local_host("fd00::1"));

        assert!(!is_local_host("hi-agent.xyz"));
        assert!(!is_local_host("8.8.8.8"));
        assert!(!is_local_host("172.32.0.1"));
        assert!(!is_local_host("2001:db8::1"));
    }

    #[test]
    fn a_subpath_core_keeps_its_subpath() {
        assert_eq!(
            endpoint("https://hi-agent.xyz/ana", "api/session"),
            "https://hi-agent.xyz/ana/api/session"
        );
        assert_eq!(
            endpoint("http://127.0.0.1:12358/", "healthz"),
            "http://127.0.0.1:12358/healthz"
        );
    }
}
