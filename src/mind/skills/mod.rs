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
//! A **tool** is a note whose prose says how to run something. `purpose:` (or the
//! `description:` the agent runtime teaches) is the line the registry scan emits and
//! the one key code needs; `use:` names a command when a note happens to have one and
//! is a convenience, not a classification — see [`FrontMatter`]. There is no `tools/`
//! tree.
//!
//! When a note does carry `use:`, [`install_tool_bin`] is the other half: [`path_entries`]
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

/// Seeded **tool**: `hi mcp`, which turns a service that speaks only MCP into an
/// ordinary command. The note is the whole of *MCP is a command, not a carrier class*
/// as far as the agent is concerned.
const MCP_SERVICE: &str = include_str!("mcp-service.md");

/// What a note's front matter says about it.
///
/// **One key that matters and one convenience** (`docs/arch/tools.md`). `purpose` is
/// the line the registry scan emits, and it is the only thing code needs. `use` names
/// a command when the note happens to have one — useful, never load-bearing: it was
/// once meant to be what *made* a note a tool, and two live runs showed the agent does
/// not write it, so a discriminator built on it would have classified every learnt
/// tool as an ordinary procedure. What makes a note runnable is that its prose says
/// how to run it, which is where the format rule wanted it anyway.
///
/// One parser, shared by the workshop API and the tests, for the same reason
/// [`crate::foundation::codex::messages::kind_of`] is shared: two copies of a
/// vocabulary are free to disagree about what a note *is*.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FrontMatter {
    /// One line saying what the note is for. `None` degrades to a bare filename —
    /// unhelpful, never a confident wrong answer.
    pub purpose: Option<String>,
    /// The command to run, when the note names one. Absent on most notes, including
    /// every learnt one so far — see the type doc.
    pub run: Option<String>,
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

/// One note in the workshop: what it is called, where it is, and when it last changed.
#[derive(Debug, Clone)]
pub struct NoteRef {
    /// Path under `skills/` with no `.md` — and for the directory shape, the directory
    /// itself. This is the note's identity everywhere.
    pub id: String,
    pub path: PathBuf,
    pub modified: std::time::SystemTime,
}

