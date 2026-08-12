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
//! `--surface` / `--surface-strong` / `--surface-border`, `--line` / `--line-strong`,
//! `--accent` (+ `-soft` / `-line` / `-wash`) and the second accent `--accent-2`,
//! `--danger` (+ `-line` / `-wash`) for a destructive verb, `--shadow` / `--shadow-strong`
//! (which are *colours*, not shadow lists), `--bg-0` / `--bg-1` and the ground they make
//! (`--paper`, which the layer already paints for you), `--font-display` /
//! `--font-mono`, the shared easing `--ease`, and the frame insets `--hi-safe-top` /
//! `-right` / `-bottom` / `-left`. Do not invent a name: `var(--card,#fff)`
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
//! **3. This system's own vocabulary is not translated.** Tools, Skills, Memory keep those
//! words in both languages, because they name parts of this architecture rather than
//! ordinary objects. Plain words do translate: Task is 任务, Drive is 文件, Sessions is 会话.
//!
//! That last one used to be Workers, untranslated under this rule, and the rule was not
//! what was wrong with it — the word was. The page lists every live session on the ladder
//! with its workers nested underneath, and the ended ones; "Workers" named the bottom rung
//! and dropped the rest. Its id is still `workers`, because that is the endpoint it reads.
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
/// same idea in its own voice. Ships with a `.geom.json` sidecar and owns the canvas
/// like every other bundled system surface.
const WELCOME: &str = include_str!("builtin/welcome.jsx");
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

/// The review surfaces: one per kind of thing the agent accumulates. Each owns the full
/// canvas and provides its own scrolling; the *safe* insets are the host's
/// (`.hi-view-fill` pads every layer clear of the window chrome and the control cluster),
/// so a view never reads `--hi-safe-*` itself. The *ground* is the host's too — the layer
/// paints `--paper` under its own padding, so a themed surface paints no background at
/// all rather than a flat `--bg-0` that stops at the padding and frames itself in the
/// paper (a visible border across the titlebar strip and both gutters in dark, where
/// `--bg-1` and `--bg-0` differ). A fixed-palette poster like `welcome` is the exception
/// and covers the frame itself by pinning at `inset: 0`. What each one does still reserve is the
/// bottom strip the caption pills rise through — deliberately unpadded by the host, since
/// reserving it would cost every view a slice of frame — which is what the 128px of
/// bottom padding in these files is for, on top of the host's own 76px.
///
/// They are siblings of `people-review`, and they exist for the same reason it does — the
/// agent's own state was only inspectable by reading files over its shoulder, so nothing
/// could be corrected. Each surface carries only the verbs its endpoint can honestly
/// honour: tasks change status, a skill deletes, a facet is rewritten; workers, tools and
/// drive are read-only, the first because the registry has no stop, the last two because
/// there is nothing there a person could fix.
const REVIEW_VIEWS: &[(&str, &str)] = &[
    ("tasks", include_str!("builtin/tasks.jsx")),
    ("skills", include_str!("builtin/skills.jsx")),
    ("memories", include_str!("builtin/memories.jsx")),
    ("workers", include_str!("builtin/workers.jsx")),
    ("tools", include_str!("builtin/tools.jsx")),
    ("drive", include_str!("builtin/drive.jsx")),
];

/// The ref and the sequencer id the host shows it under. One id, reused, so the
/// `dismiss` on recovery takes down exactly the thing the outage put up — and a second
/// outage replaces rather than stacks.
pub const OUT_OF_ENERGY_REF: &str = "_builtin/vendor-outage";
pub const OUT_OF_ENERGY_VIEW_ID: &str = "vendor-outage";

