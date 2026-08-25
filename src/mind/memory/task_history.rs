//! Prior versions of the files in a task's folder, kept by the pass that already reads them.
//!
//! ## What this is for
//!
//! A task's folder is shared. `general.md` tells a worker to build the deliverable *in* it
//! and to leave its working notes beside the `facet.md` its owner keeps there, and
//! `cognition.md` asks for one worker per task but cannot promise it: a restart leaves a
//! `doing` row with nobody on it, a worker started without a `subject` is invisible on the
//! list that would have said otherwise, and a working session may fan out sub-agents that
//! write wherever it writes.
//!
//! So two hands land in one directory, and the loser's file is replaced whole. It has
//! happened: a 445-line briefing was written and refined over ten minutes, overwritten by a
//! session whose last read of that path predated the file existing, and then read back by
//! its own author as though it were still theirs. No error, no copy, nothing said. What
//! decided the outcome was **which verb the worker reached for** — `apply_patch` checks that
//! the text it is editing is the text on disk and refuses when it isn't; `cat > file <<EOF`,
//! a `tee` and a python `write_text` check nothing at all.
//!
//! The prompts now say to reach for the one that checks ([`general.md`]). This module is
//! what makes being wrong survivable anyway, because guidance moves the verb and cannot keep
//! the bytes.
//!
//! ## Why it lives on the read, not the write
//!
//! There is no code on this write path and there should not be one. Every agent that may
//! write into a task folder has a shell, so a verb the host could be *asked* to use is a
//! door beside an open wall — absent exactly when it is forgotten, and silent about being
//! absent. That is the same argument [`super::tasks::reconcile`] is built on, and this rides
//! on that pass for the same reason: **a pass that re-reads the bytes cannot be walked
//! around, because it reads whatever is actually there however it got there.**
//!
//! ## What it does and does not promise
//!
//! Each pass keeps the bytes it can see. A version written and replaced *between* two passes
//! was never observed and is not recoverable — the exposure window is one brain turn rather
//! than forever, and that is the whole of the improvement. Nothing here prevents a
//! collision, and deliberately so: a lock or a refused write is a gate, and this path has
//! already rejected one gate ([`deliverable: <ref>`](../../../docs/user-journeys/gaps.md))
//! for a reason that still holds — a guard an agent must remember to arm is *silently*
//! disarmed when it forgets, which looks exactly like a clean delivery.
//!
//! ## Cost
//!
//! One `stat` per file per task per pass. Bytes are read only when `(len, mtime)` moved, and
//! written only when the content is one this directory has not held before, so a store where
//! nothing changed costs no reads and no writes at all. There is no size threshold: a cap
//! that silently skips the largest file would drop history for exactly the artifact most
//! expensive to lose, and would look identical to having kept it. Every task folder in every
//! store on hand holds one file, `facet.md`, the largest 6 KB — so a threshold today would
//! be guarding a case that has never occurred, at the price of one more way to be quietly
//! wrong.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::Path;
use std::time::SystemTime;

use chrono::{DateTime, SecondsFormat, Utc};
use sha2::{Digest, Sha256};

/// The folder inside a subject directory that holds prior versions.
///
/// Dot-prefixed so it stays out of the way of the person and the agent both:
/// [`super::tasks`]'s own scan already skips dot entries, and a worker listing its task
/// folder sees the work rather than the archive.
const DIR: &str = ".history";

/// How much of the content digest goes in a filename. Eight hex characters over a store
/// whose largest directory holds single digits of files; collisions here cost one skipped
/// copy of identical-prefix content, not a wrong restore.
const DIGEST_CHARS: usize = 8;

/// What one pass saw in one task directory.
///
/// Held in memory for the life of the process and rebuilt from disk on first sight, so a
/// restart costs one directory listing per task and never a re-copy of content already
/// kept.
#[derive(Debug, Default)]
pub struct DirState {
    /// File name → what its metadata said last look. The pre-check that keeps an untouched
    /// store down to one `stat` per file.
    marks: HashMap<OsString, Mark>,
    /// `<digest>-<name>` for every version already in `DIR`.
    kept: HashSet<String>,
    /// Whether `DIR` has been listed once. Deferred rather than done at construction
    /// because most passes touch nothing and should not pay for a listing.
    listed: bool,
}

