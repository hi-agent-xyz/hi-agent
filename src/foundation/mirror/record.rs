//! What this core has put in the cache, and when — `<data>/mirror.db`.
//!
//! **Its own file, not a table in `config.db`, because it is as disposable as the
//! bucket it describes.** Deleting it loses nothing: every object goes back to
//! being served from here, and the next request for it puts it back. Nothing
//! that has to survive may ever be written into it, and a reader of `config.db`
//! should never have to wonder whether a table there is one of those.
//!
//! One row per request path. `target` is the bucket and prefix it was uploaded
//! under, so a new bucket or a renamed handle reads as "not mirrored" without
//! anything having to be cleared.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension as _, params};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS mirrored (
    path        TEXT PRIMARY KEY,
    target      TEXT NOT NULL,
    len         INTEGER NOT NULL,
    uploaded_at INTEGER NOT NULL,
    changed     INTEGER NOT NULL DEFAULT 0
);
";

/// One path's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// `<bucket>/<prefix>` it was uploaded under.
    pub target: String,
    /// The full length of what was uploaded — checked against what the core
    /// would serve now, at every redirect.
    pub len: u64,
    /// Unix seconds.
    pub uploaded_at: i64,
    /// The bytes at this path were seen to change while its route still called
    /// them immutable. Never redirected or uploaded again: the edge may still hold
    /// the old bytes under this key, and a re-upload would not reach them.
    pub changed: bool,
}

/// The open database. One connection behind a lock: every call is a single
/// indexed statement, far cheaper than opening a connection per request.
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
                "SELECT target, len, uploaded_at, changed FROM mirrored WHERE path = ?1",
                params![path],
                |r| {
                    Ok(Row {
                        target: r.get(0)?,
                        len: r.get::<_, i64>(1)? as u64,
                        uploaded_at: r.get(2)?,
                        changed: r.get::<_, i64>(3)? != 0,
                    })
                },
            )
            .optional()
        })
        .flatten()
    }

    /// Record an upload, replacing whatever was known about this path.
    pub fn uploaded(&self, path: &str, target: &str, len: u64, at: i64) {
        self.with(|c| {
            c.execute(
                "INSERT OR REPLACE INTO mirrored (path, target, len, uploaded_at, changed)
                 VALUES (?1, ?2, ?3, ?4, 0)",
                params![path, target, len as i64, at],
            )
        });
    }

    /// The bytes behind `path` changed under an `immutable` label: stop for good.
    pub fn changed(&self, path: &str) {
        self.with(|c| c.execute("UPDATE mirrored SET changed = 1 WHERE path = ?1", params![path]));
    }

    /// Forget `path` — it is gone from this core, or no longer claims to be
    /// immutable. The bucket's copy is left to its lifecycle; without a row no
    /// redirect will ever name it again.
    pub fn forget(&self, path: &str) {
        self.with(|c| c.execute("DELETE FROM mirrored WHERE path = ?1", params![path]));
    }

    /// Drop rows whose objects the bucket has expired by now. Changed rows stay:
    /// they are the memory that a path must not be mirrored, and cost one row.
    pub fn purge(&self, uploaded_before: i64) {
        self.with(|c| {
            c.execute(
                "DELETE FROM mirrored WHERE changed = 0 AND uploaded_at < ?1",
                params![uploaded_before],
            )
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_round_trips_and_can_be_marked_changed() {
        let r = Records::in_memory();
        assert_eq!(r.get("/a"), None);
        r.uploaded("/a", "b-1/ana/", 10, 100);
        assert_eq!(
            r.get("/a"),
            Some(Row { target: "b-1/ana/".into(), len: 10, uploaded_at: 100, changed: false })
        );
        r.changed("/a");
        assert!(r.get("/a").unwrap().changed);
        r.forget("/a");
        assert_eq!(r.get("/a"), None);
    }

    #[test]
    fn purging_keeps_what_must_not_be_mirrored_again() {
        let r = Records::in_memory();
        r.uploaded("/old", "t", 1, 10);
        r.uploaded("/bad", "t", 1, 10);
        r.changed("/bad");
        r.uploaded("/new", "t", 1, 1000);
        r.purge(500);
        assert_eq!(r.get("/old"), None);
        assert!(r.get("/bad").is_some());
        assert!(r.get("/new").is_some());
    }
}
