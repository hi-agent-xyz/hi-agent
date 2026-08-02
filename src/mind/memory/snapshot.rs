//! Snapshot — the per-scene state projected into a scene's reactor session, and the
//! recent-signals tail underneath it.
//!
//! **Projected = what Reaction must know without reading; everything else is recall.**
//! Reaction is tools-off by design, so its window is the entirety of what it knows —
//! it cannot go and look the way an agentic session does via
//! [`crate::identity::character_seed`]. [`window`] is that whole projection, assembled here
//! and handed to the model as text.
//!
//! Three properties of it are code's, and each is a decision (`docs/arch/data.md`):
//!
//! - **Injection is every turn**, not once at session open. A window that is only
//!   correct when the session rotates is stale for the rest of the conversation — a
//!   task opened mid-thread, or a scene memory written a minute ago, would simply not
//!   be there. This is the one that everything else here exists to serve.
//! - **The bound is code's**, never the agent's: [`CARRIED_FORWARD_CHARS`], and over it
//!   the text says so. A ceiling that shows up as text is real; one that shows up as
//!   latency is not.
//! - **The floor is the log tail** ([`build_for_scene`]). An agent that never got round
//!   to writing its memory — busy, crashed, mid-restart — leaves a window that is
//!   uncurated, never empty.
//!
//! Every source is read independently and every absence is ordinary: nothing writes
//! the generated prompts yet, a fresh install owes nothing, an unreflected store has
//! no digest. A missing or unreadable file is skipped, never an error — the window
//! degrades to less context, it does not fail a turn.

use std::path::Path;

use chrono::{DateTime, Duration, Utc};

use crate::mind::memory::{Memory, layout, tasks};
use crate::types::{Channel, JournalEntry, Scene};

pub const RECENT_WINDOW_MIN: i64 = 30;
pub const RECENT_ENTRY_LIMIT: usize = 200;

/// Hard cap, in **characters**, on one generated prompt as injected.
///
/// **Six thousand, and the number is code's.** The agent decides what it carries
/// forward; it does not get to decide how much of the window that costs, because a
/// bound held in judgment is not a bound. Six thousand is roughly two pages — about
/// what a person can actually hold as "what I'm bringing into this conversation", and
/// small enough that it plus the task projection (itself bounded, at ~2.5k) plus the
/// recent tail still leave the fast model's window mostly free for the conversation
/// it is there to have. It rides *every* turn on a latency budget, so the cost is
/// paid over and over; that is what argues for a page or two rather than ten.
///
/// Characters, not bytes: a prompt written in Chinese clips at the same visible
/// length as one written in English, and costs more bytes — which is the trade we
/// want, and the same one [`tasks`] makes on its own lines.
pub const CARRIED_FORWARD_CHARS: usize = 6_000;

/// Everything the scene's reactor must know without reading, in one block, rebuilt
/// **on every turn**.
///
/// In order: what this scene carries forward (the generated prompt, capped), what the
/// agent owes ([`tasks::projection`]), what it may reach,
/// the learned read on speaking up unprompted, and the recent-signals tail — the tail
/// last, so it sits against the turn's new signals and reads as one continuous thread.
///
/// Nothing here can fail the turn. Each source resolves to `""` on absence or error
/// and drops out of the join, and the tail says so in words rather than pretending
/// nothing happened. The cost is one small read per section plus one directory scan
/// of the task dimension — small, but genuinely per-turn now, which is why every read
/// in here has to stay small.
pub async fn window(
    memory: &Memory,
    scene: &Scene,
    id: crate::foundation::registry::SessionId,
) -> String {
    let data_dir = memory.data_dir();
    let carried = carried_forward(&layout::scene_prompt_path(data_dir, scene)).await;
    let owed = match tasks::projection(data_dir).await {
        Ok(text) => text,
        Err(err) => {
            tracing::warn!(error = %err, "open tasks unreadable; window goes without them");
            String::new()
        }
    };
    let unprompted = speaking_up_unprompted(data_dir).await;
    // Who this rung may reach, by id — see [`agent_window`]. For a scene rung that is
    // the shared brain and nothing else: work goes up, and a scene is not somewhere
    // another scene's work belongs.
    let reach = crate::foundation::registry::render_reachable(
        &crate::foundation::registry::global().reachable(id),
    );
    let tail = recent_tail(memory, scene).await;
    join(&[
        carried.as_str(),
        owed.as_str(),
        unprompted.as_str(),
        reach.as_str(),
        tail.as_str(),
    ])
}

