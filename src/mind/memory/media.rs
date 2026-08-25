//! Media blob storage — out-of-log bytes for audio, vision frames, etc.
//!
//! A blob lives inside the channel-day folder that holds the channel's log, on a
//! wall-clock grid (see [`super::layout::media_rel_path`]): a one-off capture is
//! `<HH>/<MM>-<SS>.<ext>`, a streamed minute `<HH>/<MM>.<ext>`. The day-log
//! records only the path (relative to the channel-day folder) and metadata; the
//! bytes never enter the JSONL stream (which would blow up readers and bloat
//! snapshots).
//!
//! Old bytes fade (see [`super::decay`]): once a day is consolidated and cold, the
//! forgetting pass drops the full grid, keeping only chosen keepsakes under
//! `keep/`. The `.jsonl` line is never rewritten, so [`resolve`] does the
//! best-available lookup on read — original blob, else nearest keepsake, else the
//! caption alone.
//!
//! **A ref addresses one of two roots, and this module resolves both.** A signal's
//! bytes live in the fading raw store above; an artifact the agent *made* lives in
//! [`drive/`](drive_root), which never fades. One grammar spans them
//! ([`resolve_ref`]) because one argument does — `image-to-image` takes a camera
//! still, a handed file, or a picture it drew ten seconds ago, and the caller has no
//! business knowing which store each came from.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Timelike, Utc};
use tokio::io::AsyncWriteExt;

use crate::types::Channel;

use super::layout::{self, MediaSlot};

/// Open the blob for `ts` at the grid slot `slot` dictates, creating the
/// channel-day folder around it, and hand back both the path **relative to that
/// folder** and the open file.
///
/// The half of [`store_blob`] that does not need the bytes up front. A caller
/// holding a whole payload should use `store_blob`; this exists for one that is
/// still receiving it — an arriving body written through as it streams, so the
/// payload never has to fit in memory to be kept. Either way the path grammar is
/// decided here and nowhere else, which is what keeps a streamed artifact
/// addressable by the same ref as a stored one.
///
/// The file is returned **unsynced**: durability is the caller's to declare, at
/// the point it knows it has written everything it meant to.
pub async fn create_blob(
    data_dir: &Path,
    channel: Channel,
    ts: DateTime<Utc>,
    slot: MediaSlot,
    ext: &str,
) -> anyhow::Result<(String, tokio::fs::File)> {
    let dir = layout::channel_day_dir(data_dir, channel, ts);
    let rel = layout::media_rel_path(ts, slot, ext);
    let path = dir.join(&rel);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let f = tokio::fs::File::create(&path).await?;
    Ok((rel, f))
}

/// Persist `bytes` inside the channel-day folder for `ts` — the same
/// folder that holds `<channel>.jsonl` — at the grid slot `slot` dictates, and
/// return the path **relative to that folder**. The caller records it in the
/// entry's `media.file`; a reader resolves it as
/// `channel_day_dir(..).join(media.file)`.
pub async fn store_blob(
    data_dir: &Path,
    channel: Channel,
    ts: DateTime<Utc>,
    slot: MediaSlot,
    ext: &str,
    bytes: &[u8],
) -> anyhow::Result<String> {
    let (rel, mut f) = create_blob(data_dir, channel, ts, slot, ext).await?;
    f.write_all(bytes).await?;
    f.flush().await?;
    f.sync_data().await?;
    Ok(rel)
}

/// The locator a signal's text surface carries: `<channel>/<day>/<rel>`, i.e. the
/// blob's path relative to [`layout::raw_root`]. Wrapped as `⟨ref: …⟩` by whoever
/// writes the sentence.
///
/// **The channel is part of the ref, and that is the whole point.** It used to be
/// `<day>/<rel>`, which is only a path if you already know which channel it came
/// from — and the one tool that opens an image assumed the camera, so a photo the
/// person *handed over* resolved to nothing and reported "it may have faded" about
/// a file sitting on disk. A ref that names its own channel is a path; one that
/// doesn't is a path plus a guess.
pub fn signal_ref(channel: Channel, ts: DateTime<Utc>, rel: &str) -> String {
    format!("{}/{}/{rel}", channel.as_str(), layout::day_key(ts))
}

