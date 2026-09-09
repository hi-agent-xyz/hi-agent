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

/// Seeded skill: how to operate an app on a computer. **A note and not a tool, unlike
/// its two neighbours** — `browser` and `phone` each bind one binary under one name,
/// and a desktop has no such binary: capture, input synthesis and the accessibility
/// tree are a different mechanism on macOS, X11, Wayland and Windows, each with its
/// own grants. A shim would have to be rewritten per platform, and a capability in
/// this host would have to be written four times over; the judgment that reads a
/// screenshot is the same code everywhere, so that is the half that stayed. This
/// replaced the `hi_look` / `hi_act` tool pair, which had covered exactly one of
/// those platforms.
const DRIVING_A_DESKTOP: &str = include_str!("driving-a-desktop.md");

/// Seeded skill: how to equip a tool the workshop does not have yet — the *writing*
/// half of the workshop, and the only path by which a learnt tool ever exists.
const EQUIPPING_A_TOOL: &str = include_str!("equipping-a-tool.md");

/// Seeded **tool**: an Android handset under a stable name. Carries `purpose:` and
/// `use: phone`, bound by the shim [`install_tool_bin`] writes.
///
/// The note is Android-only and says so in its third paragraph, because the *name* is
/// the part that would otherwise lie — nothing about the word `phone` says "not an
/// iPhone", and a confident wrong answer about reach is the failure this repo keeps
/// paying for. What it points at instead for an iPhone is the shipped Action Button
/// handoff, not an apology.
const PHONE: &str = include_str!("phone.md");

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
    /// This skill's MCP server, as **the server object every MCP client already
    /// takes** — one line of JSON: `{"url": …, "headers": {…}}`, or
    /// `{"command": …, "args": [...], "env": {…}}`.
    ///
    /// **The whole registration.** `docs/arch/tools.md`: an MCP server is an ordinary
    /// skill whose front matter carries this object, and nothing else about it is stored
    /// anywhere — no copied schema, no residency level, no per-role list, no row in a
    /// table. Writing this line makes the server exist; deleting the file retires it.
    ///
    /// **Verbatim, rather than exploded into keys of our own.** The protocol specifies
    /// no config format, but every client takes the same object and every server's docs
    /// hand you one, so this is what a person already has. An earlier shape here had
    /// `mcp:` for a bare endpoint and `mcp_auth:` for one header, which could express
    /// exactly one header, could not express a spawned server's `env` at all, made the
    /// person translate what they were holding, and left this code sniffing a URL to
    /// guess a transport that `url` versus `command` already states. Every one of those
    /// is the cost of a dialect.
    ///
    /// Stored as text and parsed where it is used: a malformed line is one unusable
    /// server named in a warning, not a skill that fails to read.
    pub mcp: Option<String>,
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
        } else if let Some(v) = line.strip_prefix("mcp:") {
            fm.mcp = non_empty(v);
        }
    }
    // `purpose` wins when a note carries both, since it is the key this design asked
    // for; `description` is the fallback rather than an equal.
    fm.purpose = fm.purpose.or(described);
    (fm, body)
}

