//! Bundled built-in views — seeded into the views tree on startup.
//!
//! Some views are platform "stdlib": basic, universal, the same for everyone (the
//! file-upload entry — a drag-drop zone + a phone QR; the first-hello a brand-new
//! person meets). We ship their source in the binary and write it into the views
//! tree at boot, so the agent shows them with
//! `show` like any other view — and can still adapt them, since they land as
//! ordinary `.jsx` in the (disposable, re-seeded) tree. They live under
//! `_builtin/` so they never collide with the agent's own `<project>/` work.
//!
//! # Three rules every view in here follows
//!
//! These hold for the *system* surfaces only. An agent-authored content view is
//! deliberately free of all three — `aesthetic.md` says its look comes from its subject
//! and there is no house style on purpose.
//!
//! **1. Colour comes from the host's theme tokens, or from nothing.** The vocabulary is
//! whatever `ui/global.css` actually defines: `--fg` / `--fg-dim` / `--fg-mute`,
//! `--surface` / `--surface-strong`, `--line` / `--line-strong`, `--accent` (+ `-soft` /
//! `-line` / `-wash`), `--shadow` / `--shadow-strong` (which are *colours*, not shadow
//! lists), `--bg-0` / `--bg-1`, `--font-display`. Do not invent a name: `var(--card,#fff)`
//! reads like a token and is really a hardcoded white, which is how named people once went
//! invisible in dark mode. Mixing the two halves is the trap — a *fixed* palette is fine
//! and often right for a poster, but then the text colour must be fixed too, or it flips
//! against a ground that doesn't. Renderer-side, `review_view` shows both skins by default
//! so this is caught by looking rather than by review.
//!
//! **2. Copy ships in English and Chinese, English by default.** Each view carries its own
//! `T = { en, zh }` table and resolves at module scope off `<html lang>` — the app language
//! setting, published there by the web face (`lib/language.ts`) and forced by the render
//! page's `lang=` param. The chain is: app setting, then the system locale when that
//! setting says `system` (the default), then English; a language we have no strings for
//! also lands on English. Further languages are meant to be *authored at runtime* rather
//! than shipped here — see the `TODO(i18n)` in each view.
//!
//! **3. This system's own vocabulary is not translated.** Tools, Skills, Memory, Workers
//! keep those words in both languages, because they name parts of this architecture rather
//! than ordinary objects. Plain words do translate: Task is 任务, Drive is 文件.
//!
//! And one that is about honesty rather than style: a surface carries only the verbs its
//! endpoint can actually honour. Workers is read-only because the registry has no stop —
//! a button that reported a kill that never happened would be worse than no button.

use std::io;
use std::path::Path;

/// The file-handoff view shown when the user wants to hand the agent a file.
/// Ref: `_builtin/upload` (the agent puts it on screen via `show`).
const UPLOAD: &str = include_str!("builtin/upload.jsx");

/// The "认识的人" review surface — review stored faces/voices, name the unknown
/// ones, eject a mis-clustered clip, or auto-regroup a mixed cluster. Reads and
/// writes the `/api/people/*` endpoints. Ref: `_builtin/people-review`.
const PEOPLE_REVIEW: &str = include_str!("builtin/people-review.jsx");
/// The first-hello a brand-new person meets — a first *impression*, not a tutorial.
/// The agent puts it on screen (ref `_builtin/welcome`) the once, on a genuine first
/// meeting (see [`crate::identity::reaction_system_prompt`] + `reaction.md`), while it speaks the
/// same idea in its own voice. Ships with a `.geom.json` sidecar so the host floats it
/// over the living presence room.
const WELCOME: &str = include_str!("builtin/welcome.jsx");
const WELCOME_GEOM: &str = include_str!("builtin/welcome.geom.json");
/// The real, sealed "hi" mark (red h + blue i, white die-cut, soft shadow) the welcome
/// poster shows — the exact app icon, served from the views tree at
/// `/views/_builtin/hi-mark.svg`, never re-typed in a system font.
const WELCOME_MARK: &str = include_str!("builtin/hi-mark.svg");