/// Read a [`signal_ref`] back: `<channel>/<YYYY-MM-DD>/<HH>/<MM>-<SS>.<ext>` into the
/// channel, the timestamp, and the channel-day-relative path [`resolve`] wants.
///
/// **A ref with no channel is read as vision**, because that is the only kind that
/// existed before the channel was part of the grammar. Journals are never rewritten,
/// so old lines keep resolving exactly as they did.
///
/// Only the one-off still grid (`<MM>-<SS>.<ext>`) parses. A streamed minute
/// (`<MM>.<ext>`) is deliberately rejected: no signal offers one as a ref, and
/// accepting the shape would resolve a whole minute of video where a still was meant.
pub fn parse_ref(reff: &str) -> Option<(Channel, DateTime<Utc>, String)> {
    let parts: Vec<&str> = reff.trim().split('/').collect();
    let (channel, date, hh, file) = match parts[..] {
        [c, date, hh, file] => (c.parse::<Channel>().ok()?, date, hh, file),
        [date, hh, file] => (Channel::Vision, date, hh, file),
        _ => return None,
    };
    let (stem, _ext) = file.rsplit_once('.')?;
    let (mm, ss) = stem.split_once('-')?;
    // Reuse the proven RFC3339 parse rather than NaiveDate helpers — a malformed
    // part fails the parse and yields `None`.
    let ts = DateTime::parse_from_rfc3339(&format!("{date}T{hh}:{mm}:{ss}Z"))
        .ok()?
        .with_timezone(&Utc);
    Some((channel, ts, format!("{hh}/{file}")))
}

/// Resolve any ref to bytes on disk — a [`signal_ref`] through the best-available
/// lookup [`resolve`] does, or a [`drive_ref`] through the drive root. `None` when the
/// ref is malformed or nothing is there.
///
/// The `drive/` arm is checked first and explicitly. It would fall through safely
/// anyway (`drive` is no [`Channel`], and the three-segment legacy form needs the
/// first segment to parse as a date) — but "safely" there means *by accident*, via a
/// failed date parse two functions away, and a ref that names a real file must not
/// depend on that.
pub async fn resolve_ref(data_dir: &Path, reff: &str) -> Option<PathBuf> {
    if let Some(rel) = reff.trim().strip_prefix(DRIVE_PREFIX) {
        return resolve_in_drive(data_dir, rel).await;
    }
    let (channel, ts, rel) = parse_ref(reff)?;
    resolve(data_dir, channel, ts, &rel).await
}

/// Best-available path for a signal's media: the original blob if it still exists,
/// else the nearest keepsake [`super::decay`] left when the day faded, else `None`
/// (the signal survives as its text surface alone). `rel` is the entry's
/// `media.file`; `ts` its timestamp. Readers of historical media should resolve
/// through this rather than joining `media.file` directly, so faded days degrade
/// gracefully instead of 404-ing.
pub async fn resolve(
    data_dir: &Path,
    channel: Channel,
    ts: DateTime<Utc>,
    rel: &str,
) -> Option<PathBuf> {
    let dir = layout::channel_day_dir(data_dir, channel, ts);
    let original = dir.join(rel);
    if tokio::fs::try_exists(&original).await.unwrap_or(false) {
        return Some(original);
    }
    nearest_keepsake(&dir.join("keep"), ts).await
}

// ── the drive: artifacts, which do not fade ───────────────────────────────────

/// The prefix that marks a ref as addressing [`drive_root`] rather than a channel-day
/// folder. Includes the separator so `strip_prefix` yields the drive-relative path.
pub const DRIVE_PREFIX: &str = "drive/";

/// Where [`store_artifact`] files what the agent produced, under [`drive_root`].
/// A day folder keeps a long-lived tree browsable; the alternative — one flat
/// directory — is fine for a week and unopenable after a year.
const GENERATED_DIR: &str = "generated";

/// `<data_dir>/drive` — the agent's own filing cabinet (`docs/arch/data.md#drive`).
/// Sibling of the memory store, not part of it: nothing here is consolidated and
/// nothing here fades.
pub fn drive_root(data_dir: &Path) -> PathBuf {
    data_dir.join("drive")
}

/// Reject an empty path and any segment that is empty, `.` or `..`, so a joined path
/// cannot climb out of its root. An absolute path fails too — a leading `/` produces
/// an empty first segment.
///
/// The syntactic half of the guard; [`resolve_in_drive`] adds the half this cannot do.
pub fn safe_rel_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\0')
        && path.split('/').all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// Resolve a drive-relative path to a regular file inside [`drive_root`], or `None`.
