//! The credential table — the durable half of [`super`].
//!
//! One row per surface that may reach this core: `(id, label, hash, created_at,
//! last_seen_at, revoked_at)`, in the same `config.db` the vendor credentials and
//! app settings live in. The **label** is the whole reason the columns are shaped
//! this way: a device list a person can read is what makes revocation a decision
//! rather than a guess.

use std::path::Path;

use anyhow::Context;
use chrono::Utc;
use rusqlite::{Connection, params};
use serde::Serialize;

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS surface_credential (
        id           TEXT PRIMARY KEY,
        label        TEXT NOT NULL,
        hash         TEXT NOT NULL,
        created_at   TEXT NOT NULL,
        last_seen_at TEXT NOT NULL DEFAULT '',
        revoked_at   TEXT NOT NULL DEFAULT ''
    );
";

/// One surface, as the device list shows it. Never carries the credential — that
/// exists exactly once, at mint time, in the response that hands it over.
#[derive(Debug, Clone, Serialize)]
pub struct Surface {
    pub id: String,
    pub label: String,
    pub created_at: String,
    /// Empty until this surface has authenticated once.
    pub last_seen_at: String,
}

/// Open `config.db` and ensure our table. Shares the file with
/// [`crate::foundation::credentials`] — same connection settings, additive
/// `IF NOT EXISTS`, so whichever opens first is fine.
fn open(data_dir: &Path) -> anyhow::Result<Connection> {
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("creating data dir {}", data_dir.display()))?;
    let p = crate::foundation::credentials::path(data_dir);
    let conn = Connection::open(&p).with_context(|| format!("opening {}", p.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch(SCHEMA).context("initializing the surface-credential schema")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
    Ok(conn)
}

/// Record a freshly minted credential. Takes the **hash**, never the token: the
/// caller keeps the only copy of that and hands it straight to its owner.
pub fn insert(data_dir: &Path, id: &str, label: &str, hash: &str) -> anyhow::Result<()> {
    let conn = open(data_dir)?;
    conn.execute(
        "INSERT INTO surface_credential (id, label, hash, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![id, label, hash, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// Every live credential as `(id, hash)`, revoked ones excluded.
///
/// Deliberately returns *all* of them rather than looking one up by hash: the
/// caller compares in constant time (see [`super::ct_eq`]), and a `WHERE hash = ?`
/// would put that comparison inside SQLite where we cannot make that claim. The
/// row count is a person's devices — single digits — so the scan is free.
pub fn live(data_dir: &Path) -> anyhow::Result<Vec<(String, String)>> {
    let conn = open(data_dir)?;
    let mut stmt =
        conn.prepare("SELECT id, hash FROM surface_credential WHERE revoked_at = '' ORDER BY id")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// The device list: every live surface, newest first.
pub fn list(data_dir: &Path) -> anyhow::Result<Vec<Surface>> {
    let conn = open(data_dir)?;
    let mut stmt = conn.prepare(
        "SELECT id, label, created_at, last_seen_at FROM surface_credential \
         WHERE revoked_at = '' ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Surface {
            id: r.get(0)?,
            label: r.get(1)?,
            created_at: r.get(2)?,
            last_seen_at: r.get(3)?,
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// How many live credentials exist. Zero is what makes a boot the *first* boot.
pub fn count(data_dir: &Path) -> anyhow::Result<usize> {
    let conn = open(data_dir)?;
    let n: i64 =
        conn.query_row("SELECT COUNT(*) FROM surface_credential WHERE revoked_at = ''", [], |r| {
            r.get(0)
        })?;
    Ok(n.max(0) as usize)
}

/// Stamp `last_seen_at`. Best-effort by design: a surface that reached us and
/// whose bookkeeping write failed has still reached us.
pub fn touch(data_dir: &Path, id: &str) {
    let Ok(conn) = open(data_dir) else { return };
    let _ = conn.execute(
        "UPDATE surface_credential SET last_seen_at = ?2 WHERE id = ?1",
        params![id, Utc::now().to_rfc3339()],
    );
}

/// Revoke one surface. Returns whether a live row was actually revoked, so the
/// caller can answer 404 rather than pretending.
///
/// The row stays, holding its label and dates — a device list that forgets what
/// was revoked cannot answer "did I already remove the old phone?".
pub fn revoke(data_dir: &Path, id: &str) -> anyhow::Result<bool> {
    let conn = open(data_dir)?;
    let n = conn.execute(
        "UPDATE surface_credential SET revoked_at = ?2 WHERE id = ?1 AND revoked_at = ''",
        params![id, Utc::now().to_rfc3339()],
    )?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_list_touch_revoke() {
        let dir = tempdir();
        assert_eq!(count(&dir).unwrap(), 0);

        insert(&dir, "a", "the mac", "hash-a").unwrap();
        insert(&dir, "b", "the phone", "hash-b").unwrap();
        assert_eq!(count(&dir).unwrap(), 2);
        assert_eq!(live(&dir).unwrap(), vec![
            ("a".into(), "hash-a".into()),
            ("b".into(), "hash-b".into())
        ]);

        // Unseen until it authenticates once — the column a device list reads to
        // say "never used".
        assert!(list(&dir).unwrap().iter().all(|s| s.last_seen_at.is_empty()));
        touch(&dir, "a");
        let seen = list(&dir).unwrap();
        assert!(!seen.iter().find(|s| s.id == "a").unwrap().last_seen_at.is_empty());

        assert!(revoke(&dir, "a").unwrap());
        // Idempotent: revoking twice is not an error, but it is not a second event.
        assert!(!revoke(&dir, "a").unwrap());
        assert_eq!(count(&dir).unwrap(), 1);
        assert_eq!(live(&dir).unwrap(), vec![("b".into(), "hash-b".into())]);
        assert!(list(&dir).unwrap().iter().all(|s| s.id != "a"));
    }

    fn tempdir() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("hi-surfaces-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