/// Every note in the workshop, in no particular order.
///
/// **The one place that decides what counts as a note**, so the API listing, the
/// registry and any future reader cannot disagree. Two shapes: a `.md` file, or a
/// directory holding a [`SKILL_FILE`] — and that second one **ends the descent**,
/// because everything beside the note is the tool's payload rather than reading
/// material. Walking in once listed a vendored `LICENSE.md` as a skill.
///
/// Dotfiles are skipped as editor litter. A missing root is an empty workshop, not an
/// error — nothing has been learnt yet.
pub fn notes(data_dir: &Path) -> io::Result<Vec<NoteRef>> {
    let root = skills_dir(data_dir);
    let mut out = Vec::new();
    let mut stack = vec![(root.clone(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        for entry in rd {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let rel =
                if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                let note = entry.path().join(SKILL_FILE);
                if note.is_file() {
                    push_note(&mut out, note, rel);
                } else {
                    stack.push((entry.path(), rel));
                }
            } else if name.ends_with(".md") {
                push_note(&mut out, entry.path(), rel);
            }
        }
    }
    Ok(out)
}

fn push_note(out: &mut Vec<NoteRef>, path: PathBuf, rel: String) {
    let modified = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    let id = rel.strip_suffix(".md").unwrap_or(&rel).to_string();
    out.push(NoteRef { id, path, modified });
}

/// How many bytes of workshop inventory a session may carry.
///
/// **The cap is the budget, and the unit is bytes rather than a count** because one
/// note with a two-hundred-word purpose line costs what five terse ones do
/// (`docs/arch/tools.md`). ~1.5 KB is roughly 400 tokens: a real cost, and small
/// beside a single attached MCP server's schemas, which is the comparison that matters.
pub const HOT_BUDGET_BYTES: usize = 1536;

/// The inventory a session carries without asking: `name — purpose`, one line each,
/// truncated at [`HOT_BUDGET_BYTES`].
///
/// **A cut line, not a list.** Nothing stores which notes are in it; it is rebuilt at
/// every session open, so a tool reached for yesterday climbs on its own and one left
/// alone for a month falls out the same way, without anyone deciding either.
///
/// **Ranked by what was actually used**, from `usage` — counts of tool calls and shell
/// commands over a recent window
/// ([`crate::foundation::server::stats::recent_usage`]). A note is credited with the
/// command its `use:` names, and otherwise with its own last path segment, so
/// `web-to-markdown` is matched by a `web-to-markdown` invocation.
///
/// Freshness breaks ties, and only ties. It is a genuinely weaker signal — writing a
/// note is not using it — so it decides nothing except the order of things nobody has
/// run yet, where a newly written note is the better guess.
///
/// Returns an empty string for an empty workshop, so a caller can interpolate it
/// without special-casing a fresh install.
/// How often this note's tool was actually reached for.
///
/// A note is credited with the command its `use:` names — first word only, since usage
/// is counted by argv[0] — and otherwise with its own last path segment, so a
/// `web-to-markdown` note is matched by a `web-to-markdown` invocation. A note whose
/// name matches nothing scores zero and sorts on freshness, which is the right answer:
/// nobody has run it.
///
/// **Known over-credit: one binary hosting several tools shares one count.** `use: hi
/// mcp` is credited with every `hi` invocation, because the counter records the program
/// and not its subcommand — observed live, where `hi` ran three times and the MCP note
/// took all three. Harmless while `hi` is nearly always `hi mcp`, and it grows into a
/// real distortion if `hi` gains unrelated subcommands. Fixing it means recording argv[1]
/// for known multi-tool hosts, which is a special case waiting for a second example.
fn used_count(
    note: &NoteRef,
    fm: &FrontMatter,
    usage: &std::collections::HashMap<String, u64>,
) -> u64 {
    let from_use = fm
        .run
        .as_deref()
        .and_then(|cmd| cmd.split_whitespace().next())
        .and_then(|cmd| usage.get(cmd).copied());
    let stem = note.id.rsplit('/').next().unwrap_or(&note.id);
    from_use.unwrap_or_else(|| usage.get(stem).copied().unwrap_or(0))
}

pub fn hot_inventory(
    data_dir: &Path,
    budget: usize,
    usage: &std::collections::HashMap<String, u64>,
) -> String {
    let mut entries: Vec<(u64, NoteRef, FrontMatter)> = Vec::new();
    for note in notes(data_dir).unwrap_or_default() {
        let Ok(text) = std::fs::read_to_string(&note.path) else { continue };
        let (fm, _) = split_front_matter(&text);
        entries.push((used_count(&note, &fm, usage), note, fm));
    }
    // Most-used first, then freshest, then the id so the output is stable rather than
    // filesystem-ordered — two identical installs should produce the same prompt, and
    // a diff of it should be readable.
    entries.sort_by(|a, b| {
        b.0.cmp(&a.0).then_with(|| b.1.modified.cmp(&a.1.modified)).then_with(|| a.1.id.cmp(&b.1.id))
    });

    let mut out = String::new();
    for (_, note, fm) in entries {
        // A note with no purpose line degrades to a bare name — unhelpful, never a
        // confident wrong answer, which is the bargain the views toolbox already makes.
        let line = match fm.purpose {
            Some(purpose) => format!("- {} — {}\n", note.id, purpose),
            None => format!("- {}\n", note.id),
        };
        if out.len() + line.len() > budget {
            // Silently stopping at a budget is the failure shape this repo has paid
            // for before, so say what was dropped. The scan is still the floor
            // underneath: what is not in hand is one grep away.
            out.push_str("- (more in the workshop — scan it)\n");
            break;
        }
        out.push_str(&line);
    }
    out
}

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
    std::fs::write(dir.join("mcp-service.md"), MCP_SERVICE)?;
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
    write_hi_shim(&dir, &exe)?;
    tracing::info!(dir = %dir.display(), "installed tool shims");
    Ok(())
}

