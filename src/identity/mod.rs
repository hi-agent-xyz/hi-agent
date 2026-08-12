//! identity — who the agent is.
//!
//! The factory-authored character, as **one whole prompt per role**: three rungs
//! (`reaction`, `cognition`, `reflection`) and five worker types under
//! `workers/`. Standing duties are not here at all — they are tasks
//! ([`crate::mind::memory::tasks`]), one ledger, projected into every window rather
//! than pointed at by a path.
//!
//! This module owns the **prompt cascade**: the bundled bases are materialised under
//! `<data_dir>/prompts/` at boot, each composed with an optional operator `*.local.md`
//! override ([`install_prompts`]), then read back whole at session open
//! ([`role_prompt`], [`reaction_system_prompt`]). That cascade is the
//! base‹override mechanism `docs/arch/foundation.md` generalises to base‹user‹self.
//!
//! **There is no seed.** A rung used to be handed ~18 lines pointing at `core.md`,
//! `meaning.md` and `self.md` and told to go read them; that shape and why it was
//! retired are recorded on [`installed_prompt`]. `core.md`, `meaning.md`, `appearance.md`
//! and `aesthetic.md` no longer exist, and [`install_prompts`]'s tests assert they stay
//! gone.
//!
//! Nor is there a second place identity lives. `self.md`, the recency digest `hot.md`
//! and the duty ledger `commitments.md` were each read by nothing by the end: under
//! `docs/arch/data.md#memoryprompts` who this install is is a *section of a generated
//! prompt*, Cognition writes that prompt, and duties are
//! [`crate::mind::memory::tasks`]. Their readers went when that writer's job was taken
//! up; their writers are gone now too. Existing data dirs keep whatever they
//! already have on disk — nothing deletes a file someone may have authored — and
//! `snapshot`'s `leftover_legacy_files_are_never_inlined` pins that a leftover can
//! never climb back into a window.

use std::path::{Path, PathBuf};

/// Built-in base prompts, embedded at compile time and materialised to disk by
/// [`install_prompts`].
///
/// **One file per rung, and each is that rung's whole system prompt** — nothing points
/// at anything else and nothing is fetched. They are reached through [`Role::base`], and
/// divide only by which entry point reads them back: [`reaction_system_prompt`] for the
/// tools-off voice, [`role_prompt`] for everything else. All ship in the binary and
/// refresh on every build.
///
/// The cost of "whole" is that ~2,000 words of shared character live in three copies,
/// and drift between them is the live risk — which is what the prompt tests below are
/// for.
const REACTION_BASE: &str = include_str!("reaction.md");
const REFLECTION_BASE: &str = include_str!("reflection.md");
const COGNITION_BASE: &str = include_str!("cognition.md");

/// The worker prompts, under `workers/` — one file per **type**, each whole.
///
/// There is no `common.md`. One used to sit above these as the layer every type composed
/// with, and retiring it is what made a worker's prompt the same kind of object as a
/// rung's: entire, and reachable through the same [`Role::base`]. Each keeps its own
/// `.local.md`, so an operator retunes one kind without touching the others.
const WORKER_GENERAL_BASE: &str = include_str!("workers/general.md");
const WORKER_VIEW_BUILDER_BASE: &str = include_str!("workers/view-builder.md");
const WORKER_VIEW_REVIEWER_BASE: &str = include_str!("workers/view-reviewer.md");
const WORKER_DECISION_MAKER_BASE: &str = include_str!("workers/decision-maker.md");
const WORKER_FILE_FILER_BASE: &str = include_str!("workers/file-filer.md");

/// What kind of working session this is — the `type` in `CreateWorker(type)`
/// (`docs/arch/foundation.md#the-agent-session-registry`), and the payload of
/// [`Role::Worker`].
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
    pub const ALL: &'static [Self] = &[
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

