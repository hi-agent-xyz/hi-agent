//! Prepared branches — `hi_prepare`, and what the person's messages do with it.
//!
//! **`hi_say` is the reply to this moment; `hi_prepare` is where a matter may go next.** One
//! call is one matter's choice: a few mutually exclusive branches, each a condition — where the
//! person takes that matter, in Reaction's own words — and the actions Reaction would take
//! there, written exactly as it would call them. When a message of theirs lands, one System One
//! call reads it against every matter prepared: which branch it met, whether it qualified it,
//! and which matters it took up at all. A met branch runs at once, in order, stopping at the
//! first action that does not happen; and the message then drives an ordinary turn, which is
//! told what ran. See `docs/arch/agents.md` § *Prepared branches*.
//!
//! **Why this exists.** Everything Reaction does in answer to a message starts with a
//! generation, and a generation has a floor no prompt moves — 13.8 s to a first line, one
//! upstream request over ~100K tokens, nearly all of it fixed cost
//! (`docs/user-journeys/measuring.md`). The only way under it is to have decided before the
//! message came, and to spend the moment it comes only on recognizing which decision it was.
//! A generation cannot recognize in time and its self-reported confidence compares to
//! nothing; System One answers in about a second with a number that means the same thing on
//! every call, so the cut here is a policy about how often a met branch may be wrong.
//!
//! **A set belongs to a matter, not to the next message.** People step away from a subject and
//! come back to it, and what was ready for it is still ready when they do. So a set waits until
//! a message takes its matter up: met, it runs and is used up; taken somewhere no branch
//! describes, it is used up without running, because the matter has moved past it. A message
//! about something else leaves it where it is. Reports, other turns, lines on other matters and
//! restarts do not touch it — the sets are kept on disk ([`FILE`]) — and every turn's window
//! lists them, so the one thing that can go stale behind a set, what Reaction would now do,
//! is Reaction's to clear or replace. The count is bounded ([`MAX_MATTERS`], [`MAX_BRANCHES`]),
//! oldest out first. None of this is a timer.
//!
//! What keeps it from doing harm:
//!
//! - **a miss runs nothing** and the turn runs as it does today; a timeout, an error or no
//!   System One is a miss, so the worst case is today's speed;
//! - **every action is one of Reaction's own verbs**, none of which reaches outside — words to
//!   the person, a view on their screen, a message to another rung. Anything outward is a
//!   worker's, behind Cognition's judgment, exactly as when the same message is handed down
//!   in a turn;
//! - **a set is read with where its matter was left**: the lines it was prepared after ride
//!   into the reading beside the lines the message answers, so a bare 行 said after something
//!   else was proposed reads as agreeing to that, not to a matter an hour old;
//! - **one switch** — `prepared_branches` = `off` prepares nothing; otherwise a met branch runs.
//!
//! **Built and unit-tested; never watched on a live turn** — journey 43 is the spec, and the
//! events it records (`branches_prepared` / `branches_resolved` / `branches_voided`) are the
//! count to read it by.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::body::capabilities::decision;
use crate::foundation::config::tunables;
use crate::foundation::observatory::{BranchDirection, EventKind, Observatory};
use crate::foundation::registry::{self, Delivery, SessionSlug};
use crate::types::Message;

use super::Reaction;
use super::sequencer::Beat;

/// The `app_settings` key switching it off. Reads unless `off`, like every gate here.
pub(crate) const SWITCH_KEY: &str = "prepared_branches";
/// The `app_settings` key moving the cut.
pub(crate) const THRESHOLD_KEY: &str = "prepared_branches_threshold";

/// Where the sets are kept, under the data dir, so a restart does not lose what was ready.
const FILE: &str = "memory/prepared.json";

/// How much of the choice's mass a branch must carry to run, and how little `qualified` may.
/// A starting value: every reading's whole mass is recorded, so where the cut should sit is
/// read off what the readings actually were, not argued.
const THRESHOLD: f64 = 0.9;

/// How sure the reading must be that a message took a matter up for that matter's set to be
/// used up. Half, not the run cut: a set used up wrongly costs a turn preparing it again, and
/// one kept wrongly is a stale set waiting for a message to mistake.
const TAKEN_UP: f64 = 0.5;

/// How long the reading may take. System One answered the speech check's questions in p50
/// 0.53 s, p99 2.2 s on this install, and the two readings that answered on live messages took
/// 0.70 s and 0.72 s — against a one-second budget the other two ran out of. A reading asked
/// during the settle has most of this already behind it by the time the batch closes. Past it
/// the message takes today's path.
const BUDGET: Duration = Duration::from_secs(2);

/// More directions than this, across every matter, is a fan, not a guess — and System One
/// loses accuracy on a padded question as it does on a padded state.
const MAX_BRANCHES: usize = 8;

/// How many matters are kept ready at once. A few, like what a person holds in mind; the
/// oldest goes first.
const MAX_MATTERS: usize = 4;

/// How long [`Prepared::settled`] waits for a reading before letting the turn run anyway: the
/// longest a batch can be held open, the reading's budget, and a second of slack. A reading
/// that outlives this is abandoned by nobody — it finishes and records — but no turn waits on it.
const SETTLED_WITHIN: Duration = Duration::from_secs(8);

/// The key the one `choice` is asked under; its branch options are `b1`, `b2`, … in order,
/// across every matter.
const WHICH: &str = "which";
/// The option meaning none of the branches.
const REST: &str = "rest";
/// The key the one `noul` about the met branch is asked under.
const QUALIFIED: &str = "qualified";
/// The prefix of the `noul` asked per matter: `on1`, `on2`, … in the sets' order.
const ON: &str = "on";

/// Whether branches are prepared at all, from the switch.
pub fn enabled() -> bool {
    crate::body::legibility::enabled(SWITCH_KEY)
}

/// The cut, from its setting. Below one half it would run a branch the reading leans against.
fn threshold() -> f64 {
    tunables::get(THRESHOLD_KEY)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|p| (0.5..=1.0).contains(p))
        .unwrap_or(THRESHOLD)
}

/// One action in a branch — one of Reaction's own calls, with its arguments.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum Action {
    Say { text: String },
    Show { id: Option<String>, op: String, view_ref: Option<String>, source: String },
    Send { to: SessionSlug, message: String },
}

impl Action {
    /// The action as one line, for the event log, the window, and the turn told what ran.
    fn describe(&self) -> String {
        match self {
            Action::Say { text } => format!("hi_say \"{}\"", clip(text, 120)),
            Action::Show { id, op, view_ref, .. } => format!(
                "hi_show {op} {}",
                view_ref.as_deref().or(id.as_deref()).unwrap_or("an inline view")
            ),
            Action::Send { to, message } => {
                format!("hi_send_message → {to}: \"{}\"", clip(message, 120))
            }
        }
    }
}

