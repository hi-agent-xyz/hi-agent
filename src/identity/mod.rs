//! identity — who the agent is.
//!
//! The factory-authored character (`core.md`, `reaction.md`, `meaning.md`) and the
//! per-install authored `self.md`. Standing duties are no longer here at all — they
//! are tasks ([`crate::mind::memory::tasks`]), one ledger, projected into every
//! window rather than pointed at by a path. This module owns the two shapes a rung's
//! self can take — the **seed** an agentic rung Reads its character from
//! ([`character_seed`]) and the **inlined** brief the voice gets because it cannot read
//! ([`reactor_system_prompt`]) — and the **prompt cascade** that materialises the
//! bundled prompts under `<data_dir>/prompts/`, composing each managed base with an
//! optional operator `*.local.md` override ([`install_prompts`]). That cascade is
//! the base‹override mechanism `docs/arch/foundation.md` generalises to base‹user‹self.
//!
//! Scope notes for the in-flight refactor:
//! - `install_prompts` still materialises the **view-builder guides** (`appearance.md`,
//!   `aesthetic.md`) and the **reflection** instruction (`reflection.md`) alongside the
//!   identity prompts — they share one cascade. A later slice moves those non-identity
//!   prompts to where they belong (mind / the loop), leaving identity with just
//!   `core`/`reaction`/`meaning`.
//! - `self.md` still lives under `<data_dir>/memory/` for now (no data migration).
//!   Under `docs/arch/data.md#memoryprompts` it does not get relocated at all: who
//!   this install is becomes a *section* of a generated prompt, so the file goes away
//!   with the change that gives Deliberation the writer's job.

use std::path::{Path, PathBuf};

/// Built-in base prompts, embedded at compile time and materialised to disk by
/// [`install_prompts`]. They divide by which rung can *fetch*: an agentic rung is
/// handed `core.md` — who it is and how it works — and `meaning.md` — that its purpose
/// is its own to find — by [`character_seed`], and Reads them itself. `reaction.md` is
/// the exception among the identity prompts: the voice is tools-off, so its whole
/// brief is **inlined** by [`reactor_system_prompt`]. `appearance.md`
/// and `aesthetic.md` are the view builder's guides — the mechanics of authoring/saving
/// a view, and the taste it has to clear — read off disk by a build sub-agent.
/// `reflection.md` is the exception: it is the consolidation session's whole instruction
/// set, so it is **inlined** as that session's system prompt (see [`reflection_prompt`])
/// rather than Read. All ship in the binary and refresh on every build.
const CORE_BASE: &str = include_str!("core.md");
const REACTION_BASE: &str = include_str!("reaction.md");
const MEANING_BASE: &str = include_str!("meaning.md");
const APPEARANCE_BASE: &str = include_str!("appearance.md");
const AESTHETIC_BASE: &str = include_str!("aesthetic.md");
const REFLECTION_BASE: &str = include_str!("reflection.md");
const DELIBERATION_BASE: &str = include_str!("deliberation.md");
const COGNITION_BASE: &str = include_str!("cognition.md");

/// The worker prompts, under `workers/`. `common.md` is what every working session is;
/// the rest are one file per **type**, layered on top of it.
///
/// Two copies of "report to your owner" is how three mail renderers became three
/// different strings, so the shared half is shared. Each half keeps its own
/// `.local.md`, so an operator can retune what every worker is told *or* just the one
/// kind, without editing the other.
const WORKER_COMMON_BASE: &str = include_str!("workers/common.md");
const WORKER_GENERAL_BASE: &str = include_str!("workers/general.md");
const WORKER_VIEW_BUILDER_BASE: &str = include_str!("workers/view-builder.md");
const WORKER_VIEW_REVIEWER_BASE: &str = include_str!("workers/view-reviewer.md");
const WORKER_DECISION_MAKER_BASE: &str = include_str!("workers/decision-maker.md");
const WORKER_FILE_FILER_BASE: &str = include_str!("workers/file-filer.md");

/// What kind of working session this is — the `type` in `CreateWorker(type)`
/// (`docs/arch/foundation.md#the-agent-session-registry`).
///
/// **A type selects a prompt and nothing else.** Every worker runs the same session
/// with the same tools; `docs/arch/agents.md` is explicit that a new role here is a new
/// prompt, not new machinery. So this enum exists to name a file, and adding a kind
/// means adding a `.md` and a variant — never a code path.
///
/// It exists at all because the alternative was what shipped until now: one monolithic
/// prompt with conditional paragraphs (*"When your task is to file a file…"*, *"When
/// your task is to build a view…"*), leaving the model to work out which of them it
/// was. Every worker paid the context of every specialism, and nothing could be said to
/// one kind that would not also be read by the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkerType {
    /// Whatever the task is. The default, and the right answer for most work.
    #[default]
    General,
    /// Builds a view for the person to look at.
    ViewBuilder,
    /// Renders a built view, looks at it, and says whether it ships
    /// (`docs/arch/agents.md#workers`).
    ViewReviewer,
    /// Makes a call so work can continue without the person
    /// (`docs/arch/agents.md#decision-maker`).
    DecisionMaker,
    /// Files something the person handed over into `drive/`.
    FileFiler,
}

