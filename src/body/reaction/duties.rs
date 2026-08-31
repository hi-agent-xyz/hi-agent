//! The duty inbox — where a listener's traffic lands, and the handler it wakes.
//!
//! A `serving` task is kept up by machinery the agent provisioned itself: a detached
//! process holding a WebSocket or a long-poll, appending what arrives to its own ledger
//! under `drive/`. Until this module existed the only way that traffic reached the agent
//! was [`super::cognition`]'s glance-up reading the ledger on its cadence — so a message
//! arrived in real time and was noticed up to a pulse later.
//!
//! This is the other half: the listener says *something arrived*, and a working session
//! handles it. Three things make that safe to have.
//!
//! 1. **The nudge is not the truth.** A delivery carries what arrived, but the listener's
//!    own append-only ledger is the record, and `verify:` still reads that ledger on the
//!    cadence. A nudge lost to a restart, a closed handler, or an energy pause degrades
//!    to exactly the behaviour that existed before this module — the glance finds the
//!    unhandled rows. So this path is an **optimisation over the polling path, never a
//!    replacement for it**, and nothing here needs to be reliable for a duty to be kept.
//! 2. **The ledger authorises.** A delivery names a `start_key`, and a key no `serving`
//!    task claims is dropped. The route is not a door for making working sessions; it is
//!    a door for reaching one the ledger already says should exist.
//! 3. **Cognition is not in the path.** A duty worker takes its traffic straight from
//!    here, and its terminal report is dropped rather than routed (see [`run`]). What
//!    reaches Cognition is what the worker chooses to send it — a decision it cannot
//!    make, something the person has to answer — over `send_message`, the same verb any
//!    worker uses to reach its owner. Routine traffic wakes no rung but the handler.
//!
//! ## Why the handler may die at any moment
//!
//! The handler is a **cache, not the carrier**. What persists is the ledger entry and the
//! listener's rows; the session is re-derivable from both. So the binding from key to
//! session is held here, in memory, and is never written down — and a `post` that comes
//! back [`Delivery::Unknown`] (closed by its owner, or gone with the process) is not
//! an error but the signal to open a fresh one from the facet. That is also why a duty
//! worker is an ordinary [`WorkerType::General`] session with no new flag on it: nothing
//! about it needs to survive, so nothing about it needs to be special.
//!
//! It does bring [`super::workers`]'s warm-idle path a client again. That path lost its
//! only caller when Deliberation was retired — a worker made by `create_worker` runs once
//! and ends — and it is exactly right for a group conversation: a burst continues in one
//! session with full context, and a message the next morning opens a fresh one from the
//! ledger.
//!
//! ## The two clocks
//!
//! **Settle** ([`super::RESPONSE_SETTLE`], shared with the reaction loop deliberately —
//! one commit-after-quiet, one value to tune) collects a burst into one turn. Six lines
//! pasted into a group are one thing to react to, not six.
//!
//! **Floor** ([`DUTY_FLOOR`]) is the cost ceiling. A settle alone is unbounded above: an
//! LLM turn per arrival is affordable for a person typing and is not affordable for a busy
//! group, and the 402 gate is a cliff rather than a brake.
//!
//! [`DUTY_MAX_WAIT`] is what keeps the settle from starving the handler outright. A
//! trickle just faster than the settle would push the deadline forward forever, and a
//! watch that never handles anything because it is always about to is worse than a slow
//! one. The cap is a latency bound; the floor outranks it, because spending money is the
//! failure that cannot be undone by waiting.

use std::collections::HashMap;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};

use crate::foundation::registry::{self, Delivery};
use crate::identity::{Role, WorkerType};
use crate::mind::memory::tasks::{self, Task, TaskStatus};

use super::{LOOP_QUEUE_CAPACITY, LoopInput, Reaction, workers};

/// One thing a listener handed in, addressed to the duty that owns it.
///
/// `key` is a [`tasks::Liveness::start_key`] — the one durable name a duty and its
/// machinery share. Session ids are minted per boot and nothing durable may hold one, so
/// the key is what an external process can be told once and keep using.
#[derive(Debug, Clone)]
pub struct DutyDelivery {
    pub key: String,
    pub text: String,
}

