//! On-disk paths for the raw memory store.
//!
//! Raw is the lossless source of truth, organized by **channel**, then sharded by
//! UTC day. A channel is that sense's complete record; the day-folder keeps reads
//! bounded and makes per-channel fading/archival a single subtree. Each channel-day
//! carries a surface log named for the channel (`text.jsonl`, `audio.jsonl`, …) plus
//! the bytes its signals reference, laid out on a wall-clock grid.
//!
//! ```text
//! <data_dir>/memory/raw/
//!   ├── text/<YYYY-MM-DD>/text.jsonl
//!   ├── audio/<YYYY-MM-DD>/{ audio.jsonl, <HH>/<MM>-<SS>.<ext>, output/<HH>/<MM>.<ext> … }
//!   ├── vision/<YYYY-MM-DD>/{ vision.jsonl, <HH>/<MM>-<SS>.<ext> … }
//!   └── sessions/<run>/<session>.jsonl        (frame logs — see `session_frames_path`)
//! ```
//!
//! Every child of `raw/` is a channel name or `sessions/`, both of which are
//! code-supplied constants — so nothing here percent-encodes a user string.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::foundation::registry::SessionId;
use crate::types::Channel;

/// Where a signal's media bytes sit within its channel-day folder. Input is the
/// default (bare); output lives under `output/`. A one-off capture (a posted
/// clip, a still) gets a second-precision name so it never collides with a
/// streamed minute file; a streamed chunk owns the bare `<HH>/<MM>` minute slot.
#[derive(Debug, Clone, Copy)]
pub enum MediaSlot {
    /// A discrete one-off capture (posted clip / still): `<HH>/<MM>-<SS>.<ext>`.
    InputOneOff,
    /// A minute of an open input stream (mic, camera): `<HH>/<MM>.<ext>`.
    InputStream,
    /// A minute of an output stream (TTS, generated frames): `output/<HH>/<MM>.<ext>`.
    OutputStream,
}

/// `<data_dir>/memory` — the root of the whole memory store (raw + derived).
pub fn memory_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("memory")
}

/// `<memory>/raw` — the root of the lossless store.
pub fn raw_root(data_dir: &Path) -> PathBuf {
    memory_dir(data_dir).join("raw")
}

/// `<data_dir>/prompts/seed` — what each rung brings to a thread it has just opened.
///
/// **Beside the factory prompts, not under `memory/`, and the sibling is the point.**
/// `prompts/factory/` is ours — shipped in the binary, reinstalled every boot. This is the
/// agent's, written about itself. Both are text a session is given at init, which is why
/// they share a parent; the pen is what differs, which is why they do not share a
/// directory. `memory/` holds the *record* — the log, episodes, facets, tasks — and a seed
/// is a digest **over** that record, so it never belonged there. See
/// `docs/arch/data.md#prompts`.
///
/// **An absent file is ordinary**, not an error, and losing one costs a reflection pass
/// rather than knowledge: everything in a seed is a digest of things that are themselves
/// durable. Cognition writes the voice's when it has something worth carrying and leaves it
/// alone otherwise, and has written nothing at all before the first exchange. Every reader
/// degrades to the log tail.
pub fn seed_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("prompts").join("seed")
}

/// `prompts/seed/reaction.md` — what the conversation carries forward, written by
/// Cognition (the voice has no file access to write its own) and handed to the thread it
/// opens.
pub fn reaction_seed_path(data_dir: &Path) -> PathBuf {
    seed_dir(data_dir).join("reaction.md")
}

/// `prompts/seed/<rung>.md` — what a standing rung carries forward (`cognition.md`). Not a
/// full set on purpose: a rung gets a file when it outlives the work *and* cannot re-derive
/// what it lost. `rung` is a code-supplied name, never a user string.
pub fn rung_seed_path(data_dir: &Path, rung: &str) -> PathBuf {
    seed_dir(data_dir).join(format!("{rung}.md"))
}

/// `prompts/seed/proactivity.md` — the learned read on speaking up unprompted: which
/// subjects the person welcomes a proactive word on, and which they don't. A derived
/// projection — the reflection pass regenerates it from how the agent's own unprompted
/// utterances landed; the agent only reads it, to judge whether breaking silence clears the
/// bar. Absent ⇒ nothing proven ⇒ stay cautious. Regenerated wholesale, never patched, and
/// a seed by every test that word has to pass.
pub fn proactivity_path(data_dir: &Path) -> PathBuf {
    seed_dir(data_dir).join("proactivity.md")
}