/// Shown by the **host** after a managed 402, and dismissed as soon as the broker
/// reports positive energy. The persisted ref/id keep their historical
/// `vendor-outage` values so old retained snapshots are reconciled in place.
///
/// It is a bundled view rather than a sentence because of *when* it is needed: there is
/// no generation available to phrase anything at that moment, so the copy has to already
/// exist. English and Chinese are selected from the host's current language.
const OUT_OF_ENERGY: &str = include_str!("builtin/vendor-outage.jsx");
const OUT_OF_ENERGY_GEOM: &str = include_str!("builtin/vendor-outage.geom.json");

/// The review surfaces: one per kind of thing the agent accumulates, each paired with a
/// `.geom.json` that puts it up `center/wide` because every one of them is a list.
///
/// They are siblings of `people-review`, and they exist for the same reason it does — the
/// agent's own state was only inspectable by reading files over its shoulder, so nothing
/// could be corrected. Each surface carries only the verbs its endpoint can honestly
/// honour: tasks close and drop, a skill deletes, a facet is rewritten; workers, tools and
/// drive are read-only, the first because the registry has no stop, the last two because
/// there is nothing there a person could fix.
const REVIEW_VIEWS: &[(&str, &str, &str)] = &[
    ("tasks", include_str!("builtin/tasks.jsx"), include_str!("builtin/tasks.geom.json")),
    ("skills", include_str!("builtin/skills.jsx"), include_str!("builtin/skills.geom.json")),
    ("memories", include_str!("builtin/memories.jsx"), include_str!("builtin/memories.geom.json")),
    ("workers", include_str!("builtin/workers.jsx"), include_str!("builtin/workers.geom.json")),
    ("tools", include_str!("builtin/tools.jsx"), include_str!("builtin/tools.geom.json")),
    ("drive", include_str!("builtin/drive.jsx"), include_str!("builtin/drive.geom.json")),
];

/// The ref and the sequencer id the host shows it under. One id, reused, so the
/// `dismiss` on recovery takes down exactly the thing the outage put up — and a second
/// outage replaces rather than stacks.
pub const OUT_OF_ENERGY_REF: &str = "_builtin/vendor-outage";
pub const OUT_OF_ENERGY_VIEW_ID: &str = "vendor-outage";

/// The bundled source and placement for [`OUT_OF_ENERGY_REF`], read straight from
/// the binary. The host shows this when managed calls are paused, so a disk read is
/// intentionally not part of the recovery path.
pub fn out_of_energy_view() -> (&'static str, &'static str) {
    (OUT_OF_ENERGY, OUT_OF_ENERGY_GEOM)
}