/// Never dispatch a key more often than this. See the module doc — this is the cost
/// ceiling, and it outranks [`DUTY_MAX_WAIT`].
const DUTY_FLOOR: Duration = Duration::from_secs(30);

/// However busy the key, dispatch what has accumulated within this of its first arrival.
const DUTY_MAX_WAIT: Duration = Duration::from_secs(5);

/// Cap on how much one dispatch carries into a prompt. A burst is collected, not
/// concatenated without limit: past this the handler is told what it is missing and the
/// listener's ledger — which has all of it — is where it reads the rest.
const DISPATCH_CHARS: usize = 8_000;

/// Caps on how much of the duty's own record rides in a cold open's brief — the account
/// above the heading, and the running record under it.
///
/// **The facet is the brief, and for a duty that has been kept a while the facet is the
/// largest thing in the process.** One live record reached 375 KB — a 138 KB account and
/// 525 timeline lines — and every cold open pasted all of it above an arrival that was
/// clipped at [`DISPATCH_CHARS`]. The token bill is the smaller half of that. The larger
/// half is that **a handler's example of how to write a line is the last few hundred it
/// was shown**, so a record that has drifted into machine noise teaches every session
/// that opens it to write the same again, and the drift sustains itself. What the handler
/// needs is why the row exists and what has happened lately; the rest is a file it can
/// open, and [`Task::recent_record`] tells it how much it is not being shown.
const BRIEF_ACCOUNT_CHARS: usize = 6_000;
const BRIEF_RECORD_CHARS: usize = 6_000;

/// What has accumulated for one key since its last dispatch.
struct Pending {
    lines: Vec<String>,
    /// When this key becomes dispatchable.
    due: Instant,
    /// First arrival in this batch, for the [`DUTY_MAX_WAIT`] cap.
    first: Instant,
}

pub(super) fn spawn(reaction: Reaction, rx: mpsc::Receiver<DutyDelivery>) {
    tokio::spawn(async move {
        run(reaction, rx).await;
    });
}

async fn run(reaction: Reaction, mut rx: mpsc::Receiver<DutyDelivery>) {
    // **Duty workers report into a drain.** Every other working session answers whoever
    // asked, because someone asked; nobody asked for this one. Its owner is Cognition so
    // that what it *chooses* to raise has somewhere to land, but its per-message terminal
    // report is not news — routing it would wake the shared brain on every message the
    // group receives, which is the exact cost this path exists to avoid. Dropping it here
    // is that decision expressed as a channel rather than as a flag threaded through
    // `spawn_inner`.
    let (report_tx, mut report_rx) = mpsc::channel::<LoopInput>(LOOP_QUEUE_CAPACITY);
    tokio::spawn(async move {
        while let Some(input) = report_rx.recv().await {
            if let LoopInput::Worker(report) = input {
                tracing::info!(
                    worker = %report.id,
                    "duty handler finished a turn; not escalated"
                );
            }
        }
    });

    let mut workers = workers::WorkerRegistry::new(report_tx);
    // key → the session currently handling it. In memory only, and re-derived from the
    // ledger whenever it turns out to be wrong.
    let mut bound: HashMap<String, registry::SessionSlug> = HashMap::new();
    let mut pending: HashMap<String, Pending> = HashMap::new();
    let mut last_dispatch: HashMap<String, Instant> = HashMap::new();

    tracing::info!("duty inbox up");

    loop {
        let next_due = pending.values().map(|p| p.due).min();
        tokio::select! {
            biased;
            _ = reaction.inner.shutdown.cancelled() => {
                tracing::info!("duty inbox shutting down");
                break;
            }
            got = rx.recv() => match got {
                Some(delivery) => accept(&mut pending, &last_dispatch, delivery),
                // The HTTP front is gone; so is anything that could deliver.
                None => break,
            },
            _ = sleep_until_opt(next_due) => {
                let now = Instant::now();
                let ready: Vec<String> = pending
                    .iter()
                    .filter(|(_, p)| p.due <= now)
                    .map(|(key, _)| key.clone())
                    .collect();
                for key in ready {
                    let Some(batch) = pending.remove(&key) else { continue };
                    last_dispatch.insert(key.clone(), now);
                    dispatch(&reaction, &mut workers, &mut bound, &key, batch).await;
                }
                workers.reap();
            }
        }
    }
}

