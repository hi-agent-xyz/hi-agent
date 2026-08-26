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
//! `use` names a command, and [`install_tool_bin`] is the other half: `<data_dir>/bin`
//! is prepended to every session's PATH, so a note can say `browser` and mean
//! *whatever this machine turned out to have*. That is the whole point of the tree —
//! **`skills/` syncs and `bin/` does not**, so a portable note needs a stable name and
//! a machine-local binding for it. Most tools never appear in `bin/` at all: anything
//! a package manager put on the PATH already resolves without help.

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

/// The seeded browser tool note, for the cross-tree test in [`crate::identity`]:
/// the worker prompt there teaches `purpose:`/`use:` front matter, and without this
/// nothing pins that the note actually carries what the prompt describes.
#[cfg(test)]
pub(crate) fn browser_note() -> &'static str {
    BROWSER
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
    tracing::info!(dir = %dir.display(), "installed bundled skills");
    Ok(())
}

/// `<data_dir>/bin` — the agent's own PATH entry, prepended to every session's
/// environment.
///
/// **Machine-local and disposable.** A binary built on one machine does not run on
/// another, so unlike [`drive/`](crate::mind) this never syncs and may be deleted
/// whole (`docs/arch/tools.md`). Prepending rather than replacing is the load-bearing
/// half: everything a package manager already put on the PATH keeps resolving, so
/// most tools never appear here at all.
pub fn bin_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("bin")
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
pub fn install_tool_bin(data_dir: &Path) -> io::Result<()> {
    let dir = bin_dir(data_dir);
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

    /// Read one front-matter key out of a note, the way the registry scan reads
    /// `purpose`: anchored at the start of a line, in the block before any prose.
    fn front_matter(note: &str, key: &str) -> Option<String> {
        note.strip_prefix("---\n")?
            .split("\n---")
            .next()?
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}:")))
            .map(|v| v.trim().to_string())
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
        let shim = bin_dir(dir.path()).join(if cfg!(windows) {
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
        let shim = bin_dir(dir.path()).join("browser");
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