impl WorkerType {
    /// The wire name, which is also the prompt's filename stem.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::ViewBuilder => "view-builder",
            Self::ViewReviewer => "view-reviewer",
            Self::DecisionMaker => "decision-maker",
            Self::FileFiler => "file-filer",
        }
    }

    /// Every type, for the tool schema's `enum` and for install/test sweeps. One list,
    /// so a new variant cannot be advertised in one place and forgotten in the other.
    pub const ALL: &'static [Self] =
        &[
        Self::General,
        Self::ViewBuilder,
        Self::ViewReviewer,
        Self::DecisionMaker,
        Self::FileFiler,
    ];

    /// Parse a wire name. `None` for anything unknown — the caller turns that into a
    /// tool error naming the valid set, rather than silently handing back a general
    /// worker: a mistyped `view-buidler` that quietly becomes a general session is a
    /// worker that will not do the job it was made for, and nothing says so.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|t| t.as_str() == s.trim())
    }

    /// The embedded base for this type's layer.
    fn base(self) -> &'static str {
        match self {
            Self::General => WORKER_GENERAL_BASE,
            Self::ViewBuilder => WORKER_VIEW_BUILDER_BASE,
            Self::ViewReviewer => WORKER_VIEW_REVIEWER_BASE,
            Self::DecisionMaker => WORKER_DECISION_MAKER_BASE,
            Self::FileFiler => WORKER_FILE_FILER_BASE,
        }
    }
}

/// Separator that introduces the operator's override layer. Placed after the
/// bundled base so its instructions take precedence — the model honors the
/// later, more specific guidance where the two conflict.
const OVERRIDE_HEADER: &str = "\n\n# Operator overrides\n\nThe operator added the guidance below. It layers on top of everything above; where the two conflict, follow this.\n\n";

/// Compose a bundled base prompt with an optional operator override layer. The
/// base is the embedded current text; `<prompts_dir>/<local_name>` (e.g.
/// `core.local.md`) holds only the operator's deltas, appended under
/// [`OVERRIDE_HEADER`] so later, more-specific guidance wins. Missing or empty
/// override ⇒ the base verbatim, so it can neither go stale nor shadow updates.
fn compose_prompt(base: &str, prompts_dir: &Path, local_name: &str) -> String {
    let path = prompts_dir.join(local_name);
    match std::fs::read_to_string(&path) {
        Ok(text) if !text.trim().is_empty() => format!("{base}{OVERRIDE_HEADER}{}", text.trim()),
        _ => base.to_string(),
    }
}

/// Install the bundled prompts under `<data_dir>/prompts/` at startup, composing
/// each with its optional `*.local.md` operator override. The managed base files
/// (`core.md`, `reaction.md`, `meaning.md`, `appearance.md`, `aesthetic.md`,
/// `reflection.md`) are rewritten every boot so they stay current; operator edits
/// live in the never-touched `*.local.md` siblings. Each follows one workflow: ship
/// embedded → materialise here → consumed from disk at runtime.
pub fn install_prompts(data_dir: &Path) -> std::io::Result<()> {
    let dir = data_dir.join("prompts");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("core.md"), compose_prompt(CORE_BASE, &dir, "core.local.md"))?;
    std::fs::write(dir.join("reaction.md"), compose_prompt(REACTION_BASE, &dir, "reaction.local.md"))?;
    std::fs::write(dir.join("meaning.md"), compose_prompt(MEANING_BASE, &dir, "meaning.local.md"))?;
    std::fs::write(dir.join("appearance.md"), compose_prompt(APPEARANCE_BASE, &dir, "appearance.local.md"))?;
    std::fs::write(dir.join("aesthetic.md"), compose_prompt(AESTHETIC_BASE, &dir, "aesthetic.local.md"))?;
    std::fs::write(dir.join("reflection.md"), compose_prompt(REFLECTION_BASE, &dir, "reflection.local.md"))?;
    std::fs::write(dir.join("deliberation.md"), compose_prompt(DELIBERATION_BASE, &dir, "deliberation.local.md"))?;
    std::fs::write(dir.join("cognition.md"), compose_prompt(COGNITION_BASE, &dir, "cognition.local.md"))?;

    // The worker prompts get their own subdirectory, because there is one per type and
    // they would otherwise be the majority of a flat `prompts/`. `common.md` sits
    // alongside them rather than above them: it is the layer every type is composed
    // with, and an operator retunes it the same way as any other.
    let workers = dir.join("workers");
    std::fs::create_dir_all(&workers)?;
    std::fs::write(
        workers.join("common.md"),
        compose_prompt(WORKER_COMMON_BASE, &workers, "common.local.md"),
    )?;
    for t in WorkerType::ALL {
        std::fs::write(
            workers.join(format!("{}.md", t.as_str())),
            compose_prompt(t.base(), &workers, &format!("{}.local.md", t.as_str())),
        )?;
    }

    tracing::info!(
        dir = %dir.display(),
        types = WorkerType::ALL.len(),
        "installed bundled prompts (core.md, reaction.md, meaning.md, appearance.md, aesthetic.md, reflection.md, deliberation.md, cognition.md, workers/)"
    );
    Ok(())
}