/// One direction: where they take the matter, and what to do there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Branch {
    pub condition: String,
    pub actions: Vec<Action>,
}

impl Branch {
    fn direction(&self) -> BranchDirection {
        BranchDirection {
            condition: self.condition.clone(),
            actions: self.actions.iter().map(Action::describe).collect(),
        }
    }
}

/// Read a `hi_prepare` call's arguments into its matter and branches, **checking every action
/// the way its own tool would** — a line within `say_max`, a `ref` that resolves, a `to` that
/// is live — so a branch that could not run is found in the turn that can still fix it, not at
/// the reply. Any failure refuses the whole call, naming where: a choice with one option
/// missing is a different choice. An empty list is valid, and clears the matter.
pub async fn parse(args: &Value, data_dir: &Path, say_max: usize) -> Result<(String, Vec<Branch>), String> {
    let matter = args.get("matter").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    if matter.is_empty() {
        return Err("hi_prepare needs a `matter`: a few words naming what these branches are about, \
                    so they wait for that matter and preparing it again replaces them"
            .into());
    }
    let Some(list) = args.get("branches").and_then(Value::as_array) else {
        return Err("hi_prepare needs `branches`: a list of {condition, actions} — an empty list \
                    clears what is prepared for the matter"
            .into());
    };
    if list.len() > MAX_BRANCHES {
        return Err(format!(
            "{} branches is a fan, not a guess — prepare the one or two directions you actually \
             expect (at most {MAX_BRANCHES})",
            list.len()
        ));
    }
    let mut branches = Vec::with_capacity(list.len());
    for (i, b) in list.iter().enumerate() {
        let n = i + 1;
        let condition = b.get("condition").and_then(Value::as_str).map(str::trim).unwrap_or_default();
        if condition.is_empty() {
            return Err(format!(
                "branch {n} has no `condition` — say, in plain words, where they would take it. \
                 Nothing was prepared."
            ));
        }
        let list = match b.get("actions") {
            Some(Value::Array(list)) if !list.is_empty() => list,
            Some(Value::Array(_)) | None | Some(Value::Null) => {
                return Err(format!(
                    "branch {n} (\"{condition}\") has no actions — a direction with nothing ready \
                     runs nothing when met, so it is no branch; leave it out. Nothing was prepared."
                ));
            }
            Some(_) => {
                return Err(format!("branch {n}: `actions` is a list. Nothing was prepared."));
            }
        };
        let mut actions = Vec::with_capacity(list.len());
        for (j, a) in list.iter().enumerate() {
            match parse_action(a, data_dir, say_max).await {
                Ok(action) => actions.push(action),
                Err(why) => {
                    return Err(format!(
                        "branch {n} (\"{condition}\"), action {}: {why}. Nothing was prepared.",
                        j + 1
                    ));
                }
            }
        }
        branches.push(Branch { condition: condition.to_string(), actions });
    }
    Ok((matter.to_string(), branches))
}

async fn parse_action(a: &Value, data_dir: &Path, say_max: usize) -> Result<Action, String> {
    let arg = |k: &str| {
        a.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
    };
    match arg("tool").as_deref().map(|t| t.trim_start_matches("mcp__hi_agent__")) {
        Some("hi_say") => {
            let text = arg("text").ok_or("hi_say needs a non-empty `text`")?;
            let chars = text.chars().count();
            if chars > say_max {
                return Err(format!("too long for one message ({chars} characters; the most is {say_max})"));
            }
            Ok(Action::Say { text })
        }
        Some("hi_show") => {
            let op = arg("op").unwrap_or_else(|| "show".to_string());
            if !matches!(op.as_str(), "show" | "replace" | "dismiss") {
                return Err(format!("`op` is show, replace or dismiss, not `{op}`"));
            }
            let (id, view_ref, source) = (arg("id"), arg("ref"), arg("source"));
            match (&view_ref, &source) {
                (Some(r), _) => {
                    crate::mind::views::resolve_ref(data_dir, r)
                        .await
                        .map_err(|e| format!("`ref` {r}: {e}"))?;
                }
                (None, Some(_)) => {}
                (None, None) if op == "dismiss" && id.is_some() => {}
                (None, None) => return Err("hi_show needs a `ref` (or an `id` to dismiss)".into()),
            }
            Ok(Action::Show { id, op, view_ref, source: source.unwrap_or_default() })
        }
        Some("hi_send_message") => {
            let to = arg("to").ok_or("hi_send_message needs `to`")?;
            let message = arg("message").ok_or("hi_send_message needs a non-empty `message`")?;
            let to = to
                .parse::<SessionSlug>()
                .map_err(|_| format!("`{to}` is not a session slug — `cognition`, or one your window lists"))?;
            if registry::global().status(&to).is_none() {
                return Err(format!("nothing live at `{to}` to send to"));
            }
            Ok(Action::Send { to, message })
        }
        Some("hi_prepare") => Err("a branch cannot prepare — the turn after it can".into()),
        Some(other) => {
            Err(format!("`{other}` cannot be prepared — an action is hi_say, hi_show or hi_send_message"))
        }
        None => Err("each action names its `tool`: hi_say, hi_show or hi_send_message".into()),
    }
}

/// One matter's set, and where the matter was left when it was prepared.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Set {
    matter: String,
    branches: Vec<Branch>,
    /// What the agent had said since the person last wrote — where the matter was left, and
    /// what a message taking it up answers.
    said: Vec<String>,
    at: DateTime<Utc>,
    /// Which set this is within the run, so a reading removes the sets it read and not ones
    /// prepared again while it ran. Not kept: numbered again on load.
    #[serde(skip)]
    id: u64,
}

/// Where a met branch runs: the mouth's own sequencer and count, and who a message goes as.
/// Attached once, as the conversation's loop stands up.
#[derive(Clone)]
struct Runner {
    beats: mpsc::Sender<Beat>,
    said: Arc<AtomicU64>,
    from: SessionSlug,
}

#[derive(Default)]
struct State {
    /// Oldest first.
    sets: Vec<Set>,
    next_id: u64,
    /// The reading in flight, if any: further messages of the same batch go to it.
    reading: Option<mpsc::UnboundedSender<Message>>,
    /// What the last reading ran, for the turn its message drives.
    ran: Option<Ran>,
    runner: Option<Runner>,
}

impl State {
    fn number(&mut self, mut set: Set) -> Set {
        self.next_id += 1;
        set.id = self.next_id;
        set
    }
}

