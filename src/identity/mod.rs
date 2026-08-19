//! identity — who the agent is.
//!
//! The factory-authored character, as **one whole prompt per role**: three rungs
//! (`reaction`, `cognition`, `reflection`) and five worker types under
//! `workers/`. Standing duties are not here at all — they are tasks
//! ([`crate::mind::memory::tasks`]), one ledger, projected into every window rather
//! than pointed at by a path.
//!
//! This module owns the **prompt cascade**: the bundled bases are materialised under
//! `<data_dir>/prompts/factory/` at boot ([`install_prompts`]), then read back whole at
//! session open ([`role_prompt`], [`reaction_system_prompt`]). That cascade is the
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
/// tools-off Reaction, [`role_prompt`] for everything else. All ship in the binary and
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
/// rung's: entire, and reachable through the same [`Role::base`].
const WORKER_GENERAL_BASE: &str = include_str!("workers/general.md");
const WORKER_VIEW_BUILDER_BASE: &str = include_str!("workers/view-builder.md");
const WORKER_VIEW_REVIEWER_BASE: &str = include_str!("workers/view-reviewer.md");
const WORKER_DECISION_MAKER_BASE: &str = include_str!("workers/decision-maker.md");
const WORKER_DRIVE_ORGANIZER_BASE: &str = include_str!("workers/drive-organizer.md");
const WORKER_PERSON_READER_BASE: &str = include_str!("workers/person-reader.md");
const WORKER_TASK_MANAGER_BASE: &str = include_str!("workers/task-manager.md");

/// Reference pages under `craft/`, installed beside the prompts and read from disk only
/// when a job touches them.
///
/// **These are not [`Role`]s.** A role's prompt is what a session *is*, loaded whole
/// before it does anything; a craft page is something it goes and opens, the way it
/// opens a view already in the workshop. Keeping them out of `Role::ALL` is what lets
/// the set grow without every session paying for the ones it never reads —
/// `view-builder.md` names the page and the session decides.
const CRAFT_PAGES: &[(&str, &str)] = &[(
    "data-visualization.md",
    include_str!("craft/data-visualization.md"),
)];

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
    /// Knows how `drive/` is laid out: puts a new thing where the drive is already
    /// going, says where an existing one lives, and straightens a corner that has
    /// drifted (`docs/arch/agents.md#drive-organizer`). Reading and writing the drive is
    /// every agent's; this is the one that is asked when *where* is the hard part.
    DriveOrganizer,
    /// Reads one person out of the record and folds what it learns into their facet —
    /// including the `## Working with them` section Reaction is projected
    /// ([`crate::mind::memory::conduct`]).
    PersonReader,
    /// Keeps the task ledger (`docs/arch/agents.md#task-manager`) — the only role that
    /// may write a task's `status`. Split out of Cognition because the rung that hands
    /// work out is the worst-placed one to rule that its own errand ended.
    TaskManager,
}

impl WorkerType {
    /// The wire name, which is also the prompt's filename stem.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::ViewBuilder => "view-builder",
            Self::ViewReviewer => "view-reviewer",
            Self::DecisionMaker => "decision-maker",
            Self::DriveOrganizer => "drive-organizer",
            Self::PersonReader => "person-reader",
            Self::TaskManager => "task-manager",
        }
    }

    /// Every type, for the tool schema's `enum` and for install/test sweeps. One list,
    /// so a new variant cannot be advertised in one place and forgotten in the other.
    pub const ALL: &'static [Self] = &[
        Self::General,
        Self::ViewBuilder,
        Self::ViewReviewer,
        Self::DecisionMaker,
        Self::DriveOrganizer,
        Self::PersonReader,
        Self::TaskManager,
    ];

    /// Parse a wire name. `None` for anything unknown — the caller turns that into a
    /// tool error naming the valid set, rather than silently handing back a general
    /// worker: a mistyped `view-buidler` that quietly becomes a general session is a
    /// worker that will not do the job it was made for, and nothing says so.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|t| t.as_str() == s.trim())
    }

    /// Whether a session of this kind is expected to name a ledger task.
    ///
    /// Almost all of them are: a worker is how a task gets done, so one created without a
    /// `subject` leaves its task reading as owed by nobody, and that is worth saying out
    /// loud wherever a worker is listed.
    ///
    /// `person-reader` is the exception, and it is the exception by design. It is one of
    /// Reflection's **organizers** (`docs/arch/agents.md`) — housekeeping keyed to a
    /// `people/<name>` facet, dispatched one per person present in a stretch, and never
    /// work the ledger owes anyone. There is no task for it to name. Flagging it would put
    /// the phrase on every settling pass's fan-out, which is the same mistake the ledger
    /// side already refuses: "nobody on it" is said only where nobody is a problem.
    pub fn expects_a_subject(self) -> bool {
        !matches!(self, Self::PersonReader)
    }

    /// The embedded base for this type's layer.
    fn base(self) -> &'static str {
        match self {
            Self::General => WORKER_GENERAL_BASE,
            Self::ViewBuilder => WORKER_VIEW_BUILDER_BASE,
            Self::ViewReviewer => WORKER_VIEW_REVIEWER_BASE,
            Self::DecisionMaker => WORKER_DECISION_MAKER_BASE,
            Self::DriveOrganizer => WORKER_DRIVE_ORGANIZER_BASE,
            Self::PersonReader => WORKER_PERSON_READER_BASE,
            Self::TaskManager => WORKER_TASK_MANAGER_BASE,
        }
    }
}

