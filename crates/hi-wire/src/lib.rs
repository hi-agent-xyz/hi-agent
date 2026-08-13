//! The names an app and a core have to agree on.
//!
//! See [`docs/arch/topology.md`](../../../docs/arch/topology.md). A core issues
//! the session and an app presents it; a core checks the CSRF header and an app
//! sets it. Neither can be changed on one side alone, and the two sides are now
//! separate crates that build for different platforms — so the names live here,
//! where changing one is changing both.
//!
//! Nothing with behaviour belongs in this crate. It has no dependencies on
//! purpose: an iOS app links it, and so does a Docker core.

/// Cookie carrying an exchanged session.
///
/// Two presentations of one credential, because a header alone cannot carry a
/// browser: `EventSource`, browser `WebSocket` and plain navigation can none of
/// them set one, and a core serves all three.
pub const SESSION_COOKIE: &str = "hi_surface";

/// Header a browser-shaped client sets to prove its request could not have been
/// a cross-site *simple* request.
///
/// The cookie is what introduces the exposure a bearer header does not, so this
/// is only ever meaningful alongside [`SESSION_COOKIE`].
pub const CSRF_HEADER: &str = "x-hi-surface";

/// File under a data dir holding the SQLite store.
///
/// Shared because an app and the core it hosts put their tables in the same
/// file when they run on one machine — the roster beside the credentials. An app
/// with no core (a phone) owns the file alone and still uses this name, so a
/// data dir means one thing everywhere.
pub const STORE_FILE: &str = "config.db";
