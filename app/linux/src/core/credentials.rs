//! Long-lived credentials, in the Secret Service.
//!
//! `docs/arch/topology.md`: "credentials live in the OS keychain, never a plist
//! or localStorage". The Secret Service — GNOME Keyring on both targets,
//! reached through libsecret — is what that sentence means on Linux, the store
//! the Keychain, the Android Keystore and Credential Manager stand in for
//! elsewhere. A file under `~/.config` encrypted with a key kept beside it
//! would protect the bytes not at all and would still be a secret this app
//! invented a home for; the desktop has one.
//!
//! Note what is *not* stored: the session cookie. That belongs to WebKit's
//! cookie manager and is short-lived by design. Only the credential the face
//! never sees is here.

use std::collections::HashMap;

use crate::paths::log;

/// The attribute set every item is filed under. `core` is the roster id, so one
/// lookup finds one core's credential and a person browsing Seahorse sees which
/// entry belongs to what.
fn schema() -> libsecret::Schema {
    libsecret::Schema::new(
        "dev.human-interface.HiAgent",
        libsecret::SchemaFlags::NONE,
        HashMap::from([("core", libsecret::SchemaAttributeType::String)]),
    )
}

/// Store or replace a core's credential.
///
/// The default collection is the login keyring, which is unlocked by the login
/// itself on both targets. A failure here is reported rather than swallowed:
/// the alternative is a core that pairs, works until quit, and cannot be
/// reached again, with nothing anywhere saying why.
pub async fn save(core_id: &str, credential: &str) -> Result<(), glib::Error> {
    libsecret::password_store_future(
        Some(&schema()),
        HashMap::from([("core", core_id)]),
        Some(libsecret::COLLECTION_DEFAULT),
        &format!("Hi Agent — {core_id}"),
        credential,
    )
    .await
}

/// The credential for a core, or `None` when this machine has never held one.
pub async fn load(core_id: &str) -> Option<String> {
    match libsecret::password_lookup_future(Some(&schema()), HashMap::from([("core", core_id)]))
        .await
    {
        Ok(found) => found.map(|password| password.to_string()),
        Err(e) => {
            log(format!("credential lookup for {core_id}: {e}"));
            None
        }
    }
}

/// Forget a core. Idempotent: a credential that is already gone is the state
/// this asks for, so a failed clear is a log line and not an error.
pub async fn delete(core_id: &str) {
    if let Err(e) =
        libsecret::password_clear_future(Some(&schema()), HashMap::from([("core", core_id)])).await
    {
        log(format!("credential clear for {core_id}: {e}"));
    }
}