/// The matters prepared, and the reading of the message that meets them.
pub struct Prepared {
    state: std::sync::Mutex<State>,
    observatory: Observatory,
    /// `None` keeps nothing on disk — tests.
    path: Option<PathBuf>,
    /// `true` while no reading is in flight. The loop waits on it before a turn, so the turn a
    /// message drives always starts after that message's reading — never beside it.
    idle: watch::Sender<bool>,
}

impl Prepared {
    /// Stand up with what was kept under `data_dir`, if anything. A file that does not read is
    /// logged and started over: what was ready is a convenience, never load-bearing.
    pub fn new(observatory: Observatory, data_dir: Option<&Path>) -> Self {
        let (idle, _) = watch::channel(true);
        let path = data_dir.map(|d| d.join(FILE));
        let mut state = State::default();
        if let Some(path) = &path {
            match std::fs::read(path) {
                Ok(bytes) => match serde_json::from_slice::<Vec<Set>>(&bytes) {
                    Ok(sets) => {
                        let numbered: Vec<Set> = sets.into_iter().map(|s| state.number(s)).collect();
                        state.sets = numbered;
                    }
                    Err(err) => tracing::warn!(error = %err, path = %path.display(), "prepared branches: kept file unreadable; starting empty"),
                },
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => tracing::warn!(error = %err, path = %path.display(), "prepared branches: kept file unreadable; starting empty"),
            }
        }
        Self { state: std::sync::Mutex::new(state), observatory, path, idle }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Write the sets as they stand. Called under the lock, so writes land in the order the
    /// changes did; the file is a few kilobytes at most.
    fn keep(&self, sets: &[Set]) {
        let Some(path) = &self.path else { return };
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(sets).unwrap_or_default())?;
            std::fs::rename(&tmp, path)
        };
        if let Err(err) = write() {
            tracing::warn!(error = %err, path = %path.display(), "prepared branches: could not keep the sets");
        }
    }

    /// Where a met branch's actions go. Called once, by the loop standing up.
    pub(super) fn attach(&self, beats: mpsc::Sender<Beat>, said: Arc<AtomicU64>, from: SessionSlug) {
        self.lock().runner = Some(Runner { beats, said, from });
    }

    /// Prepare `matter`: replace whatever was set for it, or clear it when `branches` is empty.
    /// The newest goes last; past [`MAX_MATTERS`] or [`MAX_BRANCHES`], the oldest go. Returns
    /// the matters that went to make room.
    pub(super) async fn set(&self, matter: &str, branches: Vec<Branch>, said: Vec<String>) -> Vec<String> {
        let directions: Vec<BranchDirection> = branches.iter().map(Branch::direction).collect();
        let (cleared, evicted) = {
            let mut st = self.lock();
            let before = st.sets.len();
            st.sets.retain(|s| !same_matter(&s.matter, matter));
            let cleared = st.sets.len() < before;
            let mut evicted = Vec::new();
            if !branches.is_empty() {
                let set = st.number(Set { matter: matter.to_string(), branches, said, at: Utc::now(), id: 0 });
                st.sets.push(set);
                while st.sets.len() > MAX_MATTERS
                    || st.sets.iter().map(|s| s.branches.len()).sum::<usize>() > MAX_BRANCHES
                {
                    evicted.push(st.sets.remove(0).matter);
                }
            }
            self.keep(&st.sets);
            (cleared, evicted)
        };
        if directions.is_empty() {
            if cleared {
                self.observatory
                    .record(EventKind::BranchesVoided { matter: Some(matter.to_string()), reason: "cleared".into() })
                    .await;
            }
        } else {
            self.observatory
                .record(EventKind::BranchesPrepared { matter: matter.to_string(), directions })
                .await;
        }
        for gone in &evicted {
            self.observatory
                .record(EventKind::BranchesVoided {
                    matter: Some(gone.clone()),
                    reason: "the oldest, to make room".into(),
                })
                .await;
        }
        evicted
    }

    /// What is ready, as every turn's window carries it. Absolute times, so the text changes
    /// only when the sets do.
    pub(super) fn render(&self) -> String {
        use std::fmt::Write as _;
        let st = self.lock();
        let mut s = String::from("## What you have ready\n");
        if st.sets.is_empty() {
            s.push_str("(nothing prepared)");
            return s;
        }
        s.push_str(
            "Branches you prepared, matter by matter. Each waits until they take its matter up: \
             met, it runs; taken anywhere else, it is used up. What would no longer be right to \
             do or say, clear (hi_prepare with that matter and no branches) or prepare again.\n",
        );
        for set in &st.sets {
            let _ = writeln!(s, "\n### {} — prepared {}", set.matter, set.at.format("%m-%d %H:%MZ"));
            for b in &set.branches {
                let actions = b.actions.iter().map(Action::describe).collect::<Vec<_>>().join("; ");
                let _ = writeln!(s, "- {} → {actions}", b.condition);
            }
        }
        s
    }

    /// What the last reading ran, for the turn its message drives. Taken once.
    pub(super) fn take_ran(&self) -> Option<Ran> {
        self.lock().ran.take()
    }

    /// Whether a branch has run that no turn has been told about yet — which is also a
    /// reading's verdict that the batch was someone talking with the agent.
    pub(super) fn ran_waiting(&self) -> bool {
        self.lock().ran.is_some()
    }

    /// Wait for a reading in flight, if any — bounded by [`SETTLED_WITHIN`].
    pub(super) async fn settled(&self) {
        let mut idle = self.idle.subscribe();
        let _ = tokio::time::timeout(SETTLED_WITHIN, idle.wait_for(|idle| *idle)).await;
    }

    /// Remove the sets a reading used up, by id: one prepared again while it ran is a
    /// different set and stays.
    fn use_up(&self, ids: &[u64]) {
        if ids.is_empty() {
            return;
        }
        let mut st = self.lock();
        st.sets.retain(|s| !ids.contains(&s.id));
        self.keep(&st.sets);
    }
}

/// Two names for one matter: the same words, spacing and case aside.
fn same_matter(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.split_whitespace().collect::<String>().to_lowercase();
    norm(a) == norm(b)
}

/// What to do about one message from the person, decided under the lock.
enum OnMessage {
    Nothing,
    SwitchedOff(Vec<String>),
    Read(Vec<Set>, mpsc::UnboundedReceiver<Message>),
}

