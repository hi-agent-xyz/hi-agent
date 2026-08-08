//! identity — who the agent is.
//!
//! The factory-authored character, as **one whole prompt per role**: four rungs
//! (`reaction`, `deliberation`, `cognition`, `reflection`) and five worker types under
//! `workers/`. Standing duties are not here at all — they are tasks
//! ([`crate::mind::memory::tasks`]), one ledger, projected into every window rather
//! than pointed at by a path.
//!
//! This module owns the **prompt cascade**: the bundled bases are materialised under
//! `<data_dir>/prompts/` at boot, each composed with an optional operator `*.local.md`
//! override ([`install_prompts`]), then read back whole at session open
//! ([`rung_prompt`], [`worker_prompt`], [`reaction_system_prompt`]). That cascade is the
//! base‹override mechanism `docs/arch/foundation.md` generalises to base‹user‹self.
//!
//! **There is no seed.** A rung used to be handed ~18 lines pointing at `core.md`,
//! `meaning.md` and `self.md` and told to go read them; that shape and why it was
//! retired are recorded on [`rung_prompt`]. `core.md`, `meaning.md`, `appearance.md`
//! and `aesthetic.md` no longer exist, and [`install_prompts`]'s tests assert they stay
//! gone.
//!
//! `self.md` still lives under `<data_dir>/memory/` (no data migration). Under
//! `docs/arch/data.md#memoryprompts` it does not get relocated at all: who this install
//! is becomes a *section* of a generated prompt, so the file goes away with the change
//! that gives Deliberation the writer's job.

use std::path::{Path, PathBuf};

/// Built-in base prompts, embedded at compile time and materialised to disk by
/// [`install_prompts`].
///
/// **One file per rung, and each is that rung's whole system prompt** — nothing points
/// at anything else and nothing is fetched. They divide only by which entry point reads
/// them back: [`reaction_system_prompt`] for the tools-off voice, [`reflection_prompt`]
/// for the consolidation pass, [`cognition_prompt`] and [`deliberation_prompt`] for the
/// thinking rungs. All ship in the binary and refresh on every build.
///
/// The cost of "whole" is that ~2,000 words of shared character live in three copies,
/// and drift between them is the live risk — which is what the prompt tests below are
/// for.
const REACTION_BASE: &str = include_str!("reaction.md");
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
/// each with its optional `*.local.md` operator override. The managed base files —
/// the four rungs (`reaction.md`, `deliberation.md`, `cognition.md`, `reflection.md`)
/// and `workers/<type>.md` — are rewritten every boot so they stay current; operator
/// edits live in the never-touched `*.local.md` siblings. Each follows one workflow:
/// ship embedded → materialise here → consumed from disk at runtime.
pub fn install_prompts(data_dir: &Path) -> std::io::Result<()> {
    let dir = data_dir.join("prompts");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("reaction.md"), compose_prompt(REACTION_BASE, &dir, "reaction.local.md"))?;
    std::fs::write(dir.join("reflection.md"), compose_prompt(REFLECTION_BASE, &dir, "reflection.local.md"))?;
    std::fs::write(dir.join("deliberation.md"), compose_prompt(DELIBERATION_BASE, &dir, "deliberation.local.md"))?;
    std::fs::write(dir.join("cognition.md"), compose_prompt(COGNITION_BASE, &dir, "cognition.local.md"))?;

    // The worker prompts get their own subdirectory, because there is one per type and
    // they would otherwise be the majority of a flat `prompts/`.
    //
    // One file per type, and **no shared base**: a worker's prompt is whole, the same way
    // a rung's is. `common.md` used to sit above them as the layer every type composed
    // with, which meant a decision-maker read how to drive a camera and a file-filer read
    // how to review its own artwork. The price is duplication: a 35-line preamble
    // identical in all five, plus ~36 further lines shared by two or three of them. Drift
    // between the copies is the risk, and the prompt tests below are what hold it.
    //
    // It was roughly five times that until the reviewer stopped carrying the builder's
    // taste section verbatim — the same 74 lines, in the builder's voice, telling a rung
    // whose whole job is *not* to edit the view to "fix what doesn't before you save".
    let workers = dir.join("workers");
    std::fs::create_dir_all(&workers)?;
    for t in WorkerType::ALL {
        std::fs::write(
            workers.join(format!("{}.md", t.as_str())),
            compose_prompt(t.base(), &workers, &format!("{}.local.md", t.as_str())),
        )?;
    }

    tracing::info!(
        dir = %dir.display(),
        types = WorkerType::ALL.len(),
        "installed bundled prompts (reaction.md, deliberation.md, cognition.md, reflection.md, workers/)"
    );
    Ok(())
}

