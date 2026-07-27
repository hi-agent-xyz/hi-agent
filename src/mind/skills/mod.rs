//! `skills/` — the workshop, and its factory seed layer.
//!
//! A skill is a short note in the agent's own words on how a kind of job was done:
//! the steps that worked, the tools, the traps, what good looked like. It is a
//! *starting point, not truth* — its durable half is reused as-is, its perishable
//! half (prices, APIs, product details) is marked and re-verified every time. Facts
//! belong in memory; procedures belong here. See `docs/arch/data.md`.
//!
//! Two writers share this subtree, so the layers stay **physically separate**
//! (`docs/arch/data.md` — "who holds the pen"):
//!
//! | Layer | Path | On upgrade |
//! |---|---|---|
//! | factory seeds | `<data_dir>/skills/_builtin/*.md` | rewritten every boot |
//! | everything the agent learnt | `<data_dir>/skills/**` (outside `_builtin/`) | never touched |
//!
//! The `_builtin/` prefix is the same convention the views tree already uses
//! ([`crate::mind::views::install_builtin_views`]) — chosen over a sibling directory
//! so the workshop stays *one* place to look: a worker greps `skills/` and finds both
//! its own notes and the seeded ones, while an upgrade still has a single subtree it
//! owns and may clobber.

use std::io;
use std::path::{Path, PathBuf};

/// Seeded skill: how to give hi-agent hands and eyes on another machine or phone.
/// Deliberately soft guidance — devices are tools plus a written procedure, not a
/// subsystem.
const ADDING_A_DEVICE: &str = include_str!("adding-a-device.md");

/// `<data_dir>/skills/` — the workshop root. Agent-written skills live directly
/// under here; the factory seeds live in the `_builtin/` subdirectory.
pub fn skills_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("skills")
}

/// Create the workshop and write the bundled seed skills into
/// `<data_dir>/skills/_builtin/`, overwriting each on every boot so a binary update
/// reseeds the latest (mirrors [`crate::identity::install_prompts`] and
/// [`crate::mind::views::install_builtin_views`]).
///
/// Only `_builtin/` is rewritten. Anything the agent wrote elsewhere in the tree is
/// never read, moved or touched here — that separation is the point, not an
/// implementation detail.
pub fn install_builtin_skills(data_dir: &Path) -> io::Result<()> {
    let dir = skills_dir(data_dir).join("_builtin");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("adding-a-device.md"), ADDING_A_DEVICE)?;
    tracing::info!(dir = %dir.display(), "installed bundled skills");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_seeds_the_builtin_layer() {
        let dir = tempfile::tempdir().unwrap();
        install_builtin_skills(dir.path()).unwrap();
        let seeded = skills_dir(dir.path()).join("_builtin").join("adding-a-device.md");
        assert_eq!(std::fs::read_to_string(&seeded).unwrap(), ADDING_A_DEVICE);
    }

    #[test]
    fn reinstalling_refreshes_the_seed_and_leaves_agent_written_skills_alone() {
        let dir = tempfile::tempdir().unwrap();
        install_builtin_skills(dir.path()).unwrap();

        // The agent writes its own skill next to the seeds, and edits a seed.
        let learnt = skills_dir(dir.path()).join("posting-a-clip.md");
        std::fs::write(&learnt, "what worked last time").unwrap();
        let nested = skills_dir(dir.path()).join("video").join("trimming.md");
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
        std::fs::write(&nested, "ffmpeg -ss ...").unwrap();
        let seeded = skills_dir(dir.path()).join("_builtin").join("adding-a-device.md");
        std::fs::write(&seeded, "stale").unwrap();

        // An upgrade replaces the factory layer …
        install_builtin_skills(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&seeded).unwrap(), ADDING_A_DEVICE);
        // … and never touches the learnt one.
        assert_eq!(std::fs::read_to_string(&learnt).unwrap(), "what worked last time");
        assert_eq!(std::fs::read_to_string(&nested).unwrap(), "ffmpeg -ss ...");
    }

    #[test]
    fn the_seeded_skill_marks_its_perishable_half() {
        // A skill is a starting point, not truth: whatever rots must be flagged, or
        // the next job trusts a stale price or a dead API.
        assert!(ADDING_A_DEVICE.contains("perishable"));
        // And the durable, hard-won part: SSH gets you a shell, not a screen.
        assert!(ADDING_A_DEVICE.contains("no window server"));
    }
}