/// Buffer one arrival and work out when its key becomes dispatchable.
///
/// Split out from the loop because the three clocks interacting is the part worth being
/// able to read in one place — and worth being able to test without a `Reaction`.
fn accept(
    pending: &mut HashMap<String, Pending>,
    last_dispatch: &HashMap<String, Instant>,
    delivery: DutyDelivery,
) {
    let now = Instant::now();
    let floor_until = last_dispatch.get(&delivery.key).map(|at| *at + DUTY_FLOOR);
    let entry = pending.entry(delivery.key).or_insert(Pending {
        lines: Vec::new(),
        due: now,
        first: now,
    });
    entry.lines.push(delivery.text);
    entry.due = due_at(now, entry.first, floor_until);
}

/// The pure clock arithmetic: settle from the last arrival, capped so a trickle cannot
/// starve the key, and floored so cost cannot run away. **Floor last** — it is the only
/// one of the three that outranks the others.
fn due_at(now: Instant, first: Instant, floor_until: Option<Instant>) -> Instant {
    let settled = now + super::RESPONSE_SETTLE;
    let capped = settled.min(first + DUTY_MAX_WAIT);
    match floor_until {
        Some(floor) if floor > capped => floor,
        _ => capped,
    }
}

async fn sleep_until_opt(at: Option<Instant>) {
    match at {
        Some(at) => sleep_until(at).await,
        None => std::future::pending::<()>().await,
    }
}

/// Hand one key's accumulated traffic to its handler, opening one if there is none.
async fn dispatch(
    reaction: &Reaction,
    workers: &mut workers::WorkerRegistry,
    bound: &mut HashMap<String, registry::SessionSlug>,
    key: &str,
    batch: Pending,
) {
    let arrived = clip(
        &batch.lines.join("\n"),
        DISPATCH_CHARS,
        "the rest is in this duty's own ledger",
    );
    let count = batch.lines.len();

    // A live handler takes it straight. No rung wakes, nothing is projected, and the
    // 700ms that just elapsed is the whole latency between the group and the turn.
    if let Some(id) = bound.get(key).cloned() {
        match registry::global().post(&id, arrived.clone()) {
            Delivery::Delivered => {
                tracing::info!(worker = %id, arrivals = count, "duty traffic delivered");
                return;
            }
            // Closed by its owner, or lost with a restart. Not an error — the
            // handler was always a cache, and the ledger is about to rebuild it.
            _ => {
                bound.remove(key);
            }
        }
    }

    let Some(task) = duty_for(reaction, key).await else {
        // Either the key names nothing, or its task is no longer `serving` — a duty that
        // was stood down while its listener kept running. Both are the same answer, and
        // the listener finding out is a matter for whoever stood the duty down.
        tracing::warn!(arrivals = count, "duty delivery no serving task claims; dropped");
        return;
    };

    // A session may already hold this duty without this inbox having opened it, and
    // `bound` cannot know: it remembers only handlers minted here. Ask the roster before
    // minting — the destructive case, not the wasteful one, is two sessions under one
    // subject writing over each other in one task folder.
    if let Some(id) = handler_on(&registry::global().statuses(), &task.subject) {
        if let Delivery::Delivered = registry::global().post(&id, arrived.clone()) {
            bound.insert(key.to_owned(), id.clone());
            tracing::info!(
                worker = %id,
                arrivals = count,
                duty = %task.subject,
                "duty traffic delivered to the session already holding it"
            );
            return;
        }
    }

    // Owner is Cognition: what the handler *chooses* to raise needs somewhere to land,
    // and the rung that can amend the ledger and reach Reaction is the only sound
    // answer. `None` would leave an escalation addressed to nobody.
    let owner = registry::global()
        .session_of_role(Role::Cognition)
        .map(|status| status.id);
    let id = registry::mint(Role::Worker(WorkerType::General), Some(&task.subject));
    let brief = brief_for(&task, &arrived);

    match workers
        // Never resumed. A duty handler is re-derived from the ledger, which is exactly what
        // makes it free to die: the facet is the brief, so a cold session knows everything the
        // dead one did. See this module's doc.
        // The session's line is the duty's own title from the ledger — the name the person
        // gave this standing job — while `brief` stays the full record the handler reads.
        .spawn_with_id(
            reaction,
            id.clone(),
            task.title.clone(),
            Some(brief),
            WorkerType::General,
            owner,
            None,
            Some(task.subject.clone()),
            // A duty handler answers something that has already arrived, so it is never
            // work run ahead of the asking, however far ahead the duty itself was set up.
            false,
        )
        .await
    {
        Ok(_) => {
            bound.insert(key.to_owned(), id.clone());
            tracing::info!(
                worker = %id,
                arrivals = count,
                duty = %task.subject,
                "opened a duty handler from the ledger"
            );
        }
        Err(err) => {
            // The traffic is lost from this path, and that is survivable by design: the
            // listener's rows are still on disk and the glance-up still reads them.
            tracing::warn!(error = %format!("{err:#}"), duty = %task.subject, "could not open a duty handler");
        }
    }
}