/// Write the bundled built-in views into `<data_dir>/views/_builtin/`, overwriting
/// each on every boot so a binary update reseeds the latest (mirrors
/// [`crate::identity::install_prompts`]). The views tree is disposable, so
/// re-seeding is the point, not a hazard.
pub fn install_builtin_views(data_dir: &Path) -> io::Result<()> {
    let dir = data_dir.join("views").join("_builtin");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("upload.jsx"), UPLOAD)?;
    std::fs::write(dir.join("people-review.jsx"), PEOPLE_REVIEW)?;
    std::fs::write(dir.join("welcome.jsx"), WELCOME)?;
    std::fs::write(dir.join("welcome.geom.json"), WELCOME_GEOM)?;
    std::fs::write(dir.join("hi-mark.svg"), WELCOME_MARK)?;
    std::fs::write(dir.join("vendor-outage.jsx"), OUT_OF_ENERGY)?;
    std::fs::write(dir.join("vendor-outage.geom.json"), OUT_OF_ENERGY_GEOM)?;
    for (name, source, geom) in REVIEW_VIEWS {
        std::fs::write(dir.join(format!("{name}.jsx")), source)?;
        std::fs::write(dir.join(format!("{name}.geom.json")), geom)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The host shows this itself while managed calls are paused, so its copy has to
    /// exist in the binary and match what is seeded into the disposable views tree.
    #[test]
    fn the_out_of_energy_view_is_bundled_and_seeded() {
        let dir = tempfile::tempdir().unwrap();
        install_builtin_views(dir.path()).unwrap();
        let builtin = dir.path().join("views").join("_builtin");
        assert!(builtin.join("vendor-outage.jsx").is_file());
        assert!(builtin.join("vendor-outage.geom.json").is_file());

        let (source, geom) = out_of_energy_view();
        assert_eq!(source, std::fs::read_to_string(builtin.join("vendor-outage.jsx")).unwrap());
        assert!(serde_json::from_str::<serde_json::Value>(geom).is_ok(), "the placement must parse");

        assert!(source.contains("Your energy is used up"));
        assert!(source.contains("hi-agent.xyz"));
        assert!(source.contains("消息都已保留"));
    }

    #[test]
    fn seeds_the_welcome_hero_and_its_geom() {
        let dir = tempfile::tempdir().unwrap();
        install_builtin_views(dir.path()).unwrap();
        let builtin = dir.path().join("views").join("_builtin");
        // The first-hello view and its placement sidecar both land, so `show`
        // with ref `_builtin/welcome` resolves and the host knows where to float it.
        assert!(builtin.join("welcome.jsx").is_file());
        assert!(builtin.join("welcome.geom.json").is_file());
        assert!(builtin.join("hi-mark.svg").is_file());
        // Reseeding is idempotent (overwrite, not append) — a second boot is clean.
        install_builtin_views(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(builtin.join("welcome.jsx")).unwrap(), WELCOME);
    }

    /// Every review surface lands with its placement, and every one of them declares a
    /// placement that parses — a `show` on a ref whose sidecar is malformed puts up
    /// a view the host cannot position.
    #[test]
    fn seeds_every_review_surface_with_a_valid_placement() {
        let dir = tempfile::tempdir().unwrap();
        install_builtin_views(dir.path()).unwrap();
        let builtin = dir.path().join("views").join("_builtin");
        for (name, source, _) in REVIEW_VIEWS {
            let jsx = builtin.join(format!("{name}.jsx"));
            let geom = builtin.join(format!("{name}.geom.json"));
            assert!(jsx.is_file(), "{name}.jsx was not seeded");
            assert_eq!(&std::fs::read_to_string(&jsx).unwrap(), source);
            let raw = std::fs::read_to_string(&geom).unwrap();
            assert!(
                serde_json::from_str::<serde_json::Value>(&raw).is_ok(),
                "{name}.geom.json must parse"
            );
        }
    }

    /// The two that carry a correction verb have to keep reaching for it. If the endpoint
    /// behind one of these is ever renamed, this is what notices — a review surface whose
    /// write silently 404s still *looks* like it worked.
    #[test]
    fn the_correcting_surfaces_still_call_their_write_endpoints() {
        let by_name = |n: &str| REVIEW_VIEWS.iter().find(|(name, ..)| *name == n).unwrap().1;
        assert!(by_name("tasks").contains("/api/tasks/"), "tasks must PATCH a task");
        assert!(by_name("tasks").contains("PATCH"));
        assert!(by_name("memories").contains("/api/facets/"), "memories must PUT a facet");
        assert!(by_name("memories").contains("PUT"));
        assert!(by_name("skills").contains("DELETE"), "skills must be able to drop a stale note");
        // And the read-only ones must not have grown a verb they cannot honour: the
        // registry has no stop, so a stop button here would report a kill that never
        // happened. Checked as an absence on purpose.
        assert!(!by_name("workers").contains("method: \"POST\""));
        assert!(!by_name("tools").contains("method:"));
        assert!(!by_name("drive").contains("method:"));
    }
}