/// A file's identity as cheap metadata — the pre-check, never the decision.
///
/// `mtime` alone is not enough (a rewrite within one filesystem timestamp tick keeps it),
/// and `len` alone is not enough (an edit that preserves length is ordinary). Together they
/// miss only a same-length rewrite inside one tick, and the digest below settles anything
/// that gets past them, so the pair is a filter and never the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Mark {
    len: u64,
    mtime: Option<SystemTime>,
}

/// What one look at a directory found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Looked {
    /// Files whose content is not what it was last look, first sight included. Their bytes
    /// as of now are in `DIR`.
    pub changed: Vec<String>,
    /// Files that were here last look and are not here now. Their last observed version is
    /// in `DIR` if a pass ever saw one; nothing here can recover a file created and deleted
    /// between two passes.
    pub gone: Vec<String>,
}

impl Looked {
    pub fn is_quiet(&self) -> bool {
        self.changed.is_empty() && self.gone.is_empty()
    }
}

/// Keep whatever is in `dir` that this directory has not held before, and say what moved.
///
/// Call it **before** anything rewrites the files it covers: the point is to hold the bytes
/// that are about to stop being the ones on disk.
///
/// Only files directly in `dir` are covered — not subdirectories. Task folders are flat in
/// every store on hand, and recursing turns a bounded per-task cost into an unbounded one
/// for a shape nobody writes. A nested deliverable is therefore uncovered, which is a known
/// gap rather than an oversight.
///
/// Never fails the caller. This runs inside the pass that builds the window, and a window
/// dropped because an archive copy could not be written would turn a precaution into an
/// outage — the exact trade [`super::tasks::projection`] already makes for reconciliation.
pub async fn keep(dir: &Path, state: &mut DirState, now: DateTime<Utc>) -> Looked {
    let mut looked = Looked::default();
    let mut present: HashSet<OsString> = HashSet::new();

    let mut rd = match tokio::fs::read_dir(dir).await {
        Ok(rd) => rd,
        // A subject with no directory of its own has nothing to keep.
        Err(_) => return looked,
    };

    while let Ok(Some(entry)) = rd.next_entry().await {
        let name = entry.file_name();
        let Some(text) = name.to_str() else { continue };
        // `DIR` itself, and anything else the filesystem or an editor left behind.
        if text.starts_with('.') {
            continue;
        }
        let Ok(meta) = entry.metadata().await else { continue };
        if !meta.is_file() {
            continue;
        }
        present.insert(name.clone());

        let mark = Mark { len: meta.len(), mtime: meta.modified().ok() };
        if state.marks.get(&name) == Some(&mark) {
            continue;
        }

        let Ok(bytes) = tokio::fs::read(entry.path()).await else { continue };
        let digest = digest_of(&bytes);
        let key = format!("{digest}-{text}");

        // Deferred until something actually looks changed, so an untouched store never
        // pays for it.
        if !state.listed {
            state.kept = list_kept(dir).await;
            state.listed = true;
        }

        // Content this directory has held before is already kept — the common case on the
        // first pass after a restart, when every mark is missing but nothing has moved.
        if !state.kept.contains(&key) {
            if write_copy(dir, &key, &bytes, now).await.is_ok() {
                state.kept.insert(key);
                looked.changed.push(text.to_owned());
            }
        }

        state.marks.insert(name, mark);
    }

    // A name that was here and is not is a loss of its own shape, and the pass is the only
    // thing that would ever notice.
    let vanished: Vec<OsString> =
        state.marks.keys().filter(|name| !present.contains(*name)).cloned().collect();
    for name in vanished {
        if let Some(text) = name.to_str() {
            looked.gone.push(text.to_owned());
        }
        state.marks.remove(&name);
    }

    looked.changed.sort();
    looked.gone.sort();
    looked
}