pub async fn resolve_in_drive(data_dir: &Path, rel: &str) -> Option<PathBuf> {
    let root = drive_root(data_dir);
    resolve_in_root(&root, rel).await
}

/// Resolve `rel` inside `root`, yielding the path only if it is a regular file that is
/// *still inside the root after canonicalisation*.
///
/// Two guards, because these trees' names come from the agent. [`safe_rel_path`] stops
/// `..`; canonicalising both sides stops what it cannot see — a symlink *inside* the
/// root pointing at `~/.ssh` has no `..` anywhere in its path.
pub async fn resolve_in_root(root: &Path, rel: &str) -> Option<PathBuf> {
    if !safe_rel_path(rel) {
        return None;
    }
    // Canonicalise the root too: on macOS `/var/…` is a symlink to `/private/var/…`,
    // so comparing an un-canonicalised root against a canonicalised file never matches.
    let root = tokio::fs::canonicalize(root).await.ok()?;
    let full = tokio::fs::canonicalize(root.join(rel)).await.ok()?;
    if !full.starts_with(&root) {
        return None;
    }
    tokio::fs::metadata(&full).await.ok()?.is_file().then_some(full)
}

/// Persist an artifact the agent produced — a generated image, a rendered clip — under
/// `drive/generated/<day>/<HHMMSS>-<slug>.<ext>`, and return the [`drive_ref`] that
/// addresses it.
///
/// **The drive, not the raw store, and that is the point.** Raw holds what was
/// *perceived* and [fades](super::decay) once its day is cold; this holds what was
/// *made*, and a picture that evaporates a week after it was drawn is not a picture
/// anybody kept. `docs/arch/data.md#drive` already names this tree the home for
/// "artifacts and bytes it produced or was given".
///
/// `slug` is free text (a prompt) and is reduced to something filename-safe; a
/// same-second collision takes a `-2`, `-3` suffix rather than overwriting, because two
/// images generated in one second is an ordinary batch, not an error.
pub async fn store_artifact(
    data_dir: &Path,
    ts: DateTime<Utc>,
    slug: &str,
    ext: &str,
    bytes: &[u8],
) -> anyhow::Result<String> {
    let dir_rel = format!("{GENERATED_DIR}/{}", layout::day_key(ts));
    let dir = drive_root(data_dir).join(&dir_rel);
    tokio::fs::create_dir_all(&dir).await?;

    let stem = format!("{}-{}", ts.format("%H%M%S"), slugify(slug));
    let mut name = format!("{stem}.{ext}");
    for n in 2..100 {
        if !tokio::fs::try_exists(dir.join(&name)).await.unwrap_or(false) {
            break;
        }
        name = format!("{stem}-{n}.{ext}");
    }

    let path = dir.join(&name);
    let mut f = tokio::fs::File::create(&path).await?;
    f.write_all(bytes).await?;
    f.flush().await?;
    f.sync_data().await?;
    Ok(drive_ref(&format!("{dir_rel}/{name}")))
}

/// The locator for a file in the drive: `drive/<path>`, the counterpart of
/// [`signal_ref`]. Wrapped as `⟨ref: …⟩` by whoever writes the sentence.
pub fn drive_ref(rel: &str) -> String {
    format!("{DRIVE_PREFIX}{rel}")
}

/// Reduce free text to one filename-safe path segment: alphanumerics kept, everything
/// else collapsed to a single `-`, trimmed, capped at 40 characters.
///
/// **Unicode-aware, not ASCII-only.** `char::is_alphanumeric` keeps 中文, so a Chinese
/// prompt yields a Chinese filename instead of an empty one — the whole point of a
/// slug is that a human scanning the tree recognises the file. Text with nothing
/// alphanumeric in it falls back to `image`, since an empty segment fails
/// [`safe_rel_path`].
fn slugify(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars().filter(|c| !c.is_control()) {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
        if out.trim_end_matches('-').chars().count() >= 40 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "image".to_string() } else { out }
}