/// The live worker already on `subject`, if the roster says there is one.
///
/// **Two ways a duty is held that this inbox never sees.** A worker Cognition staffed to own
/// the machinery, and a session the host reopened after a restart: neither passes through
/// [`dispatch`], so neither is in `bound`, and an inbox trusting `bound` alone would open a
/// second handler on top of a working one. They would share the task's folder — the failure
/// there is not duplicated effort but one of them writing over the other's file with nothing
/// anywhere saying so.
///
/// The roster is the same join the ledger's *who is on it* line is rendered from
/// ([`crate::mind::memory::tasks::OnIt`]), so this can only ever agree with what the agent is
/// already being shown. Last writer wins there and first match wins here for the same reason:
/// two live workers on one subject is a mistake to see, not an invariant to enforce from a
/// delivery path.
fn handler_on(statuses: &[registry::Status], subject: &str) -> Option<registry::SessionSlug> {
    statuses
        .iter()
        .find(|status| status.role.is_worker() && status.subject.as_deref() == Some(subject))
        .map(|status| status.id.clone())
}

/// The `serving` task that claims `key`, if one does.
///
/// Scanned from the ledger on every cold start rather than cached, because the answer is
/// allowed to change — a duty stood down should stop being reachable, and it should stop
/// without anything having to remember to tell this module.
async fn duty_for(reaction: &Reaction, key: &str) -> Option<Task> {
    let data_dir = reaction.inner.memory.data_dir();
    match tasks::active_tasks(data_dir).await {
        Ok(active) => active.into_iter().find(|task| {
            task.status == TaskStatus::Serving && task.liveness.start_key.as_deref() == Some(key)
        }),
        Err(err) => {
            // Unreadable is not empty — but unlike the glance-up, there is nothing useful
            // to do about it here. Dropping falls back to the cadence, which is the
            // designed behaviour of a lost nudge.
            tracing::warn!(error = %format!("{err:#}"), "duty inbox could not read the ledger");
            None
        }
    }
}

/// What a freshly-opened handler is told.
///
/// **The facet is the brief** — that is what makes the ledger, and not the session, the
/// thing that carries a duty across a restart. Nothing here describes how to handle the
/// traffic, because the duty already does, in the words whoever opened it used.
///
/// Note what is *not* interpolated: the routing key. It arrived over HTTP, and the title
/// and body here are the host's own read of the ledger.
/// The standing tail of a duty handler's opening brief — the part that is instruction
/// rather than the ledger's own record of the duty.
///
/// A constant so the tool layer's prefix sweep can read it; see
/// [`super::heartbeat::PROACTIVITY_HEADING`] for why that sweep exists. This sentence said
/// `send_message` bare, which is the agent runtime's own tool.
pub(crate) const DUTY_BRIEF_TAIL: &str =
    "\n\nHandle it as the duty describes. You have no voice of your own: reach your \
     owner with `hi_send_message` when something needs a decision that is not yours, or \
     when it needs the person. More may arrive while you work — it will reach you here.\n";

