//! Snapshot — the state projected into the reaction session, and the
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
//! - **It is rebuilt every turn**, not once at session open. A window that is only
//!   correct when the session rotates is stale for the rest of the conversation — a
//!   task opened mid-thread, or a memory written a minute ago, would simply not
//!   be there. This is the one that everything else here exists to serve.
//! - **Rebuilt is not re-sent.** Each block carries a [`Cadence`], and the caller sends
//!   the ones the thread does not already have. Re-sending a block identical to the one
//!   three turns up buys nothing and costs a permanent copy in a finite window — which is
//!   how one thread came to be 80% its own preamble (`docs/arch/data.md`).
//! - **The bound is code's**, never the agent's: [`CARRIED_FORWARD_CHARS`], and over it
//!   the text says so. A ceiling that shows up as text is real; one that shows up as
//!   latency is not.
//! - **The floor is the log tail** ([`build`]). An agent that never got round
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
use crate::types::{Channel, JournalEntry};

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

/// How often a block of the window is worth sending **again**.
///
/// Measured, not guessed. Over 108 turns of one live thread: `## Working with them`
/// changed 10 times, the proactivity read 4, the reachable roster 0 — and all three were
/// sent 108 times, 5,848 characters apiece per turn, to deliver 14 changes. The thread
/// ended up 80% re-sent state against 20% everything the agent had ever done or said, and
/// when the window filled, codex's compaction kept ten copies of the standing preamble and
/// dropped every tool call in the history — including every example of Reaction speaking.
/// Nothing here was unnecessary *once*. All of it was unnecessary *again*.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cadence {
    /// Send when it differs from what this thread was last told. The thread can still see
    /// the last copy, so re-sending an identical block buys nothing and costs a permanent
    /// place in the history.
    OnChange,
    /// Send only when the thread **cannot** see its own history: its first turn, or the
    /// turn after a compaction replaced what it had. For a block that is a retelling of
    /// events already in the thread, that is the only moment it carries anything.
    ColdOnly,
}

/// One titled block of Reaction's window.
pub struct Block {
    /// Stable identity across turns — what "has this changed?" is asked about. Never
    /// shown to the model.
    pub key: &'static str,
    pub cadence: Cadence,
    pub text: String,
    /// What "changed" is judged on, when that is not the text itself.
    ///
    /// A block can carry something that moves on its own — an elapsed time — and re-sending
    /// a whole section because a clock ticked is the repetition this design exists to stop.
    /// The block is still *sent* verbatim; only the comparison uses this.
    pub compare_as: Option<String>,
}

impl Block {
    fn new(key: &'static str, cadence: Cadence, text: String) -> Self {
        Self { key, cadence, text, compare_as: None }
    }

    /// Judge change on `form` rather than on the text.
    fn compared_as(mut self, form: String) -> Self {
        self.compare_as = Some(form);
        self
    }
}

/// Everything the reaction must know without reading, as titled blocks the caller emits
/// on each block's own cadence.
///
/// In order: how to be with the people in front of it
/// ([`conduct::projection`](crate::mind::memory::conduct::projection)), what the conversation
/// carries forward (the generated prompt, capped), what the
/// agent owes ([`tasks::projection`]), the learned read on speaking up unprompted, what it
/// may reach, and the recent-signals tail — the tail last, so it sits against the turn's
/// new signals and reads as one continuous thread.
///
/// **Conduct is first and the tail is last, and that is the same decision twice.** What
/// stands is read as framing; what just happened is read against the turn. A manner that
/// arrives after three thousand characters of situation is a manner that gets skimmed.
///
/// **This is built every turn and mostly not sent.** Building it is cheap — small reads
/// and one directory scan — and it has to happen anyway to know whether anything moved.
/// What used to happen anyway was *sending* it.
///
/// Nothing here can fail the turn. Each source resolves to `""` on absence or error
/// and drops out of the join, and the tail says so in words rather than pretending
/// nothing happened.
pub async fn window(
    memory: &Memory,
    id: &crate::foundation::registry::SessionId,
) -> Vec<Block> {
    let data_dir = memory.data_dir();
    // First, because it is the standing one: everything after it is the situation, and
    // this is the manner the situation is met in.
    let conduct = crate::mind::memory::conduct::projection(data_dir).await;
    let carried = carried_forward(&layout::reaction_seed_path(data_dir)).await;
    let (owed, owed_compared) =
        match tasks::projection_and_comparable(data_dir, &working_on_tasks()).await {
            Ok(pair) => pair,
            Err(err) => {
                tracing::warn!(error = %format!("{err:#}"), "active tasks unreadable; window goes without them");
                (String::new(), String::new())
            }
        };
    let words = words_earned(data_dir).await;
    // The two facts that used to ride the system prompt. Both are *state* — one about this
    // install, one about a setting that can change at any moment — and `baseInstructions` is
    // fixed at `thread/start`, so a language changed mid-conversation could not reach a
    // thread already open. Here they move like everything else that moves.
    let meeting = crate::identity::first_meeting_block(data_dir);
    let language = crate::identity::language_block(data_dir);
    // Who this rung may reach, by id — see [`agent_window`]. For Reaction that is
    // the brain and nothing else: work goes up.
    let reach = crate::foundation::registry::render_reachable(
        &crate::foundation::registry::global().reachable(id),
    );
    // A retelling of signals that are already in the thread above it, which is why it is
    // the one block that only earns its place on a context that cannot look up.
    let tail = recent_tail(memory).await;
    vec![
        Block::new("conduct", Cadence::OnChange, conduct),
        Block::new("carried", Cadence::OnChange, carried),
        // The one block with a clock in it: `last confirmed alive 1h ago` becomes `2h ago`
        // without anything having happened, and the ledger is worth re-reading only when
        // something did.
        Block::new("tasks", Cadence::OnChange, owed).compared_as(owed_compared),
        Block::new("words", Cadence::OnChange, words),
        // **Neither of these can be withdrawn once sent, and neither needs to be.** The cue
        // is true only until this pair has any history at all and can only be true when a
        // thread opens, so it lands on the cold turn of the very first thread and is simply
        // absent from every one after. A language line is replaced by the next one rather
        // than retracted.
        Block::new("meeting", Cadence::OnChange, meeting),
        Block::new("language", Cadence::OnChange, language),
        Block::new("reach", Cadence::OnChange, reach),
        Block::new("recent", Cadence::ColdOnly, tail),
    ]
}