/// The window for a **sceneless** agent — what it must know without going to look.
///
/// The scene-shaped [`window`] cannot serve one: four of its five sections are a
/// conversation's (the scene's brief, its recent tail, the proactivity read that only a
/// voice can act on). What survives the loss of a scene is what belongs to the whole
/// agent — the open-task ledger, and whatever this agent has written down for itself.
///
/// **Projected, not retrieved**, which is the whole reason this exists rather than a
/// prompt line saying "read the tasks folder". Cognition is the ledger's writer, so a
/// version of it that goes looking is a version that can miss a duty and never know —
/// and invariant 4 exists because a missed duty is a silently broken promise. Reaction
/// is projected-to because it is tools-off; Cognition is projected-to because it is the
/// one that must not be wrong about this.
///
/// `agent` is a code-supplied name ([`layout::agent_prompt_path`]), never a user string.
pub async fn agent_window(
    memory: &Memory,
    agent: &str,
    id: crate::foundation::registry::SessionId,
) -> String {
    let data_dir = memory.data_dir();
    let carried = carried_forward(&layout::agent_prompt_path(data_dir, agent)).await;
    let owed = match tasks::projection(data_dir).await {
        Ok(text) => text,
        Err(err) => {
            tracing::warn!(error = %err, "open tasks unreadable; window goes without them");
            String::new()
        }
    };
    // Who it can reach, by id. This is projection for the same reason the ledger is:
    // an address that has to be guessed is an address that can be wrong in a way the
    // guesser cannot detect. A task's `report_to` is a durable scene name, and this is
    // where that becomes a live session — or visibly does not, when the scene is cold.
    let reach = crate::foundation::registry::render_reachable(
        &crate::foundation::registry::global().reachable(id),
    );
    join(&[carried.as_str(), owed.as_str(), reach.as_str()])
}

/// The learned read on speaking up unprompted (`proactivity.md`), projected rather
/// than fetched.
///
/// It is consulted **before breaking a silence**, and the only rung that can break one
/// is the voice — which cannot open a file. So a path to it would be a path nobody can
/// follow: this is the projection test (`docs/arch/data.md#what-earns-a-place`) coming
/// out the other way from the tasks ledger. Written whole by the reflection pass
/// ([`crate::mind::memory::proactivity`]); absent until the first reflection, which is
/// ordinary — `reaction.md` says an unproven subject earns no licence anyway.
async fn speaking_up_unprompted(data_dir: &Path) -> String {
    match crate::mind::memory::proactivity::read(data_dir).await {
        Ok(Some(body)) if !body.trim().is_empty() => {
            format!("## Speaking up unprompted\n{}\n", body.trim())
        }
        Ok(_) => String::new(),
        Err(err) => {
            tracing::warn!(error = %err, "proactivity read unreadable; window goes without it");
            String::new()
        }
    }
}

/// One generated prompt, read and bounded. Missing, unreadable or blank ⇒ `""`, which
/// the join drops — **absence is the normal case today**, since nothing writes these
/// files until Deliberation is given the job
/// (`docs/arch/data.md#memoryprompts`). The floor underneath is what makes that
/// survivable.
///
/// Over [`CARRIED_FORWARD_CHARS`] the text is cut and the cut is *announced in the
/// injected text itself*, addressed to the agent whose file it is: it is the only one
/// who can do anything about it, and it can only act on what it can see.
async fn carried_forward(path: &Path) -> String {
    use std::fmt::Write as _;

    let Ok(body) = tokio::fs::read_to_string(path).await else {
        return String::new();
    };
    let body = body.trim();
    if body.is_empty() {
        return String::new();
    }
    let mut s = String::from("## What I carry forward\n");
    if body.chars().count() <= CARRIED_FORWARD_CHARS {
        s.push_str(body);
        return s;
    }
    s.extend(body.chars().take(CARRIED_FORWARD_CHARS));
    let _ = write!(
        s,
        "\n\n[Cut here by the host: what you carry forward runs past the \
{CARRIED_FORWARD_CHARS}-character cap, so the rest of it is missing from this window. \
Trim the file down to what you actually need in every turn — whatever doesn't fit, you \
go without.]"
    );
    s
}

