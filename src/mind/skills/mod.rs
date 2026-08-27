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
//! | factory seeds | `<data_dir>/skills/factory/*.md` | rewritten every boot |
//! | everything the agent learnt | `<data_dir>/skills/**` (outside `factory/`) | never touched |
//!
//! The `factory/` prefix is the same convention the views tree already uses
//! ([`crate::mind::views::install_factory_views`]) — chosen over a sibling directory
//! so the workshop stays *one* place to look: a worker greps `skills/` and finds both
//! its own notes and the seeded ones, while an upgrade still has a single subtree it
//! owns and may clobber.
//!
//! # Tools live here too
//!
//! A **tool** is a skill with an invocation contract: the same note, plus `purpose:`
//! and `use:` front matter (`docs/arch/tools.md`). `purpose` is the line the registry
//! scan emits; the *presence* of `use` is what makes a note a tool rather than an
//! ordinary procedure, and its value is a command. There is no `tools/` tree.
//!
//! `use` names a command, and [`install_tool_bin`] is the other half: [`path_entries`]
//! is prepended to every session's PATH, so a note can say `browser` and mean
//! *whatever this machine turned out to have*. That is the whole point of the tree —
//! **`skills/` syncs and `bin/` does not**, so a portable note needs a stable name and
//! a machine-local binding for it. Most tools never appear in `bin/` at all: anything
//! a package manager put on the PATH already resolves without help.
//!
//! **`bin/` nests the same way, and for the same reason.** [`bin_dir`] is the agent's
//! own; [`factory_bin_dir`] inside it is ours, rewritten every boot. The learnt one is
//! searched first, so a shim the agent wrote shadows a seeded one of the same name —
//! deliberate override sticks, and an upgrade never silently reverts it.

use std::io;
use std::path::{Path, PathBuf};

/// Seeded skill: how to give hi-agent hands and eyes on another machine or phone.
/// Deliberately soft guidance — devices are tools plus a written procedure, not a
/// subsystem.
const ADDING_A_DEVICE: &str = include_str!("adding-a-device.md");

/// Seeded **tool**: this machine's Chrome under a stable name. Carries `purpose:`
/// and `use: browser`, and its `use` is bound by the shim [`install_tool_bin`]
/// writes.
const BROWSER: &str = include_str!("browser.md");

/// Resolve the `{…}` placeholders a seed may carry into this install's absolute
/// paths, the same way [`crate::identity`] does for prompts.
///
/// A note is read by a mind that will act on it, and a relative path resolves against
/// whatever cwd that session happens to have — which differs per rung. So a seed that
/// names a directory names it absolutely or not at all. A leftover `{placeholder}` on
/// disk is a note telling the agent to look somewhere that does not exist, which is
/// why [`tests::no_seed_leaves_an_unresolved_placeholder_on_disk`] pins it.
fn interpolate(note: &str, data_dir: &Path) -> String {
    let dir = |p: PathBuf| p.display().to_string();
    note.replace("{skills_dir}", &dir(skills_dir(data_dir)))
        .replace("{bin_dir}", &dir(bin_dir(data_dir)))
        .replace("{drive_dir}", &dir(data_dir.join("drive")))
}

/// The seeded browser tool note, for the cross-tree test in [`crate::identity`]:
/// the worker prompt there teaches `purpose:`/`use:` front matter, and without this
/// nothing pins that the note actually carries what the prompt describes.
#[cfg(test)]
pub(crate) fn browser_note() -> &'static str {
    BROWSER
}

/// Seeded skill: how to equip a tool the workshop does not have yet — the *writing*
/// half of the workshop, and the only path by which a learnt tool ever exists.
const EQUIPPING_A_TOOL: &str = include_str!("equipping-a-tool.md");