/// `<when>-<digest>-<name>`, so a listing reads in the order things were seen and still
/// says which version each entry is and what it came from.
///
/// The instant is when the pass **noticed**, never a claim about when the file was written —
/// nothing here can know that, and a name that implied otherwise would be a worse number
/// than none.
async fn write_copy(
    dir: &Path,
    key: &str,
    bytes: &[u8],
    now: DateTime<Utc>,
) -> std::io::Result<()> {
    let home = dir.join(DIR);
    tokio::fs::create_dir_all(&home).await?;
    // Compact, and specifically **carrying no dash of its own**: the dash after it is what
    // [`list_kept`] splits on to recover the key, and a `2026-08-24` in here would put the
    // first dash inside the instant. That is not a cosmetic choice — with the date spelled
    // out, every pass after a restart re-copies every file, because no key ever matches.
    let stamp = now.to_rfc3339_opts(SecondsFormat::Secs, true).replace([':', '-'], "");
    tokio::fs::write(home.join(format!("{stamp}-{key}")), bytes).await
}

/// The `<digest>-<name>` of every version already on disk, so a restart does not re-copy
/// content that is already held.
async fn list_kept(dir: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    let Ok(mut rd) = tokio::fs::read_dir(dir.join(DIR)).await else { return out };
    while let Ok(Some(entry)) = rd.next_entry().await {
        if let Some(text) = entry.file_name().to_str() {
            // `<when>-<digest>-<name>`: drop the instant, keep the rest verbatim. It splits
            // once and never parses further, because the name may itself carry dashes — and
            // the instant, by construction, carries none.
            if let Some((_, key)) = text.split_once('-') {
                out.insert(key.to_owned());
            }
        }
    }
    out
}