/// The bundled source for [`OUT_OF_ENERGY_REF`], read straight from the binary. The
/// host shows this when managed calls are paused, so a disk read is intentionally
/// not part of the recovery path.
///
/// It declares no traits, and the sidecar it used to ship is deleted. It carried
/// `owns_captions: true`, which suppressed the caption band behind the notice — but
/// the notice renders a fixed bilingual message, not the conversation, so the
/// declaration was never true in the sense the trait means. Under the three planes
/// (`docs/arch/stage.md`) it is also the wrong outcome: an outage is a condition of
/// the agent's half of the screen and must leave the record of what was said, and
/// the means to fix it, alone. The conversation now rails beside it.
pub fn out_of_energy_view() -> &'static str {
    OUT_OF_ENERGY
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
    std::fs::write(dir.join("hi-mark.svg"), WELCOME_MARK)?;
    std::fs::write(dir.join("vendor-outage.jsx"), OUT_OF_ENERGY)?;
    for (name, source) in REVIEW_VIEWS {
        std::fs::write(dir.join(format!("{name}.jsx")), source)?;
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

        let source = out_of_energy_view();
        assert_eq!(source, std::fs::read_to_string(builtin.join("vendor-outage.jsx")).unwrap());

        assert!(source.contains("Your energy is used up"));
        assert!(source.contains("hi-agent.xyz"));
        assert!(source.contains("消息都已保留"));
    }

    #[test]
    fn seeds_the_welcome_hero_and_its_mark() {
        let dir = tempfile::tempdir().unwrap();
        install_builtin_views(dir.path()).unwrap();
        let builtin = dir.path().join("views").join("_builtin");
        // The first-hello view and its mark land, so `show` with ref
        // `_builtin/welcome` resolves. No sidecar: it gets the full canvas like
        // everything else, and it does not render the words itself.
        assert!(builtin.join("welcome.jsx").is_file());
        assert!(!builtin.join("welcome.geom.json").exists());
        assert!(builtin.join("hi-mark.svg").is_file());
        // Reseeding is idempotent (overwrite, not append) — a second boot is clean.
        install_builtin_views(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(builtin.join("welcome.jsx")).unwrap(), WELCOME);
    }

    #[test]
    fn seeds_every_review_surface() {
        let dir = tempfile::tempdir().unwrap();
        install_builtin_views(dir.path()).unwrap();
        let builtin = dir.path().join("views").join("_builtin");
        for (name, source) in REVIEW_VIEWS {
            let jsx = builtin.join(format!("{name}.jsx"));
            assert!(jsx.is_file(), "{name}.jsx was not seeded");
            assert_eq!(&std::fs::read_to_string(&jsx).unwrap(), source);
        }
    }

    /// The toolbox is read by scanning the tree for `// purpose:` lines
    /// (`docs/arch/data.md#views`), and `_builtin/` is the only part of it a fresh
    /// install has. A bundled view without the line degrades to a bare filename in
    /// the one scan the builder runs before it authors anything — so every one of
    /// them opens with a purpose line, and it has to be the *first* line, since that
    /// is what `grep -n "^// purpose:"` returns.
    #[test]
    fn every_bundled_view_opens_with_a_purpose_line() {
        let dir = tempfile::tempdir().unwrap();
        install_builtin_views(dir.path()).unwrap();
        let builtin = dir.path().join("views").join("_builtin");
        let mut names: Vec<&str> = vec!["upload", "people-review", "welcome", "vendor-outage"];
        names.extend(REVIEW_VIEWS.iter().map(|(n, _)| *n));
        for name in names {
            let source = std::fs::read_to_string(builtin.join(format!("{name}.jsx"))).unwrap();
            let first = source.lines().next().unwrap_or_default();
            assert!(
                first.starts_with("// purpose:"),
                "{name}.jsx must open with a `// purpose:` line; it opens with {first:?}"
            );
            assert!(
                first.trim_start_matches("// purpose:").trim().len() > 20,
                "{name}.jsx's purpose line says too little to match a job against: {first:?}"
            );
        }
    }

    /// Full-bleed is the frame every view gets, so owning the canvas is no longer
    /// something a view declares — and a sidecar that exists only to say "fill" is
    /// a file that can be forgotten. What this keeps is that none of them carries
    /// one: any sidecar left behind here would be a placement the host no longer reads.
    #[test]
    fn no_bundled_view_declares_a_placement() {
        let dir = tempfile::tempdir().unwrap();
        install_builtin_views(dir.path()).unwrap();
        let builtin = dir.path().join("views").join("_builtin");
        let mut names: Vec<&str> = vec!["upload", "people-review", "welcome"];
        names.extend(REVIEW_VIEWS.iter().map(|(n, _)| *n));
        for name in names {
            assert!(
                !builtin.join(format!("{name}.geom.json")).exists(),
                "{name} must not carry a sidecar"
            );
        }
    }

    /// No bundled view declares a trait any more — the outage notice was the last
    /// one, and it claimed `owns_captions` while rendering a fixed message rather
    /// than the conversation. An outage is a condition of the agent's half of the
    /// screen; it must not take down the record of what was said or the input line
    /// to answer with (`docs/arch/stage.md`). The sidecar mechanism stays for
    /// agent-authored views, which is where a real claim can come from.
    #[test]
    fn no_bundled_view_declares_traits_and_the_outage_cannot_hide_the_conversation() {
        let dir = tempfile::tempdir().unwrap();
        install_builtin_views(dir.path()).unwrap();
        let builtin = dir.path().join("views").join("_builtin");
        let sidecars: Vec<String> = std::fs::read_dir(&builtin)
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| name.ends_with(".geom.json"))
            .collect();
        assert!(sidecars.is_empty(), "bundled views declare nothing: {sidecars:?}");
    }

    /// The Tools surface has to have a word for every rung the endpoint serves. A role
    /// with no entry in the view's own table falls back to printing its bare id — which
    /// is how `worker`, the first rung listed and the only one that does the work, sat
    /// in that view undescribed, in both languages, reading like a leaked internal name.
    #[test]
    fn the_tools_view_names_every_role_the_endpoint_serves() {
        let source = REVIEW_VIEWS.iter().find(|(name, ..)| *name == "tools").unwrap().1;
        for role in crate::foundation::server::tools::ROLES {
            // Each language table keys its label by the raw role, so the id appears once
            // per table; one occurrence would mean a language was missed.
            let key = format!("{role}: [");
            assert_eq!(
                source.matches(&key).count(),
                2,
                "tools.jsx must label role `{role}` in both en and zh"
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

    /// The Workers surface reads three routes, and two of them are the reason it can answer
    /// anything about a session that is no longer live. If either is renamed, the page
    /// degrades to the live-only roster that could not tell "ran" from "never existed" —
    /// and it degrades *silently*, because a failed fetch there deliberately keeps the last
    /// good state rather than blanking. This is what notices instead.
    #[test]
    fn the_workers_view_reads_the_ended_list_and_the_frame_log() {
        let source = REVIEW_VIEWS.iter().find(|(name, ..)| *name == "workers").unwrap().1;
        assert!(source.contains("/api/workers\""), "the live roster");
        assert!(source.contains("/api/workers/ended"), "what just ended");
        assert!(source.contains("/frames"), "one session's verbatim wire log");
        // The run has to travel with the request: a frame log is addressed by (run, session)
        // and session ids restart at 1 each boot, so an ended row that dropped its run would
        // silently read some *other* session's history under the same number.
        assert!(source.contains("run="), "an ended row must pass its own run");
        // And the restart case has to be rendered as itself. A row the process died under
        // reading as a clean finish is the failure this whole surface exists to surface.
        assert!(source.contains("\"restart\""), "a lost session must be told apart from a closed one");
    }
}