/// A message from the person landed. It joins a reading in flight; otherwise, if anything is
/// prepared, a reading of every matter starts.
///
/// **Called from [`Reaction::deliver`] before the message is queued for the loop**, so the
/// loop's wait for the reading ([`Prepared::settled`]) can never miss one that has begun.
pub(super) async fn on_message(reaction: &Reaction, message: &Message) {
    let prepared = &reaction.inner.prepared;
    let next = {
        let mut st = prepared.lock();
        if let Some(reading) = st.reading.as_ref() {
            // The batch is still open; the reading asks again over the whole of it.
            let _ = reading.send(message.clone());
            OnMessage::Nothing
        } else if st.sets.is_empty() {
            OnMessage::Nothing
        } else if !enabled() {
            let gone = std::mem::take(&mut st.sets).into_iter().map(|s| s.matter).collect();
            prepared.keep(&st.sets);
            OnMessage::SwitchedOff(gone)
        } else {
            let (tx, rx) = mpsc::unbounded_channel();
            st.reading = Some(tx);
            st.ran = None;
            OnMessage::Read(st.sets.clone(), rx)
        }
    };
    match next {
        OnMessage::Nothing => {}
        OnMessage::SwitchedOff(gone) => {
            for matter in gone {
                prepared
                    .observatory
                    .record(EventKind::BranchesVoided {
                        matter: Some(matter),
                        reason: "prepared branches are switched off".into(),
                    })
                    .await;
            }
        }
        OnMessage::Read(sets, rx) => {
            prepared.idle.send_replace(false);
            let recent = reaction.inner.speech.said_since_their_last();
            tokio::spawn(read(reaction.clone(), sets, recent, message.clone(), rx));
        }
    }
}

/// Every branch across the sets, flattened in order: `b1` is `options[0]`, as (set, branch).
fn options(sets: &[Set]) -> Vec<(usize, usize)> {
    sets.iter().enumerate().flat_map(|(k, s)| (0..s.branches.len()).map(move |i| (k, i))).collect()
}

/// Read the person's message against every matter prepared, run the branch it met, use up
/// the matters it took up, and record it all.
///
/// **The question is asked as soon as the message lands, and the answer is used only once the
/// batch has closed** — the same settle a turn waits out, held open while they are still
/// talking or typing, because a second message is exactly what turns 同意。 into 同意。不过…. A
/// message that joins the batch asks again over the whole of it. So on a typed message the
/// reading is mostly hidden inside a window the turn waits through anyway, and a miss costs
/// nothing.
async fn read(
    reaction: Reaction,
    sets: Vec<Set>,
    recent: Vec<String>,
    first: Message,
    mut more: mpsc::UnboundedReceiver<Message>,
) {
    let arrived = Instant::now();
    let floor = reaction.inner.floor.clone();
    let questions = Arc::new(questions(&sets));
    let mut messages = vec![first];
    let mut asking = ask(&sets, &recent, &messages, &questions);
    // **A microphone hears the whole room**, and a line nobody spoke to the agent is not their
    // message: it must neither run a branch — 行，就这样 said across the dinner table — nor use
    // a set up. So a batch that is only room asks the room screen's own question beside this
    // one ([`super::room`]), at every arrival, as the loop does. It cannot wait for the loop's
    // answer: while a turn is running the loop screens nothing, and that is exactly when a
    // spoken reply lands.
    let as_batch = |messages: &[Message]| -> Vec<super::LoopInput> {
        messages.iter().cloned().map(super::LoopInput::Message).collect()
    };
    let mut room = None;
    super::room::ask(&mut room, &reaction.inner.memory, &as_batch(&messages));
    let hold_until = arrived + super::BATCH_WHILE_COMPOSING;
    let mut settle_at = arrived + super::RESPONSE_SETTLE;
    loop {
        tokio::select! {
            next = more.recv() => match next {
                Some(m) => {
                    messages.push(m);
                    if let Some(a) = asking.take() {
                        a.abort();
                    }
                    asking = ask(&sets, &recent, &messages, &questions);
                    super::room::ask(&mut room, &reaction.inner.memory, &as_batch(&messages));
                    settle_at = Instant::now() + super::RESPONSE_SETTLE;
                }
                None => break,
            },
            _ = tokio::time::sleep_until(settle_at) => {
                let now = Instant::now();
                let composing = floor.voice_active(now).await || floor.typing_active(now).await;
                if composing && now < hold_until {
                    settle_at = now + super::RESPONSE_SETTLE;
                    continue;
                }
                break;
            }
        }
    }
    // Side talk: nothing runs and nothing is used up. Anything short of a clear answer from
    // the screen reads as someone talking with the agent, the way the loop reads it — the
    // branch question below still has to be met.
    if !super::room::wakes(&mut room, &reaction.inner.memory, &as_batch(&messages)).await {
        if let Some(a) = asking {
            a.abort();
        }
        reaction
            .inner
            .observatory
            .record(EventKind::BranchesResolved {
                message: render_messages(&messages),
                outcome: "side talk".into(),
                matter: None,
                direction: None,
                p: None,
                qualified: None,
                mass: None,
                model: None,
                decided_ms: arrived.elapsed().as_millis() as u64,
                first_action_ms: None,
                ran: Vec::new(),
                used_up: Vec::new(),
            })
            .await;
        reaction.inner.prepared.lock().reading = None;
        reaction.inner.prepared.idle.send_replace(true);
        return;
    }
    let answer = match asking {
        None => Err(Unread::Unavailable),
        Some(a) => match a.await {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(err)) if format!("{err:#}").contains("timed out") => Err(Unread::Timeout),
            Ok(Err(err)) => Err(Unread::Error(format!("{err:#}"))),
            Err(join) => Err(Unread::Error(join.to_string())),
        },
    };
    let decided_ms = arrived.elapsed().as_millis() as u64;
    let flat = options(&sets);
    let reading = Reading::of(&answer, flat.len(), sets.len(), threshold());
    let met = reading.met.map(|i| flat[i]);
    let mut outcome = reading.outcome.to_string();
    let mut ran: Vec<(String, Result<String, String>)> = Vec::new();
    let mut first_action_ms = None;
    if let Some((k, i)) = met {
        let branch = &sets[k].branches[i];
        let now = Instant::now();
        let runner = reaction.inner.prepared.lock().runner.clone();
        if floor.voice_active(now).await || floor.typing_active(now).await {
            // They are not done. The floor is asked once, for the branch as a whole, before
            // any of it runs — a refusal is not a hold, so the branch is simply not run.
            outcome = "still going".into();
        } else if more.try_recv().is_ok() {
            outcome = "a line landed after the batch".into();
        } else if let Some(runner) = runner {
            first_action_ms = Some(arrived.elapsed().as_millis() as u64);
            ran = run(&reaction, &runner, branch).await;
        } else {
            outcome = "no mouth".into();
        }
    }
    // The matter a met branch belongs to is used up whether or not the branch got to run —
    // the message took it up — and so is every other matter the message took up at all.
    // An unread reply uses nothing up: nothing is known about where the message went.
    let used_up: Vec<&Set> = sets
        .iter()
        .enumerate()
        .filter(|(k, _)| met.is_some_and(|(m, _)| m == *k) || reading.on.get(*k).copied().flatten().is_some_and(|p| p >= TAKEN_UP))
        .map(|(_, s)| s)
        .collect();
    reaction.inner.prepared.use_up(&used_up.iter().map(|s| s.id).collect::<Vec<_>>());
    if let Some(err) = &reading.error {
        tracing::warn!(error = %err, "prepared branches: the reading failed; the message takes the ordinary path");
    }
    tracing::info!(
        outcome = %outcome,
        p = ?reading.p,
        qualified = ?reading.qualified,
        decided_ms,
        ran = ran.len(),
        used_up = used_up.len(),
        kept = sets.len() - used_up.len(),
        "prepared branches: their message read"
    );
    reaction
        .inner
        .observatory
        .record(EventKind::BranchesResolved {
            message: render_messages(&messages),
            outcome,
            matter: met.map(|(k, _)| sets[k].matter.clone()),
            direction: met.map(|(k, i)| sets[k].branches[i].condition.clone()),
            p: reading.p,
            qualified: reading.qualified,
            mass: reading.mass,
            model: reading.model,
            decided_ms,
            first_action_ms,
            ran: ran
                .iter()
                .map(|(action, result)| match result {
                    Ok(done) => format!("{action}: {done}"),
                    Err(why) => format!("{action}: {why}"),
                })
                .collect(),
            used_up: used_up.iter().map(|s| s.matter.clone()).collect(),
        })
        .await;
    {
        let mut st = reaction.inner.prepared.lock();
        st.reading = None;
        st.ran = match (met, ran.is_empty()) {
            (Some((k, i)), false) => Some(Ran {
                matter: sets[k].matter.clone(),
                condition: sets[k].branches[i].condition.clone(),
                ran,
            }),
            _ => None,
        };
    }
    reaction.inner.prepared.idle.send_replace(true);
}

