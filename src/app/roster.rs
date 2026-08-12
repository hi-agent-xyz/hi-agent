//! The roster — the cores this app may attach to.
//!
//! One row per entry: `(base URL, credential, label)`, exactly as
//! `docs/arch/topology.md` specifies, plus an id and when it was added. Two
//! consequences fall out of that shape rather than needing rules of their own:
//! **rosters do not sync between apps** (each is paired individually) and
//! **revocation is per-core** (the credential is issued by the core it reaches).
//!
//! Stored in the same `config.db` as everything else this install owns. The
//! credential sits beside the vendor keys already there, under the same
//! owner-only permissions — the OS keychain is where it goes when the native
//! shell owns the process and can reach one (`CLAUDE.md`, phase 2).

use std::path::Path;

use anyhow::Context;
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

/// `app_settings` key naming the attached entry. One at a time: an app renders
/// *a* core, and which one is a property of the app, not of any core.
const KEY_ATTACHED: &str = "roster_attached";

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS roster (
        id         TEXT PRIMARY KEY,
        label      TEXT NOT NULL,
        base_url   TEXT NOT NULL,
        credential TEXT NOT NULL DEFAULT '',
        added_at   TEXT NOT NULL
    );
";

/// One core this app can be with. `credential` is never serialized — the face
/// renders the roster, and the face is exactly what must not hold one.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: String,
    pub label: String,
    pub base_url: String,
    pub added_at: String,
    /// Whether this entry is the one currently attached.
    pub attached: bool,
    /// Whether reaching it needs a credential at all. False for a core on this
    /// machine, which is reached over loopback and is not gated.
    pub credentialed: bool,
    #[serde(skip)]
    pub credential: String,
}