/// **Which role a session is running.** One namespace for every agent in the process,
/// and the only thing that tells one from another.
///
/// `docs/arch/agents.md` opens by saying every agent here is the same thing — a general
/// agent on a session, differing only in **system prompt** and **tool surface** — and
/// that "a new role is a new prompt, not new machinery". This enum is that sentence as a
/// type: nine roles today, four rungs and five worker types, and a tenth is a `.md` plus
/// a variant.
///
/// **It replaced three enums for the one concept**, which is why the switchboard used to
/// be blind to half of it: `registry::Role` (routing) and `agent::SessionRole` (tool
/// surface) had identical variant lists, and neither carried [`WorkerType`] (prompt). So
/// nothing stored what kind a worker was, and `GET /api/workers` reported all five
/// specialisms as a bare `worker`. The giveaway was already in this file — a worker's
/// prompt was fetched by handing the rung loader `workers/<type>` as its name, i.e. a
/// worker type was a rung with a path prefix, with no type to say so.
///
/// **Worker types nest rather than flatten**, and that is load-bearing. Routing asks one
/// question — *is this a worker?* (`Registry::send`, where a worker may address only its
/// owner). Nested, the answer is `matches!(role, Role::Worker(_))` however many types
/// exist. Flattened into nine peer variants it would be a five-arm match someone has to
/// remember to extend, which would make adding a worker type a routing edit — the exact
/// thing "a new role is a new prompt" rules out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    /// The mouth: one generation, speaks and shows, cannot wait on anything.
    #[default]
    Reaction,
    /// The shared brain — reads for the conversation, owns the task ledger,
    /// dispatches everything heavy, never speaks.
    Cognition,
    /// The inward brain — curates `data/`, answers to nobody, never speaks.
    Reflection,
    /// A working session, carrying the type that picks its prompt.
    Worker(WorkerType),
}