/// The MCP server object the skill called `name` carries, or `None` when no skill by
/// that name carries one — or when what it carries is not a JSON object.
///
/// **Resolved from the tree at call time, never from a registry.** The skill *is* the
/// registration (`docs/arch/tools.md`), so a server that was retired by deleting its
/// skill is gone the moment the file is, with nothing left holding a stale endpoint.
///
/// A bare name matches a skill at any depth — `abacad` finds `abacad.md` and
/// `vendors/abacad.md` alike — because the name a session reaches for is the one it
/// read off the inventory line, which is the note's own last segment. An exact path
/// wins over a suffix match, so a learnt skill deliberately shadowing a factory one is
/// still addressable in full.
pub fn mcp_server(data_dir: &Path, name: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    let mut suffix_match = None;
    for note in notes(data_dir).unwrap_or_default() {
        // One unreadable note must not answer for the whole workshop: `?` here would
        // report "no such server" for a permissions error three files away.
        let Ok(text) = std::fs::read_to_string(&note.path) else { continue };
        let (fm, _) = split_front_matter(&text);
        let Some(raw) = fm.mcp else { continue };
        let exact = note.id == name;
        if !exact && (note.id.rsplit('/').next() != Some(name) || suffix_match.is_some()) {
            continue;
        }
        // **`headers` is accepted as codex's `http_headers`.** The two spellings are the
        // one place the de-facto object and the runtime we feed it to disagree, and the
        // spelling in every published example is the one that would silently do nothing:
        // the server answers 401 and the reason is nowhere. Aliasing costs a line; not
        // aliasing costs the case this whole shape exists to serve.
        let server = match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(serde_json::Value::Object(mut o)) => {
                if let Some(h) = o.remove("headers") {
                    o.entry("http_headers").or_insert(h);
                }
                o
            }
            // Named in a warning rather than swallowed: a server that is registered and
            // unreachable because of a typo is exactly the failure that gets blamed on
            // the server.
            _ => {
                tracing::warn!(
                    skill = %note.id,
                    "`mcp:` is not a JSON object; expected one line like                      {{\"url\": \"https://…\"}}"
                );
                continue;
            }
        };
        if exact {
            return Some(server);
        }
        suffix_match = Some(server);
    }
    suffix_match
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
/// **Complete while it fits; ranked only when it can't be.** The cut is the only thing
/// that excludes anything — a note nobody has run is still in hand while there is
/// budget for it, and ranking decides who survives only once survival is contested.
/// This is the same tier as an agent runtime's skill-discovery layer (one line each,
/// always resident, the body on demand), differing in that theirs is complete by
/// construction and this one is complete only while it fits — which is why what it
/// dropped has to be said.
///
/// The rejected alternative was to make never-run mean never-resident outright. It
/// reads tidier and it is wrong: it empties the set on exactly the installs where
/// completeness costs nothing, and a fresh install that knows about nothing has to
/// *think* to scan. That is journey 07's live failure — a browser on the disk and "I
/// have no browser" going back — reintroduced as the default state.
///
/// **Whatever was left out is said, however it was left out.** A cut for budget and a
/// cut for never having been run are the same fact to the reader — that this is not the
/// workshop — and the failure this repo has paid for is the silent one. So a fresh
/// install, where nothing has been run yet, interpolates the invitation rather than
/// nothing: an empty spot under "what you have in hand" reads as *you have nothing*,
/// and that is precisely the confusion between an absent entry and an absent tool that
/// killed the truncated middle tier.
///
/// Returns an empty string only for a genuinely empty workshop — no notes at all —
/// where there is nothing to scan and so nothing to say.
/// How often this note's tool was actually reached for.
///
/// A note is credited with the command its `use:` names — first word only, since usage
/// is counted by argv[0] — and otherwise with its own last path segment, so a
/// `web-to-markdown` note is matched by a `web-to-markdown` invocation. A note whose
/// name matches nothing scores zero and sorts on freshness, which is the right answer:
/// nobody has run it.
///
/// **Known over-credit: one binary hosting several tools shares one count.** A note
/// whose `use:` names a multi-tool program is credited with every invocation of that
/// program, because the counter records argv[0] and not the subcommand — observed live
/// on a since-deleted note. Nothing on the PATH is shaped that way today; fixing it
/// means recording argv[1] for known multi-tool hosts, which is a special case waiting
/// for a second example.
///
/// A zero ranks last; it does not exclude. Note what the arithmetic can and cannot
/// see: a note naming no command can never accrue a count, so once a workshop outgrows
/// its budget a pure procedure sorts below every tool and falls out first, however
/// often it was *read* — reading one is credited to `sed`, not to the note. Whether
/// that matters is a question for a workshop big enough to cut, and the answer if it
/// does is to count opens, which the frame logs already record. Meanwhile the floor
/// under it is the scan, which is why every seeded note carries a `purpose:` line: the
/// scan greps for exactly that, and a note without one is invisible to it.
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
    // An empty workshop has nothing to say and nothing to scan; the caller
    // interpolates it without special-casing a fresh install.
    if entries.is_empty() {
        return String::new();
    }
    let in_workshop = entries.len();

    // Most-used first, then freshest, then the id so the output is stable rather than
    // filesystem-ordered — two identical installs should produce the same prompt, and
    // a diff of it should be readable.
    entries.sort_by(|a, b| {
        b.0.cmp(&a.0).then_with(|| b.1.modified.cmp(&a.1.modified)).then_with(|| a.1.id.cmp(&b.1.id))
    });

    let mut out = String::new();
    let mut in_hand = 0usize;
    for (_, note, fm) in entries {
        // A note with no purpose line degrades to a bare name — unhelpful, never a
        // confident wrong answer, which is the bargain the views toolbox already makes.
        let line = match fm.purpose {
            Some(purpose) => format!("- {} — {}\n", note.id, purpose),
            None => format!("- {}\n", note.id),
        };
        if out.len() + line.len() > budget {
            break;
        }
        out.push_str(&line);
        in_hand += 1;
    }
    // Silently stopping is the failure shape this repo has paid for before, so say
    // that something was left out — whether the cap cut it or nobody has ever run it.
    // The scan is the floor underneath either way: what is not in hand is one grep
    // away, and saying so is what keeps an absent *entry* from reading as an absent
    // *tool*.
    if in_hand < in_workshop {
        // `in_hand == 0` needs its own wording rather than "more": nothing is shown, so
        // there is no "more" to be had than. It takes a note whose purpose line alone
        // outgrows the whole budget, which is rare and not worth a confusing sentence.
        out.push_str(if in_hand == 0 {
            "- (the workshop is one scan away)\n"
        } else {
            "- (more in the workshop — scan it)\n"
        });
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
    // **Cleared, not just overwritten.** Everything under `factory/` is ours and is
    // rewritten here every boot; the agent's own notes live one level up, where
    // [`factory_bin_dir`] draws the same line for shims. Writing the seeds over
    // whatever was there left a retired one on disk forever — a note describing a
    // mechanism the binary no longer has, which is the failure this repo keeps paying
    // for. Removing the directory makes a deleted seed disappear by construction,
    // rather than by a list of past names that would itself go stale.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("adding-a-device.md"), ADDING_A_DEVICE)?;
    std::fs::write(dir.join("browser.md"), BROWSER)?;
    std::fs::write(dir.join("driving-a-desktop.md"), DRIVING_A_DESKTOP)?;
    std::fs::write(dir.join("equipping-a-tool.md"), interpolate(EQUIPPING_A_TOOL, data_dir))?;
    std::fs::write(dir.join("phone.md"), PHONE)?;
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
/// **Two of the three shims exist because a note cannot name its target any other
/// way** (`browser`, `phone`); the third (`hi`) is this binary under a short name. [`crate::runtime::browser::ensure`] picks a system
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
    write_phone_shim(&dir, &exe)?;
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
///
/// `--headed` is the one argument the shim reads rather than forwards, because it
/// decides the prefix instead of being part of it. It is matched **anywhere in the
/// args, not just first**: a caller writing `browser --headed <url>` and one writing
/// `browser <url> --headed` mean the same thing, and a positional rule that only one
/// of them satisfies is a trap with no way to discover it — `browser --help` is
/// Chrome's own help and will never mention this flag.
#[cfg(not(windows))]
fn write_browser_shim(dir: &Path, exe: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let script = format!(
        "#!/bin/sh\n\
         # Written by hi-agent at every start — see skills/factory/browser.md.\n\
         # Binds the name `browser` to whatever browser this machine has. Runs\n\
         # headless unless you pass --headed, which this script eats. Everything\n\
         # else goes straight through to Chrome.\n\
         set -ef\n\
         IFS='\n'\n\
         headed=\n\
         n=$#\n\
         while [ $n -gt 0 ]; do\n\
         \x20 a=$1; shift; n=$((n-1))\n\
         \x20 if [ \"$a\" = --headed ]; then headed=--headed; else set -- \"$@\" \"$a\"; fi\n\
         done\n\
         set -- $({exe} --resolve-browser $headed) \"$@\"\n\
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

/// The POSIX `phone` shim — the same argv-prefix contract as `browser` beside it, with
/// nothing to eat.
///
/// **It reads no argument of its own, and that is a decision rather than an omission.**
/// `browser` intercepts `--headed` because a window is something the *caller* wants and
/// the resolver cannot infer. adb has no such split: every flag it takes, including
/// `-s <serial>` for choosing between two attached handsets, is already adb's own and
/// means the same thing here. Adding a word of ours would put a second vocabulary in
/// front of a tool that documents itself, and `phone --help` would never mention it —
/// which is exactly the trap the browser shim's `--headed` has to be careful about.
#[cfg(not(windows))]
fn write_phone_shim(dir: &Path, exe: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let script = format!(
        "#!/bin/sh\n\
         # Written by hi-agent at every start — see skills/factory/phone.md.\n\
         # Binds the name `phone` to whatever adb this machine has. Everything goes\n\
         # straight through: `phone devices`, `phone shell input tap 500 900`.\n\
         set -ef\n\
         IFS='\n'\n\
         set -- $({exe} --resolve-phone) \"$@\"\n\
         exec \"$@\"\n",
        exe = sh_quote(exe)
    );
    let path = dir.join("phone");
    std::fs::write(&path, script)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
}

/// The Windows shim. **Never exercised**, same standing as the browser one above it.
#[cfg(windows)]
fn write_phone_shim(dir: &Path, exe: &Path) -> io::Result<()> {
    let script = format!(
        "@echo off\r\n\
         setlocal enabledelayedexpansion\r\n\
         set \"HIEXE=\"\r\n\
         set \"HIPRE=\"\r\n\
         for /f \"usebackq delims=\" %%i in (`\"\"{exe}\" --resolve-phone\"`) do (\r\n\
         \x20 if not defined HIEXE (set \"HIEXE=%%i\") else (set \"HIPRE=!HIPRE! %%i\")\r\n\
         )\r\n\
         \"!HIEXE!\" !HIPRE! %*\r\n",
        exe = exe.display()
    );
    std::fs::write(dir.join("phone.cmd"), script)
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

    /// The skill **is** the registration: an `mcp:` endpoint in front matter is the
    /// whole of what makes a server reachable, and `mcp_endpoint` resolves it from the
    /// tree rather than from anything stored beside it.
    #[test]
    fn an_mcp_endpoint_is_read_off_the_skill_that_declares_it() {
        let (fm, _) = split_front_matter(
            "---\npurpose: drive a remote handset\nmcp: {\"url\":\"https://example.com/mcp\"}\n---\n\nprose\n",
        );
        assert_eq!(fm.mcp.as_deref(), Some("{\"url\":\"https://example.com/mcp\"}"));

        let dir = tempfile::tempdir().unwrap();
        let skills = skills_dir(dir.path());
        std::fs::create_dir_all(skills.join("vendors")).unwrap();
        std::fs::write(
            skills.join("vendors/relay.md"),
            "---\npurpose: a handset relay\nmcp: {\"url\":\"https://relay.example/mcp\"}\n---\n\nhow\n",
        )
        .unwrap();
        std::fs::write(skills.join("plain.md"), "---\npurpose: just a procedure\n---\n\nhow\n")
            .unwrap();

        // Reached by the name the inventory line shows — the note's last segment —
        // and by its full path alike.
        let found = mcp_server(dir.path(), "relay").expect("reached by its bare name");
        assert_eq!(found["url"], "https://relay.example/mcp");
        assert_eq!(mcp_server(dir.path(), "vendors/relay"), Some(found));
        // A skill with no endpoint is not a server, and a name nobody claims is not one
        // either. Both answer `None`, which is what makes the verb's miss a work order
        // ("write that skill") rather than a not-found.
        assert_eq!(mcp_server(dir.path(), "plain"), None);
        assert_eq!(mcp_server(dir.path(), "nobody"), None);
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

        // **Complete while it fits.** A fresh install has run nothing, and every seed is
        // still in hand: ranking orders the set, the cap is the only thing that removes
        // from it. Emptying it on the installs where completeness is free is how an
        // agent ends up having to *think* to scan — journey 07's live failure.
        let cold = hot_inventory(dir.path(), HOT_BUDGET_BYTES, &Default::default());
        for seed in [
            "factory/browser",
            "factory/equipping-a-tool",
            // The one seed that is a note where a tool used to be. If driving a desktop
            // ever stops being reachable this way, it is reachable no way at all — there
            // is no `hi_look` behind it any more.
            "factory/driving-a-desktop",
        ] {
            assert!(cold.contains(seed), "a seed nobody ran is still in hand: {cold}");
        }
        // A purpose line is what the entry carries; a note without one degrades to a
        // bare name rather than vanishing.
        assert!(cold.contains("drive a real Chrome"), "{cold}");
        // Nothing was left out, so nothing claims it was.
        assert!(!cold.contains("more in the workshop"), "a complete set says nothing: {cold}");

        // Nothing stored: a note written now appears at the next build, no bookkeeping.
        let learnt = skills_dir(dir.path()).join("just-learnt.md");
        std::fs::write(&learnt, "---\npurpose: something the agent worked out today\n---\n\nbody\n")
            .unwrap();
        // **Say "newer" rather than assume it.** Freshness is read off mtime, and a
        // filesystem is free to stamp a whole burst of writes with one timestamp — this
        // box does, so every seed and this note tied and the ranking fell through to the
        // id, which is alphabetical and put the seeds first. The test was a coin flip
        // that happened to land right when it was written. What it means to assert is
        // that a *newer* note leads, so make it newer instead of hoping the clock does.
        std::fs::File::options()
            .write(true)
            .open(&learnt)
            .unwrap()
            .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60))
            .unwrap();
        let fresh = hot_inventory(dir.path(), HOT_BUDGET_BYTES, &Default::default());
        assert!(fresh.contains("just-learnt"), "{fresh}");
        assert!(
            fresh.find("just-learnt") < fresh.find("factory/browser"),
            "with nothing used, freshest leads: {fresh}"
        );

        let usage = std::collections::HashMap::from([
            ("browser".to_string(), 9u64),
            ("just-learnt".to_string(), 1u64),
        ]);
        let after = hot_inventory(dir.path(), HOT_BUDGET_BYTES, &usage);

        // **Use beats freshness, which is the whole point.** A note written last week
        // that the install actually reaches for outranks one written a minute ago and
        // barely run — writing a note is not using it.
        assert!(
            after.find("factory/browser") < after.find("just-learnt"),
            "a much-used tool leads a fresher, less-used one: {after}"
        );

        // An empty workshop interpolates to nothing rather than to an apology: there is
        // no scan to send anyone on.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(hot_inventory(empty.path(), HOT_BUDGET_BYTES, &Default::default()), "");
    }

    /// The scan is the floor under residency, so it has to actually reach everything:
    /// it greps for a `purpose:`/`description:` line, and a seed without one is
    /// invisible to it. That was true of the two procedure notes, which were reachable
    /// only because the inventory degrades a note with no purpose to a bare name — and
    /// once never-run notes stopped being resident, that accident would have hidden
    /// them completely.
    #[test]
    fn every_seeded_note_is_findable_by_the_scan() {
        let dir = tempfile::tempdir().unwrap();
        install_factory_skills(dir.path()).unwrap();

        for note in notes(dir.path()).unwrap() {
            let text = std::fs::read_to_string(&note.path).unwrap();
            let (fm, _) = split_front_matter(&text);
            assert!(
                fm.purpose.is_some(),
                "{} is invisible to `grep -rEn \"^(purpose|description):\"`",
                note.id
            );
        }
    }

    /// The shim is generated shell, so nothing else in the suite ever runs it — and
    /// the argument rotation that lets `--headed` appear anywhere is exactly the kind
    /// of POSIX-`sh` writing that typechecks by not being code. Run it for real
    /// against a stub resolver and look at the argv that comes out the far end.
    #[cfg(not(windows))]
    #[test]
    fn the_shim_eats_headed_wherever_it_appears_and_forwards_the_rest_in_order() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        // Stands in for the agent's own binary: answers `--resolve-browser` with a
        // prefix, and drops `--headless` when it was asked for a window.
        let stub = dir.path().join("stub-exe");
        std::fs::write(
            &stub,
            "#!/bin/sh\nprintf '/bin/echo\\n'\ncase \"$*\" in *--headed*) ;; *) printf -- '--headless\\n';; esac\n",
        )
        .unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        write_browser_shim(dir.path(), &stub).unwrap();
        let shim = dir.path().join("browser");

        let run = |args: &[&str]| -> String {
            let out = std::process::Command::new(&shim).args(args).output().unwrap();
            assert!(out.status.success(), "shim failed: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };

        // No window asked for: headless, and every argument survives untouched.
        assert_eq!(run(&["--window-size=1,2", "https://x/"]), "--headless --window-size=1,2 https://x/");
        // Leading and trailing both mean the same thing, and neither reaches Chrome.
        assert_eq!(run(&["--headed", "https://x/"]), "https://x/");
        assert_eq!(run(&["https://x/", "--headed"]), "https://x/");
        // Order is preserved through the rotation, which is the easy thing to break.
        assert_eq!(run(&["a", "--headed", "b", "c"]), "a b c");
        assert_eq!(run(&[]), "--headless");
    }

    /// The `phone` shim forwards **everything**, and the case that matters is
    /// `--headed`: it is the one word its neighbour intercepts, so if this shim were
    /// ever written by copying that one, this is where it would show. adb owns its
    /// whole argument surface here, `-s <serial>` included.
    #[cfg(not(windows))]
    #[test]
    fn the_phone_shim_intercepts_nothing_including_its_neighbours_flag() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        // Stands in for the agent's own binary answering `--resolve-phone`.
        let stub = dir.path().join("stub-exe");
        std::fs::write(&stub, "#!/bin/sh\nprintf '/bin/echo\\n'\n").unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        write_phone_shim(dir.path(), &stub).unwrap();
        let shim = dir.path().join("phone");

        let run = |args: &[&str]| -> String {
            let out = std::process::Command::new(&shim).args(args).output().unwrap();
            assert!(out.status.success(), "shim failed: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };

        assert_eq!(run(&["devices"]), "devices");
        assert_eq!(run(&["shell", "input", "tap", "500", "900"]), "shell input tap 500 900");
        // Selecting between two attached handsets is adb's own flag, not ours.
        assert_eq!(run(&["-s", "R5CT10", "shell", "screencap"]), "-s R5CT10 shell screencap");
        // The neighbour's flag is just an argument here.
        assert_eq!(run(&["--headed", "devices"]), "--headed devices");
        assert_eq!(run(&[]), "");
    }

    /// A path with a space in it stays one argument — on a packaged install the agent's
    /// own binary lives inside `Hi Agent.app`.
    #[cfg(not(windows))]
    #[test]
    fn the_phone_shim_survives_a_space_in_the_binary_path() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let spaced = dir.path().join("Hi Agent");
        std::fs::create_dir_all(&spaced).unwrap();
        let stub = spaced.join("stub-exe");
        std::fs::write(&stub, "#!/bin/sh\nprintf '/bin/echo\\n'\n").unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        write_phone_shim(dir.path(), &stub).unwrap();

        let out =
            std::process::Command::new(dir.path().join("phone")).arg("devices").output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), "devices");
    }

    /// Boot writes a shim for every seeded note that names a command, so a note saying
    /// `use: phone` cannot ship without the name existing.
    #[test]
    fn every_seeded_use_line_has_a_shim_behind_it() {
        let dir = tempfile::tempdir().unwrap();
        install_factory_skills(dir.path()).unwrap();
        install_tool_bin(dir.path()).unwrap();

        let factory = skills_dir(dir.path()).join("factory");
        for entry in std::fs::read_dir(&factory).unwrap() {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            let (fm, _) = split_front_matter(&text);
            // `use:` may name a command with arguments; the shim is the first word.
            let Some(name) = fm.run.as_deref().and_then(|r| r.split_whitespace().next()) else {
                continue;
            };
            let bin = factory_bin_dir(dir.path())
                .join(if cfg!(windows) { format!("{name}.cmd") } else { name.to_string() });
            assert!(bin.exists(), "{path:?} says `use: {name}` but {bin:?} was never written");
        }
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

    /// Writing the workshop belongs to reflection, and nowhere else may claim it —
    /// a note as much as a tool. The argument was always general: a note written from
    /// inside one job is written without the evidence that would justify it, and the
    /// rung that just did something once cannot know it was the fifth time. It used to
    /// be applied to tools only, so two prompts asked the hands to leave a note behind
    /// while the note they read on the way told them not to.
    #[test]
    fn only_reflection_writes_the_workshop() {
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
        // Being the only entrance is load-bearing and has to be said as such: whatever
        // this pass does not write down is what the agent forgets.
        assert!(
            reflection.contains("nothing that does the work writes the workshop"),
            "the rule has to be stated where it is applied"
        );

        // Every prompt that has a workshop section, found by having one rather than by
        // being listed here — the paragraph is copied verbatim across four workers, and
        // fixing the ones that came to mind left three of them contradicting the note
        // those same workers read on the way in.
        let mut checked = 0;
        for (name, base) in crate::identity::all_bases() {
            if !base.contains("{skills_dir}") || name == "reflection" {
                continue;
            }
            checked += 1;
            assert!(
                base.contains("Reading the workshop is yours; writing it is not"),
                "{name} sees the workshop and is not told which half is its"
            );
            assert!(
                !base.contains("leave a note behind") && !base.contains("leave a short note"),
                "{name} still asks the hands to write the workshop mid-errand"
            );
        }
        assert!(checked >= 5, "only {checked} prompts checked; the filter stopped matching");
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
