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
/// meeting (see [`crate::identity::reactor_system_prompt`] + `speaking.md`), while it speaks the
/// same idea in its own voice. Ships with a `.geom.json` sidecar so the host floats it
/// over the living presence room.
const WELCOME: &str = include_str!("builtin/welcome.jsx");
const WELCOME_GEOM: &str = include_str!("builtin/welcome.geom.json");
/// The real, sealed "hi" mark (red h + blue i, white die-cut, soft shadow) the welcome
/// poster shows — the exact app icon, served from the views tree at
/// `/views/_builtin/hi-mark.svg`, never re-typed in a system font.
const WELCOME_MARK: &str = include_str!("builtin/hi-mark.svg");

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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
