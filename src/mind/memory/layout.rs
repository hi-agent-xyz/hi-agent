//! On-disk paths for the raw memory store.
//!
//! Raw is the lossless source of truth, organized by scene (the isolation unit),
//! then by **channel**, then sharded by UTC day. A channel is that sense's
//! complete record; the day-folder keeps reads bounded and makes per-channel
//! fading/archival a single subtree. Each channel-day carries a surface log
//! named for the channel (`text.jsonl`, `audio.jsonl`, …) plus the bytes its
//! signals reference, laid out on a wall-clock grid.
//!
//! ```text
//! <data_dir>/memory/raw/<scene_enc>/
//!   ├── scene.json
//!   ├── text/<YYYY-MM-DD>/text.jsonl
//!   ├── audio/<YYYY-MM-DD>/{ audio.jsonl, <HH>/<MM>-<SS>.<ext>, output/<HH>/<MM>.<ext> … }
//!   └── vision/<YYYY-MM-DD>/{ vision.jsonl, <HH>/<MM>-<SS>.<ext> … }
//! ```
//!
//! Scene ids are arbitrary strings (`alice@phone`) and may carry path-unsafe
//! characters, so the directory name is a percent-encoding of the id; the true
//! id is recorded in `scene.json`.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::types::{Channel, Scene};

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

/// `<memory>/hot.md` — the always-loaded working set (a regenerable projection).
pub fn hot_path(data_dir: &Path) -> PathBuf {
    memory_dir(data_dir).join("hot.md")
}

/// `<memory>/prompts` — the **generated** system prompts: one file per agent that
/// carries state forward into every window.
///
/// The leaf name is `prompts/` on both sides of the tree on purpose — both hold the
/// same kind of thing, text handed to an agent at init — and the parent directory is
/// the whole of the difference. `<data_dir>/prompts/` is **bundled**: shipped in the
/// binary, reinstalled every boot, disposable. This one is **generated**: written by
/// the agent, precious, and rebuildable by nothing else. See
/// `docs/arch/data.md#memoryprompts`.
///
/// **Nothing writes here yet.** Deliberation gets that job in a later change, so
/// every reader today must treat an absent file as ordinary and degrade to the log
/// tail rather than raise it as an error.
pub fn generated_prompts_dir(data_dir: &Path) -> PathBuf {
    memory_dir(data_dir).join("prompts")
}

/// `<memory>/prompts/scenes/<scene_enc>.md` — what one scene carries forward,
/// written by that scene's Deliberation and injected into every one of its turns.
/// The file name is [`encode_scene`]d for the same reason the raw slice's folder is:
/// a scene id is an arbitrary string and may carry path-unsafe characters.
pub fn scene_prompt_path(data_dir: &Path, scene: &Scene) -> PathBuf {
    generated_prompts_dir(data_dir)
        .join("scenes")
        .join(format!("{}.md", encode_scene(scene)))
}

/// `<memory>/prompts/<agent>.md` — what a **sceneless** agent carries forward
/// (`cognition.md`). Not a full set on purpose: an agent gets a file when it turns
/// out to need one. `agent` is a code-supplied name, never a user string.
pub fn agent_prompt_path(data_dir: &Path, agent: &str) -> PathBuf {
    generated_prompts_dir(data_dir).join(format!("{agent}.md"))
}