fn brief_for(task: &Task, arrived: &str) -> String {
    let mut brief = String::new();
    brief.push_str("You are keeping up a standing duty. This is the ledger's own record of it.\n\n");
    brief.push_str("## ");
    brief.push_str(&task.title);
    brief.push('\n');
    // Both halves, because the running record is half of what the ledger knows about this
    // duty — what was asked for, what has landed, what was checked and when it last moved
    // — and a handler reopened after a restart has nothing else to read. Both capped,
    // because a duty's record has no end: see `BRIEF_ACCOUNT_CHARS`.
    //
    // The account is cut from its head and the record from its tail, and that is not an
    // inconsistency: the prose above the heading carries its newest reading on top and the
    // dated lines under it run oldest first, so each keeps its own current end.
    let account = task.body.trim();
    if !account.is_empty() {
        brief.push_str(&clip(
            account,
            BRIEF_ACCOUNT_CHARS,
            "older readings are in this duty's own `facet.md`",
        ));
        brief.push('\n');
    }
    let record = task.recent_record(BRIEF_RECORD_CHARS);
    if !record.is_empty() {
        brief.push('\n');
        brief.push_str(&record);
        brief.push('\n');
    }
    brief.push_str("\n## Just arrived\n");
    brief.push_str(arrived);
    brief.push_str(DUTY_BRIEF_TAIL);
    brief
}