// `legacy_working_set` lived here: `self.md` under "Who I am to this person" and
// `hot.md` under "Lately on my mind", read off disk and injected into every scene
// window. **Both are gone**, which its own TODO said would happen with the change that
// gave Deliberation the writer's job — and Deliberation now writes
// `memory/prompts/scenes/<id>.md`, which is what [`carried_forward`] projects at the top
// of this window.
//
// They were held back because removing them *before* anything wrote the generated
// prompts would have stripped the window to the log tail. That condition is met, and the
// two are the wrong shape besides: under `docs/arch/data.md#memoryprompts` who this
// install is is a **section of a generated prompt**, not a file beside it, and `hot.md`
// is a mechanical digest of recent gists — a digest is not a working memory, and it
// competed with the brief a rung actually authored for itself.
//
// The writers are untouched: reflection still refreshes `hot.md`, and `self.md` is still
// authored. What changed is that neither is injected into a window any more. If the scene
// brief turns out to be thinner in practice than what these carried, that is a fix to
// `deliberation.md` — the rung that writes it — and not a reason to re-add a second
// source of the same thing.


/// The floor: the scene's recent signals, straight off the log. Never empty — an
/// unwritten window is uncurated, not blank — and never fatal: a log that cannot be
/// read says exactly that, rather than rendering `(none)` and claiming a quiet room.
async fn recent_tail(memory: &Memory, scene: &Scene) -> String {
    match build_for_scene(memory, scene).await {
        Ok(snap) => snap.render_for_prompt(),
        Err(err) => {
            tracing::warn!(scene = %scene, error = %err, "recent tail unreadable");
            format!("## Recent (last {RECENT_WINDOW_MIN} minutes)\n(unavailable — I couldn't read the log just now)\n")
        }
    }
}

/// Join the non-empty sections with a blank line between them. Local to this module
/// so `mind` never reaches into `body` for it; the reactor's `join_sections` does the
/// same for the turn's delta sections.
fn join(sections: &[&str]) -> String {
    sections
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub scene: Scene,
    pub recent_entries: Vec<JournalEntry>,
    pub now: DateTime<Utc>,
}

pub async fn build_for_scene(memory: &Memory, scene: &Scene) -> anyhow::Result<Snapshot> {
    let now = Utc::now();
    let since = now - Duration::minutes(RECENT_WINDOW_MIN);
    let recent_entries = memory
        .journal
        .recent(Some(scene), since, RECENT_ENTRY_LIMIT)
        .await?;
    Ok(Snapshot {
        scene: scene.clone(),
        recent_entries,
        now,
    })
}

impl Snapshot {
    pub fn render_for_prompt(&self) -> String {
        use std::fmt::Write as _;
        let mut s = String::new();
        let _ = writeln!(s, "## Recent (last {} minutes)", RECENT_WINDOW_MIN);
        if self.recent_entries.is_empty() {
            s.push_str("(none)\n");
        } else {
            for e in &self.recent_entries {
                let _ = writeln!(s, "{}", render_entry(e));
            }
        }
        s
    }
}

fn render_entry(e: &JournalEntry) -> String {
    match e {
        JournalEntry::SignalIn { channel, body, stream, .. } => {
            transcript_line(Speaker::Them, &channel.with_stream(stream.as_deref()), &truncate(body, 200))
        }
        JournalEntry::SignalOut { channel, body, .. } => {
            transcript_line(Speaker::You, channel.as_str(), &truncate(body, 200))
        }
    }
}

/// Who said a line. Rendered as a single leading glyph — `>` for the person,
/// `<` for the agent — so the speaker costs one character, not a repeated word.
/// The glyphs are documented once in the soul (the system prompt), not per line.
#[derive(Clone, Copy)]
pub(crate) enum Speaker {
    /// The person — an inbound signal. Renders as `>`.
    Them,
    /// The agent itself — an outbound signal. Renders as `<`.
    You,
}