/// **Which role a session is running.** One namespace for every agent in the process,
/// and the only thing that tells one from another.
///
/// `docs/arch/agents.md` opens by saying every agent here is the same thing — a general
/// agent on a session, differing only in **system prompt** and **tool surface** — and
/// that "a new role is a new prompt, not new machinery". This enum is that sentence as a
/// type: ten roles today, three rungs and seven worker types, and an eleventh is a `.md` plus
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
        Self::Worker(WorkerType::DriveOrganizer),
        Self::Worker(WorkerType::PersonReader),
        Self::Worker(WorkerType::TaskManager),
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
            Self::Worker(WorkerType::DriveOrganizer) => "workers/drive-organizer",
            Self::Worker(WorkerType::PersonReader) => "workers/person-reader",
            Self::Worker(WorkerType::TaskManager) => "workers/task-manager",
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

/// Install the bundled prompts under `<data_dir>/prompts/` at startup. One file per
/// [`Role`], so `reaction.md` beside `workers/view-builder.md`, rewritten every boot so
/// they stay current.
///
/// **There is no override layer, and that is the whole design.** A `*.local.md` sibling
/// used to be appended under an "Operator overrides" header. Nothing ever wrote one — not
/// on any install — and it contradicted the rule these prompts exist to serve
/// (`docs/arch/data.md`): an instruction from the person becomes a facet or a task, and
/// nothing overrides the agent without going through it. A slot with no occupant is not a
/// courtesy; it is a second source of character that would have gone stale unseen.
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
/// read how to drive a camera and a drive-organizer read how to review its own artwork. The
/// price is duplication — a preamble repeated in each, plus ~36 further lines shared by
/// two or three of them — and drift between the copies is the risk the prompt tests
/// below hold.
pub fn install_prompts(data_dir: &Path) -> std::io::Result<()> {
    let dir = data_dir.join("prompts");
    // The nested ones first: creating `prompts/workers/` creates `prompts/` with it.
    std::fs::create_dir_all(dir.join("workers"))?;
    std::fs::create_dir_all(dir.join("craft"))?;
    for role in Role::ALL {
        let name = role.prompt_name();
        std::fs::write(
            dir.join(format!("{name}.md")),
            role.base(),
        )?;
    }
    for (name, body) in CRAFT_PAGES {
        std::fs::write(dir.join("craft").join(name), body)?;
    }
    for gone in RETIRED_PROMPTS {
        let at = dir.join(gone);
        match std::fs::remove_file(&at) {
            Ok(()) => tracing::info!(file = %at.display(), "removed retired prompt"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                tracing::warn!(file = %at.display(), error = %e, "could not remove retired prompt")
            }
        }
    }

    tracing::info!(
        dir = %dir.display(),
        roles = Role::ALL.len(),
        craft = CRAFT_PAGES.len(),
        "installed bundled prompts (one per role, workers under workers/, craft pages under craft/)"
    );
    Ok(())
}

/// Prompt files this binary used to install and no longer does.
///
/// Writing only the live set leaves the dead ones behind forever: a box that ran an
/// older build keeps `appearance.md` and `aesthetic.md` in `prompts/` indefinitely, and
/// a retired prompt sitting beside the live ones reads as authoritative to anyone —
/// operator or agent — who opens the directory to find out what the agent is told. The
/// tests below assert a fresh install has none of them; this is what makes that true of
/// an *upgraded* install too.
const RETIRED_PROMPTS: &[&str] = &[
    "core.md",
    "meaning.md",
    "appearance.md",
    "aesthetic.md",
    "workers/common.md",
    // The rung renames. Both were still sitting in `data/prompts/` on this machine, months
    // after the binary stopped installing them: `speaking.md` is what `reaction.md` was
    // called, and `deliberation.md` is the rung Cognition replaced. A stale prompt reads
    // exactly like a current one.
    "speaking.md",
    "deliberation.md",
    // The worker rename. `file-filer` only put a handed-over file down; `drive-organizer`
    // owns the layout — putting down, finding, and straightening — so the old file left in
    // place would read as a second, narrower filing role that nothing can dispatch.
    "workers/file-filer.md",
];

/// A role's **whole** system prompt: its installed `.md`, entire and interpolated.
///
/// Read off disk rather than from the embedded text, falling back to the base when the
/// file is missing or empty. Read fresh per open, so an edit to the installed file takes
/// effect without a restart — until the next boot rewrites it from the binary.
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
///   file-filing, and all three read how to drive a screen none of them has `hi_look` for.
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
        .replace(
            "{facets_dir}",
            &dir(crate::mind::memory::layout::facets_dir(&base)),
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
    let target = crate::mind::memory::layout::reaction_seed_path(&abs(data_dir));
    text.replace("{conversation_memory}", &target.display().to_string())
}