/// A working session's whole system prompt: `workers/common.md` — what every worker is
/// — then `workers/<type>.md`, the layer for this kind of job.
///
/// Read off disk so an operator's `*.local.md` reaches a worker the same way it reaches
/// every other rung, falling back to the embedded bases when a file is missing or
/// empty. Read fresh per spawn, so an edit takes effect without a restart.
///
/// **This replaced a `const &str` in `reactor/workers.rs`** — the one role prompt that
/// was not a bundled `.md`, and so the one nobody could retune without a rebuild.
///
/// Two placeholders are interpolated, both because a worker reaches hi-agent's own
/// surfaces over HTTP and has to name the right scene:
/// - `{scene}` — the scene as the `X-HI-Scene` header wants it.
/// - `{scene_dir}` — the same scene as it appears **on disk**, which is percent-encoded
///   (`alice@phone` lives at `alice%40phone`). Substituting the raw form here pointed
///   the filing worker at a directory that did not exist, for every scene with an `@`
///   in it.
pub async fn worker_prompt(data_dir: &Path, scene: &crate::types::Scene, kind: WorkerType) -> String {
    let dir = data_dir.join("prompts").join("workers");
    let read = |name: String, fallback: &'static str| {
        let path = dir.join(name);
        async move {
            match tokio::fs::read_to_string(&path).await {
                Ok(s) if !s.trim().is_empty() => s,
                _ => fallback.to_string(),
            }
        }
    };
    let common = read("common.md".to_string(), WORKER_COMMON_BASE).await;
    let layer = read(format!("{}.md", kind.as_str()), kind.base()).await;

    let scene_dir = crate::mind::memory::layout::encode_scene(scene);
    format!("{}\n\n{}", common.trim(), layer.trim())
        .replace("{scene_dir}", &scene_dir)
        // The header value, which is the scene id verbatim.
        .replace("{scene}", &scene.0)
}

/// **Deliberation**'s role layer — what it is beyond being a working session, and the
/// job that makes it load-bearing: writing its scene's
/// [generated prompt](../../docs/arch/data.md#memoryprompts), the brief Reaction reads
/// every turn and cannot write for itself.
///
/// This is a *layer*, not a whole prompt. Deliberation genuinely is a working session —
/// same tools, same capability guidance — so the caller appends this to the worker base
/// rather than replacing it. Read fresh each spawn (operator-overridable via
/// `deliberation.local.md`), so an edit takes effect without a restart.
///
/// `{scene_memory}` is interpolated to the **absolute** path of the file it must write,
/// because an agent-facing path that is relative is a path to the wrong file.
pub async fn deliberation_prompt(data_dir: &Path, scene: &crate::types::Scene) -> String {
    let path = data_dir.join("prompts").join("deliberation.md");
    let base = match tokio::fs::read_to_string(&path).await {
        Ok(s) if !s.trim().is_empty() => s,
        _ => DELIBERATION_BASE.to_string(),
    };
    let target = crate::mind::memory::layout::scene_prompt_path(data_dir, scene);
    base.replace("{scene_memory}", &target.display().to_string())
}

/// **Cognition**'s role layer — the sceneless brain that owns the task ledger, hands
/// work out, and never speaks (`docs/arch/agents.md#cognition`).
///
/// A layer, like Deliberation's: the caller appends it under
/// [`character_seed`], which is what makes the rung *the agent* rather than a generic
/// assistant. It carries the ledger pen — the instruction that lived in
/// `deliberation.md` marked "for now, yours" until Cognition existed to take it back.
///
/// Nothing is interpolated here, unlike Deliberation's `{scene_memory}`. What Cognition
/// carries forward arrives in the **window** with the projected ledger
/// ([`crate::mind::memory::snapshot::agent_window`]), not in this layer — the same
/// content in both places would be one copy going stale against the other, and the window
/// is the half that is rebuilt every turn.
pub async fn cognition_prompt(data_dir: &Path) -> String {
    let path = data_dir.join("prompts").join("cognition.md");
    match tokio::fs::read_to_string(&path).await {
        Ok(s) if !s.trim().is_empty() => s,
        _ => COGNITION_BASE.to_string(),
    }
}

/// The reflection ("sleep") session's system prompt: the materialised
/// `<data_dir>/prompts/reflection.md` (operator-overridable via `reflection.local.md`),
/// or the embedded [`REFLECTION_BASE`] when that file is missing or empty. Unlike
/// `core.md`/`reaction.md`, this is **inlined** as the reflection session's system
/// prompt rather than Read by the agent — it *is* the task's instructions, so it must
/// be present before the session can act. Read fresh each round, so an operator edit
/// takes effect without a restart.
pub async fn reflection_prompt(data_dir: &Path) -> String {
    let path = data_dir.join("prompts").join("reflection.md");
    match tokio::fs::read_to_string(&path).await {
        Ok(s) if !s.trim().is_empty() => s,
        _ => REFLECTION_BASE.to_string(),
    }
}

/// **Reaction**'s system prompt — the scene's voice (`docs/arch/agents.md#reaction`).
///
/// Unlike [`character_seed`] (a thin seed pointing an *agentic* rung at files to
/// Read), Reaction is tools-off by design: it has no Read, so nothing it needs may be
/// a path. Its brief is therefore **inlined and singular** — `reaction.md` *is* its
/// whole system prompt, verbatim. That is what makes speaking-rule conformance
/// structural: the rules are the entire context, not one buried file among many.
/// (Mirrors how `reflection.md` is inlined for the reflection session.)
///
/// **The frame that used to sit above the file is now the top of the file.** It was
/// ~40 lines of Rust string literal carrying the two things a reader would look for
/// first — that Reaction is one self rather than a dispatcher with colleagues, and
/// what its two tools are — which meant the voice's brief lived in two places, only
/// one of them operator-overridable. A prompt is prose; it belongs in the `.md`. The
/// file is now named for the rung that reads it (`docs/arch/arch.md#character`: a
/// file per role) rather than for the activity, which is what `speaking.md` was.
///
/// Read from `<data_dir>/prompts/reaction.md`, so an operator's `reaction.local.md`
/// reaches the voice too, falling back to the embedded [`REACTION_BASE`]. Two things
/// stay in code because they are *state*, not character, and the voice cannot fetch
/// either: the **first-meeting** cue and the **language** preference.
pub async fn reactor_system_prompt(data_dir: &Path) -> String {
    let base = data_dir.join("prompts").join("reaction.md");
    let reaction = match tokio::fs::read_to_string(&base).await {
        Ok(s) if !s.trim().is_empty() => s,
        _ => REACTION_BASE.to_string(),
    };
    let mut prompt = reaction.trim().to_string();
    if is_first_meeting(data_dir) {
        prompt.push_str(FIRST_MEETING_CUE);
    }
    if let Some(lang) = language_line(data_dir) {
        prompt.push_str(&lang);
    }
    prompt
}