impl Role {
    /// Every role, for install and for test sweeps. One list, so a role cannot exist in
    /// one place and be forgotten in another — which is what the two lists this replaced
    /// (`bundled_rung_prompts()` for four rungs, `WorkerType::ALL` for five types) made
    /// easy: a worker prompt was never held to the sweeps the rungs were.
    pub const ALL: &'static [Self] = &[
        Self::Reaction,
        Self::Cognition,
        Self::Reflection,
        Self::Worker(WorkerType::General),
        Self::Worker(WorkerType::ViewBuilder),
        Self::Worker(WorkerType::ViewReviewer),
        Self::Worker(WorkerType::DecisionMaker),
        Self::Worker(WorkerType::FileFiler),
    ];

    /// The wire name — the `X-HI-Role` header, `tools_for_role`, and the `role` field on
    /// `GET /api/workers`.
    ///
    /// **Every worker type answers `"worker"`**, and that is the type doing exactly what
    /// it claims: it selects a prompt and nothing else, so all five specialisms share one
    /// tool surface and one header value. A caller that wants the specialism asks
    /// [`Role::worker_type`].
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reaction => "reaction",
            Self::Cognition => "cognition",
            Self::Reflection => "reflection",
            Self::Worker(_) => "worker",
        }
    }

    /// This role's prompt as a path stem under `<data_dir>/prompts/` — `reaction` and
    /// `workers/view-builder` being the same kind of thing is what lets
    /// [`install_prompts`] be one loop over [`Role::ALL`].
    ///
    /// The worker arms spell their subdirectory out rather than composing
    /// `format!("workers/{}", t.as_str())`, so this can stay `&'static str` and a session
    /// open costs no allocation. That is a second spelling of each type's name, and
    /// `a_worker_prompt_stem_is_its_type_under_workers` is what keeps the two in step.
    pub fn prompt_name(self) -> &'static str {
        match self {
            Self::Reaction => "reaction",
            Self::Cognition => "cognition",
            Self::Reflection => "reflection",
            Self::Worker(WorkerType::General) => "workers/general",
            Self::Worker(WorkerType::ViewBuilder) => "workers/view-builder",
            Self::Worker(WorkerType::ViewReviewer) => "workers/view-reviewer",
            Self::Worker(WorkerType::DecisionMaker) => "workers/decision-maker",
            Self::Worker(WorkerType::FileFiler) => "workers/file-filer",
        }
    }

    /// The embedded base text for this role, compiled in and materialised by
    /// [`install_prompts`].
    pub(crate) fn base(self) -> &'static str {
        match self {
            Self::Reaction => REACTION_BASE,
            Self::Cognition => COGNITION_BASE,
            Self::Reflection => REFLECTION_BASE,
            Self::Worker(t) => t.base(),
        }
    }

    /// The specialism behind a working session; `None` for a rung. This is what the
    /// registry could not answer before, and what `GET /api/workers` reports.
    pub fn worker_type(self) -> Option<WorkerType> {
        match self {
            Self::Worker(t) => Some(t),
            _ => None,
        }
    }

    /// Whether this is a working session, of any type. The predicate routing asks.
    pub fn is_worker(self) -> bool {
        matches!(self, Self::Worker(_))
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

/// Install the bundled prompts under `<data_dir>/prompts/` at startup, composing each
/// with its optional `*.local.md` operator override. The managed base files — one per
/// [`Role`], so `reaction.md` beside `workers/view-builder.md` — are rewritten every boot
/// so they stay current; operator edits live in the never-touched `*.local.md` siblings.
/// Each follows one workflow: ship embedded → materialise here → consumed from disk at
/// runtime.
///
/// **One loop over [`Role::ALL`].** This was four hardcoded `write`s for the rungs plus a
/// loop for the worker types, which is what a split concept costs at every site that
/// touches it: adding a rung meant remembering this function, and the two halves reached
/// for their override files through different base directories to arrive at the same
/// paths.
///
/// The worker prompts keep their own subdirectory, because there is one per type and they
/// would otherwise be the majority of a flat `prompts/`. One file per type and **no
/// shared base**: a worker's prompt is whole, the same way a rung's is. `common.md` used
/// to sit above them as the layer every type composed with, which meant a decision-maker
/// read how to drive a camera and a file-filer read how to review its own artwork. The
/// price is duplication — a 35-line preamble identical in all five, plus ~36 further
/// lines shared by two or three of them — and drift between the copies is the risk the
/// prompt tests below hold.
pub fn install_prompts(data_dir: &Path) -> std::io::Result<()> {
    let dir = data_dir.join("prompts");
    // The nested one first: creating `prompts/workers/` creates `prompts/` with it.
    std::fs::create_dir_all(dir.join("workers"))?;
    for role in Role::ALL {
        let name = role.prompt_name();
        std::fs::write(
            dir.join(format!("{name}.md")),
            compose_prompt(role.base(), &dir, &format!("{name}.local.md")),
        )?;
    }

    tracing::info!(
        dir = %dir.display(),
        roles = Role::ALL.len(),
        "installed bundled prompts (one per role, workers under workers/)"
    );
    Ok(())
}

/// A role's **whole** system prompt: its installed `.md`, entire and interpolated.
///
/// Read off disk so an operator's `*.local.md` reaches every role the same way, falling
/// back to the embedded base when the file is missing or empty. Read fresh per open, so
/// an edit takes effect without a restart.
///
/// **This is the one prompt entry point**, and it is one because a worker type and a rung
/// are one concept. What it replaced — `worker_prompt(data_dir, kind)` beside three
/// per-rung wrappers — already routed through the same loader with `workers/<type>`
/// pasted in as a name; the type just had nowhere to live. The remaining wrappers below
/// exist only where a rung needs something more than its file: Cognition interpolates
/// the brief's path, Reaction is read raw and tools-off.
///
/// Only the directory placeholders [`installed_prompt`] expands are interpolated. Former
/// routing and encoded-path placeholders are gone, so no user string can redirect a
/// session's data path.
pub async fn role_prompt(data_dir: &Path, role: Role) -> String {
    installed_prompt(data_dir, role.prompt_name(), role.base()).await
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
async fn installed_prompt(data_dir: &Path, name: &str, fallback: &'static str) -> String {
    let base = abs(data_dir);
    let path = base.join("prompts").join(format!("{name}.md"));
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(s) if !s.trim().is_empty() => s,
        _ => fallback.to_string(),
    };
    let dir = |p: PathBuf| p.display().to_string();
    let mut out = text
        .replace("{skills_dir}", &dir(crate::mind::skills::skills_dir(&base)))
        // The root a `⟨ref: <day>/<file>⟩` resolves against: `<raw_dir>/<channel>/<ref>`.
        // Without it a ref is a fragment, not a path — and "a ref is a path, and an agent
        // that can read files can open it" is the reasoning that retired the perception
        // tool (`docs/arch/agents.md`). The rung was told to open refs and never told
        // where they start.
        .replace("{raw_dir}", &dir(crate::mind::memory::layout::raw_root(&base)))
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

/// **Cognition**'s whole prompt — the brain that reads for the conversation, owns the
/// task ledger, hands every heavy thing out, and never speaks
/// (`docs/arch/agents.md#cognition`).
///
/// Cognition's **whole** prompt, like every other rung's: the character and the role
/// arrive together, which is what makes the rung *the agent* rather than a generic
/// assistant. It carries the ledger pen, and — since Deliberation was retired into it —
/// the two jobs that rung existed for: opening what arrived (a ref is a path) and
/// writing the conversation's brief.
///
/// `{conversation_memory}` is interpolated to the **absolute** path of the brief it must
/// write, because an agent-facing path that is relative is a path to the wrong file.
/// That is the *only* interpolation of substance here: what Cognition carries forward
/// arrives in the **window** with the projected ledger
/// ([`crate::mind::memory::snapshot::agent_window`]), not in this layer — the same
/// content in both places would be one copy going stale against the other, and the window
/// is the half that is rebuilt every turn.
pub async fn cognition_prompt(data_dir: &Path) -> String {
    let text = role_prompt(data_dir, Role::Cognition).await;
    let target = crate::mind::memory::layout::conversation_prompt_path(&abs(data_dir));
    text.replace("{conversation_memory}", &target.display().to_string())
}

/// The reflection ("sleep") session's system prompt: the materialised
/// `<data_dir>/prompts/reflection.md` (operator-overridable via `reflection.local.md`),
/// or the embedded [`REFLECTION_BASE`] when that file is missing or empty. It is
/// **inlined** as the reflection session's system prompt rather than Read by the agent
/// — it *is* the task's instructions, so it must be present before the session can act.
/// Read fresh each round, so an operator edit takes effect without a restart.
pub async fn reflection_prompt(data_dir: &Path) -> String {
    role_prompt(data_dir, Role::Reflection).await
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
/// episode written, a duty taken on), so it can only ever colour
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


/// Whether this looks like a genuine **first meeting** — a brand-new install where the
/// agent has no history with the person yet. True when neither accruing trace exists:
/// no memory episodes, and nothing owed. An authored `self.md` is deliberately not
/// consulted — an operator may pre-author identity on a fresh box, and that says
/// nothing about whether the person has been met. The predicate self-clears: the first
/// jotted memory, reflection, or task flips it false, so the first-hello cue can never
/// repeat.
///
/// Two weaker probes used to sit beside these. `hot.md` was a projection *of* the
/// episodes, so it could never be present without them and never widened the answer;
/// `commitments.md` was the superseded duty ledger, and a duty lands in the task
/// facets now. Both files are retired, and an install old enough to hold either has
/// long since reflected at least once — which is what `no_episodes` reads.
fn is_first_meeting(base: &Path) -> bool {
    use crate::mind::memory::layout;
    let empty_dir = |dir: PathBuf| match std::fs::read_dir(dir) {
        Ok(mut entries) => entries.next().is_none(),
        Err(_) => true, // dir absent ⇒ nothing recorded
    };
    let no_episodes = empty_dir(layout::episodes_dir(base));
    let no_tasks =
        empty_dir(layout::facets_dir(base).join(crate::mind::memory::tasks::DIMENSION));
    no_episodes && no_tasks
}

#[cfg(test)]
mod soul_tests {
    use super::*;

    #[tokio::test]
    async fn fresh_install_gets_the_first_meeting_cue_in_the_voice() {
        // A brand-new data dir has no episodes and nothing owed — so the
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
        // The moment anything has accrued — here one written episode — it's no longer
        // a first meeting, so the cue disappears and can never nag on later wakes.
        let dir = tempfile::tempdir().unwrap();
        let episode = crate::mind::memory::layout::episodes_dir(dir.path()).join("2026-08-09T00-00-00");
        std::fs::create_dir_all(&episode).unwrap();
        std::fs::write(episode.join("episode.md"), "we talked about the drive view\n").unwrap();
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
        // One loop for all nine, rungs and workers alike — the four rungs were named one
        // at a time here while the worker files went unchecked.
        for role in Role::ALL {
            let at = p.join(format!("{}.md", role.prompt_name()));
            assert_eq!(
                std::fs::read_to_string(&at).unwrap(),
                role.base(),
                "{} was not installed verbatim",
                role.prompt_name()
            );
        }
        for gone in ["core.md", "meaning.md", "appearance.md", "aesthetic.md", "workers/common.md"] {
            assert!(!p.join(gone).exists(), "{gone} should be retired");
        }
    }

    /// An operator override reaches a worker exactly as it reaches a rung. Worth pinning
    /// because the two halves used to resolve their `*.local.md` through different base
    /// directories (`prompts/` vs `prompts/workers/`) to land on the same path; there is
    /// one base directory now, and the `workers/` segment rides in the stem.
    #[test]
    fn install_layers_the_operator_override_into_a_worker_file_too() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("prompts");
        std::fs::create_dir_all(p.join("workers")).unwrap();
        std::fs::write(p.join("workers/file-filer.local.md"), "File nothing on a Sunday.").unwrap();
        install_prompts(dir.path()).unwrap();
        let out = std::fs::read_to_string(p.join("workers/file-filer.md")).unwrap();
        assert!(out.starts_with(WORKER_FILE_FILER_BASE));
        assert!(out.contains("File nothing on a Sunday."));
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

    /// Every role is self-contained: it opens with its own character rather than a
    /// pointer to a file it must go and fetch. This is the property the seed did not
    /// have — the seed's instruction to Read was *conditional*, and nothing checked it.
    ///
    /// **Swept over [`Role::ALL`], so it now covers the five worker prompts too.** It ran
    /// over four rungs while `bundled_rung_prompts()` was the list, which is exactly the
    /// hole a split concept leaves: the sweep existed to stop a *new* prompt escaping the
    /// rules the others are held to, and five prompts were escaping it the whole time.
    #[tokio::test]
    async fn every_role_carries_its_own_character() {
        let dir = tempfile::tempdir().unwrap();
        install_prompts(dir.path()).unwrap();
        for role in Role::ALL {
            let (name, base) = (role.prompt_name(), role.base());
            assert!(
                !base.contains("read them all now"),
                "{name} still bootstraps instead of carrying its character"
            );
            assert!(base.len() > 2_000, "{name} looks too thin to be self-contained");
        }
        // And the interpolations resolve rather than reaching the model raw.
        for text in [cognition_prompt(dir.path()).await, reflection_prompt(dir.path()).await] {
            assert!(!text.contains("{skills_dir}"), "an unresolved placeholder reached the rung");
            assert!(!text.contains("{conversation_memory}"));
            let skills = crate::mind::skills::skills_dir(dir.path());
            assert!(skills.is_absolute());
            assert!(text.contains(&skills.display().to_string()));
        }
    }

    /// One pen on the ledger. Cognition writes it; nobody else is told how — and "nobody"
    /// now genuinely means all nine roles, not the four this used to look at.
    #[test]
    fn exactly_one_prompt_hands_out_the_ledger_pen() {
        let carriers: Vec<&str> = Role::ALL
            .iter()
            .filter(|r| r.base().contains("only writer of the task ledger"))
            .map(|r| r.prompt_name())
            .collect();
        assert_eq!(carriers, vec!["cognition"], "the pen must be held once");
    }

    /// A worker's prompt stem is its type under `workers/`. [`Role::prompt_name`] spells
    /// the five worker paths out as literals so it can return `&'static str`, which is a
    /// second spelling of each type's name; this is what stops the two drifting.
    #[test]
    fn a_worker_prompt_stem_is_its_type_under_workers() {
        for t in WorkerType::ALL {
            assert_eq!(Role::Worker(*t).prompt_name(), format!("workers/{}", t.as_str()));
        }
    }

    /// Eight roles — three rungs and five worker types — in one namespace, no duplicates,
    /// and every worker type reachable as a role. The list is what `install_prompts` and
    /// the sweeps both walk, so a variant missing from it is a prompt that is never
    /// installed and never checked.
    ///
    /// It was nine until Deliberation was retired into Cognition.
    #[test]
    fn every_role_is_in_the_one_list_exactly_once() {
        assert_eq!(Role::ALL.len(), 3 + WorkerType::ALL.len());
        let mut names: Vec<&str> = Role::ALL.iter().map(|r| r.prompt_name()).collect();
        names.sort_unstable();
        let mut deduped = names.clone();
        deduped.dedup();
        assert_eq!(names, deduped, "two roles share a prompt file");
        for t in WorkerType::ALL {
            assert!(Role::ALL.contains(&Role::Worker(*t)), "{} is not a role", t.as_str());
        }
        // The wire name collapses the five specialisms onto one tool surface.
        assert!(Role::ALL.iter().filter(|r| r.as_str() == "worker").count() == WorkerType::ALL.len());
        assert_eq!(Role::Worker(WorkerType::ViewBuilder).worker_type(), Some(WorkerType::ViewBuilder));
        assert_eq!(Role::Cognition.worker_type(), None);
        assert!(Role::Worker(WorkerType::General).is_worker());
        assert!(!Role::Reaction.is_worker());
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

    /// Nothing in a worker prompt is interpolated from a user string.
    #[tokio::test]
    async fn no_user_string_is_interpolated_into_a_worker_prompt() {
        let dir = tempfile::tempdir().unwrap();
        install_prompts(dir.path()).unwrap();

        let filer = role_prompt(dir.path(), Role::Worker(WorkerType::FileFiler)).await;
        assert!(
            filer.contains("memory/raw/file/"),
            "the filer is pointed at the flat channel directory"
        );

        for t in WorkerType::ALL {
            let p = role_prompt(dir.path(), Role::Worker(*t)).await;
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
            let p = role_prompt(dir.path(), Role::Worker(*t)).await;
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
        let builder = role_prompt(dir.path(), Role::Worker(WorkerType::ViewBuilder)).await;
        assert!(builder.contains(VIEW_LAYER));
        assert!(!builder.contains("Report the path"), "the filing layer must not ride along");

        let filer = role_prompt(dir.path(), Role::Worker(WorkerType::FileFiler)).await;
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
            let p = role_prompt(dir.path(), Role::Worker(t)).await;
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
        let general = role_prompt(dir.path(), Role::Worker(WorkerType::General)).await;
        assert!(general.contains("outside world can already see"));
        for t in [WorkerType::ViewReviewer, WorkerType::FileFiler] {
            let p = role_prompt(dir.path(), Role::Worker(t)).await;
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

    /// Reuse rests on the builder reading the toolbox before it authors, and on every
    /// view carrying the one line that makes the toolbox readable. Both are guidance —
    /// the model may skip either — but the *degradation* is what was designed for: a
    /// missing `purpose:` line costs you a filename, never a wrong "never built".
    /// That only holds while the prompt still asks for the line, so the ask is pinned.
    ///
    /// `_builtin/` is pinned with it, and it is the sharper one. Those views sit inside
    /// the workshop the builder is now told to scan, and the binary rewrites them on
    /// every boot ([`crate::mind::views::install_builtin_views`]) — so a builder that
    /// adapts one in place loses the work at the next start, silently. Telling it to
    /// read the tree without telling it about that folder is the hazard this pass
    /// introduced; do not drop the warning without removing the instruction to scan.
    #[test]
    fn the_builder_is_told_how_the_toolbox_is_read_and_which_of_it_is_not_its_own() {
        assert!(WORKER_VIEW_BUILDER_BASE.contains("// purpose:"));
        assert!(WORKER_VIEW_BUILDER_BASE.contains("_builtin/"));
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

    /// `docs/arch/agents.md` retires the perception tool because "a ref is a path, and
    /// an agent that can read files can open it" — which is only true if the rung is
    /// told where refs start. Nothing said, so the two prompts that open one now name
    /// `{raw_dir}`, and it has to survive substitution: an unexpanded placeholder is a
    /// path to nothing, and the rung reports the file missing rather than empty.
    #[tokio::test]
    async fn the_rungs_that_open_a_ref_are_told_where_refs_start() {
        assert!(COGNITION_BASE.contains("{raw_dir}"), "cognition must name the root");
        assert!(WORKER_FILE_FILER_BASE.contains("{raw_dir}"), "the filer must name the root");

        let dir = tempfile::tempdir().unwrap();
        let root = crate::mind::memory::layout::raw_root(&abs(dir.path())).display().to_string();
        for text in [
            cognition_prompt(dir.path()).await,
            role_prompt(dir.path(), Role::Worker(WorkerType::FileFiler)).await,
        ] {
            assert!(!text.contains("{raw_dir}"), "an unresolved placeholder reached the rung");
            assert!(text.contains(&root), "the substituted root must be the absolute raw root");
        }
    }

    #[test]
    fn the_voice_has_exactly_one_timer_and_is_told_its_name() {
        // The retired clock stays retired: `alarm` was a general scheduler and its
        // vocabulary must not creep back (`docs/arch/host.md#glancing-up`).
        assert!(!REACTION_BASE.contains("set an alarm"));
        assert!(!REACTION_BASE.contains("When the alarm fires"));

        // What replaced "you have no timer" is one deadline the voice arms itself, on
        // the utterance that makes the promise. The brief has to name the parameter,
        // because a promise the model never arms is the failure this was built for —
        // the person filling the silence by asking "progress?".
        assert!(REACTION_BASE.contains("back_in"), "the brief must name the parameter");
        assert!(
            REACTION_BASE.contains("only timer you have"),
            "and must not leave the voice thinking it has more than one"
        );
        assert!(
            !REACTION_BASE.contains("You have no timer"),
            "the brief still describes a host that cannot wake it"
        );
    }




    /// "Sole writer of the ledger" is not enforced by any rail — it is enforced by exactly
    /// one prompt carrying the instruction. So the thing that can silently go wrong is the
    /// instruction existing in two places, or in none: two writers means one is wrong with
    /// no way to tell which, and none means every promise the agent makes dies at the next
    /// restart. The sweep over every role is
    /// [`exactly_one_prompt_hands_out_the_ledger_pen`]; this pins the positive half.
    #[test]
    fn exactly_one_rung_is_told_to_write_the_ledger() {
        assert!(
            COGNITION_BASE.contains("only writer of the task ledger"),
            "Cognition must be told the pen is its"
        );
    }

    /// **The reply that used to be structural is now guidance, so it is pinned here.**
    /// Deliberation's answer came back as a `WorkerReport` whether or not it chose to
    /// send one — the host delivered it. Cognition has no such path: the only thing that
    /// leaves the rung is `send_message`, and everywhere else in its prompt silence is a
    /// legitimate outcome it is explicitly trusted to choose. That trust is correct for a
    /// glance-up and catastrophic for a hand-down, where a person is sitting in front of
    /// the voice waiting. `cognition::turn` carries a host-side backstop for it; this
    /// asserts the prompt asks for the right thing in the first place, because a backstop
    /// that fires every turn means the guidance is not working.
    #[test]
    fn a_hand_down_from_the_voice_is_always_answered() {
        assert!(
            COGNITION_BASE.contains("A hand-down from the voice is always answered"),
            "Cognition must be told the one case where silence is not an option"
        );
        assert!(
            COGNITION_BASE.contains("Someone is waiting on the other end of that"),
            "and must be told why — a hand-down is a person mid-conversation, not a memo"
        );
    }

    /// The two jobs Deliberation existed for, now Cognition's. Both are the kind of thing
    /// that reads as decoration and is load-bearing: without the first, the rung that can
    /// open the photo does not know it should; without the second, Reaction's brief has no
    /// writer at all and the voice walks into every turn blank.
    #[test]
    fn cognition_carries_what_deliberation_was_for() {
        assert!(
            COGNITION_BASE.contains("A ref is a path"),
            "Cognition must be told that looking is opening a path, not calling a tool"
        );
        assert!(
            COGNITION_BASE.contains("{conversation_memory}"),
            "Cognition must be pointed at the brief it writes"
        );
        // The read/dispatch line is the whole reason this merge is safe: a Cognition that
        // grinds is a Cognition the conversation waits behind.
        assert!(
            COGNITION_BASE.contains("reading versus doing"),
            "Cognition must be told where its own hands stop"
        );
    }

    /// The pen has two ends, and only one of them was ever written down. Holding the sole
    /// write on the ledger makes Cognition the sole *closer* too — nothing else in the loop
    /// can retire a task, so an instruction that only says how to open one produces a list
    /// that grows and never shrinks. That is not hypothetical: nine tasks stayed `open`
    /// across a week, three of them delivered or called off out loud, and the closing
    /// decision got handed to the person as buttons on a screen they never pressed.
    #[test]
    fn the_rung_holding_the_pen_is_told_to_close_as_well_as_open() {
        assert!(
            COGNITION_BASE.contains("Closing is yours"),
            "the ledger's only writer must be told closing is its job too"
        );
        assert!(
            COGNITION_BASE.contains("You owe the ask, not the wait"),
            "a task whose last step is theirs must not sit open as a reminder for them"
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
    /// one would have Cognition write a real file that no reader ever looks at —
    /// the failure would look like "the agent never bothered", not like a bug.
    #[tokio::test]
    async fn cognition_is_told_the_absolute_path_of_the_file_it_must_write() {
        let dir = tempfile::tempdir().unwrap();
        let prompt = cognition_prompt(dir.path()).await;

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
    }

    #[tokio::test]
    async fn cognition_prompt_takes_the_operator_override() {
        let dir = tempfile::tempdir().unwrap();
        let prompts = dir.path().join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        std::fs::write(prompts.join("cognition.local.md"), "Keep the brief in French.").unwrap();
        install_prompts(dir.path()).unwrap();
        let prompt = cognition_prompt(dir.path()).await;
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
