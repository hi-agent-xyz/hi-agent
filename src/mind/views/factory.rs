//! Bundled built-in views — seeded into the views tree on startup.
//!
//! Some views are platform "stdlib": basic, universal, the same for everyone (the
//! file-upload entry — a drag-drop zone + a phone QR; the first-hello a brand-new
//! person meets). We ship their source in the binary and write it into the views
//! tree at boot, so the agent shows them with
//! `show` like any other view — and can still adapt them, since they land as
//! ordinary `.jsx` in the (disposable, re-seeded) tree. They live under
//! `factory/` so they never collide with the agent's own `<project>/` work.
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
//! `--font-mono`, the shared easing `--ease`, and the chrome tokens `--hi-safe-top` /
//! `-right` / `-bottom` / `-left` and `--hi-chrome-bottom` (opt-in clearance, not insets). Do not invent a name: `var(--card,#fff)`
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
/// Ref: `factory/upload` (the agent puts it on screen via `show`).
const UPLOAD: &str = include_str!("factory/upload.jsx");

/// The "认识的人" review surface — review stored faces/voices, name the unknown
/// ones, eject a mis-clustered clip, or auto-regroup a mixed cluster. Reads and
/// writes the `/api/people/*` endpoints. Ref: `factory/people-review`.
const PEOPLE_REVIEW: &str = include_str!("factory/people-review.jsx");
/// The first-hello a brand-new person meets — a first *impression*, not a tutorial.
/// The agent puts it on screen (ref `factory/welcome`) the once, on a genuine first
/// meeting (see [`crate::identity::reaction_system_prompt`] + `reaction.md`), while it speaks the
/// same idea in its own voice, and owns the canvas like every other bundled system
/// surface.
const WELCOME: &str = include_str!("factory/welcome.jsx");
/// The real, sealed "hi" mark (red h + blue i, white die-cut, soft shadow) the welcome
/// poster shows — the exact app icon, served from the views tree at
/// `/views/factory/hi-mark.svg`, never re-typed in a system font.
const WELCOME_MARK: &str = include_str!("factory/hi-mark.svg");

/// Shown by the **host** after a managed 402, and dismissed as soon as the broker
/// reports positive energy. The persisted ref/id keep their historical
/// `vendor-outage` values so old retained snapshots are reconciled in place.
///
/// It is a bundled view rather than a sentence because of *when* it is needed: there is
/// no generation available to phrase anything at that moment, so the copy has to already
/// exist. English and Chinese are selected from the host's current language.
const OUT_OF_ENERGY: &str = include_str!("factory/vendor-outage.jsx");

/// The review surfaces: one per kind of thing the agent accumulates. Each owns the full
/// canvas and provides its own scrolling, and each holds its *own* content clear of the
/// window chrome: the host reserves nothing any more (`.hi-view-fill`), so these read
/// `--hi-safe-top` for the titlebar strip or a phone's notch and `--hi-chrome-bottom`
/// where a row must not sit under the control discs. The *ground* is still the host's —
/// the layer paints `--paper` behind every view — so a themed surface paints no
/// background at all rather than a flat `--bg-0` that stops where its own padding does
/// and frames itself in paper it doesn't match (a visible border in dark, where `--bg-1`
/// and `--bg-0` differ). A fixed-palette poster like `welcome` covers the frame itself by
/// pinning at `inset: 0`. The 128px of bottom padding in these files is the strip the
/// caption pills rise through, which nothing has ever reserved and nothing should.
///
/// They are siblings of `people-review`, and they exist for the same reason it does — the
/// agent's own state was only inspectable by reading files over its shoulder, so nothing
/// could be corrected. `reach` is the newest and the odd one in a second way: two of its
/// three sections are about the *outside* — the agent's address in the community and the
/// devices that hold a way in — and the third is the app's roster, which is not the
/// agent's state at all and simply is not there when no app is holding the page. Each surface carries only the verbs its endpoint can honestly
/// honour: tasks change status, a skill deletes, a facet is rewritten; workers, tools and
/// drive are read-only, the first because the registry has no stop, the last two because
/// there is nothing there a person could fix.
const REVIEW_VIEWS: &[(&str, &str)] = &[
    ("stats", include_str!("factory/stats.jsx")),
    ("tasks", include_str!("factory/tasks.jsx")),
    ("skills", include_str!("factory/skills.jsx")),
    ("memories", include_str!("factory/memories.jsx")),
    ("workers", include_str!("factory/workers.jsx")),
    ("tools", include_str!("factory/tools.jsx")),
    ("drive", include_str!("factory/drive.jsx")),
    ("reach", include_str!("factory/reach.jsx")),
];