/// The keepsake in `keep_dir` nearest `ts` — one whose span contains it, else the
/// least-distant. Names are `HHMMSS.<ext>` (a vision still) or
/// `HHMMSS-HHMMSS.<ext>` (an audio clip). `None` when there are no keepsakes.
async fn nearest_keepsake(keep_dir: &Path, ts: DateTime<Utc>) -> Option<PathBuf> {
    let target = i64::from(ts.hour() * 3600 + ts.minute() * 60 + ts.second());
    let mut rd = tokio::fs::read_dir(keep_dir).await.ok()?;
    let mut best: Option<(i64, PathBuf)> = None;
    while let Ok(Some(ent)) = rd.next_entry().await {
        let Ok(name) = ent.file_name().into_string() else { continue };
        let Some((start, end)) = parse_keep_span(&name) else { continue };
        let dist = if target < start {
            start - target
        } else if target > end {
            target - end
        } else {
            0
        };
        if best.as_ref().is_none_or(|(d, _)| dist < *d) {
            best = Some((dist, ent.path()));
        }
    }
    best.map(|(_, p)| p)
}

/// Parse a keepsake filename into its `[start, end]` seconds-of-day span. An
/// instant (`091623.jpg`) is a zero-width span; a clip (`091610-091618.wav`) the
/// two endpoints. `None` if the stem isn't `HHMMSS[-HHMMSS]`.
fn parse_keep_span(name: &str) -> Option<(i64, i64)> {
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    match stem.split_once('-') {
        Some((a, b)) => Some((hms_to_secs(a)?, hms_to_secs(b)?)),
        None => {
            let s = hms_to_secs(stem)?;
            Some((s, s))
        }
    }
}