/// The reflection ("sleep") session's system prompt: the materialised
/// `<data_dir>/prompts/reflection.md`, or the embedded [`REFLECTION_BASE`] when that file
/// is missing or empty. It is
/// **inlined** as the reflection session's system prompt rather than Read by the agent
/// — it *is* the task's instructions, so it must be present before the session can act.
/// Read fresh each round, so an operator edit takes effect without a restart.
pub async fn reflection_prompt(data_dir: &Path) -> String {
    role_prompt(data_dir, Role::Reflection).await
}

/// **Reaction**'s system prompt — what reaches the person (`docs/arch/agents.md#reaction`).
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
/// its tools are — which meant Reaction's brief lived in two places, only one of them
/// operator-overridable. A prompt is prose; it belongs in the `.md`. The file is now
/// named for the rung that reads it (`docs/arch/arch.md#character`: a file per role)
/// rather than for the activity, which is what `speaking.md` was.
///
/// Its surface is `hi_say` · `hi_show` · `hi_send_message`
/// (`docs/arch/foundation.md#default-tool-surfaces`), and `reaction.md` must name all
/// three: the file once said "you have exactly two", then told Reaction to "hand it
/// onward" without naming the verb that does it.
///
/// Read from `<data_dir>/prompts/reaction.md`, falling back to the embedded
/// [`REACTION_BASE`], and **nothing is appended to it**.
///
/// **It used to carry two facts, and a system prompt is the one place a fact cannot
/// live.** The first-meeting cue and the language preference are *state*, not character,
/// and `baseInstructions` is fixed at `thread/start`: someone who changed Settings ▸
/// Language mid-conversation went on being answered in the old one until the session
/// rotated, because the sentence saying otherwise had been sent before they touched it.
/// Both are ordinary blocks of the window now
/// ([`crate::mind::memory::snapshot::window`]), where something moving is something the
/// next turn carries. What is left is identical for every install and every thread, which
/// is what a character is.
pub async fn reaction_system_prompt(data_dir: &Path) -> String {
    let base = data_dir.join("prompts").join("reaction.md");
    let reaction = match tokio::fs::read_to_string(&base).await {
        Ok(s) if !s.trim().is_empty() => s,
        _ => REACTION_BASE.to_string(),
    };
    reaction.trim().to_string()
}

/// The first-meeting cue as a window block, or `""` once this pair has any history.
///
/// It can only be true when a thread opens — no later thread can be a first meeting — so
/// it lands on the cold turn that opens the very first one and is absent from every thread
/// after. That the window can never *withdraw* it mid-thread (an emptied block is skipped,
/// not retracted) costs nothing here: the hello it is for happens in the same breath it
/// arrives.
pub fn first_meeting_block(data_dir: &Path) -> String {
    if is_first_meeting(data_dir) { FIRST_MEETING_CUE.to_string() } else { String::new() }
}

/// The language preference as a window block, or `""` when nobody has set one.
pub fn language_block(data_dir: &Path) -> String {
    language_line(data_dir).unwrap_or_default()
}

