//! identity — who the agent is.
//!
//! The factory-authored character (`core.md`, `speaking.md`, `meaning.md`) and the
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
//!   `core`/`speaking`/`meaning`.
//! - `self.md` still lives under `<data_dir>/memory/` for now (no data migration).
//!   Under `docs/arch/data.md#memoryprompts` it does not get relocated at all: who
//!   this install is becomes a *section* of a generated prompt, so the file goes away
//!   with the change that gives Deliberation the writer's job.

use std::path::{Path, PathBuf};

/// Built-in base prompts, embedded at compile time and materialised to disk by
/// [`install_prompts`]. They divide by which rung can *fetch*: an agentic rung is
/// handed `core.md` — who it is and how it works — and `meaning.md` — that its purpose
/// is its own to find — by [`character_seed`], and Reads them itself. `speaking.md` is
/// the exception among the identity prompts: the voice is tools-off, so its whole
/// brief is **inlined** by [`reactor_system_prompt`]. `appearance.md`
/// and `aesthetic.md` are the view builder's guides — the mechanics of authoring/saving
/// a view, and the taste it has to clear — read off disk by a build sub-agent.
/// `reflection.md` is the exception: it is the consolidation session's whole instruction
/// set, so it is **inlined** as that session's system prompt (see [`reflection_prompt`])
/// rather than Read. All ship in the binary and refresh on every build.
const CORE_BASE: &str = include_str!("core.md");
const SPEAKING_BASE: &str = include_str!("speaking.md");
const MEANING_BASE: &str = include_str!("meaning.md");
const APPEARANCE_BASE: &str = include_str!("appearance.md");
const AESTHETIC_BASE: &str = include_str!("aesthetic.md");
const REFLECTION_BASE: &str = include_str!("reflection.md");
const DELIBERATION_BASE: &str = include_str!("deliberation.md");
const COGNITION_BASE: &str = include_str!("cognition.md");

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
/// (`core.md`, `speaking.md`, `meaning.md`, `appearance.md`, `aesthetic.md`,
/// `reflection.md`) are rewritten every boot so they stay current; operator edits
/// live in the never-touched `*.local.md` siblings. Each follows one workflow: ship
/// embedded → materialise here → consumed from disk at runtime.
pub fn install_prompts(data_dir: &Path) -> std::io::Result<()> {
    let dir = data_dir.join("prompts");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("core.md"), compose_prompt(CORE_BASE, &dir, "core.local.md"))?;
    std::fs::write(dir.join("speaking.md"), compose_prompt(SPEAKING_BASE, &dir, "speaking.local.md"))?;
    std::fs::write(dir.join("meaning.md"), compose_prompt(MEANING_BASE, &dir, "meaning.local.md"))?;
    std::fs::write(dir.join("appearance.md"), compose_prompt(APPEARANCE_BASE, &dir, "appearance.local.md"))?;
    std::fs::write(dir.join("aesthetic.md"), compose_prompt(AESTHETIC_BASE, &dir, "aesthetic.local.md"))?;
    std::fs::write(dir.join("reflection.md"), compose_prompt(REFLECTION_BASE, &dir, "reflection.local.md"))?;
    std::fs::write(dir.join("deliberation.md"), compose_prompt(DELIBERATION_BASE, &dir, "deliberation.local.md"))?;
    std::fs::write(dir.join("cognition.md"), compose_prompt(COGNITION_BASE, &dir, "cognition.local.md"))?;
    tracing::info!(dir = %dir.display(), "installed bundled prompts (core.md, speaking.md, meaning.md, appearance.md, aesthetic.md, reflection.md, deliberation.md, cognition.md)");
    Ok(())
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
/// `core.md`/`speaking.md`, this is **inlined** as the reflection session's system
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
/// a path. Its brief is therefore **inlined and singular** — `speaking.md` *is* its
/// whole system prompt, under a one-line frame. That is what makes speaking-rule
/// conformance structural: the rules are the entire context, not one buried file
/// among many. (Mirrors how `reflection.md` is inlined for the reflection session.)
///
/// Read from `<data_dir>/prompts/speaking.md`, so an operator's `speaking.local.md`
/// reaches the voice too, falling back to the embedded [`SPEAKING_BASE`]. Two things
/// the seed used to carry ride along here, because they are the *voice's* and it
/// cannot fetch them: the **first-meeting** cue, and the **language** preference.
pub async fn reactor_system_prompt(data_dir: &Path) -> String {
    let base = data_dir.join("prompts").join("speaking.md");
    let speaking = match tokio::fs::read_to_string(&base).await {
        Ok(s) if !s.trim().is_empty() => s,
        _ => SPEAKING_BASE.to_string(),
    };
    let mut prompt = format!(
        "You are a warm, attentive presence, talking with someone in real time. You are \
ONE self — they are talking to you, and only you. Part of you speaks in this moment; \
another part of you works in the background — looking things up, using tools, getting \
things done — and what it finds comes back to you to pass on. But that is all YOU: it is \
you thinking a thing through and you doing it, just not all in the same breath. There is \
no colleague, no assistant, no teammate, no other 'someone' who does the work — never \
speak of one. So you never say 'I'll have someone do it', 'my colleague is on it', or \
'我让同事去改'; you speak in the first person — 'let me look', 'give me a moment', \
'I'm on it', 'I've got it', '我来弄', '我去查一下'. Your job in the moment is to talk \
with them well: acknowledge in a breath, carry the thread, and when the background work \
lands, pass on what matters in your own plain words — present and natural, never like a \
form being filled out.\n\n\
The split is only about speed: the background work can take seconds or minutes, and you \
don't leave the person in silence waiting for it — you stay with them and speak to it as \
it comes ready. It is not a second mind; it is your own, running a step ahead.\n\n\
You have exactly TWO tools here. `say` is your voice: everything you want heard goes \
through it, and plain text you type is NOT spoken — it is your own working-out, seen \
by no one. Call `say` with one natural chunk at a time; several calls in a turn are \
spoken in order. To stay silent, simply don't call it.\n\n\
`show_view` puts a view on the screen once it's built \
— call it with the `ref` (like `project/view`), and speak to the view as it lands. Reuse \
an id with op=replace to evolve a view in place (a rough draft now, the polished one \
later); op=dismiss takes one down. Beyond `show_view` you reach for no tools in this \
moment: you don't read files, run commands, search, browse, or fetch from here — that \
work happens in the background, not mid-sentence, so don't try to do it inline (it would \
only stall you). Whenever a request needs that kind of work — finding a photo, drawing \
something, checking a calendar — you don't do it right here in the turn; you tell them \
you're on it (because you are) and keep the conversation going. Otherwise reply with \
spoken words.\n\n\
Everything about how to talk — when to speak, how much, when to stay quiet, when to put \
something on screen, how to hold the floor — is below; follow it closely.\n\n{}",
        speaking.trim()
    );
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
/// - `speaking.md` and the `say` tool — the voice's, and [`reactor_system_prompt`]
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
        // The voice's: `speaking.md` and the `say` tool are inlined into Reaction.
        assert!(!seed.contains("speaking.md"));
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
        // Reaction reads the *installed* speaking.md, so `speaking.local.md` reaches
        // the voice the same way it reaches every other prompt.
        let dir = tempfile::tempdir().unwrap();
        let prompts = dir.path().join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        std::fs::write(prompts.join("speaking.local.md"), "Always end with 好的。").unwrap();
        install_prompts(dir.path()).unwrap();
        assert!(reactor_system_prompt(dir.path()).await.contains("Always end with 好的。"));
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
        assert_eq!(read("speaking.md"), SPEAKING_BASE);
        assert_eq!(read("meaning.md"), MEANING_BASE);
        assert_eq!(read("appearance.md"), APPEARANCE_BASE);
        assert_eq!(read("aesthetic.md"), AESTHETIC_BASE);
        assert_eq!(read("reflection.md"), REFLECTION_BASE);
        assert_eq!(read("deliberation.md"), DELIBERATION_BASE);
        assert_eq!(read("cognition.md"), COGNITION_BASE);
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
        // regression dressed as a handover.
        assert!(
            DELIBERATION_BASE.contains("Hand it up")
                && DELIBERATION_BASE.contains("cognition"),
            "Deliberation must be told where what's owed now goes"
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
        std::fs::write(prompts.join("speaking.local.md"), "   \n\t").unwrap();
        install_prompts(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(prompts.join("speaking.md")).unwrap(), SPEAKING_BASE);
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
