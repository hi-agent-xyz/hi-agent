//! Bundled built-in views — seeded into the views tree on startup.
//!
//! Some views are platform "stdlib": basic, universal, the same for everyone (the
//! file-upload entry — a drag-drop zone + a phone QR; the first-hello a brand-new
//! person meets). We ship their source in the binary and write it into the views
//! tree at boot, so the agent shows them with
//! `show_view` like any other view — and can still adapt them, since they land as
//! ordinary `.jsx` in the (disposable, re-seeded) tree. They live under
//! `_builtin/` so they never collide with the agent's own `<project>/` work.

use std::io;
use std::path::Path;

/// The file-handoff view shown when the user wants to hand the agent a file.
/// Ref: `_builtin/upload` (the agent puts it on screen via `show_view`).
const UPLOAD: &str = include_str!("builtin/upload.jsx");

/// The "认识的人" review surface — review stored faces/voices, name the unknown
/// ones, eject a mis-clustered clip, or auto-regroup a mixed cluster. Reads and
/// writes the `/api/people/*` endpoints. Ref: `_builtin/people-review`.
const PEOPLE_REVIEW: &str = include_str!("builtin/people-review.jsx");
/// The first-hello a brand-new person meets — a first *impression*, not a tutorial.
/// The agent puts it on screen (ref `_builtin/welcome`) the once, on a genuine first
/// meeting (see [`crate::identity::reactor_system_prompt`] + `reaction.md`), while it speaks the
/// same idea in its own voice. Ships with a `.geom.json` sidecar so the host floats it
/// over the living presence room.
const WELCOME: &str = include_str!("builtin/welcome.jsx");
const WELCOME_GEOM: &str = include_str!("builtin/welcome.geom.json");
/// The real, sealed "hi" mark (red h + blue i, white die-cut, soft shadow) the welcome
/// poster shows — the exact app icon, served from the views tree at
/// `/views/_builtin/hi-mark.svg`, never re-typed in a system font.
const WELCOME_MARK: &str = include_str!("builtin/hi-mark.svg");

/// Shown by the **host** when the upstream model goes unreachable, and dismissed when it
/// comes back. Ref: `_builtin/vendor-outage`.
///
/// It is a bundled view rather than a sentence because of *when* it is needed: there is
/// no generation available to phrase anything at that moment, so the copy has to already
/// exist. It replaced a hardcoded Chinese string in `reactor/mod.rs` — English for now,
/// with a per-language variant the intended next step (see the note in the `.jsx`).
const VENDOR_OUTAGE: &str = include_str!("builtin/vendor-outage.jsx");
const VENDOR_OUTAGE_GEOM: &str = include_str!("builtin/vendor-outage.geom.json");

/// The ref and the sequencer id the host shows it under. One id, reused, so the
/// `dismiss` on recovery takes down exactly the thing the outage put up — and a second
/// outage replaces rather than stacks.
pub const VENDOR_OUTAGE_REF: &str = "_builtin/vendor-outage";
pub const VENDOR_OUTAGE_VIEW_ID: &str = "vendor-outage";

/// The bundled source and placement for [`VENDOR_OUTAGE_REF`], read straight from the
/// binary rather than off disk. The host shows this while the vendor is down, which is
/// exactly when the fewest things can be relied on — a disk read that fails then would
/// swallow the one notice the person gets.
pub fn vendor_outage_view() -> (&'static str, &'static str) {
    (VENDOR_OUTAGE, VENDOR_OUTAGE_GEOM)
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
    std::fs::write(dir.join("vendor-outage.jsx"), VENDOR_OUTAGE)?;
    std::fs::write(dir.join("vendor-outage.geom.json"), VENDOR_OUTAGE_GEOM)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The host shows this one itself, while the model is unreachable — so its copy has
    /// to exist in the binary, and the source it shows must be the same text that lands
    /// in the tree. A view the host reaches for during an outage is the worst possible
    /// place for a disk read that can fail.
    #[test]
    fn the_outage_view_is_bundled_and_seeded() {
        let dir = tempfile::tempdir().unwrap();
        install_builtin_views(dir.path()).unwrap();
        let builtin = dir.path().join("views").join("_builtin");
        assert!(builtin.join("vendor-outage.jsx").is_file());
        assert!(builtin.join("vendor-outage.geom.json").is_file());

        let (source, geom) = vendor_outage_view();
        assert_eq!(source, std::fs::read_to_string(builtin.join("vendor-outage.jsx")).unwrap());
        assert!(serde_json::from_str::<serde_json::Value>(geom).is_ok(), "the placement must parse");

        // English, and deliberately so — it replaced a hardcoded Chinese string, and the
        // per-language variant is the next step rather than a thing that was lost.
        assert!(source.contains("I can't reach my model right now"));
        assert!(source.contains("LOCALIZATION"), "the deferral has to stay written down");
    }

    #[test]
    fn seeds_the_welcome_hero_and_its_geom() {
        let dir = tempfile::tempdir().unwrap();
        install_builtin_views(dir.path()).unwrap();
        let builtin = dir.path().join("views").join("_builtin");
        // The first-hello view and its placement sidecar both land, so `show_view`
        // with ref `_builtin/welcome` resolves and the host knows where to float it.
        assert!(builtin.join("welcome.jsx").is_file());
        assert!(builtin.join("welcome.geom.json").is_file());
        assert!(builtin.join("hi-mark.svg").is_file());
        // Reseeding is idempotent (overwrite, not append) — a second boot is clean.
        install_builtin_views(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(builtin.join("welcome.jsx")).unwrap(), WELCOME);
    }
}