/// One extra line on a genuine first meeting — the brand-new install where nothing has
/// accrued yet. It disappears on its own the moment any history exists (a memory
/// episode written, a duty taken on), so it can only ever colour
/// the very first hello, never nag. It rides on **Reaction**, because the hello and the
/// welcome view are both Reaction's to give.
const FIRST_MEETING_CUE: &str = "## First meeting\nTrue only right now: this is a \
brand-new install — you and this person haven't met yet. So when they first reach out, \
treat it as a first meeting: open with a real first hello (the shape of it is above), \
put the built-in welcome on screen while you speak it (`hi_show` with ref \
`factory/welcome`), then hand over the floor. One warm beat that lands who you are — \
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
        "## Language\nSpeak with the person in {lang} by default, unless they clearly \
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
    async fn fresh_install_gets_the_first_meeting_cue_in_reaction() {
        // A brand-new data dir has no episodes and nothing owed, so the cue is there for
        // the window to carry — naming the welcome view, because the hello is Reaction's
        // to give and it cannot go and read anything.
        let dir = tempfile::tempdir().unwrap();
        assert!(is_first_meeting(dir.path()));
        let block = first_meeting_block(dir.path());
        assert!(block.contains("first meeting"), "{block}");
        assert!(block.contains("factory/welcome"), "{block}");
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
        assert!(first_meeting_block(dir.path()).is_empty());
    }

    /// **The character is the same for everyone**, which is why the two facts below moved
    /// out of it. `baseInstructions` is fixed at `thread/start`; a fact about this install
    /// or this person's Settings is not, and one that changed mid-conversation had no way
    /// to reach a thread that was already open.
    #[tokio::test]
    async fn the_system_prompt_carries_no_state() {
        use crate::foundation::credentials::set_setting;
        let dir = tempfile::tempdir().unwrap();
        set_setting(dir.path(), crate::foundation::config::KEY_LANGUAGE, "zh-Hans").unwrap();
        assert!(is_first_meeting(dir.path()));
        let prompt = reaction_system_prompt(dir.path()).await;
        // `reaction.md` names a brand-new install itself, in `# The first hello` — what
        // must be absent is the *cue*, which is this heading and the line under it.
        assert!(!prompt.contains("## First meeting"), "the cue is the window's now");
        assert!(!prompt.contains("Speak with the person in"), "the language line is too");
        assert_eq!(prompt, REACTION_BASE.trim(), "nothing at all is appended");
    }






    #[tokio::test]
    async fn reaction_gets_the_language_line_too() {
        // Settings ▸ Language has to reach the rung that actually talks, and that rung
        // cannot read a file to find it — so it is projected, and **a later change reaches
        // a thread that is already open**, which is what it could not do from a system
        // prompt fixed at `thread/start`.
        use crate::foundation::credentials::set_setting;
        let dir = tempfile::tempdir().unwrap();
        assert!(language_block(dir.path()).is_empty());
        set_setting(dir.path(), crate::foundation::config::KEY_LANGUAGE, "zh-Hans").unwrap();
        assert!(language_block(dir.path()).contains("Speak with the person in 简体中文"));
        set_setting(dir.path(), crate::foundation::config::KEY_LANGUAGE, "en").unwrap();
        assert!(language_block(dir.path()).contains("English"));
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
        for gone in RETIRED_PROMPTS {
            assert!(!p.join(gone).exists(), "{gone} should be retired");
        }
    }

    /// The upgrade path, which the test above cannot see: a fresh temp dir never had the
    /// retired files, so "they aren't there" passed for years on boxes that were keeping
    /// `appearance.md` on disk and reading it as current. Install has to *remove* them.
    #[test]
    fn install_removes_a_retired_prompt_left_by_an_older_build() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("prompts");
        std::fs::create_dir_all(p.join("workers")).unwrap();
        for gone in RETIRED_PROMPTS {
            std::fs::write(p.join(gone), "what an older build installed here").unwrap();
        }

        install_prompts(dir.path()).unwrap();

        for gone in RETIRED_PROMPTS {
            assert!(!p.join(gone).exists(), "{gone} survived the upgrade");
        }
        // The live ones are untouched by the sweep.
        assert!(p.join("reaction.md").exists());
        assert!(p.join("workers/view-builder.md").exists());
        // The craft pages land too, and `view-builder.md` sends the session to this
        // exact path — a page named in a prompt but absent from disk is a dead end the
        // session can only discover the hard way.
        assert!(p.join("craft/data-visualization.md").exists());
        assert!(
            WORKER_VIEW_BUILDER_BASE.contains("prompts/craft/data-visualization.md"),
            "the view builder stopped naming the page install puts on disk"
        );
    }

    /// The prompt names the tools; the runtime spells them `mcp__<server>__<tool>`, and the
    /// model only learns that by looking. A compaction deletes what it looked up, and the
    /// one tool whose absence throws nothing is Reaction — which is exactly how a live
    /// thread went two and a half hours without speaking on 2026-08-13. So the spelling is
    /// in the prompt, and this fails if either half of it moves.
    #[test]
    fn the_reaction_prompt_spells_its_tools_the_way_the_runtime_does() {
        let base = Role::Reaction.base();
        for tool in crate::foundation::mcp::tools_for_role(Some("reaction")) {
            let tool = tool["name"].as_str().expect("every tool is named");
            let spelled = format!("mcp__hi_agent__{tool}");
            assert!(
                base.contains(&spelled),
                "reaction.md must spell `{tool}` as `{spelled}` — the model cannot call a \
                 name it has only been told the short form of"
            );
        }
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

    /// Ten roles — three rungs and seven worker types — in one namespace, no duplicates,
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
    async fn reactions_whole_brief_is_the_file() {
        // The frame used to be a ~40-line Rust literal above `speaking.md`, which put
        // the two things a reader looks for first — that Reaction is one self, and what
        // its tools are — outside the file an operator can override. Both now live in
        // `reaction.md`, so this pins that they are in the prompt rather than the code.
        // Matched on a fragment that does not straddle the file's line wrap.
        assert!(REACTION_BASE.contains("they are talking to you, and only you"));
        assert!(REACTION_BASE.contains("no other \"someone\" who does the work"));
        assert!(REACTION_BASE.contains("`hi_say` is your voice"));
        assert!(REACTION_BASE.contains("`hi_show`"));

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

        let organizer = role_prompt(dir.path(), Role::Worker(WorkerType::DriveOrganizer)).await;
        assert!(
            organizer.contains("hi_copy_file_to_drive"),
            "the organizer must use the host attachment copier"
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
        const VIEW_LAYER: &str = "hi_review_view";
        let builder = role_prompt(dir.path(), Role::Worker(WorkerType::ViewBuilder)).await;
        assert!(builder.contains(VIEW_LAYER));
        assert!(!builder.contains("Report the paths"), "the drive layer must not ride along");

        let organizer = role_prompt(dir.path(), Role::Worker(WorkerType::DriveOrganizer)).await;
        assert!(!organizer.contains(VIEW_LAYER), "the view layer must not ride along");
    }

    /// Durability across a restart is **behaviour, not machinery**: nothing persists a
    /// worker, so a kill mid-job loses whatever lived only in its context. The answer is
    /// the prompt telling it to write as it goes — and telling it *where*, because a note
    /// in `/tmp` is written and unfindable, which is the same as lost (gaps.md #3, #11).
    ///
    /// Pinned on the two rungs that can lose real time (the general worker and the view
    /// builder) plus the decision-maker, whose whole output *is* its report. The reviewer
    /// and the organizer are left out on purpose — one returns a verdict, the other's work is
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
        for t in [WorkerType::ViewReviewer, WorkerType::DriveOrganizer] {
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

    /// **The one string two files must agree on.** The person-reader writes a section
    /// under a fixed heading; `conduct::section` slices on that exact heading and puts
    /// what it finds in front of Reaction on every turn. Nothing at runtime notices a
    /// mismatch — the slice simply finds nothing, the window quietly goes without it,
    /// and the symptom is a preference the person stated that never takes hold. Which is
    /// the exact failure this pair was built to fix, so it is pinned here rather than
    /// left to a live test to discover.
    #[test]
    fn the_reader_writes_the_heading_reaction_is_projected() {
        let heading = crate::mind::memory::conduct::HEADING;
        assert!(
            WORKER_PERSON_READER_BASE.contains(&format!("\n    {heading}\n")),
            "person-reader.md must show the heading verbatim, as a literal block"
        );
        // And it must say the copying is exact — a heading shown as an example that the
        // model then improves on is the same failure as not showing one.
        assert!(WORKER_PERSON_READER_BASE.contains("character for character"));
    }

    /// The reader is pointed at the record, not at its own account of itself. This is
    /// the instruction the whole specialism turns on: the agent's narration of a
    /// mistake is what it believed, and it comes apart from what it did exactly when
    /// it matters. Pin the destinations so a prompt edit cannot quietly drop them.
    #[test]
    fn the_reader_is_sent_to_the_wire() {
        assert!(WORKER_PERSON_READER_BASE.contains("`hi_read_journal_range`"));
        assert!(WORKER_PERSON_READER_BASE.contains("`hi_read_session_log`"));
        assert!(WORKER_PERSON_READER_BASE.contains("{facets_dir}"));
    }

    /// The worker prompt no longer names a tool the worker does not hold. `ask` was
    /// retired with the old channel; what a working session actually has is
    /// `hi_send_message` to its owner, and the instruction that matters is that it never
    /// waits for the answer.
    #[test]
    fn the_worker_is_not_told_about_a_tool_it_does_not_have() {
        for base in [WORKER_GENERAL_BASE, WORKER_VIEW_BUILDER_BASE,
                     WORKER_VIEW_REVIEWER_BASE, WORKER_DECISION_MAKER_BASE,
                     WORKER_DRIVE_ORGANIZER_BASE, WORKER_PERSON_READER_BASE] {
            assert!(!base.contains("`ask`"));
            assert!(!base.contains("`delegate`"));
            assert!(!base.contains("`alarm`"));
        }
        assert!(WORKER_GENERAL_BASE.contains("`hi_send_message`"));
        assert!(WORKER_GENERAL_BASE.contains("Never wait for an answer"));
    }

    /// **Reading is never something to ask for**, and all three rungs that can stall on it
    /// say so. On 2026-08-15 a deploy script failed its own health poll, printed the
    /// `docker compose logs` line to run next, and the run stopped there: the worker
    /// reported "exit 1" without the logs, Cognition asked the person for permission to
    /// look, and the service stayed down through the wait. Nothing gated it — codex runs
    /// these roles unsandboxed with `approvalPolicy: never`. What gated it was a limit the
    /// person had put on an earlier run — *run only the script, no backups, don't touch the
    /// repo* — read as if it covered looking, and then written into the `systems` record as
    /// a standing rule by the reflection pass, where it would have been handed to every
    /// worker that touched that deployment from then on.
    ///
    /// So each half is pinned where it failed: the two rungs that hold the shell are told
    /// that reading changes nothing and needs no yes, and the rung that writes the standing
    /// records is told it may never turn one into a permission gate.
    #[test]
    fn looking_at_a_broken_thing_is_never_gated() {
        assert!(
            COGNITION_BASE.contains("state of something is never gated"),
            "Cognition must be told that reading needs nobody's permission"
        );
        assert!(
            WORKER_GENERAL_BASE.contains("finding out why is part of running it"),
            "a worker must diagnose the failure of what it ran, not report it back bare"
        );
        assert!(
            REFLECTION_BASE.contains("never write a permission gate into a system record"),
            "the pass that writes standing records must not make one out of a one-run limit"
        );
    }

    /// The two halves of the view loop both name the tool that makes them possible.
    /// Before `hi_review_view` existed, the builder's prompt pointed at `hi_look` — which
    /// screenshots the *user's screen*, not the view — and the reviewer had no prompt
    /// at all because it had no way to render. A prompt naming a tool the session does
    /// not hold is the failure this whole pass is cleaning up, so it is pinned.
    #[test]
    fn both_halves_of_the_view_loop_name_the_render_tool() {
        assert!(WORKER_VIEW_REVIEWER_BASE.contains("`hi_review_view`"));
        assert!(WORKER_VIEW_BUILDER_BASE.contains("`hi_review_view`"));
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
    /// `factory/` is pinned with it, and it is the sharper one. Those views sit inside
    /// the workshop the builder is now told to scan, and the binary rewrites them on
    /// every boot ([`crate::mind::views::install_factory_views`]) — so a builder that
    /// adapts one in place loses the work at the next start, silently. Telling it to
    /// read the tree without telling it about that folder is the hazard this pass
    /// introduced; do not drop the warning without removing the instruction to scan.
    #[test]
    fn the_builder_is_told_how_the_toolbox_is_read_and_which_of_it_is_not_its_own() {
        assert!(WORKER_VIEW_BUILDER_BASE.contains("// purpose:"));
        assert!(WORKER_VIEW_BUILDER_BASE.contains("factory/"));
    }

    /// Quick Views are an authoring judgment over ordinary JSX, not a second
    /// representation the host has to interpret. The prompt carries only a concise
    /// direct-import listing for the installed shadcn source.
    #[test]
    fn the_builder_uses_shadcn_directly_without_a_ui_layer() {
        assert!(WORKER_VIEW_BUILDER_BASE.contains("Quick View"));
        assert!(WORKER_VIEW_BUILDER_BASE.contains("ordinary composition"));
        assert!(WORKER_VIEW_BUILDER_BASE.contains("Available shadcn components"));
        assert!(WORKER_VIEW_BUILDER_BASE.contains("@/components/ui/card"));
        assert!(WORKER_VIEW_BUILDER_BASE.contains("Do not create a JSON description"));
        assert!(
            WORKER_VIEW_BUILDER_BASE.contains("same JSX whether the result is quick or custom")
        );
    }

    /// The drive organizer copies rather than moves, and the reason has to travel with the
    /// instruction: `docs/arch/surfaces.md` forbids log-then-copy for streamed bulk, so a
    /// reasonable person reading only that rule would "fix" this into a move — dangling
    /// the journal's own reference to the bytes, for the one class of object where the
    /// bytes are the point.
    #[test]
    fn the_drive_organizer_is_told_why_it_copies() {
        assert!(WORKER_DRIVE_ORGANIZER_BASE.contains("Copy, never move"));
        assert!(WORKER_DRIVE_ORGANIZER_BASE.contains("fade"));
    }

    /// Raw refs are host handles, not paths handed to a model-controlled shell. The two
    /// prompts that act on attachments must name the projected reader/copier and must not
    /// point the model at the canonical raw root.
    #[tokio::test]
    async fn attachment_prompts_use_host_adapters_not_raw_paths() {
        assert!(COGNITION_BASE.contains("`hi_read_text_file`"));
        assert!(WORKER_DRIVE_ORGANIZER_BASE.contains("`hi_copy_file_to_drive`"));
        assert!(!COGNITION_BASE.contains("{raw_dir}"));
        assert!(!WORKER_DRIVE_ORGANIZER_BASE.contains("{raw_dir}"));

        let dir = tempfile::tempdir().unwrap();
        let root = crate::mind::memory::layout::raw_root(&abs(dir.path())).display().to_string();
        for text in [
            cognition_prompt(dir.path()).await,
            role_prompt(dir.path(), Role::Worker(WorkerType::DriveOrganizer)).await,
        ] {
            assert!(!text.contains(&root), "the canonical raw root reached a model prompt");
        }
    }

    /// The drive is **every agent's to read and write** now
    /// (`docs/arch/agents.md#drive-organizer`), and the drive organizer is who they ask when
    /// *where* is the hard part rather than a gate in front of the disk. That only works if
    /// the prompts saying so name where the drive is, and if the name survives substitution:
    /// an unexpanded `{drive_dir}` is a path to nothing, and the symptom is a rung reporting
    /// the filing cabinet empty rather than reporting it could not find one.
    #[tokio::test]
    async fn the_prompts_that_send_a_rung_to_the_drive_name_where_it_is() {
        let dir = tempfile::tempdir().unwrap();
        install_prompts(dir.path()).unwrap();
        let drive = abs(dir.path()).join("drive").display().to_string();
        for text in [
            cognition_prompt(dir.path()).await,
            role_prompt(dir.path(), Role::Worker(WorkerType::General)).await,
            role_prompt(dir.path(), Role::Worker(WorkerType::DriveOrganizer)).await,
        ] {
            assert!(!text.contains("{drive_dir}"), "an unresolved placeholder reached the rung");
            assert!(text.contains(&drive), "the substituted drive must be the absolute drive dir");
        }
    }

    /// **The other end of `a_hand_down_from_reaction_is_always_answered`, and it moved
    /// here from the host.** Cognition is told to always answer a hand-down; this is the
    /// half that makes the answer reach the person, and for one release it was a
    /// host-held `owed` flag that wrote a must-relay sentence into the window. The flag
    /// is gone (`docs/arch/agents.md#the-hand-down`): the host knows only that *a*
    /// hand-down went out, never whether *this* message answers it, while Reaction holds
    /// the request in session and the message in front of it. So the rule is prompt
    /// guidance — which means nothing enforces it but this test, and what it guards is
    /// that the three parts stay present: mail is *identified* (Reaction was never told
    /// what a `(from session …)` line is), an awaited answer is *passed on*, and an
    /// unsolicited one stays Reaction's call. Drop any one and the deletion becomes a
    /// regression.
    #[test]
    fn reaction_is_told_what_mail_is_and_which_of_it_must_be_passed_on() {
        assert!(
            REACTION_BASE.contains("(from session"),
            "Reaction must be able to recognize mail in its window at all"
        );
        assert!(
            REACTION_BASE.contains("If it answers something they asked"),
            "the awaited half — this is what the host's owed flag used to say"
        );
        assert!(
            REACTION_BASE.contains("If nobody asked for it"),
            "and the other half, or Reaction narrates every background finding"
        );
        assert!(
            REACTION_BASE.contains("Count what came in"),
            "one message carries several things and nothing keeps score (gaps.md layer 3)"
        );
        // And the same rule stated where the pressure against it lives: the brevity
        // section ("match the detail", "count the messages too") is what was pushing
        // three answers into two when #29 happened. Saying it only up by the mail
        // section leaves the two passages arguing, and brevity wins an argument like
        // that every time.
        assert!(
            REACTION_BASE.contains("never the number of them"),
            "brevity must be scoped to the depth of an answer, not the count of them"
        );
    }

    #[test]
    fn reaction_has_exactly_one_timer_and_is_told_its_name() {
        // The retired clock stays retired: `alarm` was a general scheduler and its
        // vocabulary must not creep back (`docs/arch/host.md#glancing-up`).
        assert!(!REACTION_BASE.contains("set an alarm"));
        assert!(!REACTION_BASE.contains("When the alarm fires"));

        // What replaced "you have no timer" is one deadline Reaction arms itself, on
        // the utterance that makes the promise. The brief has to name the parameter,
        // because a promise the model never arms is the failure this was built for —
        // the person filling the silence by asking "progress?".
        assert!(REACTION_BASE.contains("back_in"), "the brief must name the parameter");
        assert!(
            REACTION_BASE.contains("only timer you have"),
            "and must not leave Reaction thinking it has more than one"
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
    /// leaves the rung is `hi_send_message`, and everywhere else in its prompt silence is a
    /// legitimate outcome it is explicitly trusted to choose. That trust is correct for a
    /// glance-up and catastrophic for a hand-down, where a person is sitting in front of
    /// Reaction waiting. `cognition::turn` carries a host-side backstop for it; this
    /// asserts the prompt asks for the right thing in the first place, because a backstop
    /// that fires every turn means the guidance is not working.
    #[test]
    fn a_hand_down_from_reaction_is_always_answered() {
        assert!(
            COGNITION_BASE.contains("A hand-down from Reaction is always answered"),
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
    /// writer at all and Reaction walks into every turn blank.
    #[test]
    fn cognition_carries_what_deliberation_was_for() {
        assert!(
            COGNITION_BASE.contains("A ref is an opaque handle")
                && COGNITION_BASE.contains("`hi_read_text_file`"),
            "Cognition must route private refs through a host adapter"
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

    /// **The pen has two ends, and both must be somebody's.** An instruction that says how to
    /// open a task and not how to close one produces a list that grows and never shrinks —
    /// nine tasks stayed `open` across a week, three of them delivered or called off out loud,
    /// and the closing decision got handed to the person as buttons on a screen they never
    /// pressed.
    ///
    /// Closing has since moved off Cognition and onto the task manager, which makes this
    /// invariant *more* fragile rather than less: the hole reopens if either end goes quiet —
    /// Cognition never starting a manager, or the manager not being told closing is its job.
    /// So both ends are asserted here, and the handoff between them is named in the one place
    /// it can be forgotten.
    #[test]
    fn both_ends_of_the_pen_have_an_owner() {
        assert!(
            COGNITION_BASE.contains("Opening is yours. Changing a row is not."),
            "the rung that opens must be told in as many words that it does not close"
        );
        assert!(
            COGNITION_BASE.contains("task-manager"),
            "and must be told what does, by the type name it has to pass"
        );
        assert!(
            WORKER_TASK_MANAGER_BASE.contains("Closing is the job"),
            "the rung that closes must be told closing is the whole point of it"
        );
        assert!(
            WORKER_TASK_MANAGER_BASE.contains("The task owes the ask, not the wait"),
            "a task whose last step is theirs must not sit open as a reminder for them"
        );
    }

    /// A rung with no mouth must not be handed the words for one. Cognition proposes and
    /// Reaction speaks; a role layer that said "tell them" would have it try to speak
    /// through a sink that carries no sequencer, and blame the tool.
    #[test]
    fn cognition_is_not_told_to_speak() {
        assert!(COGNITION_BASE.contains("You do not speak"));
        assert!(
            !COGNITION_BASE.contains("`hi_say`") && !COGNITION_BASE.contains("`hi_show`"),
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

        let expected = crate::mind::memory::layout::reaction_seed_path(dir.path());
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


    /// The installed file **is** the base, byte for byte — there is no layer between them.
    /// A `*.local.md` left over from the build that had one is inert, not an override, and
    /// this is what says so.
    #[test]
    fn install_writes_the_base_verbatim_and_ignores_a_leftover_local_file() {
        let dir = tempfile::tempdir().unwrap();
        let prompts = dir.path().join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        std::fs::write(prompts.join("reaction.local.md"), "Always end with 好的。").unwrap();
        install_prompts(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(prompts.join("reaction.md")).unwrap(), REACTION_BASE);
    }

    #[tokio::test]
    async fn reflection_prompt_falls_back_to_the_embedded_base() {
        // Fallback no longer means "the embedded base, byte for byte" — every rung prompt
        // is interpolated on the way out, so the invariant worth pinning is that the
        // *content* is there and the placeholders are resolved, installed or not.
        let dir = tempfile::tempdir().unwrap();
        let bare = reflection_prompt(dir.path()).await;
        assert!(bare.contains("tends your own house"), "the embedded base must still serve");
        assert!(!bare.contains("{skills_dir}"), "even the fallback interpolates");

        // And the installed file serves the same content once boot has written it.
        install_prompts(dir.path()).unwrap();
        assert!(reflection_prompt(dir.path()).await.contains("tends your own house"));
    }

    /// **The failure this pins actually happened.** Asked to remember an API key, Reaction
    /// answered that it would keep it in a secure credential store and record only the
    /// name — and there is no such store: [`crate::foundation::credentials`] holds our own
    /// vendor keys, keyed `(mode, feature)` and written from Settings, and no tool writes
    /// one. So nothing was saved, while the key itself landed verbatim in the raw log, both
    /// session transcripts and the runtime's rollout. The exact inversion of the promise.
    ///
    /// `docs/arch/privacy.md` settles it: one ordinary `drive/accounts/secrets/*.txt` file
    /// is both the local storage and the stable reference tools and commands use. What the
    /// prompts have to carry is the part no rail enforces — that there is no vault to
    /// claim — so the voice, the brain, and the drive organizer must all name the same file.
    #[test]
    fn every_key_prompt_names_the_ordinary_drive_secret_file() {
        assert!(
            REACTION_BASE.contains("drive/accounts/secrets/openai-api-key.txt")
                && REACTION_BASE.contains("contains only the exact credential"),
            "Reaction does the promising, so Reaction must describe the implemented file"
        );
        assert!(
            COGNITION_BASE.contains("drive/accounts/secrets/openai-api-key.txt")
                && COGNITION_BASE.contains("ordinary text file"),
            "Cognition must carry the same file contract"
        );
        assert!(
            WORKER_DRIVE_ORGANIZER_BASE.contains("ordinary text file")
                && WORKER_DRIVE_ORGANIZER_BASE.contains("stable reference"),
            "the organizer must preserve the managed file"
        );
    }

    /// **The prompts may not promise a choice the host cannot honour.** The one-time
    /// *this / all / none* retention question is a documented target
    /// (`docs/arch/data.md#keys-passwords-and-the-one-question`) and nothing implements it:
    /// the projector files every detected secret automatically. Until that flow exists, a
    /// prompt that describes the question teaches the agent to claim an answer was applied,
    /// which is the same inversion as promising a vault that isn't there — so both the
    /// voice and the brain have to say plainly that retention is automatic.
    #[test]
    fn prompts_are_honest_about_current_auto_retention() {
        assert!(
            REACTION_BASE.contains("retains detected secrets automatically")
                && REACTION_BASE.contains("not implemented"),
            "Reaction is in the room, so Reaction must not imply the choice was offered"
        );
        assert!(
            COGNITION_BASE.contains("retained automatically")
                && COGNITION_BASE.contains("not implemented"),
            "Cognition dispatches the filing, so Cognition must hold the same line"
        );
    }

    /// The model keeps the operational context that makes a file reference useful, while
    /// the drive organizer preserves the stable path.
    #[test]
    fn a_secret_file_carries_what_it_opens_without_printing_the_value() {
        assert!(
            COGNITION_BASE.contains("service or endpoint")
                && COGNITION_BASE.contains("calling convention"),
            "Cognition must remember enough metadata to use the reference"
        );
        assert!(
            WORKER_DRIVE_ORGANIZER_BASE.contains("Do not move, rename, duplicate, print")
                && WORKER_DRIVE_ORGANIZER_BASE.contains("break the stable reference"),
            "the organizer must preserve the path and avoid printing the value"
        );
    }
}
