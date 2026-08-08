//! The lossless raw signal store.
//!
//! **Both directions, everything that crossed.** In *and* out: what was said and
//! what was shown, and the clock-driven wakes and worker reports that drove a turn
//! alongside them. Anything a restart would otherwise have to guess at — or repeat —
//! belongs here. Each of those has its own channel ([`Channel::View`],
//! [`Channel::Clock`], [`Channel::Worker`]) so the record says plainly where a
//! signal came from, and so one kind can be excluded from a scan without
//! filtering entry bodies.
//!
//! Every signal in and out is appended to its per-channel day-log,
//! `<data_dir>/memory/raw/<channel>/<YYYY-MM-DD>/<channel>.jsonl`
//! (see [`super::layout`]). One JSON `JournalEntry` per line. A read scans the
//! channel folders a query's time window touches and merges them by `(ts, id)`;
//! compaction and indexing are deferred.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::types::{Channel, JournalEntry};

use super::layout;

#[derive(Clone)]
pub struct Journal {
    inner: Arc<Inner>,
}

struct Inner {
    data_dir: PathBuf,
    /// Serializes all appends so concurrent writers never interleave a line.
    write_lock: Mutex<()>,
}

impl Journal {
    pub async fn open(data_dir: PathBuf) -> anyhow::Result<Self> {
        tokio::fs::create_dir_all(layout::raw_root(&data_dir)).await?;
        Ok(Self {
            inner: Arc::new(Inner {
                data_dir,
                write_lock: Mutex::new(()),
            }),
        })
    }

    /// The data directory this journal writes under — the root for the whole
    /// memory store (`<data_dir>/memory/…`).
    pub fn data_dir(&self) -> &Path {
        &self.inner.data_dir
    }

    /// Append one entry to its per-channel day-log, fsynced before returning.
    pub async fn append(&self, entry: JournalEntry) -> anyhow::Result<()> {
        let channel = entry_channel(&entry);
        let ts = entry_ts(&entry);
        let log_path = layout::channel_log_path(&self.inner.data_dir, channel, ts);

        let mut line = serde_json::to_vec(&entry)?;
        line.push(b'\n');

        let _guard = self.inner.write_lock.lock().await;
        if let Some(dir) = log_path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await?;
        file.write_all(&line).await?;
        file.flush().await?;
        file.sync_data().await?;
        Ok(())
    }

    /// The entries at or after `since`, oldest first, capped at the most recent
    /// `limit`. Entries from all channels are merged by `(ts, id)`.
    pub async fn recent(
        &self,
        since: DateTime<Utc>,
        limit: usize,
    ) -> anyhow::Result<Vec<JournalEntry>> {
        let mut entries = Vec::new();
        read_signal_dirs(&self.inner.data_dir, since, &mut entries).await?;
        entries.sort_by(|a, b| (entry_ts(a), entry_id(a)).cmp(&(entry_ts(b), entry_id(b))));
        entries.retain(|e| entry_ts(e) >= since);
        if entries.len() > limit {
            let drop = entries.len() - limit;
            entries.drain(0..drop);
        }
        Ok(entries)
    }
}

/// The signals with `id` strictly greater than `cursor`, oldest first, capped at
/// the first `limit` past it — the unconsolidated frontier a reflection consumes.
/// `cursor` is an episode's `to_id`; `None` reads from genesis. The cap takes the
/// OLDEST `limit` (not the most recent), so a large
/// backlog drains forward over several reflections rather than flooding one and
/// stranding the frontier.
///
/// A free function over `data_dir` (not a `Journal` method) so the stateless
/// `/mcp` tool handler — which holds only `data_dir` — can resolve the same
/// frontier the reflection orchestration seeded from. Reuses [`read_signal_dirs`];
/// the since-day is derived from the cursor's uuidv7 timestamp so only the
/// touched day-folders are scanned. Ordering and the cursor compare both key on
/// the uuidv7 `id` (the citation key), consistent with the cross-channel merge.
pub async fn after_cursor(
    data_dir: &Path,
    cursor: Option<&str>,
    limit: usize,
) -> anyhow::Result<Vec<JournalEntry>> {
    let since = cursor
        .and_then(uuidv7_ts)
        .unwrap_or_else(|| DateTime::from_timestamp(0, 0).expect("unix epoch is valid"));
    let mut entries = Vec::new();
    read_signal_dirs(data_dir, since, &mut entries).await?;
    entries.sort_by(|a, b| (entry_ts(a), entry_id(a)).cmp(&(entry_ts(b), entry_id(b))));
    if let Some(cur) = cursor {
        entries.retain(|e| entry_id(e) > cur);
    }
    entries.truncate(limit);
    Ok(entries)
}