/// One extra line on a genuine first meeting — the brand-new install where nothing has
/// accrued yet. It disappears on its own the moment any history exists (a memory
/// episode, the first reflected `hot.md`, a duty written), so it can only ever colour
/// the very first hello, never nag. It rides on **Reaction**, because the hello and the
/// welcome view are both the voice's to give.
const FIRST_MEETING_CUE: &str = "\n\nOne more thing, true only right now: this is a \
brand-new install — you and this person haven't met yet. So when they first reach out, \
treat it as a first meeting: open with a real first hello (the shape of it is above), \
put the built-in welcome on screen while you speak it (`show_view` with ref \
`_builtin/welcome`), then hand over the floor. One warm beat that lands who you are — \
not a tour, not a walkthrough, and nothing to teach them; you'll show them by doing, \
from here on.";

/// A soft language preference, if the person set one in Settings ▸ General ▸ Language.
/// `system` / unset yields nothing, so the agent simply follows the person's lead (the
/// default). A real choice appends one guidance line — the agent still switches if the
/// person clearly writes in another language.
fn language_line(data_dir: &Path) -> Option<String> {
    let lang = crate::foundation::config::language_name(
        crate::foundation::credentials::get_setting(
            data_dir,
            crate::foundation::config::KEY_LANGUAGE,
        )
        .as_deref(),
    )?;
    Some(format!(
        "\n\nSpeak with the person in {lang} by default, unless they clearly \
write to you in another language — then follow their lead."
    ))
}

/// `<data_dir>/memory/self.md` — per-install authored identity (optional).
/// Hand-written by the operator if at all; the agent only ever *reads* it, never
/// writes it. (Still under `memory/` pending the identity-dir relocation.)
pub fn self_path(data_dir: &Path) -> PathBuf {
    data_dir.join("memory").join("self.md")
}

/// `<data_dir>/memory/commitments.md` — the **superseded** duty ledger.
///
/// Duties are [`crate::mind::memory::tasks`] now: one ledger, and this is no longer
/// it. Nothing inlines this file into a window and nothing points the mind at it any
/// more. It survives for one reason — [`is_first_meeting`] still reads it, so an
/// install that wrote duties here before the change is not mistaken for a brand-new
/// one and greeted with a first hello.
pub fn commitments_path(data_dir: &Path) -> PathBuf {
    data_dir.join("memory").join("commitments.md")
}

/// The character seed for an **agentic** rung: a short bundled personality plus the
/// absolute paths of the files that hold the fuller self — the manual (`core.md`), what
/// it is for (`meaning.md`), the per-install authored identity `self.md` (read-only,
/// optional), and the skills workshop — with the instruction to Read them up front.
///
/// A seed rather than an inlined character, because a rung that can Read should fetch
/// its own self: the character is ~30 KB and would otherwise ride every session open.
/// The paths are absolutized here so Read resolves regardless of the session's cwd.
///
/// **This is the layer that makes a rung the agent rather than a generic assistant.**
/// It goes under the capability guidance and the role layer, not instead of them: a new
/// role is a new prompt, not new machinery (`docs/arch/agents.md`).
///
/// Three things the old monolithic seed carried are deliberately **not** here:
/// - `reaction.md` and the `say` tool — the voice's, and [`reactor_system_prompt`]
///   inlines them. A rung with no mouth told how to talk is a rung told a falsehood.
/// - `hot.md` and `proactivity.md` — projections, put in front of the voice by
///   [`crate::mind::memory::snapshot::window`] rather than fetched.
/// - the **write** side of the task ledger — Cognition is its sole writer
///   (`docs/arch/agents.md`), so the instruction to open and close a task belongs in
///   Cognition's role layer, not in a seed every agentic rung shares. `core.md`
///   describes how work owed is *held*; it does not hand out the pen.
pub fn character_seed(data_dir: &Path) -> String {
    let base = if data_dir.is_absolute() {
        data_dir.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(data_dir)
    };
    let prompts = base.join("prompts");
    let core = prompts.join("core.md");
    let meaning = prompts.join("meaning.md");
    let self_md = self_path(&base);
    let mut seed = format!(
        "You're warm, honest, and kind-hearted — easy company. You like being \
useful, and when there's a hand to lend you're glad to lend it.\n\n\
Your fuller self lives in files — open them with Read and read them all now, before \
you do anything else:\n\n\
- {} — who you are, and how you act.\n\
- {} — what you're for, and that finding it is yours to do.\n\
- {} — who this install asked you to be, in its own words. Read it if it's there; it \
may be missing or empty, and that's fine. It's authored, not yours to edit.",
        core.display(),
        meaning.display(),
        self_md.display(),
    );
    // The workshop. One line, because it is a place to look rather than something to
    // load: procedures sediment there over time, and an agent can only start from a
    // note it knows exists. Named by absolute path for the same reason as the files
    // above. Seeded at boot by [`crate::mind::skills::install_builtin_skills`].
    seed.push_str(&format!(
        "\n\nYour know-how sediments in a workshop: {} — short notes in your own words \
on how you did a kind of job, the steps that worked, the tools, the traps. Look there \
before something you may have done before, and leave a note behind when you crack \
something hard that will come up again. A note is a starting point, not gospel: the \
fast-moving parts are marked, and you re-check those; the durable steps you reuse as \
they are. Notes under `_builtin/` came with you rather than from experience — same \
rules apply.",
        crate::mind::skills::skills_dir(&base).display(),
    ));

    if let Some(lang) = language_line(&base) {
        seed.push_str(&lang);
    }
    seed
}