/// A working session's whole system prompt: `workers/<type>.md`, entire.
///
/// Read off disk so an operator's `*.local.md` reaches a worker the same way it reaches
/// every other rung, falling back to the embedded bases when a file is missing or
/// empty. Read fresh per spawn, so an edit takes effect without a restart.
///
/// **This replaced a `const &str` in `reaction/workers.rs`** — the one role prompt that
/// was not a bundled `.md`, and so the one nobody could retune without a rebuild.
///
/// Only the directory placeholders [`rung_prompt`] already expands are interpolated.
/// There were two more — `{conversation}` for the `X-HI-Conversation` header a worker had to name,
/// and `{scene_dir}` for the same id percent-encoded on disk — and both are gone with
/// the key they named. The `{scene_dir}` one was a real bug's home: the raw form was
/// substituted into a path whose directory was encoded, so for the conversation with an `@`
/// in it the filing worker was pointed somewhere empty. A path with no user string in
/// it cannot have that bug.
pub async fn worker_prompt(data_dir: &Path, kind: WorkerType) -> String {
    rung_prompt(data_dir, &format!("workers/{}", kind.as_str()), kind.base()).await
}


/// Every bundled rung prompt, as `(installed filename stem, embedded text)`.
///
/// Exists so tests can sweep the whole corpus rather than naming files one at a time —
/// the failure this guards is a *new* prompt quietly not being held to the rules the
/// others are. Adding a rung means adding a line here, and the sweeps pick it up.
#[cfg(test)]
pub(crate) fn bundled_rung_prompts() -> Vec<(&'static str, &'static str)> {
    vec![
        ("reaction", REACTION_BASE),
        ("deliberation", DELIBERATION_BASE),
        ("cognition", COGNITION_BASE),
        ("reflection", REFLECTION_BASE),
    ]
}

/// Absolutize `data_dir`: every path a prompt hands an agent must be absolute, because a
/// relative one resolves against the *session's* cwd, and those differ by rung on purpose.
fn abs(data_dir: &Path) -> PathBuf {
    if data_dir.is_absolute() {
        data_dir.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(data_dir)
    }
}

/// One rung's **whole** system prompt: the installed `<name>.md`, interpolated.
///
/// **This replaced the seed**, and the seed's shape is worth recording because it looked
/// thrifty and was not. A rung got ~18 lines pointing at `core.md`, `meaning.md` and
/// `self.md` — "read them all now, before you do anything else" — and fetched its own
/// character. Three costs, one of them silent:
///
/// - **Conditional.** Nothing verified the rung obeyed. Whether an agentic rung actually
///   Read its character was an open live-test item for weeks; now it cannot not have.
/// - **Paid per wake.** Cognition and Reflection open a fresh session every wake, so the
///   round trip was not once — it was every time, forever.
/// - **One `core.md` served three rungs with three tool surfaces**, so each was handed
///   large sections about jobs it could not do: Cognition read 56 lines on photos and
///   file-filing, and all three read how to drive a screen none of them has `look` for.
///
/// Each file is now self-contained and carries only what its rung can act on. The cost is
/// real: ~71 lines of shared character live in three copies, and drift between them is
/// the live risk — which is what the prompt tests are for.
///
/// What is per-install cannot be baked in, so it interpolates instead of being fetched:
/// the five directory placeholders below, and the language line.
///
/// **Every one of them expands to an absolute path**, which is the whole point of
/// [`abs`]: a prompt that says `memory/raw/sessions/` or `$PROMPTS/../drive/` resolves
/// against the *session's* cwd, and those differ by rung. The agent then reads a
/// directory that does not exist and reports the thing missing rather than empty.
async fn rung_prompt(data_dir: &Path, name: &str, fallback: &'static str) -> String {
    let base = abs(data_dir);
    let path = base.join("prompts").join(format!("{name}.md"));
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(s) if !s.trim().is_empty() => s,
        _ => fallback.to_string(),
    };
    let dir = |p: PathBuf| p.display().to_string();
    let mut out = text
        .replace("{skills_dir}", &dir(crate::mind::skills::skills_dir(&base)))
        .replace(
            "{sessions_dir}",
            &dir(crate::mind::memory::layout::raw_root(&base).join("sessions")),
        )
        .replace("{drive_dir}", &dir(base.join("drive")))
        .replace("{views_dir}", &dir(base.join("views")))
        .replace("{data_dir}", &dir(base.clone()));
    if let Some(lang) = language_line(&base) {
        out.push_str(&lang);
    }
    out
}