/// The wall-clock timestamp embedded in a uuidv7 string, or `None` if it doesn't
/// parse / isn't a v7. Used to pick the first day-folder [`after_cursor`] must
/// scan — an id greater than the cursor cannot predate the cursor's millisecond —
/// and by [`crate::mind::memory::decay`] to turn the consolidation cursor into a day.
pub fn uuidv7_ts(id: &str) -> Option<DateTime<Utc>> {
    let (secs, nanos) = Uuid::parse_str(id).ok()?.get_timestamp()?.to_unix();
    DateTime::from_timestamp(secs as i64, nanos)
}

pub fn entry_ts(entry: &JournalEntry) -> DateTime<Utc> {
    match entry {
        JournalEntry::SignalIn { ts, .. } | JournalEntry::SignalOut { ts, .. } => *ts,
    }
}

pub fn entry_channel(entry: &JournalEntry) -> Channel {
    match entry {
        JournalEntry::SignalIn { channel, .. } | JournalEntry::SignalOut { channel, .. } => *channel,
    }
}

pub fn entry_id(entry: &JournalEntry) -> &str {
    match entry {
        JournalEntry::SignalIn { id, .. } | JournalEntry::SignalOut { id, .. } => id,
    }
}

/// Walk every channel folder under `raw/`, appending parsed entries. Each
/// immediate sub-directory is a channel (`text/`, `audio/`, …); `files/` is
/// skipped (artifacts, not signals), `sessions/` is skipped (frame logs, not
/// signals — [`layout::is_signal_dir`]) and `appearance/` self-skips (its
/// day-folders hold state snapshots, not an `appearance.jsonl`). A missing
/// `raw/` yields nothing.
async fn read_signal_dirs(
    data_dir: &Path,
    since: DateTime<Utc>,
    out: &mut Vec<JournalEntry>,
) -> anyhow::Result<()> {
    let root = layout::raw_root(data_dir);
    let mut rd = match tokio::fs::read_dir(&root).await {
        Ok(rd) => rd,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    while let Some(ent) = rd.next_entry().await? {
        if !ent.file_type().await?.is_dir() {
            continue;
        }
        let name = match ent.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        if name == "files" || !layout::is_signal_dir(&name) {
            continue;
        }
        read_channel_dir(&ent.path(), &name, since, out).await?;
    }
    Ok(())
}

/// Read one channel folder's day-shards whose day is `since`'s or later, parsing
/// the `<channel>.jsonl` in each. A channel with no log for a day (e.g.
/// `appearance/`) simply contributes nothing.
async fn read_channel_dir(
    channel_dir: &Path,
    channel_name: &str,
    since: DateTime<Utc>,
    out: &mut Vec<JournalEntry>,
) -> anyhow::Result<()> {
    let since_day = layout::day_key(since);
    let mut rd = match tokio::fs::read_dir(channel_dir).await {
        Ok(rd) => rd,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    let log_file = format!("{channel_name}.jsonl");
    let mut days: Vec<String> = Vec::new();
    while let Some(ent) = rd.next_entry().await? {
        if let Ok(name) = ent.file_name().into_string() {
            // Day-folders are named YYYY-MM-DD, so a byte compare is a date
            // compare: keep `since`'s day and everything after.
            if name.as_str() >= since_day.as_str() {
                days.push(name);
            }
        }
    }
    days.sort();
    for day in days {
        read_log_into(&channel_dir.join(day).join(&log_file), out).await?;
    }
    Ok(())
}

/// Parse one `log.jsonl` into `out`, skipping malformed lines. A missing file is
/// fine — a day-folder may hold only blobs (e.g. un-journaled vision frames).
async fn read_log_into(path: &Path, out: &mut Vec<JournalEntry>) -> anyhow::Result<()> {
    let buf = match tokio::fs::read_to_string(path).await {
        Ok(s) => s,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    for line in buf.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<JournalEntry>(trimmed) {
            Ok(entry) => out.push(entry),
            Err(err) => {
                tracing::warn!(error = %err, line = %trimmed, "skipping malformed journal line");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod after_cursor_tests {
    use super::*;

    async fn append_text(j: &Journal, id: &str, ts: DateTime<Utc>) {
        j.append(JournalEntry::SignalIn {
            id: id.to_string(),
            ts,
            channel: Channel::Text,
            body: "x".into(),
            stream: None,
            media: None,
            origin: None,
        })
        .await
        .unwrap();
    }

    /// Append `n` text signals with strictly increasing uuidv7 ids. A 2ms gap
    /// between appends puts each in its own millisecond, so `now_v7` ids are
    /// monotonic (their trailing random bits only matter within one ms). Returns
    /// the ids in insertion (== sort) order.
    async fn seed(j: &Journal, n: usize) -> Vec<String> {
        let mut ids = Vec::new();
        for _ in 0..n {
            let id = Uuid::now_v7().to_string();
            append_text(j, &id, Utc::now()).await;
            ids.push(id);
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        ids
    }

    fn id_strings(entries: &[JournalEntry]) -> Vec<String> {
        entries.iter().map(|e| entry_id(e).to_string()).collect()
    }

    #[tokio::test]
    async fn genesis_returns_all_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path().to_path_buf()).await.unwrap();
        let ids = seed(&j, 3).await;
        let got = after_cursor(dir.path(), None, 10).await.unwrap();
        assert_eq!(id_strings(&got), ids);
    }

    #[tokio::test]
    async fn cursor_excludes_itself_and_earlier() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path().to_path_buf()).await.unwrap();
        let ids = seed(&j, 4).await;
        let got = after_cursor(dir.path(), Some(&ids[1]), 10).await.unwrap();
        assert_eq!(id_strings(&got), vec![ids[2].clone(), ids[3].clone()]);
    }

    #[tokio::test]
    async fn cap_takes_the_oldest_frontier() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path().to_path_buf()).await.unwrap();
        let ids = seed(&j, 5).await;
        let got = after_cursor(dir.path(), None, 2).await.unwrap();
        assert_eq!(id_strings(&got), vec![ids[0].clone(), ids[1].clone()]);
    }

    #[tokio::test]
    async fn empty_when_cursor_at_tail() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path().to_path_buf()).await.unwrap();
        let ids = seed(&j, 3).await;
        let got = after_cursor(dir.path(), Some(ids.last().unwrap()), 10)
            .await
            .unwrap();
        assert!(got.is_empty());
    }

    #[tokio::test]
    async fn an_empty_store_yields_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let _j = Journal::open(dir.path().to_path_buf()).await.unwrap();
        let got = after_cursor(dir.path(), None, 10).await.unwrap();
        assert!(got.is_empty());
    }

    /// The frame log lives under `raw/sessions/` and is not a channel — a walk of
    /// `raw/` must skip it, or every JSON-RPC frame would read back as a signal.
    #[tokio::test]
    async fn the_frame_log_is_not_read_as_signals() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path().to_path_buf()).await.unwrap();
        let ids = seed(&j, 1).await;
        let frames = layout::session_frames_path(dir.path(), "run-1", 7);
        tokio::fs::create_dir_all(frames.parent().unwrap()).await.unwrap();
        tokio::fs::write(&frames, "{\"jsonrpc\":\"2.0\"}\n").await.unwrap();
        let got = after_cursor(dir.path(), None, 10).await.unwrap();
        assert_eq!(id_strings(&got), ids);
    }
}