/// Format one transcript line for a prompt: `>body` (or `</chan body` off the
/// default text channel). No timestamp — within a 30-minute window the wall
/// clock rarely carries meaning, and the glyph + ordering is the whole signal.
/// The channel is shown only when it isn't text, so an ordinary text exchange
/// reads as a bare back-and-forth. This is the single place the line shape is
/// defined; both the `## Recent` snapshot and the `## New signals` batch render
/// through it, so they stay identical.
pub(crate) fn transcript_line(who: Speaker, chan: &str, body: &str) -> String {
    let mark = match who {
        Speaker::Them => '>',
        Speaker::You => '<',
    };
    if chan == Channel::Text.as_str() {
        format!("{mark}{body}")
    } else {
        format!("{mark}/{chan} {body}")
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{}\u{2026}", truncated)
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;
    use crate::mind::memory::tasks::{Task, TaskKind, write_task};

    fn scene() -> Scene {
        Scene("boss".into())
    }

    /// Put one line in the scene's log, so the floor has something to stand on.
    async fn heard(memory: &Memory, scene: &Scene, body: &str) {
        memory
            .journal
            .append(JournalEntry::SignalIn {
                id: uuid::Uuid::now_v7().to_string(),
                ts: Utc::now(),
                channel: Channel::Text,
                scene: scene.clone(),
                body: body.to_string(),
                stream: None,
                media: None,
                origin: None,
            })
            .await
            .unwrap();
    }

    async fn write_scene_prompt(data_dir: &Path, scene: &Scene, body: &str) {
        let path = layout::scene_prompt_path(data_dir, scene);
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        tokio::fs::write(&path, body).await.unwrap();
    }

    /// The absence that is normal today — nothing writes the generated prompts yet —
    /// must still leave a usable window. Uncurated, never empty.
    #[tokio::test]
    async fn a_missing_generated_prompt_still_leaves_the_log_tail() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        let scene = scene();
        heard(&memory, &scene, "把周报发我").await;

        // Not merely absent as a file — absent as a whole tree.
        assert!(!layout::generated_prompts_dir(dir.path()).exists());

        let text = window(&memory, &scene, 0).await;
        assert!(!text.trim().is_empty());
        assert!(!text.contains("## What I carry forward"), "{text}");
        assert!(text.contains("## Recent (last 30 minutes)"), "{text}");
        assert!(text.contains("把周报发我"), "{text}");

        // A blank file is the same as no file, not an empty section header.
        write_scene_prompt(dir.path(), &scene, "   \n\t\n").await;
        let text = window(&memory, &scene, 0).await;
        assert!(!text.contains("## What I carry forward"), "{text}");
        assert!(text.contains("把周报发我"), "{text}");
    }

    /// Even with nothing at all on disk the floor holds: a header and `(none)`, not
    /// an empty string the join would drop.
    #[tokio::test]
    async fn an_empty_store_still_yields_a_window() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        let text = window(&memory, &scene(), 0).await;
        assert!(text.contains("## Recent (last 30 minutes)"), "{text}");
        assert!(text.contains("(none)"), "{text}");
    }

    /// The bound is code's, and it announces itself. A ceiling that shows up as text
    /// is real; one that shows up as latency is not.
    #[tokio::test]
    async fn over_the_cap_the_text_is_cut_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        let scene = scene();

        // Just under the cap: whole, and no notice.
        write_scene_prompt(dir.path(), &scene, &"a".repeat(CARRIED_FORWARD_CHARS)).await;
        let text = window(&memory, &scene, 0).await;
        assert!(text.contains("## What I carry forward"), "{text}");
        assert!(!text.contains("Cut here by the host"), "{text}");

        // Over it: cut to the cap, and the cut is stated in the injected text.
        // `q` appears nowhere in the headings or the notice, so counting it counts
        // exactly what survived the cut.
        let long = format!("{}TAIL-THAT-MUST-NOT-SURVIVE", "q".repeat(CARRIED_FORWARD_CHARS));
        write_scene_prompt(dir.path(), &scene, &long).await;
        let text = window(&memory, &scene, 0).await;
        assert!(!text.contains("TAIL-THAT-MUST-NOT-SURVIVE"), "the tail rode past the cap");
        assert!(text.contains("Cut here by the host"), "{text}");
        assert!(text.contains(&CARRIED_FORWARD_CHARS.to_string()), "{text}");
        assert_eq!(text.matches('q').count(), CARRIED_FORWARD_CHARS);

        // Characters, not bytes — a CJK prompt clips at the same visible length.
        write_scene_prompt(dir.path(), &scene, &"记".repeat(CARRIED_FORWARD_CHARS * 2)).await;
        let text = window(&memory, &scene, 0).await;
        assert_eq!(text.matches('记').count(), CARRIED_FORWARD_CHARS);
        assert!(text.contains("Cut here by the host"), "{text}");
    }

    /// Projected, not retrieved: the reactor is tools-off, so what it owes has to be
    /// in the window before it says a word. No tool call fetched this.
    #[tokio::test]
    async fn open_tasks_are_in_the_window_with_nothing_fetching_them() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        let scene = scene();

        let mut owed = Task::new("Ship the flash cards", TaskKind::Wip);
        owed.title = "Ship the flash cards".into();
        write_task(dir.path(), &owed).await.unwrap();

        let mut done = Task::new("Renew the domain", TaskKind::Deadline);
        done.title = "Renew the domain".into();
        done.state = crate::mind::memory::tasks::TaskState::Done;
        write_task(dir.path(), &done).await.unwrap();

        let text = window(&memory, &scene, 0).await;
        assert!(text.contains("# Open tasks"), "{text}");
        assert!(text.contains("- [wip] Ship the flash cards"), "{text}");
        // Closed ones are history, not window furniture.
        assert!(!text.contains("Renew the domain"), "{text}");
    }

    /// The order the block reads in: what I carry forward, what I owe, then the tail
    /// — the tail last so it sits against the turn's new signals.
    #[tokio::test]
    async fn the_block_reads_in_one_fixed_order() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        let scene = scene();
        write_scene_prompt(dir.path(), &scene, "He is mid-migration and wants terse answers.").await;
        let mut owed = Task::new("Ship the flash cards", TaskKind::Wip);
        owed.title = "Ship the flash cards".into();
        write_task(dir.path(), &owed).await.unwrap();
        heard(&memory, &scene, "还有多久").await;

        let text = window(&memory, &scene, 0).await;
        let at = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("missing {needle}: {text}"));
        assert!(at("## What I carry forward") < at("# Open tasks"));
        assert!(at("# Open tasks") < at("## Recent (last 30 minutes)"));
    }

    /// The one ledger. `commitments.md` was the second one, and it is no longer
    /// inlined — a duty reaches the window as a task or not at all.
    #[tokio::test]
    async fn the_old_commitments_file_is_not_inlined_any_more() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        let commitments = crate::identity::commitments_path(dir.path());
        tokio::fs::create_dir_all(commitments.parent().unwrap()).await.unwrap();
        tokio::fs::write(&commitments, "- watch the ops group\n").await.unwrap();

        let text = window(&memory, &scene(), 0).await;
        assert!(!text.contains("watch the ops group"), "{text}");
        assert!(!text.contains("standing commitments"), "{text}");
    }

    /// Invariant 4 for the rung that writes the ledger. Cognition going to *look* for
    /// what is owed is a Cognition that can miss a duty and never know it missed one —
    /// so the open tasks arrive whether or not it thought to ask.
    #[tokio::test]
    async fn the_sceneless_window_projects_the_ledger_and_what_was_carried() {
        use crate::mind::memory::tasks::{Task, TaskKind, write_task};

        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();

        let mut owed = Task::new("Ship the flash cards", TaskKind::Wip);
        owed.subject = "flash-cards".into();
        write_task(dir.path(), &owed).await.unwrap();

        let carried = layout::agent_prompt_path(dir.path(), "cognition");
        tokio::fs::create_dir_all(carried.parent().unwrap()).await.unwrap();
        tokio::fs::write(&carried, "The ops group restart needs sudo; ask first.")
            .await
            .unwrap();

        let text = agent_window(&memory, "cognition", 0).await;
        assert!(text.contains("Ship the flash cards"), "{text}");
        assert!(text.contains("needs sudo"), "{text}");
    }

    /// Nothing of a *conversation* may leak into a sceneless window. The scene-shaped
    /// sections are four fifths of `window`, and each one would be answering a question
    /// about a room Cognition is not in.
    #[tokio::test]
    async fn the_sceneless_window_carries_nothing_scene_shaped() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        let scene = scene();
        heard(&memory, &scene, "把周报发我").await;
        write_scene_prompt(dir.path(), &scene, "He is mid-migration this week.").await;

        let text = agent_window(&memory, "cognition", 0).await;
        assert!(!text.contains("把周报发我"), "no scene's log tail: {text}");
        assert!(!text.contains("mid-migration"), "no scene's brief: {text}");
    }

    /// An agent that has written nothing down yet, and owes nothing yet, gets an empty
    /// window rather than a header with nothing under it — the join drops empties, and a
    /// section that is only a title reads as data that failed to load.
    #[tokio::test]
    async fn an_empty_sceneless_window_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        assert!(agent_window(&memory, "cognition", 0).await.trim().is_empty());
    }
}