/// Ask the questions about the messages so far, inside the budget. `None` when nothing is
/// configured to ask — then the reading is a miss before it begins.
fn ask(
    sets: &[Set],
    recent: &[String],
    messages: &[Message],
    questions: &Arc<Map<String, Value>>,
) -> Option<JoinHandle<anyhow::Result<decision::Reply>>> {
    if !decision::available() {
        return None;
    }
    let state = Value::String(state(sets, recent, messages));
    let questions = questions.clone();
    Some(tokio::spawn(async move {
        tokio::time::timeout(BUDGET, decision::ask(&state, &questions, None))
            .await
            .map_err(|_| anyhow::anyhow!("timed out"))?
    }))
}

/// Why a reading produced no answer.
enum Unread {
    Unavailable,
    Timeout,
    Error(String),
}

/// A reply read: which branch was met, if any, which matters the message took up, and every
/// number that decided it.
#[derive(Debug, Default, PartialEq)]
struct Reading {
    /// Into the flattened options ([`options`]).
    met: Option<usize>,
    outcome: &'static str,
    p: Option<f64>,
    qualified: Option<f64>,
    /// Per matter, in the sets' order: how sure it is the message took that matter up.
    on: Vec<Option<f64>>,
    mass: Option<Value>,
    model: Option<String>,
    error: Option<String>,
}

impl Reading {
    /// A branch is met when its option carries at least `tau` of the choice's mass and
    /// `qualified` carries at most `1 − tau`. A reply that cannot be read runs nothing and
    /// uses nothing up.
    fn of(answer: &Result<decision::Reply, Unread>, branches: usize, matters: usize, tau: f64) -> Self {
        let reply = match answer {
            Ok(reply) => reply,
            Err(Unread::Unavailable) => return Self { outcome: "unavailable", ..Self::default() },
            Err(Unread::Timeout) => return Self { outcome: "timeout", ..Self::default() },
            Err(Unread::Error(e)) => {
                return Self { outcome: "error", error: Some(e.clone()), ..Self::default() };
            }
        };
        let model = Some(reply.model.clone());
        let noul = |key: &str| match reply.answers.get(key) {
            Some(decision::Answer::Noul { p }) => Some(*p),
            _ => None,
        };
        let qualified = noul(QUALIFIED);
        let on: Vec<Option<f64>> = (1..=matters).map(|k| noul(&format!("{ON}{k}"))).collect();
        let Some(decision::Answer::Choice { probabilities: Some(Value::Object(mass)), .. }) =
            reply.answers.get(WHICH)
        else {
            return Self { outcome: "error", model, qualified, on, error: Some("no choice in the reply".into()), ..Self::default() };
        };
        let whole = Some(Value::Object(mass.clone()));
        let best = mass
            .iter()
            .filter_map(|(option, p)| Some((option.as_str(), p.as_f64()?)))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        let Some((option, p)) = best else {
            return Self { outcome: "error", model, qualified, on, mass: whole, error: Some("an empty choice".into()), ..Self::default() };
        };
        let read = |outcome, met| Self {
            met,
            outcome,
            p: Some(p),
            qualified,
            on: on.clone(),
            mass: whole.clone(),
            model: model.clone(),
            error: None,
        };
        if option == REST {
            return read("rest", None);
        }
        let Some(i) = branch_index(option).filter(|i| *i < branches) else {
            return Self { error: Some(format!("an option nobody asked: {option}")), ..read("error", None) };
        };
        if p < tau {
            return read("below", None);
        }
        match qualified {
            None => Self { error: Some("no `qualified` in the reply".into()), ..read("error", None) },
            Some(q) if q > 1.0 - tau => read("qualified", None),
            Some(_) => read("met", Some(i)),
        }
    }
}

/// `b3` → 2.
fn branch_index(option: &str) -> Option<usize> {
    option.strip_prefix('b')?.parse::<usize>().ok()?.checked_sub(1)
}

/// The questions, in `judges/prepared.md`'s words: one choice over every branch of every
/// matter, whether the met one was qualified, and per matter whether the message took it up.
fn questions(sets: &[Set]) -> Map<String, Value> {
    let rubric = crate::identity::judges::PREPARED;
    let part = |heading: &str| crate::identity::rubric_section(rubric, heading).unwrap_or_default();
    let frame = part("Frame");
    let mut criteria = Map::new();
    for (n, (k, i)) in options(sets).into_iter().enumerate() {
        let set = &sets[k];
        criteria.insert(format!("b{}", n + 1), Value::from(format!("{} — {}", set.matter, set.branches[i].condition)));
    }
    criteria.insert(REST.into(), Value::from(part("Rest")));
    let mut asked = Map::new();
    asked.insert(
        WHICH.into(),
        json!({ "type": "choice", "instructions": format!("{frame} {}", part("Which way")), "criteria": criteria }),
    );
    asked.insert(
        QUALIFIED.into(),
        json!({ "type": "noul", "instructions": format!("{frame} {}", part("Qualified")) }),
    );
    for (k, set) in sets.iter().enumerate() {
        asked.insert(
            format!("{ON}{}", k + 1),
            json!({
                "type": "noul",
                "instructions": format!("{frame} {} The matter: {}", part("Took it up"), set.matter),
            }),
        );
    }
    asked
}