/// Clip on a char boundary, saying so and saying where the rest is, so a handler can tell
/// a truncated text from a short one and go and read the whole of it.
fn clip(text: &str, max: usize, rest: &str) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}\n[… clipped; {rest}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A burst arriving inside the settle is one dispatch, not one per line.
    #[test]
    fn a_burst_settles_into_one_dispatch() {
        let mut pending = HashMap::new();
        let last = HashMap::new();
        for line in ["one", "two", "three"] {
            accept(
                &mut pending,
                &last,
                DutyDelivery { key: "ops".into(), text: line.into() },
            );
        }
        assert_eq!(pending.len(), 1, "one key, one batch");
        assert_eq!(pending["ops"].lines.len(), 3);
    }

    /// The settle is refreshed by every arrival, so a trickle just faster than it would
    /// push the deadline forward forever. The cap is what stops a watch that is
    /// permanently about to handle something.
    #[test]
    fn a_trickle_cannot_push_the_deadline_forever() {
        let first = Instant::now();
        let late = first + Duration::from_secs(60);
        let due = due_at(late, first, None);
        assert!(
            due <= first + DUTY_MAX_WAIT,
            "an arrival an hour into a burst must not extend it"
        );
    }

    /// The regression this exists for: a duty whose machinery a *staffed* worker owns. That
    /// session never went through `dispatch`, so `bound` is empty for its key, and minting on
    /// that emptiness puts a second worker under one task subject.
    #[test]
    fn a_worker_already_on_the_subject_is_the_handler() {
        let id = registry::mint(Role::Worker(WorkerType::General), Some("watch-the-queue"));
        registry::global().register(
            id.clone(),
            Role::Worker(WorkerType::General),
            None,
            "watch the queue".into(),
            Some("watch-the-queue".into()),
        );
        let statuses = registry::global().statuses();
        assert_eq!(
            handler_on(&statuses, "watch-the-queue").as_ref(),
            Some(&id),
            "the roster knows a session is on this duty"
        );
        assert!(
            handler_on(&statuses, "some-other-duty").is_none(),
            "and does not answer for a subject nobody is on"
        );
        registry::global().unregister(&id);
    }

    /// Cost outranks latency: a key inside its floor waits for the floor even though the
    /// cap says it is overdue.
    #[test]
    fn the_floor_outranks_the_cap() {
        let now = Instant::now();
        let floor = now + Duration::from_secs(20);
        let due = due_at(now, now - Duration::from_secs(60), Some(floor));
        assert_eq!(due, floor);
    }

    /// And a key past its floor is not delayed by it.
    #[test]
    fn a_quiet_key_is_not_held_by_a_stale_floor() {
        let now = Instant::now();
        let floor = now - Duration::from_secs(5);
        let due = due_at(now, now, Some(floor));
        assert_eq!(due, now + super::super::RESPONSE_SETTLE);
    }

    /// The clip says it clipped. A handler that cannot tell a truncated burst from a
    /// short one reads the second as the whole story.
    #[test]
    fn a_clipped_burst_says_so() {
        let long = "x".repeat(DISPATCH_CHARS + 100);
        let clipped = clip(&long, DISPATCH_CHARS, "the rest is in this duty's own ledger");
        assert!(clipped.contains("clipped"), "{clipped}");
        assert!(clipped.contains("this duty's own ledger"), "{clipped}");
        assert!(clip("short", DISPATCH_CHARS, "elsewhere") == "short");
    }

    /// The facet is the brief — that is the whole reason a handler may die freely. If the
    /// body stops reaching the prompt, a restart silently downgrades every duty to a
    /// session that knows only what just arrived.
    #[test]
    fn the_facet_is_the_brief() {
        let mut task = Task::new("Watch the ops group", TaskStatus::Serving);
        task.body = "Reply in thread. File anything about billing to drive/ledgers/.".into();
        let brief = brief_for(&task, "alice: the gateway is 502ing");

        assert!(brief.contains("Watch the ops group"), "{brief}");
        assert!(brief.contains("File anything about billing"), "{brief}");
        assert!(brief.contains("alice: the gateway is 502ing"), "{brief}");
    }

    /// And the brief is capped, because a duty's record has no end. The regression is not
    /// a crash — it is a cold open that pastes hundreds of kilobytes above an arrival
    /// clipped at 8,000 characters, and hands the handler the record's own worst lines as
    /// the example to write the next one by.
    #[test]
    fn a_duty_kept_for_months_still_briefs_in_one_screen() {
        use crate::mind::memory::tasks::{TimelineEntry, TimelineKind};
        let then = chrono::Utc::now();

        let mut task = Task::new("Watch the ops group", TaskStatus::Serving);
        task.body = format!("THE CURRENT READING.\n\n{}", "a superseded reading. ".repeat(2_000));
        task.timeline.push(TimelineEntry::new(
            TimelineKind::Created,
            then,
            "the digest goes to the group, not to me",
        ));
        for line in 0..400 {
            task.timeline.push(TimelineEntry::new(
                TimelineKind::Update,
                then,
                format!("line {line} {}", "x".repeat(300)),
            ));
        }
        assert!(task.record().len() > 100_000, "the record this is protecting against");

        let brief = brief_for(&task, "alice: the gateway is 502ing");

        assert!(brief.len() < 20_000, "the caps hold: {}", brief.len());
        assert!(brief.contains("THE CURRENT READING"), "the account's newest reading");
        assert!(brief.contains("the digest goes to the group"), "why the row exists");
        assert!(brief.contains("line 399"), "the newest of the record");
        assert!(!brief.contains("line 0 "), "and not the oldest");
        assert!(brief.contains("alice: the gateway is 502ing"), "what just arrived");
        assert!(brief.contains("clipped"), "and it says both were cut");
        assert!(brief.contains("earlier lines"), "{brief}");
    }

    /// The routing key arrived over HTTP and never reaches a prompt. What the handler is
    /// told about its own duty is the host's read of the ledger, start to finish.
    #[test]
    fn the_routing_key_is_never_interpolated() {
        let mut task = Task::new("Watch the ops group", TaskStatus::Serving);
        task.liveness.start_key = Some("ops-group-watcher".into());
        let brief = brief_for(&task, "hello");

        assert!(!brief.contains("ops-group-watcher"), "{brief}");
    }
}