/// Move the pre-`prompts/seed/` files into it, once, if they are still where they were.
///
/// Called at boot before anything reads them. Best-effort and silent about absence, which
/// is every boot after the first: a seed that fails to move is rebuilt, not lost, which is
/// exactly what makes this migration cheap.
pub fn migrate_seeds(data_dir: &Path) {
    let moves = [
        (memory_dir(data_dir).join("prompts").join("conversation.md"), reaction_seed_path(data_dir)),
        (memory_dir(data_dir).join("prompts").join("cognition.md"), rung_seed_path(data_dir, "cognition")),
        (memory_dir(data_dir).join("proactivity.md"), proactivity_path(data_dir)),
    ];
    if !moves.iter().any(|(from, to)| from.is_file() && !to.exists()) {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(seed_dir(data_dir)) {
        tracing::warn!(error = %e, "could not create the seed directory");
        return;
    }
    for (from, to) in moves {
        if !from.is_file() || to.exists() {
            continue;
        }
        match std::fs::rename(&from, &to) {
            Ok(()) => tracing::info!(from = %from.display(), to = %to.display(), "moved a seed"),
            Err(e) => tracing::warn!(from = %from.display(), error = %e, "could not move a seed"),
        }
    }
    // The directory it left behind held nothing else.
    let _ = std::fs::remove_dir(memory_dir(data_dir).join("prompts"));
}

/// `<memory>/episodes` — derived event bundles.
pub fn episodes_dir(data_dir: &Path) -> PathBuf {
    memory_dir(data_dir).join("episodes")
}

/// `<memory>/facets` — derived current-understanding of subjects.
pub fn facets_dir(data_dir: &Path) -> PathBuf {
    memory_dir(data_dir).join("facets")
}

/// `<memory>/reflexes` — taught quick-action reflexes (one `<id>.json` each). The
/// deepest stage of the memory gradient: a grooved action the fast-path fires
/// without the mind. Written by the `record_reflex` tool, read by the invoke path.
pub fn reflexes_dir(data_dir: &Path) -> PathBuf {
    memory_dir(data_dir).join("reflexes")
}

/// `<memory>/raw/sessions/<run>/<session>.jsonl` — **one agent session's stream,
/// verbatim**.
///
/// Every JSON-RPC line that crossed to or from that session's subprocess, in order,
/// uninterpreted — including the `initialize`/`session/new` handshake that precedes the
/// protocol's own session id. This is what
/// `docs/arch/foundation.md#full-frames-not-modelled-events` asks for and what
/// verification reads: a tool call's `raw_input`/`raw_output`/`content` live here and
/// nowhere else.
///
/// **Per session, because a session is the thing that gets replayed.** A day file mixes
/// every agent alive that day and makes reading one of them back a filtering exercise;
/// one subprocess hosts exactly one session, so the natural unit is already there.
///
/// **Under a run id, because session ids repeat every boot**
/// ([`crate::foundation::run`]). The three rungs are `cognition` and its two siblings in
/// every run by construction, and a worker slug comes round again whenever the same errand
/// does. Without the run, today's `cognition` and tomorrow's are one file, and a record
/// that silently merges two different agents is worse than no record.
///
/// **`session` is a [`SessionId`](crate::foundation::registry::SessionId), and that type is
/// what keeps this a filename rather than a path.** It admits letters, digits and `-` and
/// nothing else, so no value of it can climb out of the run directory. This function must
/// not be handed a bare `&str` — the reason `GET /api/workers/{id}/frames` is safe is that
/// the id is parsed into that type before it arrives here.
///
/// Under `raw/` because foundation holds that pen and the rule there is *written before
/// anything reacts to it*.
pub fn session_frames_path(data_dir: &Path, run: &str, session: &SessionId) -> PathBuf {
    raw_root(data_dir).join(SESSIONS_DIR).join(run).join(format!("{session}.jsonl"))
}

/// The child of `raw/` that is **not** a channel: foundation's own per-session frame
/// log ([`session_frames_path`]). Every other child is [`Channel::as_str`] or
/// `appearance`, and all of those are code-supplied constants — so a walker can tell
/// them apart by name with no ambiguity and no sidecar to consult.
pub const SESSIONS_DIR: &str = "sessions";

/// Whether a directory name directly under `raw/` holds journalled signals — i.e.
/// anything but the frame log. See [`SESSIONS_DIR`].
pub fn is_signal_dir(name: &str) -> bool {
    name != SESSIONS_DIR
}

/// `<raw>/<channel>/<YYYY-MM-DD>` — the channel-day folder a signal at `ts`
/// belongs to, holding that day's surface log and the bytes its signals
/// reference. The parent of both the log and the byte grid.
pub fn channel_day_dir(data_dir: &Path, channel: Channel, ts: DateTime<Utc>) -> PathBuf {
    raw_root(data_dir).join(channel.as_str()).join(day_key(ts))
}

/// `<channel>/<date>/<channel>.jsonl` — the day's surface log for one channel,
/// named for the channel so the file is self-describing even detached from its
/// folder.
pub fn channel_log_path(data_dir: &Path, channel: Channel, ts: DateTime<Utc>) -> PathBuf {
    channel_day_dir(data_dir, channel, ts).join(format!("{}.jsonl", channel.as_str()))
}

/// `<raw>/appearance/<YYYY-MM-DD>` — the day-folder for the screen-state history.
/// Appearance is a state channel, not an event stream: it holds timestamped
/// whole-state snapshots (`appearance-<HHMMSSZ>.json`), not a `<channel>.jsonl`, so
/// it is reached through this helper rather than [`channel_day_dir`] (there is no
/// `Channel::Appearance`).
pub fn appearance_day_dir(data_dir: &Path, ts: DateTime<Utc>) -> PathBuf {
    raw_root(data_dir).join("appearance").join(day_key(ts))
}

/// The byte path for a signal's media **relative to its channel-day folder**, by
/// slot (see [`MediaSlot`]). Stored verbatim in the entry's `media.file`, so a
/// reader resolves it as `channel_day_dir(..).join(media.file)`.
pub fn media_rel_path(ts: DateTime<Utc>, slot: MediaSlot, ext: &str) -> String {
    let hh = ts.format("%H");
    let mm = ts.format("%M");
    match slot {
        MediaSlot::InputOneOff => format!("{hh}/{mm}-{}.{ext}", ts.format("%S")),
        MediaSlot::InputStream => format!("{hh}/{mm}.{ext}"),
        MediaSlot::OutputStream => format!("output/{hh}/{mm}.{ext}"),
    }
}

/// The lexically-sortable day key (`YYYY-MM-DD`, UTC) used as a day-folder name.
pub fn day_key(ts: DateTime<Utc>) -> String {
    ts.format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seeds sit beside the factory prompts, never under `memory/`. `memory/` holds the
    /// record; a seed is a digest **over** the record, and the sibling directory is what
    /// says which pen wrote it.
    #[test]
    fn seeds_live_beside_the_factory_prompts_not_under_memory() {
        let root = Path::new("/tmp/jack.hi");
        assert_eq!(seed_dir(root), root.join("prompts").join("seed"));
        assert!(!seed_dir(root).starts_with(memory_dir(root)));
        assert_eq!(
            rung_seed_path(root, "cognition"),
            root.join("prompts").join("seed").join("cognition.md")
        );
        assert_eq!(
            reaction_seed_path(root),
            root.join("prompts").join("seed").join("reaction.md")
        );
        assert_eq!(
            proactivity_path(root),
            root.join("prompts").join("seed").join("proactivity.md")
        );
    }
}

#[cfg(test)]
mod seed_migration_tests {
    use super::*;

    /// The three files a pre-`prompts/seed/` install left under `memory/`, brought forward
    /// where their readers now look. A seed that stayed behind would not error — it would
    /// read as an agent that had never written one, which is the quiet kind of wrong.
    #[test]
    fn the_old_seed_files_move_and_the_empty_directory_goes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let old = memory_dir(root).join("prompts");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("conversation.md"), "he prefers Chinese").unwrap();
        std::fs::write(old.join("cognition.md"), "the brain's").unwrap();
        std::fs::write(memory_dir(root).join("proactivity.md"), "- delivering — `muted`").unwrap();

        migrate_seeds(root);

        assert_eq!(std::fs::read_to_string(reaction_seed_path(root)).unwrap(), "he prefers Chinese");
        assert_eq!(std::fs::read_to_string(rung_seed_path(root, "cognition")).unwrap(), "the brain's");
        assert!(std::fs::read_to_string(proactivity_path(root)).unwrap().contains("muted"));
        assert!(!old.exists(), "the directory it left held nothing else");
    }

    /// Idempotent, because it runs on every boot: a second pass finds nothing to move, and
    /// a seed already written under the new name is never overwritten by a stale one.
    #[test]
    fn migrating_twice_changes_nothing_and_never_clobbers() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let old = memory_dir(root).join("prompts");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("conversation.md"), "stale").unwrap();
        std::fs::create_dir_all(seed_dir(root)).unwrap();
        std::fs::write(reaction_seed_path(root), "current").unwrap();

        migrate_seeds(root);
        migrate_seeds(root);

        assert_eq!(std::fs::read_to_string(reaction_seed_path(root)).unwrap(), "current");
    }

    /// A fresh install has nothing to move and must not be given an empty `memory/prompts/`
    /// on the way past.
    #[test]
    fn a_fresh_install_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        migrate_seeds(dir.path());
        assert!(!memory_dir(dir.path()).join("prompts").exists());
        assert!(!seed_dir(dir.path()).exists());
    }
}
