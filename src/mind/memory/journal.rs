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

use crate::types::{
    Author, Channel, Content, FileRef, JournalEntry, Message, Origin, Sender, SenderBasis,
};

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
        JournalEntry::Message { message, .. } => message.ts,
        JournalEntry::Presentation { ts, .. }
        | JournalEntry::Observation { ts, .. }
        | JournalEntry::Internal { ts, .. } => *ts,
    }
}

/// Which day-log this line lives in. For a message this is the envelope's routing
/// key, not a field of the [`crate::types::Message`] — see `docs/arch/message.md`.
/// A presentation is always the view channel; it carries no field because there is
/// nothing else it could be.
pub fn entry_channel(entry: &JournalEntry) -> Channel {
    match entry {
        JournalEntry::Message { channel, .. }
        | JournalEntry::Observation { channel, .. }
        | JournalEntry::Internal { channel, .. } => *channel,
        JournalEntry::Presentation { .. } => Channel::View,
    }
}

pub fn entry_id(entry: &JournalEntry) -> &str {
    match entry {
        JournalEntry::Message { message, .. } => &message.id,
        JournalEntry::Presentation { id, .. }
        | JournalEntry::Observation { id, .. }
        | JournalEntry::Internal { id, .. } => id,
    }
}

/// Which person this line came from, or `None` — for the agent's own messages, for
/// machinery where there was no person, and for every entry written before
/// attribution existed. See [`crate::types::Sender`].
pub fn entry_sender(entry: &JournalEntry) -> Option<&crate::types::Sender> {
    match entry {
        JournalEntry::Message { message, .. } => message.from.sender(),
        JournalEntry::Observation { sender, .. } => sender.as_ref(),
        JournalEntry::Presentation { .. } | JournalEntry::Internal { .. } => None,
    }
}

/// The words in an entry, whatever kind it is. A file message has none — its name
/// is [`crate::types::FileRef::name`] — so this is empty rather than invented.
pub fn entry_body(entry: &JournalEntry) -> &str {
    match entry {
        JournalEntry::Message { message, .. } => message.content.text().unwrap_or_default(),
        JournalEntry::Presentation { body, .. }
        | JournalEntry::Observation { body, .. }
        | JournalEntry::Internal { body, .. } => body,
    }
}

/// The stored bytes behind an entry, when it kept any: a spoken clip or a camera
/// minute. A handed file keeps its own resolved ref instead — see
/// [`entry_media_ref`].
pub fn entry_media(entry: &JournalEntry) -> Option<&crate::types::Media> {
    match entry {
        JournalEntry::Message { message, .. } => match &message.content {
            Content::Speech { audio, .. } => audio.as_ref(),
            _ => None,
        },
        JournalEntry::Observation { media, .. } => media.as_ref(),
        _ => None,
    }
}

/// An entry's media ref in the one grammar every reader resolves
/// ([`super::media::signal_ref`]), or `None` when it kept no bytes.
pub fn entry_media_ref(entry: &JournalEntry) -> Option<String> {
    if let JournalEntry::Message { message, .. } = entry
        && let Content::File(f) = &message.content
    {
        return Some(f.reff.clone());
    }
    let m = entry_media(entry)?;
    Some(super::media::signal_ref(entry_channel(entry), entry_ts(entry), &m.file))
}

/// What those bytes are, for a reader deciding whether to look at them.
pub fn entry_mime(entry: &JournalEntry) -> Option<&str> {
    if let JournalEntry::Message { message, .. } = entry
        && let Content::File(f) = &message.content
    {
        return Some(&f.mime);
    }
    entry_media(entry).map(|m| m.mime.as_str())
}

// -----------------------------------------------------------------------------
// Reading lines written before the four-kind split
// -----------------------------------------------------------------------------

/// A line as it may appear on disk: the shape written now, or the `signal_in` /
/// `signal_out` pair written before [`JournalEntry`] split into four kinds.
///
/// **Nothing on disk is rewritten.** Old lines are classified on the way in and the
/// file keeps whatever it already said, which is the same rule attribution follows:
/// the record is what happened, and a migration that edits it is a migration that
/// can be wrong about it.
#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum StoredLine {
    Current(JournalEntry),
    Legacy(LegacyEntry),
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum LegacyEntry {
    SignalIn {
        id: String,
        ts: DateTime<Utc>,
        channel: Channel,
        body: String,
        #[serde(default)]
        stream: Option<String>,
        #[serde(default)]
        media: Option<crate::types::Media>,
        #[serde(default)]
        origin: Option<Origin>,
        #[serde(default)]
        sender: Option<Sender>,
    },
    SignalOut {
        id: String,
        ts: DateTime<Utc>,
        channel: Channel,
        body: String,
        #[serde(default)]
        #[allow(dead_code)]
        media: Option<crate::types::Media>,
        #[serde(default)]
        origin: Option<Origin>,
    },
}