/// The window for an agent that is not Reaction — what it must know without going to
/// look.
///
/// The Reaction-shaped [`window`] cannot serve one: four of its five sections are the
/// conversation's (its brief, its recent tail, the proactivity read that only
/// Reaction can act on). What survives without a conversation is what belongs to the whole
/// agent — the open-task ledger, and whatever this agent has written down for itself.
///
/// **Projected, not retrieved**, which is the whole reason this exists rather than a
/// prompt line saying "read the tasks folder". Cognition is the ledger's writer, so a
/// version of it that goes looking is a version that can miss a duty and never know —
/// and invariant 4 exists because a missed duty is a silently broken promise. Reaction
/// is projected-to because it is tools-off; Cognition is projected-to because it is the
/// one that must not be wrong about this.
///
/// `agent` is a code-supplied name ([`layout::rung_seed_path`]), never a user string.
pub async fn agent_window(
    memory: &Memory,
    agent: &str,
    id: &crate::foundation::registry::SessionId,
) -> String {
    let data_dir = memory.data_dir();
    let carried = carried_forward(&layout::rung_seed_path(data_dir, agent)).await;
    let owed = match tasks::projection(data_dir, &working_on_tasks()).await {
        Ok(text) => text,
        Err(err) => {
            tracing::warn!(error = %format!("{err:#}"), "active tasks unreadable; window goes without them");
            String::new()
        }
    };
    // Who it can reach, by id. This is projection for the same reason the ledger is:
    // an address that has to be guessed is an address that can be wrong in a way the
    // guesser cannot detect. Durable work is recovered from the ledger, and this is
    // where a live session to say it through becomes visible — or visibly does not.
    let reach = crate::foundation::registry::render_reachable(
        &crate::foundation::registry::global().reachable(id),
    );
    let shown = shown_recently(memory).await;
    join(&[carried.as_str(), owed.as_str(), shown.as_str(), reach.as_str()])
}

/// How far back [`shown_recently`] looks. Long enough to cover a piece of work finishing
/// and being handed over across a few turns; short enough that it is a list of what just
/// happened rather than a history to read.
const SHOWN_WINDOW_MIN: i64 = 90;