/// The state a reading is about: where each matter was left, what the agent has said since
/// the person last wrote, then what they wrote. Nothing else — System One loses accuracy on a
/// padded state, which is also why a matter left by the very lines the message answers says
/// so instead of repeating them.
fn state(sets: &[Set], recent: &[String], messages: &[Message]) -> String {
    let lines = |said: &[String]| -> String {
        said.iter().map(|l| format!("< {}\n", l.replace('\n', "\n  "))).collect()
    };
    let mut s = String::from("## Where each matter was left\n");
    for set in sets {
        s.push_str(&format!("### {}\n", set.matter));
        if set.said.is_empty() {
            s.push_str("(nothing the assistant said)\n");
        } else if set.said == recent {
            s.push_str("(the most recent lines, below)\n");
        } else {
            s.push_str(&lines(&set.said));
        }
    }
    s.push_str("\n## What the assistant said most recently\n");
    if recent.is_empty() {
        s.push_str("(nothing since the person last spoke)\n");
    }
    s.push_str(&lines(recent));
    s.push_str("\n## What the person said next\n");
    s.push_str(&render_messages(messages));
    s.push('\n');
    s
}

fn render_messages(messages: &[Message]) -> String {
    messages.iter().map(super::render_message_line).collect::<Vec<_>>().join("\n")
}

/// Run a met branch's actions in order, through the seams they would have gone through if
/// called at this moment. **The first action that does not happen stops the rest**, because a
/// later one may lean on it — 好，我把数据摆上来 after a show that failed is a false line.
///
/// Its output is bracketed as a sequencer turn of its own, under a fresh id, so its voice span,
/// a barge-in on it, and the reply the floor remembers are its own and not the running
/// turn's. Nothing here freezes the floor's `seen`: that belongs to the model turn the message
/// drives next, which has still not seen it.
async fn run(reaction: &Reaction, runner: &Runner, branch: &Branch) -> Vec<(String, Result<String, String>)> {
    let turn = reaction.inner.turn_seq.fetch_add(1, Ordering::Relaxed);
    let _ = runner.beats.send(Beat::TurnStart { turn }).await;
    let data_dir = reaction.inner.memory.data_dir().to_path_buf();
    let mut ran = Vec::with_capacity(branch.actions.len());
    for action in &branch.actions {
        let result = match action {
            Action::Say { text } => {
                if reaction.inner.unanswered.is_full() {
                    Err(format!(
                        "not sent — {} messages have already gone out since their last one",
                        super::unanswered::MAX_UNANSWERED
                    ))
                } else if runner.beats.send(Beat::Say(text.clone())).await.is_err() {
                    Err("not sent — the sequencer is gone".to_string())
                } else {
                    runner.said.fetch_add(1, Ordering::Relaxed);
                    reaction.inner.unanswered.note_sent();
                    Ok("sent".to_string())
                }
            }
            Action::Show { id, op, view_ref, source } => {
                // Resolved again now rather than trusted from when it was prepared: a view is
                // live, and what it was is not what it is.
                let source = match view_ref {
                    Some(r) => crate::mind::views::resolve_ref(&data_dir, r)
                        .await
                        .map_err(|e| format!("not shown — `ref` {r}: {e}")),
                    None => Ok(source.clone()),
                };
                match source {
                    Err(why) => Err(why),
                    Ok(source) => {
                        // Never kept: a met branch runs on their message, so what it shows is
                        // the answer to it.
                        let beat = Beat::Show {
                            id: id.clone(),
                            op: op.clone(),
                            source,
                            view_ref: view_ref.clone(),
                            keep: false,
                        };
                        match runner.beats.send(beat).await {
                            Ok(()) => Ok("shown".to_string()),
                            Err(_) => Err("not shown — the sequencer is gone".to_string()),
                        }
                    }
                }
            }
            Action::Send { to, message } => {
                let delivery = registry::global().send(&runner.from, to, message.clone());
                reaction
                    .inner
                    .observatory
                    .record(EventKind::MessageSent {
                        from: Some(runner.from.clone()),
                        to: to.clone(),
                        delivery,
                        message: message.clone(),
                    })
                    .await;
                match delivery {
                    Delivery::Delivered => Ok("delivered".to_string()),
                    Delivery::Unknown => Err(format!("not delivered — nothing live at `{to}`")),
                    Delivery::UnknownSender => Err("not delivered — this session is no longer registered".to_string()),
                    Delivery::NotPermitted => Err(format!("not delivered — `{to}` is not reachable from here")),
                }
            }
        };
        let stop = result.is_err();
        ran.push((action.describe(), result));
        if stop {
            break;
        }
    }
    let (done, reply) = oneshot::channel();
    let _ = runner.beats.send(Beat::TurnEnd { done }).await;
    let reply = reply.await.unwrap_or_default();
    reaction.inner.floor.end_turn(turn, &reply).await;
    ran
}

/// A branch that ran: its matter, where the message took it, and each action with what
/// became of it.
pub(super) struct Ran {
    matter: String,
    condition: String,
    ran: Vec<(String, Result<String, String>)>,
}

impl Ran {
    /// As the turn the message drives reads it. Facts only: what to do about it is
    /// `reaction.md`'s.
    pub(super) fn for_reaction(&self) -> String {
        format!(
            "\n## What your prepared branch did\nTheir message took \"{}\" where you had prepared \
             for: \"{}\". Before this turn began, that branch ran, in order:\n{}What you had \
             prepared for that matter is used up.\n",
            self.matter,
            self.condition,
            self.lines()
        )
    }

    /// As Cognition reads it, riding the hand-down of the same message. The branch may have
    /// handed work on seconds before the turn did, and a message in the inbox twice — once
    /// as Reaction's instruction, once as the person's words — reads as two requests unless
    /// something says they are one.
    pub(super) fn for_cognition(&self) -> String {
        format!(
            "\n(Before Reaction's turn, a branch it had prepared for this message going this way \
             — \"{}\" — already ran:\n{})",
            self.condition,
            self.lines().trim_end()
        )
    }