/// What a note's front matter says about it. Two keys and no more
/// (`docs/arch/tools.md`): `purpose` is the line the registry scan emits, and the
/// **presence** of `use` is the whole discriminator between a tool and an ordinary
/// procedure.
///
/// One parser, shared by the workshop API and the tests, for the same reason
/// [`crate::foundation::codex::messages::kind_of`] is shared: two copies of a
/// vocabulary are free to disagree about what a note *is*.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FrontMatter {
    /// One line saying what the note is for. `None` degrades to a bare filename —
    /// unhelpful, never a confident wrong answer.
    pub purpose: Option<String>,
    /// The command to run. Present iff this note is a tool.
    pub run: Option<String>,
}

impl FrontMatter {
    /// True when this note names something runnable.
    pub fn is_tool(&self) -> bool {
        self.run.is_some()
    }
}

/// Split a note into its front matter and its body.
///
/// Deliberately strict: the block must open on the very first line and close on a
/// line of its own. A note without one is all body, which is the common case — most
/// skills are procedures and carry no front matter at all.
///
/// **`description:` is read as `purpose:`.** They are the same idea, and the agent
/// runtime's own skills feature teaches the second spelling — watched 2026-08-27, a
/// worker asked to build a capability wrote a `SKILL.md` with `name:`/`description:`
/// rather than our two keys. Accepting the spelling the model already reaches for
/// costs nothing; a `purpose` line that reads as absent because it was spelt the
/// common way costs a tool that cannot be found. `name:` is ignored outright — the
/// tree is already addressed by path, and a key that restates it is free to disagree
/// with it.
pub fn split_front_matter(note: &str) -> (FrontMatter, &str) {
    let Some(rest) = note.strip_prefix("---\n") else {
        return (FrontMatter::default(), note);
    };
    let Some(end) = rest.find("\n---") else {
        // An unterminated block is not front matter; treating it as one would eat
        // the whole note and leave the view blank.
        return (FrontMatter::default(), note);
    };
    let (block, after) = rest.split_at(end);
    let body = after.trim_start_matches('\n').strip_prefix("---").unwrap_or(after).trim_start();

    let mut fm = FrontMatter::default();
    let mut described: Option<String> = None;
    for line in block.lines() {
        if let Some(v) = line.strip_prefix("purpose:") {
            fm.purpose = non_empty(v);
        } else if let Some(v) = line.strip_prefix("description:") {
            described = non_empty(v);
        } else if let Some(v) = line.strip_prefix("use:") {
            fm.run = non_empty(v);
        }
    }
    // `purpose` wins when a note carries both, since it is the key this design asked
    // for; `description` is the fallback rather than an equal.
    fm.purpose = fm.purpose.or(described);
    (fm, body)
}

/// The filename the agent runtime's own skills feature uses for the note inside a
/// skill directory. A note at `<dir>/SKILL.md` **is** the tool `<dir>` — the name
/// still comes from the tree, one level up.
pub const SKILL_FILE: &str = "SKILL.md";

fn non_empty(v: &str) -> Option<String> {
    let v = v.trim();
    (!v.is_empty()).then(|| v.to_string())
}

/// `<data_dir>/skills/` — the workshop root. Agent-written skills live directly
/// under here; the factory seeds live in the `factory/` subdirectory.
pub fn skills_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("skills")
}

/// Create the workshop and write the bundled seed skills into
/// `<data_dir>/skills/factory/`, overwriting each on every boot so a binary update
/// reseeds the latest (mirrors [`crate::identity::install_prompts`] and
/// [`crate::mind::views::install_factory_views`]).
///
/// Only `factory/` is rewritten. Anything the agent wrote elsewhere in the tree is
/// never read, moved or touched here — that separation is the point, not an
/// implementation detail.
pub fn install_factory_skills(data_dir: &Path) -> io::Result<()> {
    let dir = skills_dir(data_dir).join("factory");
    crate::mind::views::factory::rename_legacy_dir(&skills_dir(data_dir).join("_builtin"), &dir);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("adding-a-device.md"), ADDING_A_DEVICE)?;
    std::fs::write(dir.join("browser.md"), BROWSER)?;
    std::fs::write(dir.join("equipping-a-tool.md"), interpolate(EQUIPPING_A_TOOL, data_dir))?;
    tracing::info!(dir = %dir.display(), "installed bundled skills");
    Ok(())
}