/// What has actually been on the person's screen — the one fact about its own work that
/// Cognition cannot find out any other way.
///
/// **It has no eyes and no confirmation.** It sends Reaction a message; its own prompt
/// says everything it sends is a proposal, never a delivery; nothing comes back. So when it
/// decides a piece of work is finished it is deciding on a belief it has no way to check,
/// and the failure that follows is not carelessness — it is a rung reasoning correctly from
/// the only information it has. On 2026-08-18 a finished trip view was closed as delivered
/// forty-six seconds after the worker reported it and fifty-six seconds *before* Reaction
/// was told the view existed; Reaction then dropped it from a three-item message, and
/// nothing in the system disagreed with anything, because nothing in the system knew.
///
/// **Information, not a rail.** The alternative was a `deliverable:` field on the task plus
/// a refusal to let it close until the host had seen that ref go out — enforcement resting
/// on the agent remembering to fill in the field that enforces it, whose failure mode is
/// silence and therefore indistinguishable from success. This says what happened and leaves
/// the judgment where it was. A rung that reads *"you have not shown them this"* and closes
/// the task anyway has made a decision; the old one had not.
///
/// Read from the journal rather than the appearance state because the journal is the
/// durable record and is already memory's to read — and because the appearance keeps only
/// what is reachable now, which is a different question from what they have been shown.
pub async fn shown_recently(memory: &Memory) -> String {
    let since = Utc::now() - Duration::minutes(SHOWN_WINDOW_MIN);
    let entries = match memory.journal.recent(since, RECENT_ENTRY_LIMIT).await {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!(error = %format!("{err:#}"), "view log unreadable; window goes without it");
            return String::new();
        }
    };
    let mut seen: Vec<String> = Vec::new();
    for entry in &entries {
        // `SignalOut` only, and now that means something: the view channel has an inbound
        // half (the person going to a view), and this section is about what the *agent*
        // put in front of them. Where they went of their own accord is Reaction's
        // context to hold — `## On screen now` carries it — not a claim about what has
        // been delivered.
        let JournalEntry::SignalOut { channel: Channel::View, body, .. } = entry else {
            continue;
        };
        let Some(name) = shown_name(body) else {
            continue;
        };
        // Newest-last, one entry per destination: a view shown, moved past and shown
        // again is one place they have been, not two.
        seen.retain(|s| s != &name);
        seen.push(name);
    }
    if seen.is_empty() {
        return format!(
            "# On their screen

_Nothing has been put on their screen in the last {SHOWN_WINDOW_MIN} minutes._
"
        );
    }
    let mut out = format!(
        "# On their screen

_What they have actually been shown in the last {SHOWN_WINDOW_MIN} minutes, oldest first. Work they have not seen is work they are still waiting for, whatever its task says._

"
    );
    for name in &seen {
        out.push_str("- ");
        out.push_str(name);
        out.push('\n');
    }
    out
}

/// The durable name out of one journalled view line, or `None` if that line did not put
/// anything up.
///
/// The line is `showed "<id>" [<ref>] (<module>)` — see `render_view_line`. The ref is what
/// this wants, because the ref is what a piece of work is known by everywhere else; the id
/// is whatever Reaction called it in that moment. A dismissal is not a show, and an
/// inline view with no ref is named by its id, which is all it has.
fn shown_name(body: &str) -> Option<String> {
    let rest = body
        .strip_prefix("showed ")
        .or_else(|| body.strip_prefix("replaced "))?;
    let quoted = rest.strip_prefix('"')?;
    let (id, after) = quoted.split_once('"')?;
    match after.trim_start().strip_prefix('[').and_then(|r| r.split_once(']')) {
        Some((view_ref, _)) => Some(view_ref.to_owned()),
        None => Some(id.to_owned()),
    }
}


/// Who is working which task, right now — the task↔worker join, computed fresh.
///
/// **The switchboard is the only source, and nothing is written down.** "Is anyone on this
/// task" is a question about the present, so the answer is derived from what is actually
/// registered at the moment the window is built. That is what makes it impossible for this to
/// be stale: after a restart the switchboard is empty, so every `doing` task reads *nobody on
/// it*, which is exactly true.
///
/// **True is not the whole answer, though, and the gap between them is a whole minute long.**
/// The switchboard being empty right after a boot says nothing about whether the work was
/// abandoned or whether the process simply died holding it, and the second is the common
/// case — Cognition needs an LLM turn to read the boot offer and put people back on things.
/// So the restart's own casualties are laid down first, from the offer, and the live join
/// goes on top: a subject that has been restaffed is live, and one that has not says why it
/// is not ([`tasks::OnIt::CutOff`]).
///
/// The order is also the tie-break, and it is deliberately this way round. Registering a live
/// worker is what drains the cut-off entry, so the two sets are disjoint except during that
/// drain — and a race there should resolve to the live answer, which cannot be a false alarm.
///
/// Keyed by subject, last writer wins. Two live workers on one task is a mistake worth seeing
/// rather than an invariant worth enforcing here — the ledger line will name one of them, and
/// the roster (`GET /api/workers`) shows both.
fn working_on_tasks() -> std::collections::HashMap<String, tasks::OnIt> {
    let registry = crate::foundation::registry::global();
    let mut join: std::collections::HashMap<String, tasks::OnIt> = registry
        .lost_subjects()
        .into_iter()
        .map(|subject| (subject, tasks::OnIt::CutOff))
        .collect();
    join.extend(registry.statuses().into_iter().filter(|st| st.role.is_worker()).filter_map(
        |st| {
            let subject = st.subject.clone()?;
            Some((
                subject,
                tasks::OnIt::Live(tasks::WorkingOnIt {
                    session: st.id,
                    busy: st.busy,
                    doing: st.doing.clone(),
                    // The state clock, not the `doing` clock: what a reader is asking is how
                    // long this session has been in the shape it is in.
                    since: st.state_since,
                    last_turn: st.last_turn.as_ref().map(|end| end.outcome.clone()),
                }),
            ))
        },
    ));
    join
}