/// Build an entry from the field set an old `signal_in` line carried, classified by
/// the same rule [`LegacyEntry::classify`] applies on read. Fixtures and callers
/// written against the pre-split shape go through here rather than each deciding
/// for themselves what kind a channel implies.
/// Parse one journal line — written now or before the four-kind split — into the
/// kind it is. The read path's own entry point, exposed so a test can assert what a
/// line on disk becomes without going through a file.
pub fn classify_line(line: &str) -> Option<JournalEntry> {
    match serde_json::from_str::<StoredLine>(line).ok()? {
        StoredLine::Current(e) => Some(e),
        StoredLine::Legacy(old) => Some(old.classify()),
    }
}

pub fn legacy_signal_in(
    id: String,
    ts: DateTime<Utc>,
    channel: Channel,
    body: String,
    stream: Option<String>,
    media: Option<crate::types::Media>,
    origin: Option<Origin>,
    sender: Option<Sender>,
) -> JournalEntry {
    LegacyEntry::SignalIn { id, ts, channel, body, stream, media, origin, sender }.classify()
}

/// The same, for an old `signal_out` line.
pub fn legacy_signal_out(
    id: String,
    ts: DateTime<Utc>,
    channel: Channel,
    body: String,
    media: Option<crate::types::Media>,
    origin: Option<Origin>,
) -> JournalEntry {
    LegacyEntry::SignalOut { id, ts, channel, body, media, origin }.classify()
}

impl LegacyEntry {
    /// Sort one old line into the kind it always was.
    ///
    /// The channel is what decides, because before the split it was the only thing
    /// that could: `transcript::from_journal` picked conversation out of the log by
    /// matching exactly `Text`, `Audio` and `File` inbound and `Text` outbound, and
    /// this reproduces that judgement rather than inventing a new one.
    fn classify(self) -> JournalEntry {
        match self {
            LegacyEntry::SignalIn { id, ts, channel, body, stream, media, origin, sender } => {
                // An absent origin is a line older than the field, and on an input
                // channel every one of those was a person. Anything explicitly not
                // human is machinery whatever channel it rode in on.
                if !matches!(origin, None | Some(Origin::Human)) {
                    return JournalEntry::Internal { id, ts, channel, body, origin };
                }
                let from = Author::Person(recover_voice_sender(sender.clone(), &body));
                match channel {
                    Channel::Text => JournalEntry::Message {
                        channel,
                        message: Message { id, ts, from, content: Content::Text(strip_markers(&body)) },
                    },
                    Channel::Audio => JournalEntry::Message {
                        channel,
                        message: Message { id, ts, from, content: Content::Speech { text: strip_markers(&body), audio: media } },
                    },
                    // A file line without media is a framing with nothing behind it —
                    // it was never renderable and is not a message now.
                    Channel::File => match media {
                        Some(m) => {
                            let reff = super::media::signal_ref(Channel::File, ts, &m.file);
                            let name = recover_file_name(&body, &m.file);
                            JournalEntry::Message {
                                channel,
                                message: Message {
                                    id,
                                    ts,
                                    from,
                                    content: Content::File(FileRef { reff, mime: m.mime, name }),
                                },
                            }
                        }
                        None => JournalEntry::Observation {
                            id,
                            ts,
                            channel,
                            body,
                            stream,
                            media: None,
                            sender,
                        },
                    },
                    Channel::Clock | Channel::Worker => {
                        JournalEntry::Internal { id, ts, channel, body, origin }
                    }
                    _ => JournalEntry::Observation { id, ts, channel, body, stream, media, sender },
                }
            }
            LegacyEntry::SignalOut { id, ts, channel, body, origin, .. } => match channel {
                Channel::Text => JournalEntry::Message {
                    channel,
                    message: Message {
                        id,
                        ts,
                        from: Author::Agent,
                        content: Content::Text(body),
                    },
                },
                Channel::View => JournalEntry::Presentation { id, ts, body },
                _ => JournalEntry::Internal { id, ts, channel, body, origin },
            },
        }
    }
}