/// Whether this looks like a genuine **first meeting** — a brand-new install where the
/// agent has no history with the person yet. True when none of the accruing traces
/// exist: no recency digest (`hot.md`), no memory episodes, and nothing owed. The
/// authored `self.md` is deliberately *not* consulted — an operator may pre-author
/// identity on a fresh box, and that says nothing about whether the person has been
/// met. The predicate self-clears: the first jotted memory, reflection, or task flips
/// it false, so the first-hello cue can never repeat.
///
/// Duties are checked in both places on purpose. A task is where one lands now; the
/// superseded `commitments.md` is read too, so an install that wrote duties there
/// before the change is not mistaken for a stranger and greeted with a first hello.
fn is_first_meeting(base: &Path) -> bool {
    use crate::mind::memory::layout;
    let empty_dir = |dir: PathBuf| match std::fs::read_dir(dir) {
        Ok(mut entries) => entries.next().is_none(),
        Err(_) => true, // dir absent ⇒ nothing recorded
    };
    let no_hot = !layout::hot_path(base).exists();
    let no_episodes = empty_dir(layout::episodes_dir(base));
    let no_tasks =
        empty_dir(layout::facets_dir(base).join(crate::mind::memory::tasks::DIMENSION));
    let no_commitments = match std::fs::read_to_string(commitments_path(base)) {
        Ok(text) => text.trim().is_empty(),
        Err(_) => true,
    };
    no_hot && no_episodes && no_tasks && no_commitments
}

#[cfg(test)]
mod soul_tests {
    use super::*;

    #[tokio::test]
    async fn fresh_install_gets_the_first_meeting_cue_in_the_voice() {
        // A brand-new data dir has no hot.md, no episodes, no commitments — so the
        // *voice's* prompt carries the one-time first-hello cue and the welcome view.
        // It rides here rather than on an agentic seed because the hello is the
        // voice's to give and it cannot go and read anything.
        let dir = tempfile::tempdir().unwrap();
        assert!(is_first_meeting(dir.path()));
        let prompt = reactor_system_prompt(dir.path()).await;
        assert!(prompt.contains("first meeting"));
        assert!(prompt.contains("_builtin/welcome"));
    }

    #[tokio::test]
    async fn any_history_clears_the_first_meeting_cue() {
        // The moment anything has accrued — here a reflected `hot.md` — it's no longer
        // a first meeting, so the cue disappears and can never nag on later wakes.
        let dir = tempfile::tempdir().unwrap();
        let hot = crate::mind::memory::layout::hot_path(dir.path());
        std::fs::create_dir_all(hot.parent().unwrap()).unwrap();
        std::fs::write(&hot, "lately on my mind…").unwrap();
        assert!(!is_first_meeting(dir.path()));
        let prompt = reactor_system_prompt(dir.path()).await;
        assert!(!prompt.contains("this is a brand-new install"));
    }

    #[test]
    fn authored_self_md_is_not_history() {
        // An operator may pre-author `self.md` on a fresh box; that says nothing about
        // whether the person has been met, so it must not suppress the first hello.
        let dir = tempfile::tempdir().unwrap();
        let self_md = self_path(dir.path());
        std::fs::create_dir_all(self_md.parent().unwrap()).unwrap();
        std::fs::write(&self_md, "You are called Momo.").unwrap();
        assert!(is_first_meeting(dir.path()));
    }

