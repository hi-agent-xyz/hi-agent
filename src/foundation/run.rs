//! The **run** — one continuous life of the process, named.
//!
//! Everything the host counts is counted from zero at boot. Session ids come off an
//! atomic that starts at 1 ([`crate::foundation::registry::mint`]), connection ids off
//! another. That is right for what they are — an address for a live agent, dying with
//! the process — and it is exactly why nothing durable may be filed under one alone:
//! session 3 today and session 3 tomorrow are the same string, and a record that files
//! them together is a record that has silently merged two different agents.
//!
//! So a run gets an id, minted once at startup, and durable per-session artefacts are
//! filed under it. It is the missing half of every process-local counter: the counter
//! says *which* within a life, the run says *which life*.
//!
//! **"Run", not "session" or "instance".** Session is taken, and taken by the thing this
//! disambiguates. Instance suggests a deployment — the box, the install, the data dir —
//! which survives restarts and is a different question. A run is one stretch of being
//! awake: it begins at boot, ends at exit, and cannot be resumed. That is the whole of
//! what it means.
//!
//! It is deliberately **not** persisted or derived from anything. A run id read back
//! from disk would be an install id wearing the wrong name.

use std::sync::OnceLock;

/// This process's run id — twelve hex characters, minted on first read.
///
/// Short because it appears in paths a person reads, and random rather than a timestamp
/// because two runs starting in the same second must not collide, and because a sortable
/// id invites sorting by it when the thing you actually want ordered is time.
pub fn id() -> &'static str {
    static RUN: OnceLock<String> = OnceLock::new();
    RUN.get_or_init(|| {
        let raw = uuid::Uuid::new_v4().simple().to_string();
        raw[..12].to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_id_is_stable_within_the_process() {
        // The whole point: everything filed under it during one life agrees.
        assert_eq!(id(), id());
    }

    #[test]
    fn a_run_id_is_short_and_path_safe() {
        let r = id();
        assert_eq!(r.len(), 12);
        assert!(
            r.chars().all(|c| c.is_ascii_hexdigit()),
            "goes in a path and in a log line: {r}"
        );
    }
}