/// Take the sender back out of a carrier's own `⟨voice: …⟩` marker.
///
/// **A recovery, not a backfill, and the difference is where the name came from.**
/// `signal-attribution.md` forbids deriving a sender from *content* and accepts that
/// old signals are unattributed — because who sent them is not recoverable and
/// inventing it is the failure that document exists to stop. Here it *is*
/// recoverable, verbatim: the voiceprint matched at the boundary and the carrier
/// wrote its conclusion down. It wrote it into the body because at the time the body
/// was the only place there was; reading it back is finding the boundary's own record
/// where the boundary happened to put it.
///
/// The `⟨…⟩` grammar is what makes this safe rather than a parse of prose: only
/// carriers write it and a person cannot type it. Nothing is read out of what
/// anybody *said*.
///
/// Deliberately partial. The live mic wrote the tag only when the **speaker
/// changed**, so within one person's run only the first line carries it and the rest
/// stay unattributed. Carrying a name forward across untagged lines would mean
/// assuming the speaker did not change, and an assumption is exactly what may not be
/// written into this field.
fn recover_voice_sender(sender: Option<Sender>, body: &str) -> Sender {
    // Defer only to a sender that actually names somebody. The hardcoded
    // `Sender::unknown()` the old audio path wrote is *present* without being
    // grounded, and treating its presence as an answer is how a name that had been
    // visible for months became a silhouette.
    if let Some(s) = sender
        && s.subject.is_some()
    {
        return s;
    }
    // `⟨voice: 老王 ~0.82⟩` — the score is the carrier's confidence, not part of
    // anybody's name, and writing it into `subject` would open a facet called
    // "老王 ~0.82" that no later match ever joins.
    match marker_value(body, "voice: ").map(|v| v.split(" ~").next().unwrap_or(v).trim()) {
        Some(name) if !name.is_empty() => Sender {
            subject: Some(name.to_owned()),
            basis: SenderBasis::Cluster,
        },
        _ => Sender { subject: None, basis: SenderBasis::Unknown },
    }
}

/// The file's name as the carrier wrote it: `The user handed you a file: passport.jpg
/// (image/jpeg, 240 KB).` — the one framing `files.rs` has ever used. Falls back to
/// the stored blob's own basename, which is a real name for the bytes even though it
/// is not the one the person chose.
fn recover_file_name(body: &str, blob_rel: &str) -> String {
    let body = strip_markers(body);
    if let Some(rest) = body.split("file: ").nth(1) {
        let name = rest.split(" (").next().unwrap_or("").trim().trim_end_matches('.');
        if !name.is_empty() {
            return name.to_owned();
        }
    }
    blob_rel.rsplit('/').next().unwrap_or(blob_rel).to_owned()
}

/// Strip every `⟨…⟩` marker a carrier wrote for the mind. Under
/// `docs/arch/message.md` nothing writes them any more — carriers set the field —
/// so this runs only over lines old enough to still carry them.
fn strip_markers(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut depth = 0usize;
    for ch in body.chars() {
        match ch {
            '⟨' => depth += 1,
            '⟩' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_owned()
}

/// The text of the first `⟨<prefix>…⟩` marker in `body`, if there is one.
///
/// Scans **every** marker, not just the leading one: a carrier may have written a
/// locator and a voice tag on the same line, in either order, and reading only the
/// first is how the speaker went missing from a line that plainly named them.
fn marker_value<'a>(body: &'a str, prefix: &str) -> Option<&'a str> {
    let mut rest = body;
    while let Some(start) = rest.find('⟨') {
        rest = &rest[start + '⟨'.len_utf8()..];
        let end = rest.find('⟩')?;
        if let Some(v) = rest[..end].strip_prefix(prefix) {
            return Some(v);
        }
        rest = &rest[end + '⟩'.len_utf8()..];
    }
    None
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
        match serde_json::from_str::<StoredLine>(trimmed) {
            Ok(StoredLine::Current(entry)) => out.push(entry),
            Ok(StoredLine::Legacy(old)) => out.push(old.classify()),
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
        j.append(crate::mind::memory::journal::legacy_signal_in(id.to_string(), ts, Channel::Text, "x".to_string(), None, None, None, None))
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
        let frames = layout::session_frames_path(dir.path(), "run-1", &7.into());
        tokio::fs::create_dir_all(frames.parent().unwrap()).await.unwrap();
        tokio::fs::write(&frames, "{\"jsonrpc\":\"2.0\"}\n").await.unwrap();
        let got = after_cursor(dir.path(), None, 10).await.unwrap();
        assert_eq!(id_strings(&got), ids);
    }
}