/// `hi` — the agent's own binary under a short name, so a note can say
/// `use: hi mcp <endpoint> call <tool> <json>`.
///
/// This is the whole of *MCP is a command*: one program on the PATH turns a service
/// that speaks only MCP into an ordinary note, with no loader and no carrier class.
/// It is a shim rather than a rename because the binary's real path is wherever this
/// install put it, and a note must not have to know that.
#[cfg(not(windows))]
fn write_hi_shim(dir: &Path, exe: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let script = format!(
        "#!/bin/sh\n\
         # Written by hi-agent at every start. `hi` is this agent's own binary — see\n\
         # `hi mcp --help`. Everything you pass goes straight through.\n\
         exec {exe} \"$@\"\n",
        exe = sh_quote(exe)
    );
    let path = dir.join("hi");
    std::fs::write(&path, script)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
}

/// **Never exercised** — same standing as the browser shim beside it.
#[cfg(windows)]
fn write_hi_shim(dir: &Path, exe: &Path) -> io::Result<()> {
    let script = format!("@echo off\r\n\"{exe}\" %*\r\n", exe = exe.display());
    std::fs::write(dir.join("hi.cmd"), script)
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
/// never been run on Windows, so this mirrors the POSIX logic on paper: the
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

    #[test]
    fn front_matter_is_read_and_the_body_survives_it() {
        let (fm, body) = split_front_matter("---\npurpose: do a thing\nuse: thing\n---\n\n# T\n\nprose\n");
        assert_eq!(fm.purpose.as_deref(), Some("do a thing"));
        assert_eq!(fm.run.as_deref(), Some("thing"));
        assert!(body.starts_with("# T"), "the body must not keep the block: {body:?}");

        // A note with no block is all body — the common case, since most skills are
        // procedures.
        let (fm, body) = split_front_matter("# Just a skill\n\nhow it went\n");
        assert_eq!(fm, FrontMatter::default());
        assert!(body.starts_with("# Just a skill"));

        // An unterminated block is not front matter. Treating it as one would eat the
        // whole note and leave the workshop view blank.
        let (fm, body) = split_front_matter("---\npurpose: oops\nno end marker\n");
        assert_eq!(fm, FrontMatter::default());
        assert!(body.starts_with("---"));

        // A blank value is absent, not an empty command.
        let (fm, _) = split_front_matter("---\nuse:   \n---\nx\n");
        assert_eq!(fm.run, None);
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
        // No command named — the shape the agent actually writes, and the reason
        // nothing is classified by whether `use:` is present.
        assert_eq!(fm.run, None);

        // `purpose` wins when a note carries both — it is the key this design asked
        // for, and `description` is the fallback rather than an equal.
        let (fm, _) = split_front_matter("---\ndescription: second\npurpose: first\n---\nx\n");
        assert_eq!(fm.purpose.as_deref(), Some("first"));

        // `name:` is ignored outright; the tree already addresses the note.
        let (fm, _) = split_front_matter("---\nname: whatever\n---\nx\n");
        assert_eq!(fm.purpose, None);
    }

    /// **The cap is the budget, and a budget that is not a test is not a budget.**
    /// "Add tools continuously" is pressure on exactly this tier, so the failure mode
    /// to prevent is a workshop that quietly grows the opening prompt forever.
    #[test]
    fn the_inventory_is_capped_and_says_what_it_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let skills = skills_dir(dir.path());
        std::fs::create_dir_all(&skills).unwrap();
        for i in 0..80 {
            std::fs::write(
                skills.join(format!("tool-{i:02}.md")),
                format!("---\npurpose: a reasonably wordy line about what tool {i:02} is for, long enough to cost real bytes\n---\n\nbody\n"),
            )
            .unwrap();
        }

        let out = hot_inventory(dir.path(), HOT_BUDGET_BYTES, &Default::default());
        assert!(
            out.len() <= HOT_BUDGET_BYTES + 40,
            "the inventory blew its budget: {} bytes",
            out.len()
        );
        // Silently stopping at a cap is the failure shape this repo has paid for.
        assert!(
            out.contains("more in the workshop"),
            "a truncated inventory must say so: {out}"
        );
        assert!(out.lines().count() < 80, "it did not actually cut anything");
    }

    #[test]
    fn the_inventory_is_a_cut_line_rebuilt_not_a_stored_list() {
        let dir = tempfile::tempdir().unwrap();
        install_factory_skills(dir.path()).unwrap();

        let before = hot_inventory(dir.path(), HOT_BUDGET_BYTES, &Default::default());
        assert!(before.contains("factory/browser"), "seeded tools are in hand: {before}");
        // A purpose line is what the entry carries; a note without one degrades to a
        // bare name rather than vanishing.
        assert!(before.contains("drive a real Chrome"), "{before}");

        // Nothing stored: a note written now appears at the next build, no bookkeeping.
        std::fs::write(
            skills_dir(dir.path()).join("just-learnt.md"),
            "---\npurpose: something the agent worked out today\n---\n\nbody\n",
        )
        .unwrap();
        let after = hot_inventory(dir.path(), HOT_BUDGET_BYTES, &Default::default());
        assert!(after.contains("just-learnt"), "{after}");
        assert!(
            after.find("just-learnt") < after.find("factory/browser"),
            "with nothing used, freshest leads: {after}"
        );

        // **Use beats freshness, which is the whole point.** A note written last week
        // that the install actually reaches for outranks one written a minute ago and
        // never run — writing a note is not using it.
        let usage = std::collections::HashMap::from([("browser".to_string(), 9u64)]);
        let ranked = hot_inventory(dir.path(), HOT_BUDGET_BYTES, &usage);
        assert!(
            ranked.find("factory/browser") < ranked.find("just-learnt"),
            "a used tool leads a fresh unused one: {ranked}"
        );

        // An empty workshop interpolates to nothing rather than to an apology.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(hot_inventory(empty.path(), HOT_BUDGET_BYTES, &Default::default()), "");
    }

    #[test]
    fn a_skill_directory_is_one_note_to_the_walk() {
        let dir = tempfile::tempdir().unwrap();
        let skills = skills_dir(dir.path());
        std::fs::create_dir_all(skills.join("web-to-markdown/scripts/vendor")).unwrap();
        std::fs::write(
            skills.join("web-to-markdown").join(SKILL_FILE),
            "---\ndescription: turn a URL into Markdown\n---\n\n# W\n",
        )
        .unwrap();
        std::fs::write(skills.join("web-to-markdown/scripts/vendor/LICENSE.md"), "MIT\n").unwrap();

        let ids: Vec<String> = notes(dir.path()).unwrap().into_iter().map(|n| n.id).collect();
        assert_eq!(ids, vec!["web-to-markdown".to_string()], "payload is not reading material");
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

    /// **The equipping seed is about finishing a job, not about building a tool.**
    ///
    /// It used to end by telling the worker to write the note. Two live runs showed why
    /// that is wrong: a worker inside one job cannot see whether the shape recurred, so
    /// under an instruction to equip it reliably resolves to *build* — once vendoring
    /// 76 MB of Python for a job that ad-hoc code would have finished. The investment
    /// decision moved to reflection, which is the only rung that reads across days.
    ///
    /// Both halves are pinned: the one it must still say (don't stall, get what the job
    /// needs) and the one it must no longer say (write it up).
    #[test]
    fn the_equipping_seed_finishes_jobs_and_does_not_build_tools() {
        assert!(
            EQUIPPING_A_TOOL.contains("getting it is part of the job"),
            "the anti-stall rule is the reason this note exists"
        );
        assert!(
            EQUIPPING_A_TOOL.contains("This is not about building a tool"),
            "the cost of a tool must be stated where the temptation is"
        );
        assert!(
            EQUIPPING_A_TOOL.contains("Do not write the note yet"),
            "a call that never ran is the expensive kind of false confidence"
        );
        assert!(
            EQUIPPING_A_TOOL.contains("logged-in session is a credential"),
            "the one class of state a note cannot rebuild must be named"
        );
        // It is a procedure: there is no command to run.
        let (fm, _) = split_front_matter(EQUIPPING_A_TOOL);
        assert_eq!(fm.run, None);
    }

    /// The ROI decision belongs to reflection, and nowhere else may claim it.
    #[test]
    fn only_reflection_decides_that_a_tool_should_exist() {
        let reflection = crate::identity::reflection_base();
        assert!(
            reflection.contains("an intention is not evidence"),
            "a stated plan to reuse something is not a usage count"
        );
        assert!(
            reflection.contains("costs a line in the window of *every* session"),
            "the cost is paid by jobs that never use the tool; that has to be said"
        );
        assert!(
            reflection.contains("the deciding is yours, the building is not"),
            "reflection weighs it and dispatches; it does not build inline"
        );
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