/// **Deliberation**'s role layer — what it is beyond being a working session, and the
/// job that makes it load-bearing: writing its conversation's
/// [generated prompt](../../docs/arch/data.md#memoryprompts), the brief Reaction reads
/// every turn and cannot write for itself.
///
/// This is a *layer*, not a whole prompt. Deliberation genuinely is a working session —
/// same tools, same capability guidance — so the caller appends this to the worker base
/// rather than replacing it. Read fresh each spawn (operator-overridable via
/// `deliberation.local.md`), so an edit takes effect without a restart.
///
/// `{conversation_memory}` is interpolated to the **absolute** path of the file it must
/// write, because an agent-facing path that is relative is a path to the wrong file.
pub async fn deliberation_prompt(data_dir: &Path) -> String {
    let text = rung_prompt(data_dir, "deliberation", DELIBERATION_BASE).await;
    let target = crate::mind::memory::layout::conversation_prompt_path(&abs(data_dir));
    text.replace("{conversation_memory}", &target.display().to_string())
}

/// **Cognition**'s role layer — the brain that owns the task ledger, hands
/// work out, and never speaks (`docs/arch/agents.md#cognition`).
///
/// Cognition's **whole** prompt, like every other rung's: the character and the role
/// arrive together, which is what makes the rung *the agent* rather than a generic
/// assistant. It carries the ledger pen — the instruction that lived in
/// `deliberation.md` marked "for now, yours" until Cognition existed to take it back.
///
/// Nothing is interpolated here, unlike Deliberation's `{scene_memory}`. What Cognition
/// carries forward arrives in the **window** with the projected ledger
/// ([`crate::mind::memory::snapshot::agent_window`]), not in this layer — the same
/// content in both places would be one copy going stale against the other, and the window
/// is the half that is rebuilt every turn.
pub async fn cognition_prompt(data_dir: &Path) -> String {
    rung_prompt(data_dir, "cognition", COGNITION_BASE).await
}

/// The reflection ("sleep") session's system prompt: the materialised
/// `<data_dir>/prompts/reflection.md` (operator-overridable via `reflection.local.md`),
/// or the embedded [`REFLECTION_BASE`] when that file is missing or empty. It is
/// **inlined** as the reflection session's system prompt rather than Read by the agent
/// — it *is* the task's instructions, so it must be present before the session can act.
/// Read fresh each round, so an operator edit takes effect without a restart.
pub async fn reflection_prompt(data_dir: &Path) -> String {
    rung_prompt(data_dir, "reflection", REFLECTION_BASE).await
}