fn digest_of(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())[..DIGEST_CHARS].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
    }

    async fn history_of(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let Ok(mut rd) = tokio::fs::read_dir(dir.join(DIR)).await else { return out };
        while let Ok(Some(entry)) = rd.next_entry().await {
            out.push(entry.file_name().to_string_lossy().into_owned());
        }
        out.sort();
        out
    }

    /// The failure this exists for: one session's file replaced whole by another's, with
    /// nothing between the two passes to say so. The replaced bytes have to still be here.
    #[tokio::test]
    async fn a_clobbered_file_is_still_recoverable() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut state = DirState::default();

        tokio::fs::write(dir.join("briefing.md"), b"the first author's 445 lines").await.unwrap();
        let first = keep(dir, &mut state, at("2026-08-24T09:00:00Z")).await;
        assert_eq!(first.changed, vec!["briefing.md"]);

        tokio::fs::write(dir.join("briefing.md"), b"a stranger's whole-file overwrite")
            .await
            .unwrap();
        let second = keep(dir, &mut state, at("2026-08-24T09:11:00Z")).await;
        assert_eq!(second.changed, vec!["briefing.md"], "the replacement is a change");

        let kept = history_of(dir).await;
        assert_eq!(kept.len(), 2, "both versions are on disk: {kept:?}");
        let mut bodies = Vec::new();
        for name in kept {
            bodies.push(tokio::fs::read_to_string(dir.join(DIR).join(name)).await.unwrap());
        }
        bodies.sort();
        assert!(
            bodies.iter().any(|body| body == "the first author's 445 lines"),
            "the overwritten version survived: {bodies:?}"
        );
    }

    /// The cost claim. A pass over a store where nothing moved must not write, and must not
    /// even reach for the listing that dedup needs.
    #[tokio::test]
    async fn an_untouched_directory_costs_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut state = DirState::default();

        tokio::fs::write(dir.join("facet.md"), b"---\nstatus: doing\n---\n\nprose").await.unwrap();
        keep(dir, &mut state, at("2026-08-24T09:00:00Z")).await;
        let after_first = history_of(dir).await;

        for minute in 1..5 {
            let looked = keep(dir, &mut state, at(&format!("2026-08-24T09:0{minute}:00Z"))).await;
            assert!(looked.is_quiet(), "nothing moved, so nothing is reported");
        }
        assert_eq!(history_of(dir).await, after_first, "and nothing more was written");
    }

    /// A restart empties every mark, so the next pass sees every file as new. It must
    /// recognise content it already holds rather than filling the archive with copies.
    #[tokio::test]
    async fn a_restart_does_not_re_copy_what_is_already_held() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        tokio::fs::write(dir.join("facet.md"), b"unchanged across the restart").await.unwrap();
        let mut before = DirState::default();
        keep(dir, &mut before, at("2026-08-24T09:00:00Z")).await;
        let kept = history_of(dir).await;
        assert_eq!(kept.len(), 1);

        let mut after = DirState::default();
        let looked = keep(dir, &mut after, at("2026-08-24T10:00:00Z")).await;
        assert!(looked.is_quiet(), "already-held content is not news");
        assert_eq!(history_of(dir).await, kept, "and not a second copy");
    }

    /// Reverting to a version the directory held before is not a new version. The archive
    /// is content-addressed, so an edit-and-undo leaves two entries, not three.
    #[tokio::test]
    async fn returning_to_an_earlier_version_keeps_no_third_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut state = DirState::default();

        tokio::fs::write(dir.join("notes.md"), b"one").await.unwrap();
        keep(dir, &mut state, at("2026-08-24T09:00:00Z")).await;
        tokio::fs::write(dir.join("notes.md"), b"two").await.unwrap();
        keep(dir, &mut state, at("2026-08-24T09:01:00Z")).await;
        tokio::fs::write(dir.join("notes.md"), b"one").await.unwrap();
        let looked = keep(dir, &mut state, at("2026-08-24T09:02:00Z")).await;

        assert!(looked.is_quiet(), "content already held is not reported as new");
        assert_eq!(history_of(dir).await.len(), 2);
    }

    /// The archive is not part of the work, so it never keeps copies of itself — the shape
    /// that would turn one edit into unbounded growth.
    #[tokio::test]
    async fn the_archive_is_not_itself_archived() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut state = DirState::default();

        tokio::fs::write(dir.join("facet.md"), b"one").await.unwrap();
        for minute in 0..4 {
            tokio::fs::write(dir.join("facet.md"), format!("version {minute}")).await.unwrap();
            keep(dir, &mut state, at(&format!("2026-08-24T09:0{minute}:00Z"))).await;
        }
        assert_eq!(history_of(dir).await.len(), 4, "four versions, and nothing recursive");
    }

    /// A file that disappears is the other shape of loss, and the pass is the only thing
    /// positioned to notice it.
    #[tokio::test]
    async fn a_deleted_file_is_reported_and_its_last_version_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut state = DirState::default();

        tokio::fs::write(dir.join("draft.md"), b"work someone did").await.unwrap();
        keep(dir, &mut state, at("2026-08-24T09:00:00Z")).await;
        tokio::fs::remove_file(dir.join("draft.md")).await.unwrap();

        let looked = keep(dir, &mut state, at("2026-08-24T09:05:00Z")).await;
        assert_eq!(looked.gone, vec!["draft.md"]);
        assert_eq!(history_of(dir).await.len(), 1, "the last version it had is still here");
    }

    /// A name carrying dashes must round-trip through the dedup key, or every pass after a
    /// restart re-copies it.
    #[tokio::test]
    async fn a_dashed_name_still_dedupes_across_a_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        tokio::fs::write(dir.join("terminal-bench-briefing.md"), b"held").await.unwrap();
        keep(dir, &mut DirState::default(), at("2026-08-24T09:00:00Z")).await;
        keep(dir, &mut DirState::default(), at("2026-08-24T10:00:00Z")).await;

        assert_eq!(history_of(dir).await.len(), 1);
    }

    /// A directory that is not there is not an error: a subject can be listed before
    /// anything has been written into it.
    #[tokio::test]
    async fn a_missing_directory_is_quiet() {
        let tmp = tempfile::tempdir().unwrap();
        let mut state = DirState::default();
        let looked = keep(&tmp.path().join("never-created"), &mut state, at("2026-08-24T09:00:00Z"))
            .await;
        assert!(looked.is_quiet());
    }
}