/// The learned read on what Reaction's words have earned (`proactivity.md`), projected
/// rather than fetched.
///
/// It is consulted **before anything is said**, and the only rung that says anything is
/// Reaction — which cannot open a file. So a path to it would be a path nobody can
/// follow: this is the projection test (`docs/arch/data.md#what-earns-a-place`) coming
/// out the other way from the tasks ledger. Written whole by the reflection pass
/// ([`crate::mind::memory::proactivity`]); absent until the first reflection, which is
/// ordinary — `reaction.md` says a subject with no line has no record, and no record is
/// not permission.
///
/// **It used to be scoped to breaking a silence, and that was the smaller half.** The
/// heading read `## Speaking up unprompted`, the regeneration trigger fired only on an
/// unprompted word, and the prompt pointed at it only before a guess — so the one
/// artifact in the system that actually *learns* what the agent's words cost was blind to
/// the ones it says with the floor already its own: every reply, every mid-flight line,
/// every hand-back, which is most of what it ever says. A read on speaking that skips
/// replies is a read on speaking in name only.
async fn words_earned(data_dir: &Path) -> String {
    match crate::mind::memory::proactivity::read(data_dir).await {
        Ok(Some(body)) if !body.trim().is_empty() => {
            format!("## What your words have earned\n{}\n", body.trim())
        }
        Ok(_) => String::new(),
        Err(err) => {
            tracing::warn!(error = %format!("{err:#}"), "proactivity read unreadable; window goes without it");
            String::new()
        }
    }
}

/// One generated prompt, read and bounded. Missing, unreadable or blank ⇒ `""`, which
/// the join drops — **absence is ordinary**: the brief is written when there is
/// something worth carrying and not otherwise (`docs/arch/data.md#memoryprompts`). The
/// floor underneath is what makes that survivable.
///
/// Over [`CARRIED_FORWARD_CHARS`] the text is cut and the cut is *announced in the
/// injected text itself*, addressed to the agent whose file it is: it is the only one
/// who can do anything about it, and it can only act on what it can see.
/// Hard cap, in characters, on one record projected into a worker's opening prompt.
///
/// Smaller than [`CARRIED_FORWARD_CHARS`] and per-record rather than per-window, because
/// a worker's brief is the thing it is actually there to read and this rides in front of
/// it. Three thousand is a page: enough for "here is the script, here is what bit us last
/// time", not enough to bury the job.
pub const WORK_RECORD_CHARS: usize = 3_000;

/// The dimension holding what the agent knows about a system it operates — one subject
/// per system, `systems/<name>`.
///
/// **The gap this fills is not storage, it is delivery.** Episodes already recorded that
/// songguo had been deployed before, and a facet could always have been written; what did
/// not exist was any path from that record to the rung with its hands on the machine. A
/// worker is prompted with its brief and nothing else, so *how a thing is operated* could
/// only reach it by being retyped into that brief from someone's memory of a conversation
/// — and a procedure that travels by retyping is a procedure that drifts. The second
/// deploy of a system should start from the first one's record, not from recall.
pub const SYSTEMS: &str = "systems";

/// Frontmatter key on a task naming the systems it touches, comma-separated.
///
/// Deliberately *not* part of [`tasks::Task`]'s schema: it stays an unknown frontmatter
/// line, preserved verbatim by the writer like every other note the agent keeps on a
/// task, and is read here on the way past. A field the status-writer had to understand is
/// a field the status-writer could drop.
const SYSTEMS_KEY: &str = "systems";