/// The ref and the sequencer id the host shows it under. One id, reused, so the
/// `dismiss` on recovery takes down exactly the thing the outage put up — and a second
/// outage replaces rather than stacks.
pub const OUT_OF_ENERGY_REF: &str = "factory/vendor-outage";
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

/// Write the bundled built-in views into `<data_dir>/views/factory/`, overwriting
/// each on every boot so a binary update reseeds the latest (mirrors
/// [`crate::identity::install_prompts`]). The views tree is disposable, so
/// re-seeding is the point, not a hazard.
pub fn install_factory_views(data_dir: &Path) -> io::Result<()> {
    let dir = data_dir.join("views").join("factory");
    rename_legacy_dir(&data_dir.join("views").join("_builtin"), &dir);
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

/// Move a pre-rename `_builtin/` tree to `factory/`, once, if it is still there.
///
/// Best-effort and silent about absence, which is the normal case on every install after
/// the first boot that ran it. It matters for what the agent *built*: a view it wrote
/// under the old tree would otherwise sit at a ref nothing resolves, and the install
/// would happily write a fresh `factory/` beside it and look correct.
pub(crate) fn rename_legacy_dir(from: &Path, to: &Path) {
    if !from.is_dir() || to.exists() {
        return;
    }
    match std::fs::rename(from, to) {
        Ok(()) => tracing::info!(from = %from.display(), to = %to.display(), "renamed the pre-factory tree"),
        Err(e) => {
            tracing::warn!(from = %from.display(), error = %e, "could not rename the pre-factory tree")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The host shows this itself while managed calls are paused, so its copy has to
    /// exist in the binary and match what is seeded into the disposable views tree.
    #[test]
    fn the_out_of_energy_view_is_bundled_and_seeded() {
        let dir = tempfile::tempdir().unwrap();
        install_factory_views(dir.path()).unwrap();
        let builtin = dir.path().join("views").join("factory");
        assert!(builtin.join("vendor-outage.jsx").is_file());

        let source = out_of_energy_view();
        assert_eq!(source, std::fs::read_to_string(builtin.join("vendor-outage.jsx")).unwrap());

        assert!(source.contains("Your energy is used up"));
        assert!(source.contains("hi-agent.xyz"));
        assert!(source.contains("消息都已保留"));
    }

    /// **Every worker specialism is named on the roster, or it shows up as a bare `Worker`.**
    ///
    /// The sessions view labels a row `TYPE[row.type] || ROLE[row.role]`, so a type missing
    /// from that table does not fail — it falls through to the word `Worker` and becomes
    /// indistinguishable from a general session on screen. That is not hypothetical: it is
    /// what `person-reader` did for as long as it existed, and the whole reason the type is
    /// carried on the wire at all is to answer "which kind of session was that".
    ///
    /// `general` is the one that must *not* be there — the role word already says `Worker`,
    /// and a `General` entry would print it twice.
    #[test]
    fn the_sessions_view_names_every_worker_specialism() {
        let source = REVIEW_VIEWS
            .iter()
            .find(|(name, _)| *name == "workers")
            .map(|(_, src)| *src)
            .expect("the sessions view is bundled");
        for t in crate::identity::WorkerType::ALL {
            let entry = format!("\"{}\":", t.as_str());
            if *t == crate::identity::WorkerType::General {
                assert!(!source.contains(&entry), "`general` is spelled by the role word alone");
            } else {
                assert!(source.contains(&entry), "{} has no label in the sessions view", t.as_str());
            }
        }
    }

    #[test]
    fn seeds_the_welcome_hero_and_its_mark() {
        let dir = tempfile::tempdir().unwrap();
        install_factory_views(dir.path()).unwrap();
        let builtin = dir.path().join("views").join("factory");
        // The first-hello view and its mark land, so `show` with ref
        // `factory/welcome` resolves.
        assert!(builtin.join("welcome.jsx").is_file());
        assert!(builtin.join("hi-mark.svg").is_file());
        // Reseeding is idempotent (overwrite, not append) — a second boot is clean.
        install_factory_views(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(builtin.join("welcome.jsx")).unwrap(), WELCOME);
    }

    #[test]
    fn seeds_every_review_surface() {
        let dir = tempfile::tempdir().unwrap();
        install_factory_views(dir.path()).unwrap();
        let builtin = dir.path().join("views").join("factory");
        for (name, source) in REVIEW_VIEWS {
            let jsx = builtin.join(format!("{name}.jsx"));
            assert!(jsx.is_file(), "{name}.jsx was not seeded");
            assert_eq!(&std::fs::read_to_string(&jsx).unwrap(), source);
        }
    }

    /// **A backtick inside a view's `CSS` template ends the template.** Every bundled view
    /// keeps its stylesheet in `` const CSS = `…` ``, so one backtick in a CSS comment —
    /// quoting a property name, the natural thing to write — closes the string early and the
    /// file stops being JavaScript. That shipped once: the whole Sessions view failed to
    /// compile, and nothing caught it, because the Rust tests here read the source as *text*
    /// and the view only meets a parser when esbuild compiles it at runtime, on a machine
    /// that has esbuild.
    ///
    /// This is the cheap half of that check — no toolchain, and it names the one mistake
    /// rather than waiting for a compile that may happen elsewhere.
    #[test]
    fn no_bundled_view_has_a_backtick_inside_its_css() {
        let dir = tempfile::tempdir().unwrap();
        install_factory_views(dir.path()).unwrap();
        let builtin = dir.path().join("views").join("factory");
        for entry in std::fs::read_dir(&builtin).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|x| x != "jsx") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            let Some(open) = source.find("const CSS = `") else { continue };
            let body = &source[open + "const CSS = `".len()..];
            let end = body.find("`;").unwrap_or(body.len());
            let stray = body[..end].find('`');
            assert!(
                stray.is_none(),
                "{}: a backtick inside the CSS template ends it — line {}",
                path.file_name().unwrap().to_string_lossy(),
                source[..open + stray.unwrap_or(0)].lines().count()
            );
        }
    }

    /// **Every bundled view actually compiles.** The rest of this module's tests read the
    /// sources as text — a purpose line, a stray backtick, a placement key — and text checks
    /// cannot see a syntax error. A bundled view with broken JSX ships, installs, and fails
    /// only when a person opens it, which is the worst place to find out and the one place
    /// nothing is watching.
    ///
    /// Skipped where esbuild is not provisioned, the same way [`super::super::tests`] skips
    /// its compile test — a host without the runtime is not a host this can answer for.
    #[tokio::test]
    async fn every_bundled_view_compiles() {
        let Some(esbuild_bin) = super::super::tests::esbuild_probe() else {
            eprintln!("skipping: esbuild not provisioned on this host");
            return;
        };
        let tmp = std::env::temp_dir().join(format!("hi-builtin-compile-{}", std::process::id()));
        let compiler = super::super::ViewCompiler::with_paths(esbuild_bin, tmp.clone());

        for (name, source) in REVIEW_VIEWS {
            compiler
                .compile(source)
                .await
                .unwrap_or_else(|err| panic!("bundled view `{name}` does not compile: {err}"));
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The toolbox is read by scanning the tree for `// purpose:` lines
    /// (`docs/arch/data.md#views`), and `factory/` is the only part of it a fresh
    /// install has. A bundled view without the line degrades to a bare filename in
    /// the one scan the builder runs before it authors anything — so every one of
    /// them opens with a purpose line, and it has to be the *first* line, since that
    /// is what `grep -n "^// purpose:"` returns.
    #[test]
    fn every_bundled_view_opens_with_a_purpose_line() {
        let dir = tempfile::tempdir().unwrap();
        install_factory_views(dir.path()).unwrap();
        let builtin = dir.path().join("views").join("factory");
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

    /// **A path in an attribute goes through `url()`.** Served under the community's
    /// subpath this page is at `/ana`, and a bare `src="/views/…"` then asks the
    /// community for the file and renders a broken image. `fetch` is covered by the
    /// host (`lib/base.ts`, `installBase`); an attribute has no such seam, so the
    /// bundled views have to say it, and the guidance in
    /// `identity/workers/view-builder.md` tells authored views the same.
    ///
    /// Matched on the literal attribute openers rather than every `"/` in the file:
    /// a path inside a `fetch` is fine, and flagging it would teach the wrong rule.
    #[test]
    fn no_bundled_view_puts_a_bare_absolute_path_in_an_attribute() {
        let dir = tempfile::tempdir().unwrap();
        install_factory_views(dir.path()).unwrap();
        let builtin = dir.path().join("views").join("factory");
        let mut names: Vec<&str> = vec!["upload", "people-review", "welcome", "vendor-outage"];
        names.extend(REVIEW_VIEWS.iter().map(|(n, _)| *n));
        for name in names {
            let source = std::fs::read_to_string(builtin.join(format!("{name}.jsx"))).unwrap();
            for bare in ["src=\"/", "href=\"/", "src={\"/", "href={\"/", "src={`/", "href={`/"] {
                assert!(
                    !source.contains(bare),
                    "{name}.jsx has a bare `{bare}…` — wrap the path in url() from @hi/core"
                );
            }
        }

        // And the one this was written for: the welcome poster's mark.
        assert!(WELCOME.contains(r#"url("/views/factory/hi-mark.svg")"#));
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

    /// **The task panel is the only place a waiting row's door is read, and it has no
    /// address bar.**
    ///
    /// A `waiting` line carries where the person acts on it
    /// (`docs/arch/data.md`), which is prose, which means a URL. Rendered inert it is a URL
    /// somebody has to retype off their own screen — the panel is a modal inside a native
    /// window, so there is nothing to paste it into and nothing to copy it from but the
    /// pixels. The renderer's stated rule used to be "no links" and the reason it gave was
    /// sound; what the rule missed is that an *autolink* is not the thing it was refusing.
    /// So both halves are checked: the anchor exists, and no markdown-label link ever does
    /// — a label with a separate href is text the agent wrote naming one destination and
    /// going to another.
    #[test]
    fn the_task_panel_makes_a_waiting_row_s_url_clickable() {
        let tasks = REVIEW_VIEWS.iter().find(|(name, ..)| *name == "tasks").unwrap().1;
        assert!(tasks.contains("AUTOLINK"), "tasks.jsx must find URLs in the record it renders");
        assert!(tasks.contains("hi-tasks__link"), "tasks.jsx must render them as anchors");
        assert!(
            tasks.contains("target=\"_blank\""),
            "a review URL opens in a real browser, not inside the panel"
        );
        // The timeline is where a `waiting` line lands, so it is the run that must go
        // through the inline renderer rather than being dropped in as a bare string.
        assert!(
            tasks.contains("{inline(moment.text,"),
            "the timeline must render through the inline vocabulary, or its URLs stay text"
        );
        // **Two link sites, and each one's anchor text *is* its destination.** That is the
        // property, and the count is only how it is held: a third site cannot appear without
        // failing here and having to say why it is self-naming too. The autolinker's anchor
        // is `{url}` going to `{url}`. The file link's is the code span the record wrote
        // going to that same token under the task's own folder — `linkFile` builds the path
        // out of the token and the server only answers for a regular file that resolves
        // inside the folder, so the worst it can do is open the file it plainly names.
        //
        // What neither can be is a *label*: text a session wrote naming one destination and
        // going to another. That is the reach being refused, and it needs a link site whose
        // text and href are separate bindings — which is what this count notices.
        assert_eq!(
            tasks.matches("href={").count(),
            2,
            "a link in this panel is the autolinker's or the record's own filename, and nothing else"
        );
        assert!(tasks.contains("href={url}") && tasks.contains("{url}\n      </a>"));
        assert!(
            tasks.contains("href={href}") && tasks.contains("<code>{token}</code>"),
            "the file link's anchor text is the token it opens"
        );
        assert!(
            tasks.contains("/files/${token"),
            "and its destination is built from that token, never from a separate binding"
        );
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
        // Reach carries three writes and each has to keep reaching for its endpoint —
        // a name that silently 404s looks exactly like a name that was refused.
        let reach = by_name("reach");
        assert!(reach.contains("/api/handle"), "reach must be able to claim a name");
        assert!(reach.contains("/api/pair"), "reach must be able to let a device in");
        assert!(reach.contains("/api/surfaces/"), "reach must be able to take one back");
        assert!(reach.contains("DELETE"));
        // Every one of them is a state change, and off-box the core refuses a bare
        // one as something a cross-site form could have sent.
        assert!(reach.contains("X-HI-Surface"), "reach's writes must be provably not cross-site");
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
        // The panel opens on the *fold* — the frames are the fallback reading, not the
        // first one. If this route goes, the page silently reverts to the wall of
        // fragments the fold exists to replace.
        assert!(source.contains("/messages"), "what the session did");
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