fn open(data_dir: &Path) -> anyhow::Result<Connection> {
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("creating data dir {}", data_dir.display()))?;
    let p = crate::foundation::credentials::path(data_dir);
    let conn = Connection::open(&p).with_context(|| format!("opening {}", p.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch(SCHEMA).context("initializing the roster schema")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
    Ok(conn)
}

/// Add a core. Returns its id.
pub fn add(
    data_dir: &Path,
    label: &str,
    base_url: &str,
    credential: &str,
) -> anyhow::Result<String> {
    let id = uuid::Uuid::now_v7().to_string();
    let conn = open(data_dir)?;
    conn.execute(
        "INSERT INTO roster (id, label, base_url, credential, added_at) VALUES (?1,?2,?3,?4,?5)",
        params![id, label, base_url.trim_end_matches('/'), credential, Utc::now().to_rfc3339()],
    )?;
    Ok(id)
}

/// Every entry, oldest first, with the attached one marked.
pub fn list(data_dir: &Path) -> anyhow::Result<Vec<Entry>> {
    let conn = open(data_dir)?;
    let attached = attached_id(&conn)?;
    let mut stmt = conn.prepare(
        "SELECT id, label, base_url, credential, added_at FROM roster ORDER BY added_at",
    )?;
    let rows = stmt.query_map([], |r| {
        let id: String = r.get(0)?;
        let credential: String = r.get(3)?;
        Ok(Entry {
            attached: attached.as_deref() == Some(id.as_str()),
            credentialed: !credential.is_empty(),
            id,
            label: r.get(1)?,
            base_url: r.get(2)?,
            added_at: r.get(4)?,
            credential,
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// The entry this app is attached to.
///
/// Falls back to the first entry when nothing is recorded or the recorded id has
/// been forgotten, so an app is never attached to nothing while it holds a core
/// it could render. `None` means the roster is genuinely empty.
pub fn attached(data_dir: &Path) -> Option<Entry> {
    let all = list(data_dir).ok()?;
    all.iter().find(|e| e.attached).cloned().or_else(|| all.first().cloned())
}

/// Attach `id`. Errors when there is no such entry, rather than recording an
/// attachment to nothing.
pub fn attach(data_dir: &Path, id: &str) -> anyhow::Result<()> {
    let conn = open(data_dir)?;
    let exists: Option<String> = conn
        .query_row("SELECT id FROM roster WHERE id = ?1", params![id], |r| r.get(0))
        .optional()?;
    anyhow::ensure!(exists.is_some(), "no such roster entry");
    conn.execute(
        "INSERT INTO app_settings (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![KEY_ATTACHED, id],
    )?;
    Ok(())
}

/// Forget a core. Local to this app: the credential it held is now waste, and
/// the core it reached is unaffected — revoking *there* is a separate act, and
/// deliberately so (losing a phone must not need the phone).
pub fn forget(data_dir: &Path, id: &str) -> anyhow::Result<bool> {
    let conn = open(data_dir)?;
    let n = conn.execute("DELETE FROM roster WHERE id = ?1", params![id])?;
    Ok(n > 0)
}

/// Seed the entry for the core this app runs itself, if the roster is empty.
///
/// A first run has a core and no roster, and an app with an empty roster renders
/// nothing — so the local core is entry #1. It carries no credential because
/// loopback is not gated, which is also what makes hosting-and-attaching one act
/// rather than a pairing dance with yourself.
pub fn ensure_local(data_dir: &Path, port: u16) -> anyhow::Result<()> {
    let conn = open(data_dir)?;
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM roster", [], |r| r.get(0))?;
    if n > 0 {
        return Ok(());
    }
    drop(conn);
    let id = add(data_dir, "this machine", &format!("http://127.0.0.1:{port}"), "")?;
    attach(data_dir, &id)?;
    tracing::info!(port, "roster seeded with the core on this machine");
    Ok(())
}

fn attached_id(conn: &Connection) -> anyhow::Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM app_settings WHERE key = ?1", params![KEY_ATTACHED], |r| {
            r.get(0)
        })
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("hi-roster-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        // The roster shares `config.db` with the settings table it writes the
        // attachment into, so that table has to exist first — as it does in a
        // real install, where the credential store opens at startup.
        crate::foundation::credentials::set_setting(&p, "seed", "1").unwrap();
        p
    }

    #[test]
    fn the_local_core_is_entry_one_and_needs_no_credential() {
        let d = dir();
        ensure_local(&d, 12358).unwrap();
        let all = list(&d).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].base_url, "http://127.0.0.1:12358");
        assert!(all[0].attached);
        assert!(!all[0].credentialed, "loopback is not gated");

        // Idempotent: a second boot does not add a second copy of this machine.
        ensure_local(&d, 12358).unwrap();
        assert_eq!(list(&d).unwrap().len(), 1);
    }

    #[test]
    fn attaching_moves_which_core_is_rendered() {
        let d = dir();
        ensure_local(&d, 12358).unwrap();
        let remote = add(&d, "ana", "https://hi-agent.xyz/ana/", "cred").unwrap();
        assert_eq!(
            list(&d).unwrap().iter().find(|e| e.id == remote).unwrap().base_url,
            "https://hi-agent.xyz/ana",
            "a base URL keeps no trailing slash, so joining a path is unambiguous"
        );

        attach(&d, &remote).unwrap();
        let now = attached(&d).unwrap();
        assert_eq!(now.id, remote);
        assert_eq!(now.credential, "cred");
        assert!(attach(&d, "nope").is_err(), "attaching to nothing is not a state");
    }

    #[test]
    fn forgetting_is_local_and_leaves_an_attachment_that_still_resolves() {
        let d = dir();
        ensure_local(&d, 12358).unwrap();
        let remote = add(&d, "ana", "https://hi-agent.xyz/ana", "cred").unwrap();
        attach(&d, &remote).unwrap();
        assert!(forget(&d, &remote).unwrap());
        assert!(!forget(&d, &remote).unwrap());

        // The recorded attachment now names nothing, and the app still renders
        // the core it holds rather than sitting on an empty screen.
        let now = attached(&d).expect("something to be with");
        assert_eq!(now.label, "this machine");
    }

    #[test]
    fn an_empty_roster_is_attached_to_nothing() {
        assert!(attached(&dir()).is_none());
    }
}