/// What the rung actually doing the work must know without going to look: the standing
/// record for the systems this job touches, then the task's own record.
///
/// **Systems first, task second, and that is the same decision Reaction's window makes
/// with conduct.** What stands is read as framing; what is in flight is read as the
/// situation. A procedure that arrives after two pages of ledger is a procedure that gets
/// skimmed — and skimming it is how a canonical script ends up reinvented.
///
/// Empty is the ordinary answer and never an error: a task with no `systems:` line, a
/// system with no record yet, a first-ever job. The worker then gets exactly what it got
/// before this existed, which is its brief.
pub async fn work_record(data_dir: &Path, subject: &str) -> String {
    let facets = layout::facets_dir(data_dir);
    let task_path = facets.join(tasks::DIMENSION).join(subject).join("facet.md");
    let task = tokio::fs::read_to_string(&task_path).await.unwrap_or_default();

    let mut sections: Vec<String> = Vec::new();
    for name in named_systems(&task) {
        let path = facets.join(SYSTEMS).join(&name).join("facet.md");
        let Ok(body) = tokio::fs::read_to_string(&path).await else {
            // Named but not written yet. Say so rather than staying silent: "we have no
            // record of how this is operated" is the sentence that gets one written, and
            // its absence is what let a script be re-derived from scratch.
            sections.push(format!(
                "### {name}\nNo record of this system yet. If you learn how it is \
                 operated, that is worth writing down."
            ));
            continue;
        };
        sections.push(format!("### {name}\n{}", clip(body.trim())));
    }

    let mut out = String::new();
    if !sections.is_empty() {
        out.push_str("## The systems this touches\n");
        out.push_str(&sections.join("\n\n"));
    }
    let task = crate::mind::memory::episodes::strip_frontmatter(&task).trim();
    if !task.is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str("## The record on this task\n");
        out.push_str(&clip(task));
    }
    out
}

/// The `systems:` names on a task, in written order, deduplicated.
///
/// Both spellings the agent actually writes are accepted — `systems: a, b` and
/// `systems: [a, b]` — because the frontmatter here is hand-written prose, not something
/// a serializer produced, and refusing one of two natural spellings would fail silently.
fn named_systems(task: &str) -> Vec<String> {
    let Some(raw) = crate::mind::memory::episodes::frontmatter_field(task, SYSTEMS_KEY) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for name in raw.trim().trim_start_matches('[').trim_end_matches(']').split(',') {
        let name = name.trim().trim_matches('"').trim();
        // A name is a directory component; anything that could climb out of the facet
        // store is not a system name, whatever it is.
        if name.is_empty() || name.contains('/') || name.contains("..") {
            continue;
        }
        if !out.iter().any(|seen| seen == name) {
            out.push(name.to_string());
        }
    }
    out
}

fn clip(body: &str) -> String {
    if body.chars().count() <= WORK_RECORD_CHARS {
        return body.to_string();
    }
    let mut s: String = body.chars().take(WORK_RECORD_CHARS).collect();
    s.push_str("\n[Cut here by the host; the rest is in the facet on disk.]");
    s
}

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

// What a window carries forward is [`carried_forward`] — the generated prompt Cognition
// writes at `memory/prompts/conversation.md` — and nothing beside it.
// Two files used to be injected here as well (`self.md` as "Who I am to this person",
// a recency digest as "Lately on my mind"); their injection went when that writer's job
// was taken up, and their writers are gone too. If the brief turns out thinner in
// practice than what those carried, the fix is to `cognition.md` — the rung that writes
// it — and not a second source of the same thing.


/// The floor: the recent signals, straight off the log. Never empty — an
/// unwritten window is uncurated, not blank — and never fatal: a log that cannot be
/// read says exactly that, rather than rendering `(none)` and claiming a quiet room.
async fn recent_tail(memory: &Memory) -> String {
    match build(memory).await {
        Ok(snap) => snap.render_for_prompt(),
        Err(err) => {
            tracing::warn!(error = %format!("{err:#}"), "recent tail unreadable");
            format!("## Recent (last {RECENT_WINDOW_MIN} minutes)\n(unavailable — I couldn't read the log just now)\n")
        }
    }
}

/// Join the non-empty sections with a blank line between them. Local to this module
/// so `mind` never reaches into `body` for it; the reaction's `join_sections` does the
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
    pub recent_entries: Vec<JournalEntry>,
    pub now: DateTime<Utc>,
}

