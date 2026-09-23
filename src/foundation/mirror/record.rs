//! What this core has put in the cache, and when — `<data>/mirror.db`.
//!
//! **Its own file, not a table in `config.db`, because it is as disposable as the
//! bucket it describes.** Deleting it loses nothing: the next miss for each object
//! puts it back. Nothing that has to survive may ever be written into it, and a
//! reader of `config.db` should never have to wonder whether a table there is one
//! of those.
//!
//! **It is what keeps a miss from being an upload per look.** A request the edge
//! should have answered and did not — a rule missing, a bucket not yet
//! consistent — reaches the core carrying a signed session, and without this every
//! such look would put the same object up again (`docs/arch/cache.md`
//! § *Uploading*).
//!
//! One row per request path. `target` is the bucket and prefix it was uploaded
//! under, so a new bucket or a renamed handle reads as "not uploaded" without
//! anything having to be cleared.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension as _, params};

/// `mirrored` was the table of the redirect design, which also tracked lengths and
/// changed objects; it is dropped rather than migrated, which this file allows.
const SCHEMA: &str = "
DROP TABLE IF EXISTS mirrored;
CREATE TABLE IF NOT EXISTS uploaded (
    path        TEXT PRIMARY KEY,
    target      TEXT NOT NULL,
    uploaded_at INTEGER NOT NULL
);
";

/// One path's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// `<bucket>/<prefix>` it was uploaded under.
    pub target: String,
    /// Unix seconds.
    pub uploaded_at: i64,
}

/// The open database. One connection behind a lock: every call is a single
/// indexed statement.
pub struct Records {
    conn: Mutex<Connection>,
}

impl Records {
    pub fn open(data_dir: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(data_dir.join("mirror.db"))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        let conn = Connection::open_in_memory().expect("sqlite");
        conn.execute_batch(SCHEMA).expect("schema");
        Self { conn: Mutex::new(conn) }
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Option<T> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        match f(&conn) {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!(error = %e, "mirror record");
                None
            }
        }
    }

    pub fn get(&self, path: &str) -> Option<Row> {
        self.with(|c| {
            c.query_row(
                "SELECT target, uploaded_at FROM uploaded WHERE path = ?1",
                params![path],
                |r| Ok(Row { target: r.get(0)?, uploaded_at: r.get(1)? }),
            )
            .optional()
        })
        .flatten()
    }

    /// Record an upload, replacing whatever was known about this path.
    pub fn uploaded(&self, path: &str, target: &str, at: i64) {
        self.with(|c| {
            c.execute(
                "INSERT OR REPLACE INTO uploaded (path, target, uploaded_at) VALUES (?1, ?2, ?3)",
                params![path, target, at],
            )
        });
    }

    /// Drop rows whose objects the bucket has expired by now.
    pub fn purge(&self, uploaded_before: i64) {
        self.with(|c| c.execute("DELETE FROM uploaded WHERE uploaded_at < ?1", params![uploaded_before]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_round_trips_and_old_ones_are_purged() {
        let r = Records::in_memory();
        assert_eq!(r.get("/a"), None);
        r.uploaded("/a", "b-1/cache/ana/", 100);
        assert_eq!(r.get("/a"), Some(Row { target: "b-1/cache/ana/".into(), uploaded_at: 100 }));
        r.uploaded("/b", "b-1/cache/ana/", 1000);
        r.purge(500);
        assert_eq!(r.get("/a"), None);
        assert!(r.get("/b").is_some());
    }
}