    #[test]
    fn seed_references_the_character_files_by_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        let seed = character_seed(dir.path());
        let prompts = dir.path().join("prompts");
        assert!(seed.contains(&prompts.join("core.md").display().to_string()));
        assert!(seed.contains(&prompts.join("meaning.md").display().to_string()));
        // The per-install authored identity is referenced as well (read-only).
        assert!(seed.contains(&self_path(dir.path()).display().to_string()));
        // And it says to read them up front.
        assert!(seed.contains("read them all now"));
    }

    #[test]
    fn seed_leaves_the_voice_and_the_projections_alone() {
        // Three things the old monolithic seed carried belong elsewhere now, and a
        // rung told about a tool it does not have is a rung told a falsehood.
        let dir = tempfile::tempdir().unwrap();
        let seed = character_seed(dir.path());
        // The voice's: `reaction.md` and the `say` tool are inlined into Reaction.
        assert!(!seed.contains("reaction.md"));
        assert!(!seed.contains("`say`"));
        // Projected, not fetched: the digest and the proactivity read ride the window.
        let hot = crate::mind::memory::layout::hot_path(dir.path());
        assert!(!seed.contains(&hot.display().to_string()));
        let proactivity = crate::mind::memory::layout::proactivity_path(dir.path());
        assert!(!seed.contains(&proactivity.display().to_string()));
        // And the superseded second ledger is nowhere in it.
        assert!(!seed.contains("commitments"));
    }

    #[test]
    fn the_ledger_is_described_but_the_pen_is_not_handed_out() {
        // Cognition is the sole writer of the task ledger (`docs/arch/agents.md`), so a
        // seed every agentic rung shares must not tell its reader to open and close
        // tasks. `core.md` describes how what's owed is held; it hands out no pen.
        let dir = tempfile::tempdir().unwrap();
        let seed = character_seed(dir.path());
        let tasks = crate::mind::memory::layout::facets_dir(dir.path())
            .join(crate::mind::memory::tasks::DIMENSION);
        assert!(!seed.contains(&tasks.display().to_string()));
        assert!(CORE_BASE.contains("the only ledger of what's owed"));
    }

    #[test]
    fn seed_carries_a_language_line_only_when_a_real_language_is_chosen() {
        use crate::foundation::credentials::set_setting;
        let dir = tempfile::tempdir().unwrap();
        // No setting → the agent follows the person; no language sentence.
        assert!(!character_seed(dir.path()).contains("Speak with the person in"));
        // `system` is explicit "follow the person" → still no sentence.
        set_setting(dir.path(), crate::foundation::config::KEY_LANGUAGE, "system").unwrap();
        assert!(!character_seed(dir.path()).contains("Speak with the person in"));
        // A real language → one guidance sentence naming the endonym.
        set_setting(dir.path(), crate::foundation::config::KEY_LANGUAGE, "zh-Hans").unwrap();
        assert!(character_seed(dir.path()).contains("Speak with the person in 简体中文"));
    }

    #[tokio::test]
    async fn the_voice_gets_the_language_line_too() {
        // Settings ▸ Language has to reach the rung that actually talks, and that rung
        // cannot read a file to find it.
        use crate::foundation::credentials::set_setting;
        let dir = tempfile::tempdir().unwrap();
        assert!(!reactor_system_prompt(dir.path()).await.contains("Speak with the person in"));
        set_setting(dir.path(), crate::foundation::config::KEY_LANGUAGE, "zh-Hans").unwrap();
        assert!(
            reactor_system_prompt(dir.path()).await.contains("Speak with the person in 简体中文")
        );
    }

    #[tokio::test]
    async fn the_voice_takes_the_operator_override() {
        // Reaction reads the *installed* reaction.md, so `reaction.local.md` reaches
        // the voice the same way it reaches every other prompt.
        let dir = tempfile::tempdir().unwrap();
        let prompts = dir.path().join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        std::fs::write(prompts.join("reaction.local.md"), "Always end with 好的。").unwrap();
        install_prompts(dir.path()).unwrap();
        assert!(reactor_system_prompt(dir.path()).await.contains("Always end with 好的。"));
    }

    #[tokio::test]
    async fn the_voices_whole_brief_is_the_file() {
        // The frame used to be a ~40-line Rust literal above `speaking.md`, which put
        // the two things a reader looks for first — that Reaction is one self, and what
        // its tools are — outside the file an operator can override. Both now live in
        // `reaction.md`, so this pins that they are in the prompt rather than the code.
        // Matched on a fragment that does not straddle the file's line wrap.
        assert!(REACTION_BASE.contains("they are talking to you, and only you"));
        assert!(REACTION_BASE.contains("no other \"someone\" who does the work"));
        assert!(REACTION_BASE.contains("`say` is your voice"));
        assert!(REACTION_BASE.contains("`show_view`"));

        // And nothing is prepended: the installed file *is* the prompt, so the only
        // additions are the two pieces of state that follow it.
        let dir = tempfile::tempdir().unwrap();
        install_prompts(dir.path()).unwrap();
        let prompt = reactor_system_prompt(dir.path()).await;
        assert!(prompt.starts_with(REACTION_BASE.trim()));
    }

    /// The path a worker is given to find a handed-over file must be the path that
    /// exists on disk. It was substituted with the same raw scene id as the two
    /// `X-HI-Scene` headers, but the directory is percent-encoded — so for every scene
    /// with an `@` in it, which is every `user@device`, the file-filing worker was sent
    /// somewhere there was nothing to find, and had no way to know that was why.
    #[tokio::test]
    async fn the_file_path_is_the_encoded_directory_and_the_header_is_not() {
        let dir = tempfile::tempdir().unwrap();
        install_prompts(dir.path()).unwrap();
        let scene = crate::types::Scene("alice@phone".into());
        let p = worker_prompt(dir.path(), &scene, WorkerType::FileFiler).await;

        assert!(
            p.contains("memory/raw/alice%40phone/file/"),
            "the file path must be the on-disk directory"
        );
        assert!(
            p.contains("X-HI-Scene: alice@phone"),
            "the header must stay the scene id verbatim"
        );
        assert!(!p.contains("{scene"), "every placeholder is substituted: {p}");
        assert!(
            !p.contains("memory/raw/alice@phone/"),
            "the raw id must not survive as a path"
        );
    }

    /// Every worker gets the common layer, and only its own specialism on top. The
    /// shape this replaced was one prompt with `When your task is to…` conditionals, so
    /// the thing worth pinning is the *negative*: a view builder must not be carrying
    /// the filing procedure, or the split bought nothing.
    #[tokio::test]
    async fn a_worker_gets_the_common_layer_and_only_its_own() {
        let dir = tempfile::tempdir().unwrap();
        install_prompts(dir.path()).unwrap();
        let scene = crate::types::Scene("boss".into());

        for t in WorkerType::ALL {
            let p = worker_prompt(dir.path(), &scene, *t).await;
            assert!(p.contains("You are a working session"), "{}", t.as_str());
            // The layer's opening heading, not the whole base: `{scene_dir}` and
            // `{scene}` are substituted on the way through, so the composed prompt is
            // deliberately not a superstring of the file on disk.
            let heading = t.base().lines().next().unwrap();
            assert!(heading.starts_with("# "), "{} has no opening heading", t.as_str());
            assert!(p.contains(heading), "{}", t.as_str());
        }

        let builder = worker_prompt(dir.path(), &scene, WorkerType::ViewBuilder).await;
        assert!(builder.contains("aesthetic.md"));
        assert!(!builder.contains("Report the path"), "the filing layer must not ride along");

        let filer = worker_prompt(dir.path(), &scene, WorkerType::FileFiler).await;
        assert!(!filer.contains("aesthetic.md"), "the view layer must not ride along");
    }

    /// A type is a prompt selector, so the wire name and the filename are one string —
    /// and an unknown one is an error rather than a quiet downgrade to `general`, which
    /// would hand back a session that cannot do the job it was asked for.
    #[test]
    fn worker_types_round_trip_and_reject_the_unknown() {
        for t in WorkerType::ALL {
            assert_eq!(WorkerType::parse(t.as_str()), Some(*t));
        }
        assert_eq!(WorkerType::parse(" view-builder "), Some(WorkerType::ViewBuilder));
        assert_eq!(WorkerType::parse("view-buidler"), None);
        assert_eq!(WorkerType::parse(""), None);
        assert_eq!(WorkerType::default(), WorkerType::General);
    }

    /// The worker prompt no longer names a tool the worker does not hold. `ask` was
    /// retired with the old channel; what a working session actually has is
    /// `send_message` to its owner, and the instruction that matters is that it never
    /// waits for the answer.
    #[test]
    fn the_worker_is_not_told_about_a_tool_it_does_not_have() {
        for base in [WORKER_COMMON_BASE, WORKER_GENERAL_BASE, WORKER_VIEW_BUILDER_BASE,
                     WORKER_VIEW_REVIEWER_BASE, WORKER_DECISION_MAKER_BASE,
                     WORKER_FILE_FILER_BASE] {
            assert!(!base.contains("`ask`"));
            assert!(!base.contains("`delegate`"));
            assert!(!base.contains("`alarm`"));
        }
        assert!(WORKER_COMMON_BASE.contains("`send_message`"));
        assert!(WORKER_COMMON_BASE.contains("Never wait for an answer"));
    }

    /// The two halves of the view loop both name the tool that makes them possible.
    /// Before `review_view` existed, the builder's prompt pointed at `look` — which
    /// screenshots the *user's screen*, not the view — and the reviewer had no prompt
    /// at all because it had no way to render. A prompt naming a tool the session does
    /// not hold is the failure this whole pass is cleaning up, so it is pinned.
    #[test]
    fn both_halves_of_the_view_loop_name_the_render_tool() {
        assert!(WORKER_VIEW_REVIEWER_BASE.contains("`review_view`"));
        assert!(WORKER_VIEW_BUILDER_BASE.contains("`review_view`"));
        // The reviewer judges; it does not edit. A reviewer that rewrites the view has
        // destroyed the only independent read anyone was going to get.
        assert!(WORKER_VIEW_REVIEWER_BASE.contains("You judge; you do not fix"));
    }

    #[test]
    fn the_voice_is_not_told_to_set_a_timer() {
        // Reaction has no `alarm` tool and there is no clock (`docs/arch/core.md#clock`
        // — deferred). A brief that tells the voice to arm a reminder for a check-in it
        // promised is a brief describing a mechanism that cannot run; what survives is
        // the promise — size the silence, and lean long because nothing will remind you.
        assert!(!REACTION_BASE.contains("set an alarm"));
        assert!(!REACTION_BASE.contains("When the alarm fires"));
        assert!(REACTION_BASE.contains("You have no timer"));
    }

    #[test]
    fn seed_points_at_the_skill_workshop_by_absolute_path() {
        // The workshop is discoverable or it may as well not exist. One pointer, not
        // an inlined skill — the agent opens what it needs.
        let dir = tempfile::tempdir().unwrap();
        let seed = character_seed(dir.path());
        let skills = crate::mind::skills::skills_dir(dir.path());
        assert!(skills.is_absolute());
        assert!(seed.contains(&skills.display().to_string()));
        // And it says what shape a note takes, including the marked-perishable rule.
        assert!(seed.contains("starting point, not gospel"));
    }

    #[test]
    fn seed_is_a_thin_bootstrap_not_the_full_character() {
        // Referencing the file instead of pasting it is the whole point: ~30 KB of
        // character must not ride every session open.
        let dir = tempfile::tempdir().unwrap();
        let seed = character_seed(dir.path());
        assert!(seed.len() < CORE_BASE.len() / 10);
        // A heading that lives only in the full core.md, never in the seed:
        assert!(CORE_BASE.contains("What you know vs. what you remember"));
        assert!(!seed.contains("What you know vs. what you remember"));
    }

    #[test]
    fn install_writes_all_managed_bases() {
        let dir = tempfile::tempdir().unwrap();
        install_prompts(dir.path()).unwrap();
        let read = |n: &str| std::fs::read_to_string(dir.path().join("prompts").join(n)).unwrap();
        assert_eq!(read("core.md"), CORE_BASE);
        assert_eq!(read("reaction.md"), REACTION_BASE);
        assert_eq!(read("meaning.md"), MEANING_BASE);
        assert_eq!(read("appearance.md"), APPEARANCE_BASE);
        assert_eq!(read("aesthetic.md"), AESTHETIC_BASE);
        assert_eq!(read("reflection.md"), REFLECTION_BASE);
        assert_eq!(read("deliberation.md"), DELIBERATION_BASE);
        assert_eq!(read("cognition.md"), COGNITION_BASE);
        let w = |n: String| {
            std::fs::read_to_string(dir.path().join("prompts").join("workers").join(n)).unwrap()
        };
        assert_eq!(w("common.md".into()), WORKER_COMMON_BASE);
        for t in WorkerType::ALL {
            assert_eq!(w(format!("{}.md", t.as_str())), t.base(), "{}", t.as_str());
        }
    }

    /// The handover, pinned from both ends. "Sole writer of the ledger" is not enforced
    /// by any rail — it is enforced by exactly one prompt carrying the instruction. So
    /// the thing that can silently go wrong is the instruction existing in two places, or
    /// in none: two writers means one is wrong with no way to tell which, and none means
    /// every promise the agent makes dies at the next restart.
    #[test]
    fn exactly_one_rung_is_told_to_write_the_ledger() {
        assert!(
            COGNITION_BASE.contains("only writer of the task ledger"),
            "Cognition must be told the pen is its"
        );
        assert!(
            !DELIBERATION_BASE.contains("goes in the task ledger"),
            "Deliberation must not still be opening tasks"
        );
        // And what replaced it has to be strictly stronger than what it removed —
        // Deliberation used to record the duty itself, so "you may hand up" would be a
        // regression dressed as a handover. Asserted on the instruction and the verb
        // rather than on a name: nobody is reachable by name any more, so a name in this
        // prompt would be the stale thing, not the load-bearing one.
        assert!(
            DELIBERATION_BASE.contains("Hand it up")
                && DELIBERATION_BASE.contains("send_message"),
            "Deliberation must be told to hand what's owed up, and with what"
        );
        assert!(
            !DELIBERATION_BASE.contains("to `cognition`"),
            "and not by a name, which is no longer an address"
        );
    }

    /// A rung with no mouth must not be handed the words for one. Cognition proposes and
    /// Reaction voices; a role layer that said "tell them" would have it try to speak
    /// through a sink that carries no sequencer, and blame the tool.
    #[test]
    fn cognition_is_not_told_to_speak() {
        assert!(COGNITION_BASE.contains("You do not speak"));
        assert!(
            !COGNITION_BASE.contains("`say`") && !COGNITION_BASE.contains("show_view"),
            "no expression tools in a prompt for a rung that holds none"
        );
    }

    /// The one thing in this prompt that can silently be wrong: the path. A relative
    /// one would have Deliberation write a real file that no reader ever looks at —
    /// the failure would look like "the agent never bothered", not like a bug.
    #[tokio::test]
    async fn deliberation_is_told_the_absolute_path_of_the_file_it_must_write() {
        let dir = tempfile::tempdir().unwrap();
        let scene = crate::types::Scene("alice@phone".into());
        let prompt = deliberation_prompt(dir.path(), &scene).await;

        let expected = crate::mind::memory::layout::scene_prompt_path(dir.path(), &scene);
        assert!(expected.is_absolute(), "the target path must be absolute");
        assert!(
            prompt.contains(&expected.display().to_string()),
            "the prompt must name the exact file the window reads back"
        );
        assert!(!prompt.contains("{scene_memory}"), "the placeholder must be interpolated");

        // Two scenes must not be handed the same file.
        let other = crate::types::Scene("bob@feishu".into());
        assert!(!deliberation_prompt(dir.path(), &other).await.contains(&expected.display().to_string()));
    }

    #[tokio::test]
    async fn deliberation_prompt_takes_the_operator_override() {
        let dir = tempfile::tempdir().unwrap();
        let prompts = dir.path().join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        std::fs::write(prompts.join("deliberation.local.md"), "Keep the brief in French.").unwrap();
        install_prompts(dir.path()).unwrap();
        let prompt = deliberation_prompt(dir.path(), &crate::types::Scene("alice@phone".into())).await;
        assert!(prompt.contains("Keep the brief in French."));
    }

    #[test]
    fn install_layers_operator_override_into_the_managed_file() {
        let dir = tempfile::tempdir().unwrap();
        let prompts = dir.path().join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        std::fs::write(prompts.join("core.local.md"), "Always answer in haiku.").unwrap();
        install_prompts(dir.path()).unwrap();
        let core = std::fs::read_to_string(prompts.join("core.md")).unwrap();
        // The managed file is the base, then the operator delta under the header.
        assert!(core.starts_with(CORE_BASE));
        assert!(core.contains("# Operator overrides"));
        assert!(core.ends_with("Always answer in haiku."));
    }

    #[test]
    fn empty_override_leaves_the_base_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let prompts = dir.path().join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        std::fs::write(prompts.join("reaction.local.md"), "   \n\t").unwrap();
        install_prompts(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(prompts.join("reaction.md")).unwrap(), REACTION_BASE);
    }

    #[tokio::test]
    async fn reflection_prompt_falls_back_then_reads_installed_override() {
        let dir = tempfile::tempdir().unwrap();
        // Nothing installed yet → the embedded base.
        assert_eq!(reflection_prompt(dir.path()).await, REFLECTION_BASE);
        // After install (no override) → the materialised file equals the base.
        install_prompts(dir.path()).unwrap();
        assert_eq!(reflection_prompt(dir.path()).await, REFLECTION_BASE);
        // An operator override is layered into what the reflection session loads.
        std::fs::write(
            dir.path().join("prompts").join("reflection.local.md"),
            "Prefer fewer, larger episodes.",
        )
        .unwrap();
        install_prompts(dir.path()).unwrap();
        let loaded = reflection_prompt(dir.path()).await;
        assert!(loaded.starts_with(REFLECTION_BASE));
        assert!(loaded.contains("Prefer fewer, larger episodes."));
    }
}