pub async fn build(memory: &Memory) -> anyhow::Result<Snapshot> {
    let now = Utc::now();
    let since = now - Duration::minutes(RECENT_WINDOW_MIN);
    let recent_entries = memory
        .journal
        .recent(since, RECENT_ENTRY_LIMIT)
        .await?;
    Ok(Snapshot {
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

/// The one thing this projection has to get right is *which name* it reports, because the
/// name is what the rung reading it will compare against its own record of the work.
#[cfg(test)]
mod shown_tests {
    use super::shown_name;

    /// The ref wins, because the ref is what a piece of work is known by everywhere else.
    /// The live instance really did log `showed "agent-learning"` for a view built at
    /// `agent-context-reading/path`, and no reader of that line could have connected them.
    #[test]
    fn a_show_is_named_by_its_ref_not_the_id_of_the_moment() {
        assert_eq!(
            shown_name(r#"showed "agent-learning" [agent-context-reading/path] (/views/_compiled/ab.mjs)"#),
            Some("agent-context-reading/path".to_owned())
        );
        assert_eq!(
            shown_name(r#"replaced "board" [zhao-li-kt-status/board] (/views/_compiled/cd.mjs)"#),
            Some("zhao-li-kt-status/board".to_owned())
        );
    }

    /// An inline view has no ref, and its id is the only name it will ever have.
    #[test]
    fn a_view_with_no_ref_falls_back_to_its_id() {
        assert_eq!(
            shown_name(r#"showed "a-quick-sketch" (/views/_compiled/ef.mjs)"#),
            Some("a-quick-sketch".to_owned())
        );
    }

    /// Clearing the screen is not showing them something, and counting it as one would put
    /// a name on this list that the person never saw.
    #[test]
    fn a_dismissal_is_not_a_show() {
        assert_eq!(shown_name(r#"dismissed "tasks""#), None);
    }

    /// Lines this does not understand are skipped rather than guessed at — every old line
    /// in the journal predates the ref and must not be read as something it is not.
    #[test]
    fn an_unparseable_line_names_nothing() {
        assert_eq!(shown_name("showed something"), None);
        assert_eq!(shown_name(""), None);
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;
    use crate::mind::memory::tasks::{Task, TaskStatus, write_task};

    /// Put one line in the log, so the floor has something to stand on.
    async fn heard(memory: &Memory, body: &str) {
        memory
            .journal
            .append(JournalEntry::SignalIn {
                id: uuid::Uuid::now_v7().to_string(),
                ts: Utc::now(),
                channel: Channel::Text,
                body: body.to_string(),
                stream: None,
                media: None,
                origin: None,
                sender: None,
            })
            .await
            .unwrap();
    }

    async fn write_conversation_prompt(data_dir: &Path, body: &str) {
        let path = layout::reaction_seed_path(data_dir);
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        tokio::fs::write(&path, body).await.unwrap();
    }

    /// Every block the window would build, joined — what a cold turn carries. The live
    /// caller emits each block on its own cadence ([`Cadence`]); a test asking "is it in
    /// the window at all" wants them all.
    async fn whole(memory: &Memory) -> String {
        let blocks = window(memory, &0.into()).await;
        join(&blocks.iter().map(|b| b.text.as_str()).collect::<Vec<_>>())
    }

    /// The absence that is normal today — nothing writes the generated prompts yet —
    /// must still leave a usable window. Uncurated, never empty.
    #[tokio::test]
    async fn a_missing_generated_prompt_still_leaves_the_log_tail() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        heard(&memory, "把周报发我").await;

        // Not merely absent as a file — absent as a whole tree.
        assert!(!layout::seed_dir(dir.path()).exists());

        let text = whole(&memory).await;
        assert!(!text.trim().is_empty());
        assert!(!text.contains("## What I carry forward"), "{text}");
        assert!(text.contains("## Recent (last 30 minutes)"), "{text}");
        assert!(text.contains("把周报发我"), "{text}");

        // A blank file is the same as no file, not an empty section header.
        write_conversation_prompt(dir.path(), "   \n\t\n").await;
        let text = whole(&memory).await;
        assert!(!text.contains("## What I carry forward"), "{text}");
        assert!(text.contains("把周报发我"), "{text}");
    }

    /// Even with nothing at all on disk the floor holds: a header and `(none)`, not
    /// an empty string the join would drop.
    #[tokio::test]
    async fn an_empty_store_still_yields_a_window() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        let text = whole(&memory).await;
        assert!(text.contains("## Recent (last 30 minutes)"), "{text}");
        assert!(text.contains("(none)"), "{text}");
    }

    /// The bound is code's, and it announces itself. A ceiling that shows up as text
    /// is real; one that shows up as latency is not.
    #[tokio::test]
    async fn over_the_cap_the_text_is_cut_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();

        // Just under the cap: whole, and no notice.
        write_conversation_prompt(dir.path(), &"a".repeat(CARRIED_FORWARD_CHARS)).await;
        let text = whole(&memory).await;
        assert!(text.contains("## What I carry forward"), "{text}");
        assert!(!text.contains("Cut here by the host"), "{text}");

        // Over it: cut to the cap, and the cut is stated in the injected text.
        // `q` appears nowhere in the headings or the notice, so counting it counts
        // exactly what survived the cut.
        let long = format!("{}TAIL-THAT-MUST-NOT-SURVIVE", "q".repeat(CARRIED_FORWARD_CHARS));
        write_conversation_prompt(dir.path(), &long).await;
        let text = whole(&memory).await;
        assert!(!text.contains("TAIL-THAT-MUST-NOT-SURVIVE"), "the tail rode past the cap");
        assert!(text.contains("Cut here by the host"), "{text}");
        assert!(text.contains(&CARRIED_FORWARD_CHARS.to_string()), "{text}");
        assert_eq!(text.matches('q').count(), CARRIED_FORWARD_CHARS);

        // Characters, not bytes — a CJK prompt clips at the same visible length.
        write_conversation_prompt(dir.path(), &"记".repeat(CARRIED_FORWARD_CHARS * 2)).await;
        let text = whole(&memory).await;
        assert_eq!(text.matches('记').count(), CARRIED_FORWARD_CHARS);
        assert!(text.contains("Cut here by the host"), "{text}");
    }

    /// Projected, not retrieved: the reaction is tools-off, so what it owes has to be
    /// in the window before it says a word. No tool call fetched this.
    #[tokio::test]
    async fn active_tasks_are_in_the_window_with_nothing_fetching_them() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();

        let mut owed = Task::new("Ship the flash cards", TaskStatus::Doing);
        owed.title = "Ship the flash cards".into();
        write_task(dir.path(), &owed).await.unwrap();

        let mut done = Task::new("Renew the domain", TaskStatus::Done);
        done.title = "Renew the domain".into();
        write_task(dir.path(), &done).await.unwrap();

        let text = whole(&memory).await;
        assert!(text.contains("# Active tasks"), "{text}");
        assert!(text.contains("- [doing] Ship the flash cards"), "{text}");
        // Closed ones are history, not window furniture.
        assert!(!text.contains("Renew the domain"), "{text}");
    }

    /// The order the block reads in: what I carry forward, what I owe, then the tail
    /// — the tail last so it sits against the turn's new signals.
    #[tokio::test]
    async fn the_block_reads_in_one_fixed_order() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        write_conversation_prompt(dir.path(), "He is mid-migration and wants terse answers.").await;
        let mut owed = Task::new("Ship the flash cards", TaskStatus::Doing);
        owed.title = "Ship the flash cards".into();
        write_task(dir.path(), &owed).await.unwrap();
        heard(&memory, "还有多久").await;

        let text = whole(&memory).await;
        let at = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("missing {needle}: {text}"));
        assert!(at("## What I carry forward") < at("# Active tasks"));
        assert!(at("# Active tasks") < at("## Recent (last 30 minutes)"));
    }

    /// Three files an older install still has on disk, because retiring them removed
    /// their readers and writers but deliberately did not delete anyone's data:
    /// `commitments.md` (the second duty ledger — a duty now reaches the window as a
    /// task or not at all), `self.md`, and `hot.md`. None may leak back into a window.
    /// Built by literal path on purpose: the layout helpers that named them are gone,
    /// and this test outliving them is the point.
    #[tokio::test]
    async fn leftover_legacy_files_are_never_inlined() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        let mem_dir = dir.path().join("memory");
        tokio::fs::create_dir_all(&mem_dir).await.unwrap();
        for (name, body) in [
            ("commitments.md", "- watch the ops group\n"),
            ("self.md", "They prefer to be called 老板.\n"),
            ("hot.md", "- shipped the drive view\n"),
        ] {
            tokio::fs::write(mem_dir.join(name), body).await.unwrap();
        }
        // A real signal first, so the window has content to leak *into*. Without it an
        // empty window would satisfy every assertion below and the guard would pass by
        // saying nothing.
        heard(&memory, "这周的卡片做完了吗").await;

        let text = whole(&memory).await;
        assert!(text.contains("这周的卡片做完了吗"), "the window is empty, so this proves nothing: {text}");
        for leaked in ["watch the ops group", "老板", "shipped the drive view", "standing commitments"] {
            assert!(!text.contains(leaked), "leaked {leaked}: {text}");
        }
    }

    /// Invariant 4 for the rung that writes the ledger. Cognition going to *look* for
    /// what is owed is a Cognition that can miss a duty and never know it missed one —
    /// so the active tasks arrive whether or not it thought to ask.
    #[tokio::test]
    async fn the_standing_window_projects_the_ledger_and_what_was_carried() {
        use crate::mind::memory::tasks::{Task, TaskStatus, write_task};

        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();

        let mut owed = Task::new("Ship the flash cards", TaskStatus::Doing);
        owed.subject = "flash-cards".into();
        write_task(dir.path(), &owed).await.unwrap();

        let carried = layout::rung_seed_path(dir.path(), "cognition");
        tokio::fs::create_dir_all(carried.parent().unwrap()).await.unwrap();
        tokio::fs::write(&carried, "The ops group restart needs sudo; ask first.")
            .await
            .unwrap();

        let text = agent_window(&memory, "cognition", &0.into()).await;
        assert!(text.contains("Ship the flash cards"), "{text}");
        assert!(text.contains("needs sudo"), "{text}");
    }

    /// Nothing of a *conversation* may leak into a standing agent's window. The Reaction-shaped
    /// sections are four fifths of `window`, and each one would be answering a question
    /// about a room Cognition is not in.
    #[tokio::test]
    async fn the_standing_window_carries_nothing_conversational() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        heard(&memory, "把周报发我").await;
        write_conversation_prompt(dir.path(), "He is mid-migration this week.").await;

        let text = agent_window(&memory, "cognition", &0.into()).await;
        assert!(!text.contains("把周报发我"), "no log tail: {text}");
        assert!(!text.contains("mid-migration"), "no conversation brief: {text}");
    }

    /// An agent that has written nothing down yet carries no brief and no tail — the join
    /// drops empties, and a section that is only a title reads as data that failed to load.
    ///
    /// The ledger is the one exception, and it earns it: **empty is a fact about what is
    /// owed**, not an absent section. Since the window is sent on change, a ledger that
    /// renders to nothing would be skipped rather than sent, and the last duty could close
    /// with Reaction still believing it was owed — nothing else would tell it, because a
    /// task is closed by a file edit and not by a message.
    #[tokio::test]
    async fn an_empty_standing_window_carries_only_that_nothing_is_owed() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        let text = agent_window(&memory, "cognition", &0.into()).await;
        assert!(text.contains("# Active tasks"), "{text}");
        assert!(text.contains("Nothing open right now"), "{text}");
        assert!(!text.contains("## What I carry forward"), "nothing written yet: {text}");
    }

    async fn write_facet(dir: &Path, dimension: &str, subject: &str, body: &str) {
        let path = layout::facets_dir(dir).join(dimension).join(subject);
        tokio::fs::create_dir_all(&path).await.unwrap();
        tokio::fs::write(path.join("facet.md"), body).await.unwrap();
    }

    /// **The line this whole projection exists for.** A worker used to open with its brief
    /// and nothing else, so how a system is operated could only reach it by being retyped
    /// — and the canonical script went unrun while its own container was stopped to satisfy
    /// a precondition invented in the retyping.
    #[tokio::test]
    async fn the_record_for_a_named_system_reaches_the_work() {
        let dir = tempfile::tempdir().unwrap();
        write_facet(
            dir.path(),
            "tasks",
            "deploy-songguo",
            "---\nstatus: doing\nsystems: songguo\n---\nDeploy it.",
        )
        .await;
        write_facet(dir.path(), "systems", "songguo", "Deployed by `./deploy.sh`. Nothing else.")
            .await;

        let text = work_record(dir.path(), "deploy-songguo").await;
        assert!(text.contains("./deploy.sh"), "the script must reach the doer: {text}");
        assert!(text.contains("Deploy it."), "so must the task's own record: {text}");
        assert!(
            text.find("./deploy.sh") < text.find("Deploy it."),
            "what stands is read before the situation: {text}"
        );
    }

    /// Both spellings the agent actually writes, several systems, and no duplicates. The
    /// frontmatter here is hand-written prose, so refusing one natural spelling would fail
    /// silently — the record would simply not arrive.
    #[tokio::test]
    async fn systems_are_read_in_either_spelling() {
        assert_eq!(
            named_systems("---\nsystems: songguo, hi-agent-xyz\n---\nx"),
            vec!["songguo".to_string(), "hi-agent-xyz".to_string()]
        );
        assert_eq!(
            named_systems("---\nsystems: [songguo, songguo]\n---\nx"),
            vec!["songguo".to_string()],
            "named twice is one system"
        );
        assert!(named_systems("---\nstatus: doing\n---\nx").is_empty());
        assert!(
            named_systems("---\nsystems: ../../etc\n---\nx").is_empty(),
            "a name is a directory component, not a path"
        );
    }

    /// A system named but never written up says so. Silence would read as "nothing to
    /// know", which is the state that gets a procedure re-derived from scratch.
    #[tokio::test]
    async fn a_named_system_with_no_record_says_so() {
        let dir = tempfile::tempdir().unwrap();
        write_facet(dir.path(), "tasks", "deploy-songguo", "---\nsystems: songguo\n---\nGo.").await;

        let text = work_record(dir.path(), "deploy-songguo").await;
        assert!(text.contains("No record of this system yet"), "{text}");
    }

    /// The ordinary case on a fresh install and on any task that names nothing: the worker
    /// gets its brief, exactly as before. An empty projection must not become a heading
    /// with nothing under it.
    #[tokio::test]
    async fn no_named_systems_and_no_task_record_projects_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(work_record(dir.path(), "never-heard-of-it").await.is_empty());
        write_facet(dir.path(), "tasks", "bare", "---\nstatus: doing\n---\n").await;
        assert!(work_record(dir.path(), "bare").await.is_empty());
    }
}