/// `<data_dir>/bin` — where the **agent's own** scripts go, and the first of two PATH
/// entries.
///
/// **Machine-local and disposable.** A binary built on one machine does not run on
/// another, so unlike [`drive/`](crate::mind) this never syncs and may be deleted
/// whole (`docs/arch/tools.md`). Prepending rather than replacing is the load-bearing
/// half: everything a package manager already put on the PATH keeps resolving, so
/// most tools never appear here at all.
pub fn bin_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("bin")
}

/// `<data_dir>/bin/factory` — where **hi-agent's own** shims go, rewritten every boot.
///
/// **`bin/` nests because `skills/` nests.** The knowledge layer is path-scoped, so
/// `skills/factory/browser.md` and `skills/browser.md` are two notes that coexist;
/// a flat `bin/` had no such room, and two `browser` commands cannot. That mismatch
/// was the open question in `docs/arch/tools.md`, and the answer is to remove it
/// rather than to pick a winner: the execution layer gets the same split as the
/// knowledge layer.
///
/// **The learnt directory comes first on the PATH**, so a shim the agent wrote
/// deliberately shadows a seeded one of the same name. That direction is the whole
/// point — overriding a factory tool is a legitimate act, and an upgrade silently
/// reverting it is not. Boot may clobber anything under here and nothing above it.
pub fn factory_bin_dir(data_dir: &Path) -> PathBuf {
    bin_dir(data_dir).join("factory")
}

/// The agent's PATH entries, in the order they should be searched: what it wrote,
/// then what shipped. Callers prepend these to the inherited `PATH`, so the system's
/// own binaries come last and keep working.
pub fn path_entries(data_dir: &Path) -> [PathBuf; 2] {
    [bin_dir(data_dir), factory_bin_dir(data_dir)]
}

/// Create `<data_dir>/bin` and write the shims that bind a seeded tool note's `use:`
/// name to this machine.
///
/// **One shim today — `browser` — and it exists because the note cannot name its
/// target any other way.** [`crate::runtime::browser::ensure`] picks a system
/// Chrome, a canonical install location, or a pinned download, so the winning path
/// differs per machine (on macOS it is usually inside an `.app` bundle, on no PATH
/// at all), *and* a full Chrome must be told `--headless` while
/// `chrome-headless-shell` rejects the flag. The shim absorbs exactly that
/// difference. It publishes no interface of its own and passes every argument
/// through, because a tool's signature comes from its carrier at call time.
///
/// **Resolution is deferred to call time, deliberately.** `ensure` is lazy — a
/// managed browser is a ~100 MB download — so this writes a shim that asks on each
/// invocation instead of baking a path at boot. A machine that never opens a page
/// never downloads one, which is the property that would be lost by resolving here.
///
/// Rewritten every boot, like the factory notes beside it. Nothing else in the tree
/// is read or touched: a script the agent wrote itself lives here too and is none of
/// this function's business.
/// Only `bin/factory/` is written. The agent's own `bin/` is created and then left
/// alone — a script it wrote lives there and is none of this function's business.
pub fn install_tool_bin(data_dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(bin_dir(data_dir))?;
    let dir = factory_bin_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let exe = std::env::current_exe()?;
    write_browser_shim(&dir, &exe)?;
    tracing::info!(dir = %dir.display(), "installed tool shims");
    Ok(())
}

/// Single-quote a path for `sh`, so a space or a `$` in it cannot be re-read as
/// syntax. macOS puts the usual browser under `/Applications/Google Chrome.app/…`,
/// so this is the common case rather than the paranoid one.
fn sh_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}