    fn lines(&self) -> String {
        let mut s = String::new();
        for (action, result) in &self.ran {
            match result {
                Ok(done) => s.push_str(&format!("- {action}: {done}\n")),
                Err(why) => s.push_str(&format!("- {action}: {why}\n")),
            }
        }
        if self.ran.last().is_some_and(|(_, r)| r.is_err()) {
            s.push_str("It stopped there; nothing after that action ran.\n");
        }
        s
    }
}

fn clip(s: &str, max: usize) -> String {
    let flat = s.replace('\n', " ");
    if flat.chars().count() <= max {
        flat
    } else {
        format!("{}…", flat.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn reply(which: Value, qualified: Option<f64>, on: &[f64]) -> decision::Reply {
        let mut answers = BTreeMap::new();
        answers.insert(
            WHICH.to_string(),
            decision::Answer::Choice { choice: String::new(), probabilities: Some(which), confidence: None },
        );
        if let Some(p) = qualified {
            answers.insert(QUALIFIED.to_string(), decision::Answer::Noul { p });
        }
        for (k, p) in on.iter().enumerate() {
            answers.insert(format!("{ON}{}", k + 1), decision::Answer::Noul { p: *p });
        }
        decision::Reply { model: "jev-1.13.0".into(), answers, usage: decision::Usage::default() }
    }

    fn read(which: Value, qualified: Option<f64>) -> Reading {
        Reading::of(&Ok(reply(which, qualified, &[0.9])), 2, 1, 0.9)
    }

    fn say(text: &str) -> Vec<Action> {
        vec![Action::Say { text: text.into() }]
    }

    fn branch(condition: &str) -> Branch {
        Branch { condition: condition.into(), actions: say("好") }
    }

    #[test]
    fn a_branch_runs_only_when_it_carries_the_mass_and_nothing_qualifies_it() {
        let met = read(json!({"b1": 0.95, "b2": 0.03, "rest": 0.02}), Some(0.04));
        assert_eq!((met.met, met.outcome), (Some(0), "met"));
        assert_eq!(met.model.as_deref(), Some("jev-1.13.0"));
        assert!(met.mass.is_some(), "the whole mass is kept, so the cut can move later");

        let below = read(json!({"b1": 0.7, "b2": 0.2, "rest": 0.1}), Some(0.04));
        assert_eq!((below.met, below.outcome), (None, "below"));

        // 对，不过预算砍一半 — agreement with a condition attached does not run the branch.
        let qualified = read(json!({"b1": 0.95, "b2": 0.03, "rest": 0.02}), Some(0.6));
        assert_eq!((qualified.met, qualified.outcome), (None, "qualified"));

        let rest = read(json!({"b1": 0.1, "b2": 0.1, "rest": 0.8}), Some(0.1));
        assert_eq!((rest.met, rest.outcome), (None, "rest"));
        assert_eq!(rest.on, vec![Some(0.9)], "whether each matter was taken up is read either way");
    }

    #[test]
    fn a_reply_that_cannot_be_read_runs_nothing_and_uses_nothing_up() {
        assert_eq!(read(json!({"b1": 0.99}), None).outcome, "error", "no `qualified`, no vouching");
        assert_eq!(read(json!({"b7": 0.99}), Some(0.0)).outcome, "error", "an option nobody asked");
        assert_eq!(read(json!({}), Some(0.0)).outcome, "error");
        let timeout = Reading::of(&Err(Unread::Timeout), 2, 1, 0.9);
        assert_eq!(timeout.outcome, "timeout");
        assert!(timeout.on.is_empty(), "nothing known about where it went, so nothing is used up");
        assert_eq!(Reading::of(&Err(Unread::Unavailable), 2, 1, 0.9).outcome, "unavailable");
        for r in [read(json!({"b1": 0.99}), None), timeout] {
            assert_eq!(r.met, None);
        }
    }

    fn set(matter: &str, conditions: &[&str], said: &[&str]) -> Set {
        Set {
            matter: matter.into(),
            branches: conditions.iter().map(|c| branch(c)).collect(),
            said: said.iter().map(|s| s.to_string()).collect(),
            at: Utc::now(),
            id: 0,
        }
    }

    #[test]
    fn options_run_across_every_matter_in_order_and_each_matter_is_asked_about() {
        let sets = vec![set("VLX 要不要试", &["让试 VLX"], &[]), set("A 还是 B", &["同意 A", "选 B"], &[])];
        assert_eq!(options(&sets), vec![(0, 0), (1, 0), (1, 1)]);
        let asked = questions(&sets);
        let criteria = asked[WHICH]["criteria"].as_object().unwrap();
        assert_eq!(criteria.keys().collect::<Vec<_>>(), vec!["b1", "b2", "b3", "rest"]);
        assert_eq!(criteria["b2"], "A 还是 B — 同意 A", "a condition is read with its matter");
        assert!(!criteria["rest"].as_str().unwrap().is_empty(), "the rubric's Rest is read");
        assert_eq!(asked[QUALIFIED]["type"], "noul");
        assert_eq!(asked["on1"]["type"], "noul");
        assert!(asked["on2"]["instructions"].as_str().unwrap().ends_with("The matter: A 还是 B"));
        assert!(!asked["on1"]["instructions"].as_str().unwrap().contains("  The matter:"), "Took it up is read");
        assert_eq!(branch_index("b1"), Some(0));
        assert_eq!(branch_index("b0"), None);
        assert_eq!(branch_index("rest"), None);
    }

    #[test]
    fn a_matter_is_read_with_where_it_was_left() {
        let sets = vec![set("VLX", &["让试"], &["VLX 跑得动，你要试吗？"]), set("A/B", &["同意 A"], &["我倾向 A"])];
        let s = state(&sets, &["我倾向 A".to_string()], &[]);
        assert!(s.contains("### VLX\n< VLX 跑得动，你要试吗？"), "{s}");
        assert!(s.contains("### A/B\n(the most recent lines, below)"), "not repeated: {s}");
        assert!(s.find("## What the assistant said most recently").unwrap() < s.find("## What the person said next").unwrap());
    }

    #[tokio::test]
    async fn a_call_is_one_matter_checked_whole() {
        let dir = std::env::temp_dir();
        let (matter, ok) = parse(
            &json!({"matter": " A 还是 B ", "branches": [
                {"condition": "agrees to A", "actions": [{"tool": "hi_say", "text": "好，A。"}]}
            ]}),
            &dir,
            400,
        )
        .await
        .unwrap();
        assert_eq!(matter, "A 还是 B");
        assert_eq!(ok[0].actions, say("好，A。"));

        let (_, cleared) = parse(&json!({"matter": "x", "branches": []}), &dir, 400).await.unwrap();
        assert!(cleared.is_empty(), "empty clears");

        let refused = |args: Value| {
            let dir = dir.clone();
            async move { parse(&args, &dir, 400).await.unwrap_err() }
        };
        assert!(refused(json!({"branches": []})).await.contains("`matter`"));
        assert!(refused(json!({"matter": "x"})).await.contains("branches"));
        assert!(refused(json!({"matter": "x", "branches": [{"actions": []}]})).await.contains("no `condition`"));
        for empty in [json!({"condition": "picks B"}), json!({"condition": "picks B", "actions": []})] {
            let why = refused(json!({"matter": "x", "branches": [empty]})).await;
            assert!(why.contains("no actions") && why.contains("Nothing was prepared"), "{why}");
        }
        let long = refused(json!({"matter": "x", "branches": [{"condition": "x", "actions": [{"tool": "hi_say", "text": "字".repeat(401)}]}]})).await;
        assert!(long.contains("too long") && long.contains("Nothing was prepared"), "{long}");
        assert!(refused(json!({"matter": "x", "branches": [{"condition": "x", "actions": [{"tool": "hi_prepare"}]}]})).await.contains("cannot prepare"));
        assert!(refused(json!({"matter": "x", "branches": [{"condition": "x", "actions": [{"tool": "shell"}]}]})).await.contains("cannot be prepared"));
        assert!(refused(json!({"matter": "x", "branches": [{"condition": "x", "actions": [{"tool": "hi_show"}]}]})).await.contains("needs a `ref`"));
        assert!(refused(json!({"matter": "x", "branches": [{"condition": "x", "actions": [{"tool": "hi_send_message", "to": "nobody-here", "message": "go"}]}]})).await.contains("nothing live"));
        let fan: Vec<Value> = (0..=MAX_BRANCHES).map(|i| json!({"condition": format!("d{i}"), "actions": [{"tool": "hi_say", "text": "好"}]})).collect();
        assert!(refused(json!({"matter": "x", "branches": fan})).await.contains("a fan"));
    }

    fn matters(p: &Prepared) -> Vec<String> {
        p.lock().sets.iter().map(|s| s.matter.clone()).collect()
    }

    #[tokio::test]
    async fn matters_stand_side_by_side_and_one_prepared_again_is_replaced() {
        let p = Prepared::new(Observatory::new(None), None);
        p.set("A 还是 B", vec![branch("同意 A")], vec!["我倾向 A".into()]).await;
        p.set("VLX", vec![branch("让试")], vec![]).await;
        assert_eq!(matters(&p), vec!["A 还是 B", "VLX"]);

        // The same matter, spaced differently, is the same matter: replaced, and newest last.
        p.set("A还是B", vec![branch("选 B")], vec![]).await;
        assert_eq!(matters(&p), vec!["VLX", "A还是B"]);
        assert_eq!(p.lock().sets[1].branches[0].condition, "选 B");

        p.set("vlx", vec![], vec![]).await;
        assert_eq!(matters(&p), vec!["A还是B"], "an empty list clears that matter only");
    }

    #[tokio::test]
    async fn the_oldest_goes_to_make_room() {
        let p = Prepared::new(Observatory::new(None), None);
        for m in ["一", "二", "三", "四"] {
            assert!(p.set(m, vec![branch("x")], vec![]).await.is_empty());
        }
        assert_eq!(p.set("五", vec![branch("x")], vec![]).await, vec!["一"], "past the matters kept");
        let wide: Vec<Branch> = (0..6).map(|i| branch(&format!("d{i}"))).collect();
        assert_eq!(p.set("六", wide, vec![]).await, vec!["二", "三"], "past the branches kept");
        assert_eq!(matters(&p), vec!["四", "五", "六"]);
    }

    #[tokio::test]
    async fn what_is_ready_outlives_a_restart_and_a_reading_uses_up_only_what_it_read() {
        let dir = tempfile::tempdir().unwrap();
        let p = Prepared::new(Observatory::new(None), Some(dir.path()));
        p.set("VLX", vec![branch("让试")], vec!["跑得动".into()]).await;
        p.set("A/B", vec![branch("同意 A")], vec![]).await;
        drop(p);

        let p = Prepared::new(Observatory::new(None), Some(dir.path()));
        assert_eq!(matters(&p), vec!["VLX", "A/B"], "kept on disk, in order");
        assert_eq!(p.lock().sets[0].said, vec!["跑得动"]);
        let read = p.lock().sets.clone();
        // Prepared again while the reading ran: a different set, which stays.
        p.set("A/B", vec![branch("选 B")], vec![]).await;
        p.use_up(&read.iter().map(|s| s.id).collect::<Vec<_>>());
        assert_eq!(matters(&p), vec!["A/B"]);
        assert_eq!(p.lock().sets[0].branches[0].condition, "选 B");

        let p = Prepared::new(Observatory::new(None), Some(dir.path()));
        assert_eq!(matters(&p), vec!["A/B"], "what was used up stays used up");
    }

    #[test]
    fn the_window_lists_what_is_ready_and_says_when_nothing_is() {
        let p = Prepared::new(Observatory::new(None), None);
        assert_eq!(p.render(), "## What you have ready\n(nothing prepared)");
        p.lock().sets.push(set("VLX 要不要试", &["让试 VLX"], &[]));
        let s = p.render();
        assert!(s.contains("### VLX 要不要试 — prepared "), "{s}");
        assert!(s.contains("- 让试 VLX → hi_say \"好\""), "{s}");
        assert!(s.contains("no branches"), "how to clear one is said: {s}");
    }

    #[test]
    fn what_ran_says_where_it_stopped() {
        let stopped = Ran {
            matter: "A/B".into(),
            condition: "想看两边的数".into(),
            ran: vec![(
                "hi_show show plan/compare".to_string(),
                Err("not shown — `ref` plan/compare: no such view".to_string()),
            )],
        };
        let s = stopped.for_reaction();
        assert!(s.contains("## What your prepared branch did"));
        assert!(s.contains("想看两边的数") && s.contains("A/B"));
        assert!(s.contains("It stopped there"));
        assert!(s.contains("used up"));
        let ok = Ran {
            matter: "A/B".into(),
            condition: "agrees".into(),
            ran: vec![
                ("hi_send_message → cognition: \"按 A 开工\"".to_string(), Ok("delivered".to_string())),
                ("hi_say \"好\"".to_string(), Ok("sent".to_string())),
            ],
        };
        assert!(!ok.for_reaction().contains("stopped"));
        // Cognition is told the two arrivals are one request, not two.
        let c = ok.for_cognition();
        assert!(c.contains("already ran") && c.contains("按 A 开工\": delivered"), "{c}");
    }
}
