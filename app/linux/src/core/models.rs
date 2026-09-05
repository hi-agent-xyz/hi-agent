//! What the shell knows about a core, and what it is currently showing.

use serde::{Deserialize, Serialize};

/// One core this app may attach to: an address, a label, and — in the Secret
/// Service rather than here — a credential. `docs/arch/topology.md`: "a roster
/// entry is (base URL, credential, label)".
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RosterEntry {
    /// Stable local id. Also the key the credential is stored under.
    pub id: String,
    /// Canonical base URL, as [`super::client::normalize_base_url`] returned it.
    pub base_url: String,
    /// What the person calls this core. App state; never sent to the core.
    pub label: String,
    /// True for the engine this shell starts or adopts. Exactly one entry may
    /// be local: it is this machine, and there is only one of those.
    #[serde(default)]
    pub is_local: bool,
}

/// What the roster file holds. No secrets — see [`super::credentials`].
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RosterSnapshot {
    #[serde(default)]
    pub entries: Vec<RosterEntry>,
    #[serde(default)]
    pub attached_id: Option<String>,
}

/// The body of `POST /api/session`.
#[derive(Clone, Debug)]
pub struct SessionExchange {
    pub id: String,
    pub credential: Option<String>,
}

/// An attached core: where it is, and the session the face will carry.
///
/// The cookie is `None` for the local engine, and that is not an omission. The
/// core's loopback listener is ungated by construction — `docs/arch/topology.md`
/// § *What is gated* — so exchanging a credential to reach `127.0.0.1` would be
/// the shell authenticating to a door that is open. That reasoning is what
/// deleted `crates/hi-app`; repeating the exchange here would repeat the
/// mistake in a third language.
#[derive(Clone)]
pub struct CoreSession {
    pub entry: RosterEntry,
    pub cookie: Option<soup::Cookie>,
}

impl CoreSession {
    /// Whether the face is already showing this exact session. Compared rather
    /// than reloaded on every state change, because a reload restarts the
    /// conversation's stream for no reason.
    pub fn same_as(&self, other: &CoreSession) -> bool {
        self.entry.base_url == other.entry.base_url
            && self.cookie.as_ref().and_then(cookie_value)
                == other.cookie.as_ref().and_then(cookie_value)
    }
}

/// libsoup's accessors take `&mut self` — a GBoxed getter is a mutable-pointer
/// call in C — so reading one field off a shared cookie means cloning it first.
/// `SoupCookie` is a handful of strings and these are the only two readers.
pub fn cookie_value(cookie: &soup::Cookie) -> Option<String> {
    cookie.clone().value().map(|value| value.to_string())
}

pub fn cookie_name(cookie: &soup::Cookie) -> Option<String> {
    cookie.clone().name().map(|name| name.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HealthState {
    /// `200` — the process answers.
    Here,
    /// `503` — reachable, not ready.
    Asleep,
    /// Answered, but not in a way `/healthz` is documented to.
    Unknown,
    /// Nothing answered.
    Unreachable,
}

/// What the window is showing. One enum rather than a handful of booleans,
/// because the states are exclusive and every pair of booleans eventually
/// represents a state that cannot happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreStage {
    /// No core in the roster at all — first run, before the engine is up.
    Empty,
    /// Starting the local engine, or exchanging a session.
    Connecting,
    /// The face is loaded.
    Ready,
    /// Reachable but not answering yet.
    Waiting,
    /// Something to tell the person, in `AppModel::stage_detail`.
    Failed,
}

/// Everything [`super::client`] can go wrong with. The message is shown to the
/// person, so each one is a sentence rather than a code.
#[derive(Clone, Debug)]
pub enum CoreError {
    /// The address could not be used at all.
    InvalidAddress(String),
    /// The request did not complete.
    RequestFailed(String),
    /// The core answered, and said no.
    Rejected { status: u32, detail: String },
    /// The core answered, and did not set the session cookie.
    MissingSessionCookie,
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidAddress(detail) | Self::RequestFailed(detail) => f.write_str(detail),
            Self::MissingSessionCookie => {
                f.write_str("The core did not return a session cookie.")
            }
            Self::Rejected { status, detail } if detail.is_empty() => {
                write!(f, "The core rejected the request (HTTP {status}).")
            }
            Self::Rejected { status, detail } => {
                write!(f, "The core rejected the request (HTTP {status}): {detail}")
            }
        }
    }
}

impl std::error::Error for CoreError {}