/// The POSIX shim. `--resolve-browser` prints the argv prefix one element per line;
/// splitting on newline alone (`IFS`) with globbing off (`set -f`) keeps a path
/// containing spaces or `*` in one piece.
#[cfg(not(windows))]
fn write_browser_shim(dir: &Path, exe: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let script = format!(
        "#!/bin/sh\n\
         # Written by hi-agent at every start — see skills/factory/browser.md.\n\
         # Binds the name `browser` to whatever browser this machine has, adding\n\
         # --headless only if that binary needs telling. Everything you pass goes\n\
         # straight through to Chrome.\n\
         set -ef\n\
         IFS='\n'\n\
         set -- $({exe} --resolve-browser) \"$@\"\n\
         exec \"$@\"\n",
        exe = sh_quote(exe)
    );
    let path = dir.join("browser");
    std::fs::write(&path, script)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
}

/// The Windows shim. **Never exercised** — the Windows port cross-compiles and has
/// never been run (`docs/status.md`), so this mirrors the POSIX logic on paper: the
/// first line of output is the executable, any later line an argument to prepend.
#[cfg(windows)]
fn write_browser_shim(dir: &Path, exe: &Path) -> io::Result<()> {
    let script = format!(
        "@echo off\r\n\
         setlocal enabledelayedexpansion\r\n\
         set \"HIEXE=\"\r\n\
         set \"HIPRE=\"\r\n\
         for /f \"usebackq delims=\" %%i in (`\"\"{exe}\" --resolve-browser\"`) do (\r\n\
         \x20 if not defined HIEXE (set \"HIEXE=%%i\") else (set \"HIPRE=!HIPRE! %%i\")\r\n\
         )\r\n\
         \"!HIEXE!\" !HIPRE! %*\r\n",
        exe = exe.display()
    );
    std::fs::write(dir.join("browser.cmd"), script)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_seeds_the_builtin_layer() {
        let dir = tempfile::tempdir().unwrap();
        install_factory_skills(dir.path()).unwrap();
        let seeded = skills_dir(dir.path()).join("factory").join("adding-a-device.md");
        assert_eq!(std::fs::read_to_string(&seeded).unwrap(), ADDING_A_DEVICE);
    }

    #[test]
    fn reinstalling_refreshes_the_seed_and_leaves_agent_written_skills_alone() {
        let dir = tempfile::tempdir().unwrap();
        install_factory_skills(dir.path()).unwrap();

        // The agent writes its own skill next to the seeds, and edits a seed.
        let learnt = skills_dir(dir.path()).join("posting-a-clip.md");
        std::fs::write(&learnt, "what worked last time").unwrap();
        let nested = skills_dir(dir.path()).join("video").join("trimming.md");
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
        std::fs::write(&nested, "ffmpeg -ss ...").unwrap();
        let seeded = skills_dir(dir.path()).join("factory").join("adding-a-device.md");
        std::fs::write(&seeded, "stale").unwrap();

        // An upgrade replaces the factory layer …
        install_factory_skills(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&seeded).unwrap(), ADDING_A_DEVICE);
        // … and never touches the learnt one.
        assert_eq!(std::fs::read_to_string(&learnt).unwrap(), "what worked last time");
        assert_eq!(std::fs::read_to_string(&nested).unwrap(), "ffmpeg -ss ...");
    }

    /// One front-matter key, via the shared parser the workshop API also uses.
    fn front_matter(note: &str, key: &str) -> Option<String> {
        let (fm, _) = split_front_matter(note);
        match key {
            "purpose" => fm.purpose,
            "use" => fm.run,
            other => panic!("no such front-matter key: {other}"),
        }
    }

    #[test]
    fn front_matter_is_read_and_the_body_survives_it() {
        let (fm, body) = split_front_matter("---\npurpose: do a thing\nuse: thing\n---\n\n# T\n\nprose\n");
        assert_eq!(fm.purpose.as_deref(), Some("do a thing"));
        assert_eq!(fm.run.as_deref(), Some("thing"));
        assert!(fm.is_tool());
        assert!(body.starts_with("# T"), "the body must not keep the block: {body:?}");

        // A note with no block is all body — the common case, since most skills are
        // procedures.
        let (fm, body) = split_front_matter("# Just a skill\n\nhow it went\n");
        assert_eq!(fm, FrontMatter::default());
        assert!(!fm.is_tool());
        assert!(body.starts_with("# Just a skill"));

        // An unterminated block is not front matter. Treating it as one would eat the
        // whole note and leave the workshop view blank.
        let (fm, body) = split_front_matter("---\npurpose: oops\nno end marker\n");
        assert_eq!(fm, FrontMatter::default());
        assert!(body.starts_with("---"));

        // A blank value is absent, not an empty tool.
        let (fm, _) = split_front_matter("---\nuse:   \n---\nx\n");
        assert!(!fm.is_tool());
    }

    /// **`description:` is read as `purpose:`.** Watched 2026-08-27: asked to build a
    /// capability, a worker wrote the agent runtime's own skill shape — a directory
    /// with `SKILL.md` carrying `name:`/`description:` — and our reader saw no purpose
    /// line at all, so the tool was invisible to the registry. Accepting the spelling
    /// the model already reaches for is free; refusing it costs a tool that cannot be
    /// found.
    #[test]
    fn the_common_spelling_of_purpose_is_accepted() {
        let codex_shape = "---\nname: extract-webpage-markdown\ndescription: Extract a page into Markdown\n---\n\n# Extract\n";
        let (fm, body) = split_front_matter(codex_shape);
        assert_eq!(fm.purpose.as_deref(), Some("Extract a page into Markdown"));
        assert!(body.starts_with("# Extract"));
        // Still not a tool: nothing named a command, which is exactly the gap that
        // run showed and the reason `use:` survives as its own key.
        assert!(!fm.is_tool());

        // `purpose` wins when a note carries both — it is the key this design asked
        // for, and `description` is the fallback rather than an equal.
        let (fm, _) = split_front_matter("---\ndescription: second\npurpose: first\n---\nx\n");
        assert_eq!(fm.purpose.as_deref(), Some("first"));

        // `name:` is ignored outright; the tree already addresses the note.
        let (fm, _) = split_front_matter("---\nname: whatever\n---\nx\n");
        assert_eq!(fm.purpose, None);
    }

    #[test]
    fn a_learnt_shim_shadows_a_seeded_one_of_the_same_name() {
        // The collision rule, and the reason `bin/` nests at all: `skills/` is
        // path-scoped so two notes can share a name, and a flat `bin/` had no such
        // room. Learnt comes first, so overriding a factory tool sticks and an
        // upgrade never silently reverts it.
        let dir = tempfile::tempdir().unwrap();
        let entries = path_entries(dir.path());
        assert_eq!(entries[0], bin_dir(dir.path()));
        assert_eq!(entries[1], factory_bin_dir(dir.path()));
        assert!(
            entries[1].starts_with(&entries[0]),
            "the factory layer nests inside the agent's own, mirroring skills/"
        );

        // Boot writes only under `factory/`, and leaves a same-named script alone.
        install_tool_bin(dir.path()).unwrap();
        let learnt = bin_dir(dir.path()).join("browser");
        std::fs::write(&learnt, "#!/bin/sh\n# mine\n").unwrap();
        install_tool_bin(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&learnt).unwrap(), "#!/bin/sh\n# mine\n");
        let seeded = factory_bin_dir(dir.path())
            .join(if cfg!(windows) { "browser.cmd" } else { "browser" });
        assert!(seeded.exists(), "boot must still write its own layer");
    }

    #[test]
    fn no_seed_leaves_an_unresolved_placeholder_on_disk() {
        // A note is read by a mind that will act on it, so a `{placeholder}` reaching
        // disk is an instruction to look somewhere that does not exist. Prompts have
        // had this test since before skills carried any placeholder at all.
        let dir = tempfile::tempdir().unwrap();
        install_factory_skills(dir.path()).unwrap();
        let factory = skills_dir(dir.path()).join("factory");
        for entry in std::fs::read_dir(&factory).unwrap() {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            for placeholder in ["{skills_dir}", "{bin_dir}", "{drive_dir}", "{data_dir}"] {
                assert!(
                    !text.contains(placeholder),
                    "{path:?} still carries {placeholder}"
                );
            }
        }
        // And the interpolation actually landed an absolute path.
        let equipping = std::fs::read_to_string(factory.join("equipping-a-tool.md")).unwrap();
        assert!(equipping.contains(&skills_dir(dir.path()).display().to_string()));
        assert!(equipping.contains(&bin_dir(dir.path()).display().to_string()));
    }

    /// The seeded tool notes, and what each must say. `equipping-a-tool` is the
    /// *writing* half of the workshop — without it the learnt layer stays empty
    /// forever — so the two rules it exists to carry are pinned: exercise a real call
    /// before writing the note, and keep what only the person can create out of the
    /// disposable tree.
    #[test]
    fn the_equipping_seed_carries_the_two_rules_it_exists_for() {
        assert!(
            EQUIPPING_A_TOOL.contains("Do not write the note yet"),
            "a note for a call that never ran is the expensive kind of false confidence"
        );
        assert!(
            EQUIPPING_A_TOOL.contains("logged-in session is a credential"),
            "the one class of state a note cannot rebuild must be named"
        );
        assert!(
            EQUIPPING_A_TOOL.contains("Mark what rots"),
            "the perishable half must be marked or the next job trusts a stale note"
        );
        // It is a procedure, not a tool: there is no command to run.
        let (fm, _) = split_front_matter(EQUIPPING_A_TOOL);
        assert!(!fm.is_tool());
    }

    #[test]
    fn the_browser_note_is_a_tool_and_not_merely_a_skill() {
        // `purpose` is what the registry scan emits; the *presence* of `use` is the
        // whole discriminator between a tool and an ordinary procedure
        // (`docs/arch/tools.md`). A seed missing either is a note the workshop
        // cannot run.
        let purpose = front_matter(BROWSER, "purpose").expect("browser.md needs a purpose:");
        assert!(
            purpose.len() > 20,
            "the purpose line is what a job is matched against; it says too little: {purpose:?}"
        );
        assert_eq!(front_matter(BROWSER, "use").as_deref(), Some("browser"));

        // And the counter-example, so the discriminator is exercised in both
        // directions: adding a device is a procedure, with nothing to invoke.
        assert_eq!(front_matter(ADDING_A_DEVICE, "use"), None);
    }

    #[test]
    fn the_shim_is_named_by_the_note_that_calls_it() {
        // The command name in `use:` is the entire link between what the agent knows
        // and what it can run. If these two ever drift, the note names a command
        // that does not exist — which reads to the agent as "I can't do that".
        let dir = tempfile::tempdir().unwrap();
        install_tool_bin(dir.path()).unwrap();

        let name = front_matter(BROWSER, "use").unwrap();
        let shim = factory_bin_dir(dir.path()).join(if cfg!(windows) {
            format!("{name}.cmd")
        } else {
            name.clone()
        });
        assert!(shim.exists(), "browser.md says `use: {name}` but {shim:?} was not written");

        let script = std::fs::read_to_string(&shim).unwrap();
        // Resolution is deferred to call time — the shim asks, it does not carry a
        // path. Baking one here would force the ~100 MB download at boot.
        assert!(script.contains("--resolve-browser"), "the shim must resolve at call time");
        assert!(
            !script.contains("chrome-headless-shell-"),
            "the shim carries no resolved path: {script}"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn the_shim_is_executable_and_survives_a_path_with_spaces() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        install_tool_bin(dir.path()).unwrap();
        let shim = factory_bin_dir(dir.path()).join("browser");
        let mode = std::fs::metadata(&shim).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "a shim nobody may execute is not on the PATH");

        // macOS resolves the usual browser under `/Applications/Google Chrome.app/…`,
        // so an unquoted path is the common failure, not the paranoid one.
        assert_eq!(sh_quote(Path::new("/Applications/Google Chrome.app/x")), "'/Applications/Google Chrome.app/x'");
        assert_eq!(sh_quote(Path::new("/tmp/it's")), r"'/tmp/it'\''s'");
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