/// `<memory>/proactivity.md` — the learned read on speaking up unprompted: which
/// subjects the person welcomes a proactive word on, and which they don't. A
/// derived projection like [`hot_path`] — the reflection pass regenerates it from
/// how the agent's own unprompted utterances landed; the agent only reads it, to
/// judge whether breaking silence clears the bar. Absent ⇒ nothing proven ⇒ stay
/// cautious.
pub fn proactivity_path(data_dir: &Path) -> PathBuf {
    memory_dir(data_dir).join("proactivity.md")
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

/// `<raw>/<scene_enc>` — one slice per scene.
/// `<memory>/raw/_acp/<YYYY-MM-DD>.jsonl` — the **session stream, verbatim**.
///
/// One append-only file per day holding every JSON-RPC line that crossed to or from an
/// agent subprocess, in order, uninterpreted. This is what
/// `docs/arch/foundation.md#full-frames-not-modelled-events` asks for and what
/// verification reads: a tool call's `raw_input`/`raw_output`/`content` live here and
/// nowhere else.
///
/// Under `raw/` because foundation holds that pen and the rule there is *written before
/// anything reacts to it*. Not scene-partitioned like a channel: a connection precedes
/// its own `sessionId` (the `initialize` handshake carries none), and each line names
/// its scene anyway, so partitioning by day keeps the record whole rather than splitting
/// a session's own opening away from it.
pub fn acp_frames_path(data_dir: &Path, day: DateTime<Utc>) -> PathBuf {
    raw_root(data_dir).join("_acp").join(format!("{}.jsonl", day.format("%Y-%m-%d")))
}

pub fn scene_dir(data_dir: &Path, scene: &Scene) -> PathBuf {
    raw_root(data_dir).join(encode_scene(scene))
}

/// `<scene>/<channel>/<YYYY-MM-DD>` — the channel-day folder a signal at `ts`
/// belongs to, holding that day's surface log and the bytes its signals
/// reference. The parent of both the log and the byte grid.
pub fn channel_day_dir(
    data_dir: &Path,
    scene: &Scene,
    channel: Channel,
    ts: DateTime<Utc>,
) -> PathBuf {
    scene_dir(data_dir, scene)
        .join(channel.as_str())
        .join(day_key(ts))
}

/// `<channel>/<date>/<channel>.jsonl` — the day's surface log for one channel,
/// named for the channel so the file is self-describing even detached from its
/// folder.
pub fn channel_log_path(
    data_dir: &Path,
    scene: &Scene,
    channel: Channel,
    ts: DateTime<Utc>,
) -> PathBuf {
    channel_day_dir(data_dir, scene, channel, ts).join(format!("{}.jsonl", channel.as_str()))
}

/// `<scene>/appearance/<YYYY-MM-DD>` — the day-folder for a scene's screen-state
/// history. Appearance is a state channel, not an event stream: it holds
/// timestamped whole-state snapshots (`appearance-<HHMMSSZ>.json`), not a
/// `<channel>.jsonl`, so it is reached through this helper rather than
/// [`channel_day_dir`] (there is no `Channel::Appearance`).
pub fn appearance_day_dir(data_dir: &Path, scene: &Scene, ts: DateTime<Utc>) -> PathBuf {
    scene_dir(data_dir, scene)
        .join("appearance")
        .join(day_key(ts))
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

/// Path-safe directory name for a scene: percent-encode every byte outside the
/// unreserved set `[A-Za-z0-9._-]`. Deterministic, so a scene always maps to the
/// same folder; the inverse is never needed (the true id lives in `scene.json`).
pub fn encode_scene(scene: &Scene) -> String {
    let mut out = String::with_capacity(scene.0.len());
    for b in scene.0.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_path_unsafe_chars() {
        assert_eq!(encode_scene(&Scene("alice@phone".into())), "alice%40phone");
        assert_eq!(encode_scene(&Scene("a/b".into())), "a%2Fb");
        assert_eq!(encode_scene(&Scene("plain-1.0_x".into())), "plain-1.0_x");
    }

    /// The generated tree sits under `memory/`, never beside the bundled
    /// `<data_dir>/prompts/` — the parent directory is what says who wrote the file.
    #[test]
    fn generated_prompts_live_under_memory_not_beside_the_bundled_ones() {
        let root = Path::new("/tmp/jack.hi");
        assert_eq!(generated_prompts_dir(root), root.join("memory").join("prompts"));
        assert_ne!(generated_prompts_dir(root), root.join("prompts"));
        assert_eq!(
            agent_prompt_path(root, "cognition"),
            root.join("memory").join("prompts").join("cognition.md")
        );
        // A scene id with path-unsafe characters cannot escape the tree.
        assert_eq!(
            scene_prompt_path(root, &Scene("alice@phone".into())),
            root.join("memory").join("prompts").join("scenes").join("alice%40phone.md")
        );
        assert!(
            scene_prompt_path(root, &Scene("../../etc/passwd".into()))
                .starts_with(root.join("memory").join("prompts").join("scenes"))
        );
    }
}