/// `HHMMSS` → seconds of day, or `None` if it isn't six digits.
fn hms_to_secs(s: &str) -> Option<i64> {
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hh: i64 = s[0..2].parse().ok()?;
    let mm: i64 = s[2..4].parse().ok()?;
    let ss: i64 = s[4..6].parse().ok()?;
    Some(hh * 3600 + mm * 60 + ss)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 25, 14, 23, 7).unwrap()
    }

    #[test]
    fn a_ref_round_trips_through_its_channel() {
        for channel in [Channel::Vision, Channel::File, Channel::Audio] {
            let reff = signal_ref(channel, ts(), "14/23-07.jpg");
            let (back, when, rel) = parse_ref(&reff).expect("round trip");
            assert_eq!(back, channel, "{reff} lost its channel");
            assert_eq!(when, ts());
            assert_eq!(rel, "14/23-07.jpg");
        }
    }

    /// Journals are never rewritten, so refs written before the channel was part of
    /// the grammar keep resolving — and vision is what they all were.
    #[test]
    fn a_ref_without_a_channel_is_read_as_vision() {
        let (channel, when, rel) = parse_ref("2026-06-25/14/23-07.jpg").expect("legacy ref");
        assert_eq!(channel, Channel::Vision);
        assert_eq!(when, ts());
        assert_eq!(rel, "14/23-07.jpg");
    }

    /// The bug this grammar exists for: a handed image resolved under the camera,
    /// found nothing, and reported it as faded.
    #[tokio::test]
    async fn a_handed_image_resolves_where_it_was_stored_not_where_the_camera_keeps_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let rel = store_blob(dir.path(), Channel::File, ts(), MediaSlot::InputOneOff, "png", b"x")
            .await
            .unwrap();

        let reff = signal_ref(Channel::File, ts(), &rel);
        assert!(resolve_ref(dir.path(), &reff).await.is_some(), "{reff} must resolve");

        let as_vision = signal_ref(Channel::Vision, ts(), &rel);
        assert!(
            resolve_ref(dir.path(), &as_vision).await.is_none(),
            "the camera channel holds nothing here — that mistake is what the channel prefix ends"
        );
    }

    #[tokio::test]
    async fn a_stored_artifact_resolves_through_the_ref_it_returns() {
        let dir = tempfile::tempdir().unwrap();
        let reff = store_artifact(dir.path(), ts(), "a red bicycle", "png", b"x").await.unwrap();

        assert_eq!(reff, "drive/generated/2026-06-25/142307-a-red-bicycle.png");
        let path = resolve_ref(dir.path(), &reff).await.expect("must resolve");
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"x");
    }

    /// Two images in one second is an ordinary batch (`n=2`), not an error — the
    /// second must not land on top of the first.
    #[tokio::test]
    async fn a_same_second_artifact_takes_a_suffix_rather_than_the_other_s_place() {
        let dir = tempfile::tempdir().unwrap();
        let first = store_artifact(dir.path(), ts(), "a cat", "png", b"one").await.unwrap();
        let second = store_artifact(dir.path(), ts(), "a cat", "png", b"two").await.unwrap();

        assert_ne!(first, second);
        assert!(second.ends_with("142307-a-cat-2.png"), "{second}");
        let path = resolve_ref(dir.path(), &first).await.unwrap();
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"one", "the first was overwritten");
    }

    /// A slug is derived from a prompt, and a prompt is whatever the person said. A
    /// Chinese one must still name its file — dropping to ASCII would leave every
    /// Chinese generation called `image`.
    #[test]
    fn a_slug_keeps_the_letters_it_is_given() {
        assert_eq!(slugify("A Red Bicycle!"), "a-red-bicycle");
        assert_eq!(slugify("  ...  "), "image");
        assert_eq!(slugify("一只猫"), "一只猫");
        assert!(slugify(&"x".repeat(200)).chars().count() <= 40);
        assert!(safe_rel_path(&slugify("../../etc/passwd")));
    }

    /// The drive tree's names come from the agent, so the ref that addresses it is an
    /// attack surface. Both guards are load-bearing: `..` is syntax, a symlink is not.
    #[tokio::test]
    async fn a_drive_ref_cannot_address_anything_outside_the_drive() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("secret"), b"s").await.unwrap();
        let root = drive_root(dir.path());
        tokio::fs::create_dir_all(&root).await.unwrap();

        assert!(resolve_ref(dir.path(), "drive/../secret").await.is_none(), "climbed out");

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path().join("secret"), root.join("link")).unwrap();
            assert!(
                resolve_ref(dir.path(), "drive/link").await.is_none(),
                "a symlink out of the drive has no `..` for the syntactic guard to catch"
            );
        }
    }

    /// The two roots share one grammar, so each must keep resolving where it lives —
    /// a channel ref must not start reading the drive, or the reverse.
    #[tokio::test]
    async fn the_two_roots_do_not_shadow_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let rel = store_blob(dir.path(), Channel::Vision, ts(), MediaSlot::InputOneOff, "jpg", b"v")
            .await
            .unwrap();
        let signal = signal_ref(Channel::Vision, ts(), &rel);
        let artifact = store_artifact(dir.path(), ts(), "made", "png", b"a").await.unwrap();

        let from_signal = resolve_ref(dir.path(), &signal).await.unwrap();
        assert_eq!(tokio::fs::read(from_signal).await.unwrap(), b"v");
        let from_drive = resolve_ref(dir.path(), &artifact).await.unwrap();
        assert_eq!(tokio::fs::read(from_drive).await.unwrap(), b"a");
    }

    #[test]
    fn parses_keep_names() {
        assert_eq!(parse_keep_span("091623.jpg"), Some((33383, 33383)));
        assert_eq!(parse_keep_span("091610-091618.wav"), Some((33370, 33378)));
        assert_eq!(parse_keep_span("keep"), None);
    }

    #[tokio::test]
    async fn resolves_original_then_keepsake_then_none() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let when = Utc.with_ymd_and_hms(2000, 1, 1, 9, 16, 0).unwrap();
        let rel = store_blob(dir, Channel::Audio, when, MediaSlot::InputStream, "wav", b"x")
            .await
            .unwrap();

        // Original present → returns it.
        let got = resolve(dir, Channel::Audio, when, &rel).await.unwrap();
        assert!(got.ends_with("09/16.wav"));

        // Original gone, a keepsake left → falls back to the keepsake.
        let day = layout::channel_day_dir(dir, Channel::Audio, when);
        tokio::fs::remove_file(day.join(&rel)).await.unwrap();
        tokio::fs::create_dir_all(day.join("keep")).await.unwrap();
        tokio::fs::write(day.join("keep").join("091610-091618.wav"), b"k").await.unwrap();
        let got = resolve(dir, Channel::Audio, when, &rel).await.unwrap();
        assert!(got.ends_with("keep/091610-091618.wav"));

        // Keepsake gone too → caption-only (None).
        tokio::fs::remove_file(day.join("keep").join("091610-091618.wav")).await.unwrap();
        assert!(resolve(dir, Channel::Audio, when, &rel).await.is_none());
    }
}