/// **Reaction**'s system prompt — the conversation's voice (`docs/arch/agents.md#reaction`).
///
/// Reaction is tools-off by design: it has no Read, so nothing it needs may be a path.
/// Its brief is therefore **inlined and singular** — `reaction.md` *is* its whole system
/// prompt, verbatim. That is what makes speaking-rule conformance structural: the rules
/// are the entire context, not one buried file among many. (Mirrors how `reflection.md`
/// is inlined for the reflection session.)
///
/// **The frame that used to sit above the file is now the top of the file.** It was
/// ~40 lines of Rust string literal carrying the two things a reader would look for
/// first — that Reaction is one self rather than a dispatcher with colleagues, and what
/// its tools are — which meant the voice's brief lived in two places, only one of them
/// operator-overridable. A prompt is prose; it belongs in the `.md`. The file is now
/// named for the rung that reads it (`docs/arch/arch.md#character`: a file per role)
/// rather than for the activity, which is what `speaking.md` was.
///
/// Its surface is `say` · `show` · `send_message`
/// (`docs/arch/foundation.md#default-tool-surfaces`), and `reaction.md` must name all
/// three: the file once said "you have exactly two", then told the voice to "hand it
/// onward" without naming the verb that does it.
///
/// Read from `<data_dir>/prompts/reaction.md`, so an operator's `reaction.local.md`
/// reaches the voice too, falling back to the embedded [`REACTION_BASE`]. Two things
/// stay in code because they are *state*, not character, and the voice cannot fetch
/// either: the **first-meeting** cue and the **language** preference.
pub async fn reaction_system_prompt(data_dir: &Path) -> String {
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
put the built-in welcome on screen while you speak it (`show` with ref \
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
        let prompt = reaction_system_prompt(dir.path()).await;
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
        let prompt = reaction_system_prompt(dir.path()).await;
        assert!(!prompt.contains("this is a brand-new install"));
    }






    #[tokio::test]
    async fn the_voice_gets_the_language_line_too() {
        // Settings ▸ Language has to reach the rung that actually talks, and that rung
        // cannot read a file to find it.
        use crate::foundation::credentials::set_setting;
        let dir = tempfile::tempdir().unwrap();
        assert!(!reaction_system_prompt(dir.path()).await.contains("Speak with the person in"));
        set_setting(dir.path(), crate::foundation::config::KEY_LANGUAGE, "zh-Hans").unwrap();
        assert!(
            reaction_system_prompt(dir.path()).await.contains("Speak with the person in 简体中文")
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
        assert!(reaction_system_prompt(dir.path()).await.contains("Always end with 好的。"));
    }


    /// The managed bases all land, and `core.md`/`meaning.md` are **gone** — a rung's
    /// prompt is one file now, so a leftover shared file would be a second source of
    /// character with nothing reading it.
    #[test]
    fn install_writes_every_managed_base_and_no_retired_one() {
        let dir = tempfile::tempdir().unwrap();
        install_prompts(dir.path()).unwrap();
        let p = dir.path().join("prompts");
        let read = |n: &str| std::fs::read_to_string(p.join(n)).unwrap();
        assert_eq!(read("reaction.md"), REACTION_BASE);
        assert_eq!(read("deliberation.md"), DELIBERATION_BASE);
        assert_eq!(read("cognition.md"), COGNITION_BASE);
        assert_eq!(read("reflection.md"), REFLECTION_BASE);
        for gone in ["core.md", "meaning.md", "appearance.md", "aesthetic.md"] {
            assert!(!p.join(gone).exists(), "{gone} should be retired");
        }
    }

    #[test]
    fn install_layers_the_operator_override_into_the_managed_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("prompts");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("cognition.local.md"), "Prefer small workers.").unwrap();
        install_prompts(dir.path()).unwrap();
        let out = std::fs::read_to_string(p.join("cognition.md")).unwrap();
        assert!(out.starts_with(COGNITION_BASE));
        assert!(out.contains("Prefer small workers."));
    }

    /// Every thinking rung is self-contained: it opens with its own character rather than
    /// a pointer to a file it must go and fetch. This is the property the seed did not
    /// have — the seed's instruction to Read was *conditional*, and nothing checked it.
    #[tokio::test]
    async fn every_rung_carries_its_own_character() {
        let dir = tempfile::tempdir().unwrap();
        install_prompts(dir.path()).unwrap();
        for (name, base) in bundled_rung_prompts() {
            assert!(
                !base.contains("read them all now"),
                "{name} still bootstraps instead of carrying its character"
            );
            assert!(base.len() > 2_000, "{name} looks too thin to be self-contained");
        }
        // And the interpolations resolve rather than reaching the model raw.
        for text in [
            deliberation_prompt(dir.path()).await,
            cognition_prompt(dir.path()).await,
            reflection_prompt(dir.path()).await,
        ] {
            assert!(!text.contains("{skills_dir}"), "an unresolved placeholder reached the rung");
            assert!(!text.contains("{conversation_memory}"));
            let skills = crate::mind::skills::skills_dir(dir.path());
            assert!(skills.is_absolute());
            assert!(text.contains(&skills.display().to_string()));
        }
    }

    /// One pen on the ledger. Cognition writes it; nobody else is told how.
    #[test]
    fn exactly_one_prompt_hands_out_the_ledger_pen() {
        let carriers: Vec<&str> = bundled_rung_prompts()
            .into_iter()
            .filter(|(_, b)| b.contains("only writer of the task ledger"))
            .map(|(n, _)| n)
            .collect();
        assert_eq!(carriers, vec!["cognition"], "the pen must be held once");
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
        assert!(REACTION_BASE.contains("`show`"));

        // And nothing is prepended: the installed file *is* the prompt, so the only
        // additions are the two pieces of state that follow it.
        let dir = tempfile::tempdir().unwrap();
        install_prompts(dir.path()).unwrap();
        let prompt = reaction_system_prompt(dir.path()).await;
        assert!(prompt.starts_with(REACTION_BASE.trim()));
    }

    /// Nothing in a worker prompt is interpolated from a user string any more. The
    /// two placeholders that were — `{scene}` for a header, `{scene_dir}` for the same
    /// id percent-encoded on disk — are gone with the key they named, and with them the
    /// bug where the raw form was substituted into an encoded path.
    #[tokio::test]
    async fn no_user_string_is_interpolated_into_a_worker_prompt() {
        let dir = tempfile::tempdir().unwrap();
        install_prompts(dir.path()).unwrap();

        let filer = worker_prompt(dir.path(), WorkerType::FileFiler).await;
        assert!(
            filer.contains("memory/raw/file/"),
            "the filer is pointed at the flat channel directory"
        );

        for t in WorkerType::ALL {
            let p = worker_prompt(dir.path(), *t).await;
            assert!(!p.contains("{conversation"), "unsubstituted placeholder in {}", t.as_str());
        }
    }

    /// Every worker gets the common layer, and only its own specialism on top. The
    /// shape this replaced was one prompt with `When your task is to…` conditionals, so
    /// the thing worth pinning is the *negative*: a view builder must not be carrying
    /// the filing procedure, or the split bought nothing.
    #[tokio::test]
    async fn a_worker_gets_the_common_layer_and_only_its_own() {
        let dir = tempfile::tempdir().unwrap();
        install_prompts(dir.path()).unwrap();

        for t in WorkerType::ALL {
            let p = worker_prompt(dir.path(), *t).await;
            assert!(p.contains("You are a working session"), "{}", t.as_str());
            // The layer's opening heading, not the whole base: the directory
            // placeholders are substituted on the way through, so the composed prompt is
            // deliberately not a superstring of the file on disk.
            let heading = t.base().lines().next().unwrap();
            assert!(heading.starts_with("# "), "{} has no opening heading", t.as_str());
            assert!(p.contains(heading), "{}", t.as_str());
        }

        // The sentinel is a phrase from the view layer itself. It used to be
        // `"aesthetic.md"` — a *filename*, which kept passing after that file was
        // retired and the builder was left ordered to Read something that no longer
        // existed. Pin prose the layer actually carries, so deleting the layer fails
        // the test and deleting a file it merely mentions does not.
        const VIEW_LAYER: &str = "review_view";
        let builder = worker_prompt(dir.path(), WorkerType::ViewBuilder).await;
        assert!(builder.contains(VIEW_LAYER));
        assert!(!builder.contains("Report the path"), "the filing layer must not ride along");

        let filer = worker_prompt(dir.path(), WorkerType::FileFiler).await;
        assert!(!filer.contains(VIEW_LAYER), "the view layer must not ride along");
    }

    /// Durability across a restart is **behaviour, not machinery**: nothing persists a
    /// worker, so a kill mid-job loses whatever lived only in its context. The answer is
    /// the prompt telling it to write as it goes — and telling it *where*, because a note
    /// in `/tmp` is written and unfindable, which is the same as lost (gaps.md #3, #11).
    ///
    /// Pinned on the two rungs that can lose real time (the general worker and the view
    /// builder) plus the decision-maker, whose whole output *is* its report. The reviewer
    /// and the filer are left out on purpose — one returns a verdict, the other's work is
    /// the files it has already written.
    #[tokio::test]
    async fn the_long_running_workers_are_told_to_write_as_they_go() {
        let dir = tempfile::tempdir().unwrap();
        install_prompts(dir.path()).unwrap();

        for t in [WorkerType::General, WorkerType::ViewBuilder, WorkerType::DecisionMaker] {
            let p = worker_prompt(dir.path(), t).await;
            assert!(p.contains("only copy of the work"), "{} loses its work", t.as_str());
            // The location is the load-bearing half, and it has to arrive absolute: a
            // relative `memory/facets/...` resolves against a cwd that differs per rung.
            let tasks = crate::mind::memory::layout::facets_dir(&abs(dir.path())).join("tasks");
            assert!(
                p.contains(&tasks.display().to_string()),
                "{} is not told where to write",
                t.as_str()
            );
            assert!(!p.contains("{data_dir}"), "unsubstituted placeholder in {}", t.as_str());
        }

        // Redoing lost work is fine; redoing something a person already saw is not. Only
        // the general worker acts on the outside world, so only it carries the ordering
        // rule — the others build, judge, or file.
        let general = worker_prompt(dir.path(), WorkerType::General).await;
        assert!(general.contains("outside world can already see"));
        for t in [WorkerType::ViewReviewer, WorkerType::FileFiler] {
            let p = worker_prompt(dir.path(), t).await;
            assert!(!p.contains("only copy of the work"), "{} rode along", t.as_str());
        }
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
        for base in [WORKER_GENERAL_BASE, WORKER_VIEW_BUILDER_BASE,
                     WORKER_VIEW_REVIEWER_BASE, WORKER_DECISION_MAKER_BASE,
                     WORKER_FILE_FILER_BASE] {
            assert!(!base.contains("`ask`"));
            assert!(!base.contains("`delegate`"));
            assert!(!base.contains("`alarm`"));
        }
        assert!(WORKER_GENERAL_BASE.contains("`send_message`"));
        assert!(WORKER_GENERAL_BASE.contains("Never wait for an answer"));
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

    /// The frame log had a writer since `70479a9` and no *pointer* — no prompt anywhere
    /// named `memory/raw/sessions/`, so the one consumer the design allows (an agent that
    /// goes and looks) could not know it existed. `docs/arch/foundation.md` is explicit
    /// that the host "records the session stream verbatim and interprets none of it", so a
    /// code-level reader was never the answer; a path in the character was.

    /// The filing worker copies rather than moves, and the reason has to travel with the
    /// instruction: `docs/arch/surfaces.md` forbids log-then-copy for streamed bulk, so a
    /// reasonable person reading only that rule would "fix" this into a move — dangling
    /// the journal's own reference to the bytes, for the one class of object where the
    /// bytes are the point.
    #[test]
    fn the_filing_worker_is_told_why_it_copies() {
        assert!(WORKER_FILE_FILER_BASE.contains("copy, never move"));
        assert!(WORKER_FILE_FILER_BASE.contains("fades"));
    }

    #[test]
    fn the_voice_is_not_told_to_set_a_timer() {
        // There is no clock, and there will not be one — it was designed, deferred, and
        // declined (`docs/arch/core.md#glancing-up--and-why-there-is-no-clock`). Reaction
        // in particular is tools-off, so it cannot build one either. A brief that tells
        // the voice to arm a reminder for a check-in it promised is a brief describing a
        // mechanism that cannot run; what survives is the promise — size the silence, and
        // lean long because nothing will remind you.
        assert!(!REACTION_BASE.contains("set an alarm"));
        assert!(!REACTION_BASE.contains("When the alarm fires"));
        assert!(REACTION_BASE.contains("You have no timer"));
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
            !COGNITION_BASE.contains("`say`") && !COGNITION_BASE.contains("`show`"),
            "no expression tools in a prompt for a rung that holds none"
        );
    }

    /// The one thing in this prompt that can silently be wrong: the path. A relative
    /// one would have Deliberation write a real file that no reader ever looks at —
    /// the failure would look like "the agent never bothered", not like a bug.
    #[tokio::test]
    async fn deliberation_is_told_the_absolute_path_of_the_file_it_must_write() {
        let dir = tempfile::tempdir().unwrap();
        let prompt = deliberation_prompt(dir.path()).await;

        let expected = crate::mind::memory::layout::conversation_prompt_path(dir.path());
        assert!(expected.is_absolute(), "the target path must be absolute");
        assert!(
            prompt.contains(&expected.display().to_string()),
            "the prompt must name the exact file the window reads back"
        );
        assert!(
            !prompt.contains("{conversation_memory}"),
            "the placeholder must be interpolated"
        );

        // Two conversations must not be handed the same file.
    }

    #[tokio::test]
    async fn deliberation_prompt_takes_the_operator_override() {
        let dir = tempfile::tempdir().unwrap();
        let prompts = dir.path().join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        std::fs::write(prompts.join("deliberation.local.md"), "Keep the brief in French.").unwrap();
        install_prompts(dir.path()).unwrap();
        let prompt = deliberation_prompt(dir.path()).await;
        assert!(prompt.contains("Keep the brief in French."));
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
        // Fallback no longer means "the embedded base, byte for byte" — every rung prompt
        // is interpolated on the way out, so the invariant worth pinning is that the
        // *content* is there and the placeholders are resolved, installed or not.
        let dir = tempfile::tempdir().unwrap();
        let bare = reflection_prompt(dir.path()).await;
        assert!(bare.contains("tends your own house"), "the embedded base must still serve");
        assert!(!bare.contains("{skills_dir}"), "even the fallback interpolates");

        let prompts = dir.path().join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        std::fs::write(prompts.join("reflection.local.md"), "Prune harder.").unwrap();
        install_prompts(dir.path()).unwrap();
        assert!(reflection_prompt(dir.path()).await.contains("Prune harder."));
    }
}
