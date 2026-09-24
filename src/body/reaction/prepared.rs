//! Prepared actions — `hi_prepare`, the floor that releases what it sets, and what the person's
//! messages do with it.
//!
//! **Nothing Reaction writes reaches the person at the instant it is written.** Everything it
//! would say or show is an action in a set prepared for one matter, each branch naming the moment
//! it is for: `finished` (they stopped and are done), `paused` (they stopped with more to come),
//! or a condition on their next message. The host releases a branch when the room reaches its
//! moment and a reading says it still fits, then tells Reaction what happened with whatever
//! wakes it next. See `docs/arch/agents.md` § *Prepared actions* and `docs/arch/host.md`
//! § *The floor*.
//!
//! **Why it is built this way.** The gate this replaced could only say *now* or *never*: a line
//! checked when it was ready was refused if they had said something the turn had not seen, and a
//! refused line was simply lost. Someone mid-thought pauses for less than a generation takes
//! (median 15.5 s between two of their lines against 18 s to a first line), so a turn written
//! during a run of questions was refused almost by construction, and a backstop let the fourth
//! attempt through whatever it was. Holding a line is safe only if it is read again before it
//! goes, so every stop asks System One, once, everything the moment raises: are they finished,
//! does each held set still fit what they said since it was written, and — when the stop carries
//! a message — which condition branch it met.
//!
//! **One reading per stop, three things that start one:** a message of theirs landing
//! ([`on_message`]), a floor set prepared while the room is already stopped ([`Kick::Prepared`]),
//! and the one wait running out ([`Kick::WaitRanOut`]). A reading that cannot be read — no
//! System One, a timeout, an error — is what the floor did before it asked: a set written after
//! everything they have said goes out if they are not talking, and one written before a line of
//! theirs stays held.
//!
//! **A condition set belongs to a matter, not to the next message.** People step away from a
//! subject and come back to it, so a set waits until a message takes its matter up: met, it runs
//! and is used up; taken somewhere no branch describes, it is used up without running. A
//! matter's condition branches are not read while its `finished` branch is still waiting — they
//! were written for the message *after* that answer.
//!
//! What keeps it from doing harm:
//!
//! - **a miss runs nothing**, and a reading that could not be read never runs a condition branch;
//! - **every action is one of Reaction's own**: words to the person, a view on their screen, a
//!   message to another rung. Anything outward is a worker's, behind Cognition's judgment;
//! - **the first action that does not happen stops the rest**, and a show that went into their
//!   list instead of in front of them is one that did not happen as written.
//!
//! **Built and unit-tested; never watched on a live turn.** The events it records
//! (`branches_prepared` / `branches_resolved` / `branches_voided` / `floor_read`) are the count
//! to read it by, and the journey that will settle it is a run of questions like 09-24's.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value, json};
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::body::capabilities::decision;
use crate::foundation::config::tunables;
use crate::foundation::observatory::{BranchDirection, EventKind, Observatory};
use crate::foundation::registry::{self, Delivery, SessionSlug};
use crate::types::{FileRef, Message};

use super::sequencer::Beat;
use super::{LoopInput, Reaction};

/// The `app_settings` key switching the reading of condition branches off. Reads unless `off`,
/// like every gate here. Floor branches are the mouth and have no switch.
pub(crate) const SWITCH_KEY: &str = "prepared_branches";
/// The `app_settings` key moving the condition cut.
pub(crate) const THRESHOLD_KEY: &str = "prepared_branches_threshold";
/// The `app_settings` key moving the `finished` cut.
pub(crate) const FINISHED_KEY: &str = "floor_finished_at";

/// Where the sets are kept, under the data dir, so a restart does not lose what was ready.
const FILE: &str = "memory/prepared.json";

/// How much of the choice's mass a condition branch must carry to run, and how little
/// `qualified` may. A starting value: every reading's whole mass is recorded.
const THRESHOLD: f64 = 0.9;

/// How sure the reading must be that a message took a matter up for that matter's condition
/// branches to be used up. Half, not the run cut: one used up wrongly costs a turn preparing it
/// again, and one kept wrongly is a stale set waiting for a message to mistake.
const TAKEN_UP: f64 = 0.5;

/// How sure the reading must be that they have finished for `finished` sets to go. Half: the two
/// errors are close to even — a line released on a pause is talking over a thought, a line held
/// on a finish costs the wait — and the wait is the backstop for the second. A starting value.
const FINISHED_AT: f64 = 0.5;

/// How long the reading may take. System One answered the speech check's questions in p50
/// 0.53 s, p99 2.2 s on this install; of the first sixty live readings of condition branches, 15
/// ran out of this. Past it, the floor does what it did before it asked.
const BUDGET: Duration = Duration::from_secs(2);

/// More condition directions than this, across every matter, is a fan, not a guess — and System
/// One loses accuracy on a padded question as it does on a padded state.
const MAX_BRANCHES: usize = 8;

/// How many matters with condition branches are kept at once. A few, like what a person holds in
/// mind; the oldest goes first. Floor branches are not counted: each is one `fits` question, and
/// they go at the next stop.
const MAX_MATTERS: usize = 4;

/// How long [`Prepared::settled`] waits for a reading before letting the turn run anyway.
const SETTLED_WITHIN: Duration = Duration::from_secs(8);

/// After a `paused` release: they have been asked whether there is more, and a short silence
/// answers no. `docs/arch/host.md` § *The one wait*. A starting value.
const PAUSE_RELEASE: Duration = Duration::from_secs(8);

/// After a stop read as *has more* with nothing said to them: about the p90 of the pauses between
/// two of their lines (09-22 → 09-24), so someone finishing a thought is left to finish it.
const THOUGHT_RELEASE: Duration = Duration::from_secs(50);

/// How long the room must be quiet before a set held `hold`, or an action that did not happen,
/// wakes Reaction on its own (`docs/arch/host.md` § *What Reaction learns, and when*).
const QUIET_WAKE: Duration = Duration::from_secs(60);

/// How many of their lines are kept to say what a held set has not seen.
const LINES_KEPT: usize = 40;

/// How many of their lines before this stop a reading is shown: what `finished` reads against,
/// when the stop itself carried nothing new.
const RECENT_LINES: usize = 6;

const WHICH: &str = "which";
const REST: &str = "rest";
const QUALIFIED: &str = "qualified";
const ON: &str = "on";
const FINISHED: &str = "finished";
/// The prefix of the choice asked per floor branch: `fits1`, `fits2`, … in order.
const FITS: &str = "fits";

/// Whether condition branches are read at all, from the switch.
pub fn enabled() -> bool {
    crate::body::legibility::enabled(SWITCH_KEY)
}

fn cut(key: &str, default: f64, lowest: f64) -> f64 {
    tunables::get(key)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|p| (lowest..=1.0).contains(p))
        .unwrap_or(default)
}

/// The moment a branch is for.
#[derive(Clone, Debug, PartialEq)]
pub enum When {
    /// They stopped, and are done.
    Finished,
    /// They stopped, with more to come.
    Paused,
    /// Their next message goes where this says, in Reaction's words.
    Condition(String),
}

impl When {
    fn parse(raw: &str) -> Self {
        match raw.trim() {
            "finished" => When::Finished,
            "paused" => When::Paused,
            other => When::Condition(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            When::Finished => "finished",
            When::Paused => "paused",
            When::Condition(c) => c,
        }
    }

    fn is_floor(&self) -> bool {
        matches!(self, When::Finished | When::Paused)
    }
}

impl Serialize for When {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for When {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(When::parse(&String::deserialize(d)?))
    }
}

/// One action in a branch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Action {
    /// A line, and what it hands over — each attachment a message of its own right behind it.
    Say {
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        hands: Vec<FileRef>,
    },
    /// A view by ref, an attachment by its `att:` id, or a trivial inline one.
    Show { id: Option<String>, op: String, view_ref: Option<String>, source: String },
    #[serde(rename = "send_message")]
    Send { to: SessionSlug, message: String },
}

impl Action {
    /// The action as one line, for the event log, the window, and what Reaction is told.
    fn describe(&self) -> String {
        match self {
            Action::Say { text, hands } if hands.is_empty() => format!("say \"{}\"", clip(text, 120)),
            Action::Say { text, hands } => format!(
                "say \"{}\" handing over {}",
                clip(text, 120),
                hands.iter().map(|f| f.reff.as_str()).collect::<Vec<_>>().join(", ")
            ),
            Action::Show { id, op, view_ref, .. } => format!(
                "show {op} {}",
                view_ref.as_deref().or(id.as_deref()).unwrap_or("an inline view")
            ),
            Action::Send { to, message } => format!("send_message → {to}: \"{}\"", clip(message, 120)),
        }
    }

    fn line(&self) -> Option<&str> {
        match self {
            Action::Say { text, .. } => Some(text),
            _ => None,
        }
    }
}

/// One direction: the moment it is for, and what to do there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Branch {
    pub when: When,
    pub actions: Vec<Action>,
}

impl Branch {
    fn direction(&self) -> BranchDirection {
        BranchDirection {
            condition: self.when.as_str().to_string(),
            actions: self.actions.iter().map(Action::describe).collect(),
        }
    }

    /// The lines it would say, for the pre-send check.
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.actions.iter().filter_map(Action::line)
    }
}

/// Read a `hi_prepare` call's arguments into its matter and branches, **checking every action the
/// way it will be run** — a line within `say_max`, a `ref` that resolves, a `to` that is live, an
/// attachment that exists — so a branch that could not run is found in the turn that can still
/// fix it. Any failure refuses the whole call, naming where. An empty list is valid, and clears
/// the matter.
pub async fn parse(args: &Value, data_dir: &Path, say_max: usize) -> Result<(String, Vec<Branch>), String> {
    let matter = args.get("matter").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    if matter.is_empty() {
        return Err("hi_prepare needs a `matter`: a few words naming what this is about, so preparing \
                    it again replaces it"
            .into());
    }
    let Some(list) = args.get("branches").and_then(Value::as_array) else {
        return Err("hi_prepare needs `branches`: a list of {when, actions} — an empty list clears \
                    what is prepared for the matter"
            .into());
    };
    let mut branches: Vec<Branch> = Vec::with_capacity(list.len());
    for (i, b) in list.iter().enumerate() {
        let n = i + 1;
        let when = b.get("when").and_then(Value::as_str).map(str::trim).unwrap_or_default();
        if when.is_empty() {
            return Err(format!(
                "branch {n} has no `when` — `finished`, `paused`, or where their next message would \
                 take it, in plain words. Nothing was prepared."
            ));
        }
        let when = When::parse(when);
        if when.is_floor() && branches.iter().any(|b| b.when == when) {
            return Err(format!(
                "branch {n}: a matter has one `{}` branch — put everything for that moment in its \
                 actions. Nothing was prepared.",
                when.as_str()
            ));
        }
        let list = match b.get("actions") {
            Some(Value::Array(list)) if !list.is_empty() => list,
            Some(Value::Array(_)) | None | Some(Value::Null) => {
                return Err(format!(
                    "branch {n} (\"{}\") has no actions — a direction with nothing ready runs \
                     nothing, so it is no branch; leave it out. Nothing was prepared.",
                    when.as_str()
                ));
            }
            Some(_) => return Err(format!("branch {n}: `actions` is a list. Nothing was prepared.")),
        };
        let mut actions = Vec::with_capacity(list.len());
        for (j, a) in list.iter().enumerate() {
            match parse_action(a, data_dir, say_max).await {
                Ok(action) => actions.push(action),
                Err(why) => {
                    return Err(format!(
                        "branch {n} (\"{}\"), action {}: {why}. Nothing was prepared.",
                        when.as_str(),
                        j + 1
                    ));
                }
            }
        }
        branches.push(Branch { when, actions });
    }
    let conditions = branches.iter().filter(|b| !b.when.is_floor()).count();
    if conditions > MAX_BRANCHES {
        return Err(format!(
            "{conditions} directions is a fan, not a guess — prepare the one or two you actually \
             expect (at most {MAX_BRANCHES}). Nothing was prepared."
        ));
    }
    Ok((matter.to_string(), branches))
}

async fn parse_action(a: &Value, data_dir: &Path, say_max: usize) -> Result<Action, String> {
    let arg = |k: &str| {
        a.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
    };
    match arg("do").as_deref() {
        Some("say") => {
            let text = arg("text").ok_or("say needs a non-empty `text`")?;
            let chars = text.chars().count();
            if chars > say_max {
                return Err(format!("too long for one message ({chars} characters; the most is {say_max})"));
            }
            let hands = crate::foundation::mcp::handed_over(data_dir, a).await?;
            Ok(Action::Say { text, hands })
        }
        Some("show") => {
            let op = arg("op").unwrap_or_else(|| "show".to_string());
            if !matches!(op.as_str(), "show" | "replace" | "dismiss") {
                return Err(format!("`op` is show, replace or dismiss, not `{op}`"));
            }
            let (id, view_ref, source) = (arg("id"), arg("ref"), arg("source"));
            let view_ref = match (&view_ref, &source) {
                // An attachment goes up as itself: no source, no compile — the stage mounts the
                // host's own viewer for it (`docs/arch/showing.md` § *On the stage*).
                (Some(r), _) if crate::foundation::attachments::ref_id(r).is_some() => {
                    let id = crate::foundation::attachments::ref_id(r).unwrap_or_default();
                    if crate::foundation::attachments::probe(data_dir, id).await.is_none() {
                        return Err(crate::foundation::attachments::Refusal::UnknownId(id.to_owned()).to_string());
                    }
                    Some(format!("{}{id}", crate::foundation::attachments::PREFIX))
                }
                (Some(r), _) => {
                    crate::mind::views::resolve_ref(data_dir, r).await.map_err(|e| format!("`ref` {r}: {e}"))?;
                    Some(r.clone())
                }
                (None, Some(_)) => None,
                (None, None) if op == "dismiss" && id.is_some() => None,
                (None, None) => return Err("show needs a `ref` (or an `id` to dismiss)".into()),
            };
            Ok(Action::Show { id, op, view_ref, source: source.unwrap_or_default() })
        }
        Some("send_message") => {
            let to = arg("to").ok_or("send_message needs `to`")?;
            let message = arg("message").ok_or("send_message needs a non-empty `message`")?;
            let to = to
                .parse::<SessionSlug>()
                .map_err(|_| format!("`{to}` is not a session slug — `cognition`, or one your window lists"))?;
            if registry::global().status(&to).is_none() {
                return Err(format!("nothing live at `{to}` to send to"));
            }
            Ok(Action::Send { to, message })
        }
        Some(other) => Err(format!("`{other}` is not an action — an action does say, show or send_message")),
        None => Err("each action names what it does in `do`: say, show or send_message".into()),
    }
}

/// One matter's set, and what it was written against.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Set {
    matter: String,
    branches: Vec<Branch>,
    /// What the agent had said since the person last wrote — where the matter was left.
    said: Vec<String>,
    at: DateTime<Utc>,
    /// How many of their lines the turn that wrote it had seen ([`super::Floor::seen`]). What
    /// they said after is what `fits` reads it against.
    #[serde(default)]
    heard: u64,
    /// Whether the turn that wrote it was started by something they said — what it shows is the
    /// answer to them, and an answer takes the screen ([`crate::foundation::server::ViewBus::claim`]).
    #[serde(default)]
    answering: bool,
    /// Held `hold` at a stop and already told so — told once, not at every stop after.
    #[serde(default)]
    held_told: bool,
    /// Which set this is within the run, so a reading changes the sets it read and not ones
    /// prepared again while it ran. Not kept: numbered again on load.
    #[serde(skip)]
    id: u64,
}

impl Set {
    fn branch(&self, when: &When) -> Option<&Branch> {
        self.branches.iter().find(|b| &b.when == when)
    }

    fn conditions(&self) -> impl Iterator<Item = &Branch> {
        self.branches.iter().filter(|b| !b.when.is_floor())
    }

    /// Its condition branches are for the message after its answer, so they are read only once
    /// the answer is not waiting any more.
    fn conditions_live(&self) -> bool {
        self.branch(&When::Finished).is_none() && self.conditions().next().is_some()
    }
}

/// A stop, as read: how many of their lines had landed, and System One's `finished` — `None`
/// when it could not be read.
#[derive(Clone, Copy, Debug)]
struct Stop {
    heard: u64,
    finished: Option<f64>,
}

/// What starts a reading without a message of theirs.
#[derive(Debug)]
pub(super) enum Kick {
    /// A floor set was prepared; read it at once if the room is stopped.
    Prepared,
    /// The one wait ran out: read the `finished` sets as if they had finished.
    WaitRanOut,
}

/// Where a released branch runs: the sequencer, who a message goes as, the loop to wake, and the
/// floor task to kick. Attached once, as the conversation's loop stands up.
#[derive(Clone)]
struct Runner {
    beats: mpsc::Sender<Beat>,
    from: SessionSlug,
    inbox: mpsc::Sender<LoopInput>,
    kick: mpsc::UnboundedSender<Kick>,
}

#[derive(Default)]
struct State {
    /// Oldest first.
    sets: Vec<Set>,
    next_id: u64,
    /// The message reading in flight, if any: further messages of the same batch go to it.
    reading: Option<mpsc::UnboundedSender<Message>>,
    /// A floor set was prepared while a reading ran; read again when it ends.
    again: bool,
    /// What the last message reading ran, for Cognition's hand-down of that message.
    ran: Option<Ran>,
    runner: Option<Runner>,
    /// Their lines, as `(heard index, line)`, newest last.
    lines: VecDeque<(u64, String)>,
    /// What Reaction is owed about its sets, one line each.
    told: Vec<String>,
    /// What the latest stop read about whether they are done, and how many of their lines it
    /// had — what a set prepared after it, with nothing said since, is released against.
    last_stop: Option<Stop>,
    /// The one wait, when armed.
    wait: Option<JoinHandle<()>>,
    /// The minute of quiet before a held set wakes Reaction, when armed.
    quiet: Option<JoinHandle<()>>,
}

impl State {
    fn number(&mut self, mut set: Set) -> Set {
        self.next_id += 1;
        set.id = self.next_id;
        set
    }

    fn cancel_wait(&mut self) {
        if let Some(w) = self.wait.take() {
            w.abort();
        }
    }
}

/// The matters prepared, the floor that releases them, and what Reaction is owed about them.
pub struct Prepared {
    state: std::sync::Mutex<State>,
    observatory: Observatory,
    /// `None` keeps nothing on disk — tests.
    path: Option<PathBuf>,
    /// `true` while no message reading is in flight. The loop waits on it before a turn, so the
    /// turn a message drives always starts after that message's reading.
    idle: watch::Sender<bool>,
    /// Something was told while a turn may be running — the turn steers it in.
    pub(super) told_ready: Notify,
    /// Releases go out one at a time, each bracketed as a sequencer turn of its own.
    releasing: tokio::sync::Mutex<()>,
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
                    Err(err) => tracing::warn!(error = %err, path = %path.display(), "prepared: kept file unreadable; starting empty"),
                },
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => tracing::warn!(error = %err, path = %path.display(), "prepared: kept file unreadable; starting empty"),
            }
        }
        Self {
            state: std::sync::Mutex::new(state),
            observatory,
            path,
            idle,
            told_ready: Notify::new(),
            releasing: tokio::sync::Mutex::new(()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Write the sets as they stand. Called under the lock, so writes land in order.
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
            tracing::warn!(error = %err, path = %path.display(), "prepared: could not keep the sets");
        }
    }

    /// Where released branches go, and how to kick the floor task. Called once, by the loop.
    pub(super) fn attach(
        &self,
        beats: mpsc::Sender<Beat>,
        from: SessionSlug,
        inbox: mpsc::Sender<LoopInput>,
        kick: mpsc::UnboundedSender<Kick>,
    ) {
        self.lock().runner = Some(Runner { beats, from, inbox, kick });
    }

    /// Prepare `matter`: replace whatever was set for it, or clear it when `branches` is empty.
    /// `heard` is how many of their lines the writing turn had seen, `answering` whether they
    /// started it. The newest goes last; past [`MAX_MATTERS`] or [`MAX_BRANCHES`] condition
    /// matters, the oldest go. Returns the matters that went to make room. A set with a floor
    /// branch kicks the floor, which reads it at once if the room is stopped.
    pub(super) async fn set(
        &self,
        matter: &str,
        branches: Vec<Branch>,
        said: Vec<String>,
        heard: u64,
        answering: bool,
    ) -> Vec<String> {
        let directions: Vec<BranchDirection> = branches.iter().map(Branch::direction).collect();
        let floor = branches.iter().any(|b| b.when.is_floor());
        let (cleared, evicted, kick) = {
            let mut st = self.lock();
            let before = st.sets.len();
            st.sets.retain(|s| !same_matter(&s.matter, matter));
            let cleared = st.sets.len() < before;
            let mut evicted = Vec::new();
            if !branches.is_empty() {
                let set = st.number(Set {
                    matter: matter.to_string(),
                    branches,
                    said,
                    at: Utc::now(),
                    heard,
                    answering,
                    held_told: false,
                    id: 0,
                });
                st.sets.push(set);
                loop {
                    let with: Vec<usize> =
                        (0..st.sets.len()).filter(|&k| st.sets[k].conditions().next().is_some()).collect();
                    let count: usize = with.iter().map(|&k| st.sets[k].conditions().count()).sum();
                    if with.len() <= MAX_MATTERS && count <= MAX_BRANCHES {
                        break;
                    }
                    let oldest = with[0];
                    let set = &mut st.sets[oldest];
                    set.branches.retain(|b| b.when.is_floor());
                    evicted.push(set.matter.clone());
                    if set.branches.is_empty() {
                        st.sets.remove(oldest);
                    }
                }
            }
            self.keep(&st.sets);
            (cleared, evicted, st.runner.as_ref().map(|r| r.kick.clone()).filter(|_| floor))
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
                .record(EventKind::BranchesVoided { matter: Some(gone.clone()), reason: "the oldest, to make room".into() })
                .await;
        }
        if let Some(kick) = kick {
            let _ = kick.send(Kick::Prepared);
        }
        evicted
    }

    /// What is ready, as every turn's window carries it. Absolute times, so the text changes only
    /// when the sets do.
    pub(super) fn render(&self) -> String {
        use std::fmt::Write as _;
        let st = self.lock();
        let mut s = String::from("## What you have ready\n");
        if st.sets.is_empty() {
            s.push_str("(nothing prepared)");
            return s;
        }
        s.push_str(
            "What you prepared, matter by matter. `finished` goes when they stop and are done, \
             `paused` when they stop with more to come, a condition when their next message goes \
             there. What would no longer be right, clear (hi_prepare with that matter and no \
             branches) or prepare again.\n",
        );
        for set in &st.sets {
            let held = if set.held_told { " — held at the last stop" } else { "" };
            let _ = writeln!(s, "\n### {} — prepared {}{held}", set.matter, set.at.format("%m-%d %H:%M:%SZ"));
            for b in &set.branches {
                let actions = b.actions.iter().map(Action::describe).collect::<Vec<_>>().join("; ");
                let _ = writeln!(s, "- {} → {actions}", b.when.as_str());
            }
        }
        s
    }

    /// What the last message reading ran, for Cognition's hand-down of that message. Taken once.
    pub(super) fn take_ran(&self) -> Option<Ran> {
        self.lock().ran.take()
    }

    /// Whether a condition branch has run that no turn has been told about yet — which is also a
    /// reading's verdict that the batch was someone talking with the agent.
    pub(super) fn ran_waiting(&self) -> bool {
        self.lock().ran.is_some()
    }

    /// Wait for a message reading in flight, if any — bounded by [`SETTLED_WITHIN`].
    pub(super) async fn settled(&self) {
        let mut idle = self.idle.subscribe();
        let _ = tokio::time::timeout(SETTLED_WITHIN, idle.wait_for(|idle| *idle)).await;
    }

    /// What happened to its sets since Reaction was last told, as its window or a steer carries
    /// it; `None` when nothing is owed. Taking it is telling it, so the minute of quiet stops
    /// waiting.
    pub(super) fn take_told(&self) -> Option<String> {
        let mut st = self.lock();
        if st.told.is_empty() {
            return None;
        }
        if let Some(q) = st.quiet.take() {
            q.abort();
        }
        let lines: Vec<String> = st.told.drain(..).collect();
        Some(format!("## What happened to what you prepared\n{}\n", lines.join("\n")))
    }

    /// Put back what could not be steered in — it is owed, and rides the next wake.
    pub(super) fn untake(&self, text: String) {
        let body = text.trim_start_matches("## What happened to what you prepared\n").trim_end().to_string();
        self.lock().told.insert(0, body);
    }

    /// Owe Reaction a line about its sets. One that waits on its judgment arms the minute of quiet.
    fn tell(&self, text: String, judgment: bool) {
        {
            let mut st = self.lock();
            st.told.push(text);
            if judgment && st.quiet.is_none() {
                if let Some(runner) = st.runner.clone() {
                    st.quiet = Some(tokio::spawn(async move {
                        tokio::time::sleep(QUIET_WAKE).await;
                        let _ = runner.inbox.send(LoopInput::PreparedWaiting).await;
                    }));
                }
            }
        }
        self.told_ready.notify_one();
    }

    /// A line of theirs, at `heard` — what a held set is read against.
    fn note_line(&self, heard: u64, line: String) {
        let mut st = self.lock();
        st.lines.push_back((heard, line));
        while st.lines.len() > LINES_KEPT {
            st.lines.pop_front();
        }
    }

    /// Anything they say or type ends the wait and the minute of quiet: they are not quiet.
    fn note_them(&self) {
        let mut st = self.lock();
        st.cancel_wait();
        if let Some(q) = st.quiet.take() {
            q.abort();
        }
    }

    /// Arm the one wait. Replaces one already armed.
    fn arm_wait(&self, after: Duration) {
        let mut st = self.lock();
        st.cancel_wait();
        let Some(runner) = st.runner.clone() else { return };
        st.wait = Some(tokio::spawn(async move {
            tokio::time::sleep(after).await;
            let _ = runner.kick.send(Kick::WaitRanOut);
        }));
    }

    /// Apply what a reading decided to the sets, by id: one prepared again while it ran is a
    /// different set and stays.
    fn apply(&self, gone: &[(u64, Vec<When>)], used_up: &[u64], held: &[u64]) {
        let mut st = self.lock();
        for set in st.sets.iter_mut() {
            if let Some((_, whens)) = gone.iter().find(|(id, _)| *id == set.id) {
                set.branches.retain(|b| !whens.contains(&b.when));
            }
            if used_up.contains(&set.id) {
                set.branches.retain(|b| b.when.is_floor());
            }
            if held.contains(&set.id) {
                set.held_told = true;
            }
        }
        st.sets.retain(|s| !s.branches.is_empty());
        self.keep(&st.sets);
    }
}

/// Two names for one matter: the same words, spacing and case aside.
fn same_matter(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.split_whitespace().collect::<String>().to_lowercase();
    norm(a) == norm(b)
}

/// A message from the person landed. It ends the wait, is kept for what held sets are read
/// against, and joins a reading in flight — or starts one, if anything is prepared.
///
/// **Called from [`Reaction::deliver`] before the message is queued for the loop**, so the loop's
/// wait for the reading ([`Prepared::settled`]) can never miss one that has begun.
pub(super) async fn on_message(reaction: &Reaction, message: &Message) {
    let prepared = &reaction.inner.prepared;
    prepared.note_them();
    prepared.note_line(reaction.inner.floor.heard(), super::render_message_line(message));
    let start = {
        let mut st = prepared.lock();
        if let Some(reading) = st.reading.as_ref() {
            // The batch is still open; the reading asks again over the whole of it.
            let _ = reading.send(message.clone());
            None
        } else {
            // **Every stop is read, prepared or not**: whether they are done is what a reply
            // written after it goes out against, and it is being written right now.
            let (tx, rx) = mpsc::unbounded_channel();
            st.reading = Some(tx);
            st.ran = None;
            Some((rx, st.sets.iter().any(Set::conditions_live)))
        }
    };
    if let Some((rx, conditions)) = start {
        // The turn this message drives waits for the reading only when a condition branch
        // could run on it — it has to be told what ran. A stop read for `finished` alone
        // holds no turn up.
        if conditions {
            prepared.idle.send_replace(false);
        }
        tokio::spawn(read(reaction.clone(), Some(message.clone()), rx, false));
    }
}

/// The floor's own task: reads started by something other than a message — a floor set prepared
/// while the room is stopped, and the wait running out. Spawned once per conversation loop.
pub(super) async fn serve(reaction: Reaction, mut kicks: mpsc::UnboundedReceiver<Kick>) {
    while let Some(kick) = kicks.recv().await {
        let forced = matches!(kick, Kick::WaitRanOut);
        let floor = &reaction.inner.floor;
        let now = Instant::now();
        // They are talking or typing: their line will land and start a reading of its own. A
        // wait that runs out on someone still going is armed again rather than spent.
        if floor.voice_active(now).await || floor.typing_active(now).await {
            if forced {
                reaction.inner.prepared.arm_wait(PAUSE_RELEASE);
            }
            continue;
        }
        enum Next {
            Nothing,
            /// Every floor set was written after the latest stop, and nothing has been said
            /// since: that stop's reading is the answer, so nothing is asked again.
            AsRead(Vec<Set>, Stop),
            Read(mpsc::UnboundedReceiver<Message>),
        }
        let heard = floor.heard();
        let next = {
            let mut st = reaction.inner.prepared.lock();
            let floor_sets: Vec<Set> =
                st.sets.iter().filter(|s| s.branches.iter().any(|b| b.when.is_floor())).cloned().collect();
            if st.reading.is_some() {
                st.again = true;
                Next::Nothing
            } else if floor_sets.is_empty() {
                Next::Nothing
            } else {
                match st.last_stop {
                    Some(stop) if !forced && stop.heard == heard && floor_sets.iter().all(|s| s.heard >= heard) => {
                        Next::AsRead(floor_sets, stop)
                    }
                    _ => {
                        let (tx, rx) = mpsc::unbounded_channel();
                        st.reading = Some(tx);
                        Next::Read(rx)
                    }
                }
            }
        };
        match next {
            Next::Nothing => {}
            Next::Read(rx) => read(reaction.clone(), None, rx, forced).await,
            Next::AsRead(sets, stop) => as_read(&reaction, sets, stop).await,
        }
    }
}

/// Release floor sets against the latest stop's reading: they were written with everything
/// they have said, so each fits by construction, and only whether they are done decides. An
/// unread stop is them done — what the floor did before it asked, for a set written after their
/// last line.
async fn as_read(reaction: &Reaction, sets: Vec<Set>, stop: Stop) {
    let started = Instant::now();
    let prepared = &reaction.inner.prepared;
    let Some(runner) = prepared.lock().runner.clone() else { return };
    let floors = floor_options(&sets);
    let fits = vec![Some(Fits::Say); floors.len()];
    let done = stop.finished.is_none_or(|p| p >= cut(FINISHED_KEY, FINISHED_AT, 0.0));
    let out = release_floor(reaction, &runner, &sets, &floors, &fits, done, false, false, stop.heard, started).await;
    prepared.apply(&out.gone, &[], &out.held);
    let pending = prepared.lock().sets.iter().any(|s| s.branch(&When::Finished).is_some());
    if pending && !done {
        prepared.arm_wait(if out.acknowledged { PAUSE_RELEASE } else { THOUGHT_RELEASE });
    }
    reaction
        .inner
        .observatory
        .record(EventKind::FloorRead {
            lines: 0,
            forced: false,
            finished: stop.finished,
            outcome: if done { "finished, as the last stop read".into() } else { "has more, as the last stop read".into() },
            verdicts: out.verdicts,
            released: out.released,
            model: None,
            decided_ms: started.elapsed().as_millis() as u64,
        })
        .await;
}

/// Every condition branch of every live matter, flattened: `b1` is `options[0]`, as (set, index
/// into its branches).
fn condition_options(sets: &[Set]) -> Vec<(usize, usize)> {
    sets.iter()
        .enumerate()
        .filter(|(_, s)| s.conditions_live())
        .flat_map(|(k, s)| {
            s.branches.iter().enumerate().filter(|(_, b)| !b.when.is_floor()).map(move |(i, _)| (k, i))
        })
        .collect()
}

/// Every floor branch, flattened: `fits1` is `options[0]`, as (set, when).
fn floor_options(sets: &[Set]) -> Vec<(usize, When)> {
    sets.iter()
        .enumerate()
        .flat_map(|(k, s)| s.branches.iter().filter(|b| b.when.is_floor()).map(move |b| (k, b.when.clone())))
        .collect()
}

/// What `fits` said about one floor branch.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fits {
    Say,
    Hold,
    Drop,
}

/// One stop: settle the batch if it carries a message, ask everything at once, release what goes,
/// and record it all.
///
/// **The questions are asked as soon as the message lands, and the answer is used only once the
/// batch has closed** — the same settle a turn waits out, held open while they are still talking
/// or typing. A message that joins the batch asks again over the whole of it.
async fn read(reaction: Reaction, first: Option<Message>, mut more: mpsc::UnboundedReceiver<Message>, forced: bool) {
    let arrived = Instant::now();
    let floor = reaction.inner.floor.clone();
    let prepared = &reaction.inner.prepared;
    let sets = prepared.lock().sets.clone();
    let mut messages: Vec<Message> = first.into_iter().collect();
    let recent = reaction.inner.speech.said_since_their_last();
    // Read fresh at every ask: a line that joins the batch is in the ring by the time it is.
    let lines = || prepared.lock().lines.iter().cloned().collect::<Vec<_>>();
    let questions = |messages: &[Message]| {
        Arc::new(questions(&sets, !messages.is_empty() && enabled(), forced, !messages.is_empty()))
    };
    let ask_now = |messages: &[Message]| {
        ask(state(&sets, &recent, &lines(), messages), questions(messages))
    };
    let mut asking = ask_now(&messages);
    let as_batch = |messages: &[Message]| -> Vec<LoopInput> {
        messages.iter().cloned().map(LoopInput::Message).collect()
    };
    let mut room = None;
    if !messages.is_empty() {
        // **A microphone hears the whole room**, and a line nobody spoke to the agent is not
        // their message: it must neither run a branch nor count as a stop. So a batch that is
        // only room asks the room screen's own question beside this one ([`super::room`]).
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
                        asking = ask_now(&messages);
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
        if !super::room::wakes(&mut room, &reaction.inner.memory, &as_batch(&messages)).await {
            if let Some(a) = asking {
                a.abort();
            }
            record_resolved(&reaction, &messages, "side talk", None, &Reading::default(), arrived, None, &[], &[]).await;
            finish(&reaction, None, more);
            return;
        }
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
    // Everything they had said by the time the batch closed.
    let heard_now = floor.heard();
    if let Err(Unread::Error(err)) = &answer {
        tracing::warn!(error = %err, "floor: the reading failed; it does what it did before it asked");
    }

    // They started again while it was being read: nothing goes out, and their line starts a
    // reading of its own.
    let now = Instant::now();
    let still_going = floor.voice_active(now).await || floor.typing_active(now).await || !more.is_empty();

    let conds = condition_options(&sets);
    let reading = Reading::of(&answer, conds.len(), sets.len(), cut(THRESHOLD_KEY, THRESHOLD, 0.5));
    let floors = floor_options(&sets);
    let finished_p = noul(&answer, FINISHED);
    let fits: Vec<Option<Fits>> = (1..=floors.len()).map(|n| fits_of(&answer, n)).collect();
    let unread = answer.is_err();
    let done = forced || finished_p.is_some_and(|p| p >= cut(FINISHED_KEY, FINISHED_AT, 0.0));

    let runner = prepared.lock().runner.clone();
    let mut first_action_ms = None;

    // A condition branch their message met goes first: it was prepared for exactly this message.
    let met = reading.met.map(|i| conds[i]);
    let mut ran: Vec<(String, Result<String, Failed>)> = Vec::new();
    let mut outcome = reading.outcome.to_string();
    if let Some((k, i)) = met {
        if still_going {
            outcome = "still going".into();
        } else if let Some(runner) = &runner {
            first_action_ms.get_or_insert(arrived.elapsed().as_millis() as u64);
            ran = run(&reaction, runner, sets[k].answering, &sets[k].branches[i]).await;
        } else {
            outcome = "no mouth".into();
        }
    }

    let floor_out = if still_going {
        FloorOut::default()
    } else {
        match &runner {
            Some(runner) => {
                release_floor(&reaction, runner, &sets, &floors, &fits, done, unread, forced, heard_now, arrived).await
            }
            None => FloorOut::default(),
        }
    };
    if let Some(ms) = floor_out.first_action_ms {
        first_action_ms.get_or_insert(ms);
    }
    let FloorOut { gone, held, verdicts, released, acknowledged, waiting, .. } = floor_out;
    if !messages.is_empty() || !floors.is_empty() {
        // What this stop said about whether they are done, for a set prepared before the next
        // line of theirs: it is read against this, at once, rather than waiting on a stop that
        // will not come. The wait running out is them done.
        prepared.lock().last_stop = Some(Stop {
            heard: heard_now,
            finished: if forced { Some(1.0) } else { finished_p },
        });
    }

    // What their message took up, of the condition matters: used up whether or not a branch ran.
    let used_up: Vec<u64> = sets
        .iter()
        .enumerate()
        .filter(|(k, s)| {
            s.conditions_live()
                && (met.is_some_and(|(m, _)| m == *k) || reading.on.get(*k).copied().flatten().is_some_and(|p| p >= TAKEN_UP))
        })
        .map(|(_, s)| s.id)
        .collect();
    prepared.apply(&gone, &used_up, &held);

    // Anything `finished` still waiting on a stop that did not release it arms the one wait.
    let pending = prepared.lock().sets.iter().any(|s| s.branch(&When::Finished).is_some());
    if pending && (waiting || !done) && !still_going && !forced {
        prepared.arm_wait(if acknowledged { PAUSE_RELEASE } else { THOUGHT_RELEASE });
    }

    if !messages.is_empty() && !conds.is_empty() {
        record_resolved(&reaction, &messages, &outcome, met.map(|(k, i)| (&sets[k], i)), &reading, arrived, first_action_ms, &ran, &used_up_names(&sets, &used_up)).await;
    }
    if !floors.is_empty() || !messages.is_empty() {
        reaction
            .inner
            .observatory
            .record(EventKind::FloorRead {
                lines: messages.len(),
                forced,
                finished: finished_p,
                outcome: match (&answer, still_going) {
                    (_, true) => "still going".into(),
                    (Err(Unread::Timeout), _) => "timeout".into(),
                    (Err(Unread::Unavailable), _) => "unavailable".into(),
                    (Err(Unread::Error(_)), _) => "error".into(),
                    (Ok(_), _) if done => "finished".into(),
                    (Ok(_), _) => "has more".into(),
                },
                verdicts,
                released,
                model: answer.as_ref().ok().map(|r| r.model.clone()),
                decided_ms,
            })
            .await;
    }
    tracing::info!(
        finished = ?finished_p,
        forced,
        unread,
        still_going,
        decided_ms,
        "floor: a stop read"
    );
    let ran_for_cognition = match met {
        Some((k, i)) if !ran.is_empty() => {
            prepared.tell(told_ran(&sets[k].matter, sets[k].branches[i].when.as_str(), &ran), ran.iter().any(|(_, r)| r.as_ref().is_err_and(|f| f.judgment)));
            Some(Ran { condition: sets[k].branches[i].when.as_str().to_string(), ran: ran.iter().map(|(a, r)| (a.clone(), r.clone().map_err(|f| f.why))).collect() })
        }
        _ => None,
    };
    finish(&reaction, ran_for_cognition, more);
}

/// What one stop released, held and dropped of the floor sets, for the caller to apply.
#[derive(Default)]
struct FloorOut {
    gone: Vec<(u64, Vec<When>)>,
    held: Vec<u64>,
    verdicts: Vec<String>,
    released: Vec<String>,
    acknowledged: bool,
    waiting: bool,
    first_action_ms: Option<u64>,
}

/// Release what this stop lets out of the floor sets: when they are `done`, each `finished`
/// set its `fits` says is `say`, in the order prepared; when they have more, the newest `paused`
/// that fits, at most one. `unread` is a reading that could not be read, which does what the
/// floor did before it asked.
#[allow(clippy::too_many_arguments)]
async fn release_floor(
    reaction: &Reaction,
    runner: &Runner,
    sets: &[Set],
    floors: &[(usize, When)],
    fits: &[Option<Fits>],
    done: bool,
    unread: bool,
    forced: bool,
    heard_now: u64,
    arrived: Instant,
) -> FloorOut {
    let prepared = &reaction.inner.prepared;
    let mut out = FloorOut::default();
    // `finished`, in the order prepared.
    for (n, (k, when)) in floors.iter().enumerate() {
        if *when != When::Finished {
            continue;
        }
        let set = &sets[*k];
        let verdict = match fits[n] {
            // Unread: what it did before it asked — a set written with everything they
            // have said goes if they are done or cannot be read otherwise; one written
            // before a line of theirs waits.
            None if unread => Some(if forced || set.heard >= heard_now { Fits::Say } else { Fits::Hold }),
            None => Some(Fits::Say),
            some => some,
        };
        if !(done || unread) {
            out.waiting = true;
            continue;
        }
        match verdict {
            Some(Fits::Say) => {
                out.first_action_ms.get_or_insert(arrived.elapsed().as_millis() as u64);
                let branch = set.branch(&When::Finished).expect("a finished option has a finished branch");
                let results = run(reaction, runner, set.answering, branch).await;
                prepared.tell(told_ran(&set.matter, "finished", &results), results.iter().any(|(_, r)| r.as_ref().is_err_and(|f| f.judgment)));
                out.released.push(format!("{} (finished)", set.matter));
                out.gone.push((set.id, vec![When::Finished, When::Paused]));
            }
            Some(Fits::Hold) if unread => out.waiting = true,
            Some(Fits::Hold) => {
                out.waiting = true;
                if !set.held_told {
                    prepared.tell(
                        format!("- \"{}\" (finished): held — at this stop it did not fit where the conversation is; it waits for a later stop", set.matter),
                        true,
                    );
                    out.held.push(set.id);
                }
            }
            Some(Fits::Drop) | None => {
                prepared.tell(
                    format!("- \"{}\" (finished): dropped — what they said after it was written changed what it should be; nothing of it went out", set.matter),
                    false,
                );
                out.gone.push((set.id, vec![When::Finished, When::Paused]));
            }
        }
        out.verdicts.push(format!("{}: {:?}", set.matter, verdict));
    }
    // Not done: the newest `paused` that fits this stop, at most one.
    if !done && !unread {
        if let Some((n, (k, _))) =
            floors.iter().enumerate().rev().find(|(n, (_, w))| *w == When::Paused && fits[*n] == Some(Fits::Say))
        {
            let _ = n;
            let set = &sets[*k];
            let branch = set.branch(&When::Paused).expect("a paused option has a paused branch");
            out.first_action_ms.get_or_insert(arrived.elapsed().as_millis() as u64);
            let results = run(reaction, runner, set.answering, branch).await;
            prepared.tell(told_ran(&set.matter, "paused", &results), false);
            out.released.push(format!("{} (paused)", set.matter));
            out.gone.push((set.id, vec![When::Paused]));
            out.acknowledged = true;
        }
    }
    out
}

fn used_up_names(sets: &[Set], ids: &[u64]) -> Vec<String> {
    sets.iter().filter(|s| ids.contains(&s.id)).map(|s| s.matter.clone()).collect()
}

/// A reading ended: free the slot, and read again — a line of theirs that landed after its
/// batch closed is a stop of its own, and a floor set prepared while it ran is waiting to be read.
///
/// **The slot is freed and the leftovers drained under one lock**, so a line landing now either
/// reached this reading's channel before the slot was freed — and is drained here — or finds the
/// slot free and starts a reading itself. There is no moment in which it can reach neither.
fn finish(reaction: &Reaction, ran: Option<Ran>, mut more: mpsc::UnboundedReceiver<Message>) {
    let prepared = &reaction.inner.prepared;
    let (again, leftover) = {
        let mut st = prepared.lock();
        st.reading = None;
        if ran.is_some() {
            st.ran = ran;
        }
        let mut leftover = Vec::new();
        while let Ok(m) = more.try_recv() {
            leftover.push(m);
        }
        let leftover = if leftover.is_empty() {
            None
        } else {
            let (tx, rx) = mpsc::unbounded_channel();
            for m in leftover.iter().skip(1) {
                let _ = tx.send(m.clone());
            }
            st.reading = Some(tx);
            Some((leftover.swap_remove(0), rx))
        };
        let again = std::mem::take(&mut st.again).then(|| st.runner.as_ref().map(|r| r.kick.clone())).flatten();
        (again, leftover)
    };
    match leftover {
        Some((first, rx)) => {
            tokio::spawn(read(reaction.clone(), Some(first), rx, false));
        }
        None => {
            prepared.idle.send_replace(true);
        }
    }
    if let Some(kick) = again {
        let _ = kick.send(Kick::Prepared);
    }
}

#[allow(clippy::too_many_arguments)]
async fn record_resolved(
    reaction: &Reaction,
    messages: &[Message],
    outcome: &str,
    met: Option<(&Set, usize)>,
    reading: &Reading,
    arrived: Instant,
    first_action_ms: Option<u64>,
    ran: &[(String, Result<String, Failed>)],
    used_up: &[String],
) {
    reaction
        .inner
        .observatory
        .record(EventKind::BranchesResolved {
            message: render_messages(messages),
            outcome: outcome.to_string(),
            matter: met.map(|(s, _)| s.matter.clone()),
            direction: met.map(|(s, i)| s.branches[i].when.as_str().to_string()),
            p: reading.p,
            qualified: reading.qualified,
            mass: reading.mass.clone(),
            model: reading.model.clone(),
            decided_ms: arrived.elapsed().as_millis() as u64,
            first_action_ms,
            ran: ran
                .iter()
                .map(|(action, result)| match result {
                    Ok(done) => format!("{action}: {done}"),
                    Err(f) => format!("{action}: {}", f.why),
                })
                .collect(),
            used_up: used_up.to_vec(),
        })
        .await;
}

/// Ask the questions, inside the budget. `None` when nothing is configured to ask — then the
/// reading is unread before it begins.
fn ask(state: String, questions: Arc<Map<String, Value>>) -> Option<JoinHandle<anyhow::Result<decision::Reply>>> {
    if !decision::available() || questions.is_empty() {
        return None;
    }
    let state = Value::String(state);
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

fn noul(answer: &Result<decision::Reply, Unread>, key: &str) -> Option<f64> {
    match answer.as_ref().ok()?.answers.get(key) {
        Some(decision::Answer::Noul { p }) => Some(*p),
        _ => None,
    }
}

/// `fits{n}` as read: the option carrying the most mass.
fn fits_of(answer: &Result<decision::Reply, Unread>, n: usize) -> Option<Fits> {
    let reply = answer.as_ref().ok()?;
    let Some(decision::Answer::Choice { choice, probabilities, .. }) = reply.answers.get(&format!("{FITS}{n}")) else {
        return None;
    };
    let best = match probabilities {
        Some(Value::Object(mass)) => mass
            .iter()
            .filter_map(|(o, p)| Some((o.as_str(), p.as_f64()?)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(o, _)| o.to_string()),
        _ => None,
    }
    .unwrap_or_else(|| choice.clone());
    match best.as_str() {
        "say" => Some(Fits::Say),
        "hold" => Some(Fits::Hold),
        "drop" => Some(Fits::Drop),
        _ => None,
    }
}

/// A condition reading: which branch was met, if any, which matters the message took up, and
/// every number that decided it.
#[derive(Debug, Default, PartialEq)]
struct Reading {
    /// Into the flattened condition options ([`condition_options`]).
    met: Option<usize>,
    outcome: &'static str,
    p: Option<f64>,
    qualified: Option<f64>,
    /// Per set, in the sets' order: how sure it is the message took that matter up.
    on: Vec<Option<f64>>,
    mass: Option<Value>,
    model: Option<String>,
    error: Option<String>,
}

impl Reading {
    /// A branch is met when its option carries at least `tau` of the choice's mass and
    /// `qualified` carries at most `1 − tau`. A reply that cannot be read runs nothing and uses
    /// nothing up.
    fn of(answer: &Result<decision::Reply, Unread>, branches: usize, matters: usize, tau: f64) -> Self {
        let reply = match answer {
            Ok(reply) => reply,
            Err(Unread::Unavailable) => return Self { outcome: "unavailable", ..Self::default() },
            Err(Unread::Timeout) => return Self { outcome: "timeout", ..Self::default() },
            Err(Unread::Error(e)) => return Self { outcome: "error", error: Some(e.clone()), ..Self::default() },
        };
        if branches == 0 {
            return Self { outcome: "none asked", ..Self::default() };
        }
        let model = Some(reply.model.clone());
        let noul = |key: &str| match reply.answers.get(key) {
            Some(decision::Answer::Noul { p }) => Some(*p),
            _ => None,
        };
        let qualified = noul(QUALIFIED);
        let on: Vec<Option<f64>> = (1..=matters).map(|k| noul(&format!("{ON}{k}"))).collect();
        let Some(decision::Answer::Choice { probabilities: Some(Value::Object(mass)), .. }) = reply.answers.get(WHICH) else {
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
        let read = |outcome, met| Self { met, outcome, p: Some(p), qualified, on: on.clone(), mass: whole.clone(), model: model.clone(), error: None };
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

/// The questions, in `judges/prepared.md`'s words: whether they are finished (unless the wait ran
/// out), whether each floor branch fits this stop, and — when the stop carries a message and
/// condition branches are live — the condition choice, whether it was qualified, and per matter
/// whether it was taken up. The `on` keys are numbered by set, so a set with no live conditions
/// is simply not asked about.
fn questions(sets: &[Set], conditions: bool, forced: bool, stop: bool) -> Map<String, Value> {
    let rubric = crate::identity::judges::PREPARED;
    let part = |heading: &str| crate::identity::rubric_section(rubric, heading).unwrap_or_default();
    let frame = part("Frame");
    let mut asked = Map::new();
    let floors = floor_options(sets);
    if (stop || !floors.is_empty()) && !forced {
        asked.insert(FINISHED.into(), json!({ "type": "noul", "instructions": format!("{frame} {}", part("Finished")) }));
    }
    let fits_criteria = json!({ "say": part("Say"), "hold": part("Hold"), "drop": part("Drop") });
    for (n, (k, when)) in floors.iter().enumerate() {
        let set = &sets[*k];
        let moment = match when {
            When::Paused => part("Fits paused"),
            _ => part("Fits finished"),
        };
        let lines = set.branch(when).map(|b| b.actions.iter().map(Action::describe).collect::<Vec<_>>().join("; ")).unwrap_or_default();
        asked.insert(
            format!("{FITS}{}", n + 1),
            json!({
                "type": "choice",
                "instructions": format!("{frame} {moment} The matter: {} — what is ready: {lines}", set.matter),
                "criteria": fits_criteria,
            }),
        );
    }
    let conds = condition_options(sets);
    if conditions && !conds.is_empty() {
        let mut criteria = Map::new();
        for (n, (k, i)) in conds.iter().enumerate() {
            let set = &sets[*k];
            criteria.insert(format!("b{}", n + 1), Value::from(format!("{} — {}", set.matter, set.branches[*i].when.as_str())));
        }
        criteria.insert(REST.into(), Value::from(part("Rest")));
        asked.insert(WHICH.into(), json!({ "type": "choice", "instructions": format!("{frame} {}", part("Which way")), "criteria": criteria }));
        asked.insert(QUALIFIED.into(), json!({ "type": "noul", "instructions": format!("{frame} {}", part("Qualified")) }));
        for (k, set) in sets.iter().enumerate() {
            if set.conditions_live() {
                asked.insert(
                    format!("{ON}{}", k + 1),
                    json!({ "type": "noul", "instructions": format!("{frame} {} The matter: {}", part("Took it up"), set.matter) }),
                );
            }
        }
    }
    asked
}

/// The state a reading is about: where each matter was left and what is ready for it, with what
/// they said after it was written; what the agent said most recently; then what they said at this
/// stop. Nothing else — System One loses accuracy on a padded state.
fn state(sets: &[Set], recent: &[String], lines: &[(u64, String)], messages: &[Message]) -> String {
    let said = |said: &[String]| -> String { said.iter().map(|l| format!("< {}\n", l.replace('\n', "\n  "))).collect() };
    let mut s = String::from("## Where each matter was left\n");
    for set in sets {
        s.push_str(&format!("### {}\n", set.matter));
        if set.said.is_empty() {
            s.push_str("(nothing the assistant said)\n");
        } else if set.said == recent {
            s.push_str("(the most recent lines, below)\n");
        } else {
            s.push_str(&said(&set.said));
        }
        if set.branches.iter().any(|b| b.when.is_floor()) {
            let since: Vec<&str> = lines.iter().filter(|(i, _)| *i > set.heard).map(|(_, l)| l.as_str()).collect();
            if since.is_empty() {
                s.push_str("(written after everything the person has said)\n");
            } else {
                s.push_str("What the person said after this was written:\n");
                for l in since {
                    s.push_str(&format!("> {}\n", l.replace('\n', "\n  ")));
                }
            }
        }
    }
    s.push_str("\n## What the assistant said most recently\n");
    if recent.is_empty() {
        s.push_str("(nothing since the person last spoke)\n");
    }
    s.push_str(&said(recent));
    let earlier: Vec<&str> = lines
        .iter()
        .rev()
        .skip(messages.len())
        .take(RECENT_LINES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|(_, l)| l.as_str())
        .collect();
    if !earlier.is_empty() {
        s.push_str("\n## What the person said before this stop\n");
        for l in earlier {
            s.push_str(&format!("> {}\n", l.replace('\n', "\n  ")));
        }
    }
    s.push_str("\n## What the person said at this stop\n");
    if messages.is_empty() {
        s.push_str("(nothing new — they have stopped)\n");
    } else {
        s.push_str(&render_messages(messages));
        s.push('\n');
    }
    s
}

fn render_messages(messages: &[Message]) -> String {
    messages.iter().map(super::render_message_line).collect::<Vec<_>>().join("\n")
}

/// Why an action did not happen as written, and whether that waits on Reaction's judgment. The
/// cap on messages since their last does not: nothing Reaction decides can send it until they
/// write, and their message is the wake that carries it.
#[derive(Clone, Debug)]
pub(super) struct Failed {
    why: String,
    judgment: bool,
}

impl Failed {
    fn judged(why: String) -> Self {
        Self { why, judgment: true }
    }
}

/// Run a released branch's actions in order, through the seams they always went through. **The
/// first action that does not happen stops the rest**, because a later one may lean on it —
/// 好，我把数据摆上来 after a show that failed, or went into their list, is a false line.
///
/// One release at a time, each bracketed as a sequencer turn of its own under a fresh id, so its
/// voice span, a barge-in on it, and the reply the floor remembers are its own.
async fn run(reaction: &Reaction, runner: &Runner, answering: bool, branch: &Branch) -> Vec<(String, Result<String, Failed>)> {
    use crate::foundation::server::view_bus::{AWAY_FOR, Claim, ShowFrom};
    let prepared = &reaction.inner.prepared;
    let _one_at_a_time = prepared.releasing.lock().await;
    let turn = reaction.inner.turn_seq.fetch_add(1, Ordering::Relaxed);
    let _ = runner.beats.send(Beat::TurnStart { turn }).await;
    let data_dir = reaction.inner.memory.data_dir().to_path_buf();
    let unanswered = &reaction.inner.unanswered;
    let mut ran = Vec::with_capacity(branch.actions.len());
    for action in &branch.actions {
        let result: Result<String, Failed> = match action {
            Action::Say { text, hands } => {
                if !unanswered.fits(1 + hands.len() as u64) {
                    Err(Failed {
                        why: format!(
                            "not sent — {} messages go out at most since their last one, and this would pass it",
                            super::unanswered::MAX_UNANSWERED
                        ),
                        judgment: false,
                    })
                } else if runner.beats.send(Beat::Say(text.clone())).await.is_err() {
                    Err(Failed::judged("not sent — the sequencer is gone".into()))
                } else {
                    reaction.inner.speech.note_sent(text);
                    unanswered.note_sent();
                    for file in hands {
                        let _ = runner.beats.send(Beat::Hand(file.clone())).await;
                        unanswered.note_sent();
                    }
                    Ok("sent".to_string())
                }
            }
            Action::Show { id, op, view_ref, source } => {
                // Resolved again now rather than trusted from when it was prepared: a view is
                // live, and what it was is not what it is. An attachment has no source.
                let source = match view_ref {
                    Some(r) if crate::foundation::attachments::ref_id(r).is_some() => Ok(String::new()),
                    Some(r) => crate::mind::views::resolve_ref(&data_dir, r)
                        .await
                        .map_err(|e| Failed::judged(format!("not shown — `ref` {r}: {e}"))),
                    None => Ok(source.clone()),
                };
                match source {
                    Err(why) => Err(why),
                    Ok(source) => {
                        let claim = if op == "dismiss" {
                            Claim::Takes
                        } else {
                            let back_at = reaction.inner.attachments.back_from_away(AWAY_FOR);
                            reaction.inner.views.claim(id.as_deref(), view_ref.as_deref(), ShowFrom { turn, answering }, back_at).await
                        };
                        let named = view_ref.as_deref().or(id.as_deref()).unwrap_or("an inline view").to_owned();
                        let keep = matches!(claim, Claim::Keeps { .. });
                        let beat = Beat::Show { id: id.clone(), op: op.clone(), source, view_ref: view_ref.clone(), keep };
                        if runner.beats.send(beat).await.is_err() {
                            Err(Failed::judged("not shown — the sequencer is gone".into()))
                        } else {
                            match claim {
                                Claim::Takes => {
                                    reaction.inner.speech.note_shown(&format!("{op} {named}"));
                                    Ok("shown".to_string())
                                }
                                Claim::Keeps { reading } => {
                                    reaction.inner.speech.note_shown(&format!("{op} {named} — into their list; the screen stayed on {reading}"));
                                    Err(Failed::judged(format!(
                                        "shown into their list, not in front of them: they are still on \"{reading}\", \
                                         which only just went up, so the screen stayed there — the rest did not run"
                                    )))
                                }
                            }
                        }
                    }
                }
            }
            Action::Send { to, message } => {
                let delivery = registry::global().send(&runner.from, to, message.clone());
                reaction
                    .inner
                    .observatory
                    .record(EventKind::MessageSent { from: Some(runner.from.clone()), to: to.clone(), delivery, message: message.clone() })
                    .await;
                match delivery {
                    Delivery::Delivered => Ok("delivered".to_string()),
                    Delivery::Unknown => Err(Failed::judged(format!("not delivered — nothing live at `{to}`"))),
                    Delivery::UnknownSender => Err(Failed::judged("not delivered — this session is no longer registered".into())),
                    Delivery::NotPermitted => Err(Failed::judged(format!("not delivered — `{to}` is not reachable from here"))),
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

/// A released branch as Reaction is told it: the matter, the moment, each action and what became
/// of it. Facts only — what to do about it is `reaction.md`'s.
fn told_ran(matter: &str, when: &str, ran: &[(String, Result<String, Failed>)]) -> String {
    let mut s = format!("- \"{matter}\" ({when}) went:");
    for (action, result) in ran {
        match result {
            Ok(done) => s.push_str(&format!("\n  - {action}: {done}")),
            Err(f) => s.push_str(&format!("\n  - {action}: {}", f.why)),
        }
    }
    if ran.last().is_some_and(|(_, r)| r.is_err()) {
        s.push_str("\n  It stopped there; nothing after that action ran.");
    }
    s
}

/// A condition branch that ran on their message, as Cognition reads it beside the hand-down of
/// the same message. The branch may have handed work on seconds before the turn did, and a
/// message in the inbox twice reads as two requests unless something says they are one.
pub(super) struct Ran {
    condition: String,
    ran: Vec<(String, Result<String, String>)>,
}

impl Ran {
    pub(super) fn for_cognition(&self) -> String {
        let lines = self
            .ran
            .iter()
            .map(|(action, result)| match result {
                Ok(done) => format!("- {action}: {done}"),
                Err(why) => format!("- {action}: {why}"),
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\n(Before Reaction's turn, a branch it had prepared for this message going this way — \"{}\" — already ran:\n{lines})",
            self.condition
        )
    }
}

fn clip(s: &str, max: usize) -> String {
    let flat = s.replace('\n', " ");
    if flat.chars().count() <= max { flat } else { format!("{}…", flat.chars().take(max).collect::<String>()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn reply(answers: Vec<(&str, decision::Answer)>) -> decision::Reply {
        let answers: BTreeMap<String, decision::Answer> = answers.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        decision::Reply { model: "jev-1.13.0".into(), answers, usage: decision::Usage::default() }
    }

    fn choice(mass: Value) -> decision::Answer {
        decision::Answer::Choice { choice: String::new(), probabilities: Some(mass), confidence: None }
    }

    fn read(which: Value, qualified: Option<f64>) -> Reading {
        let mut answers = vec![(WHICH, choice(which)), ("on1", decision::Answer::Noul { p: 0.9 })];
        if let Some(p) = qualified {
            answers.push((QUALIFIED, decision::Answer::Noul { p }));
        }
        Reading::of(&Ok(reply(answers)), 2, 1, 0.9)
    }

    fn say(text: &str) -> Vec<Action> {
        vec![Action::Say { text: text.into(), hands: Vec::new() }]
    }

    fn branch(when: &str) -> Branch {
        Branch { when: When::parse(when), actions: say("好") }
    }

    fn set(matter: &str, whens: &[&str], said: &[&str]) -> Set {
        Set {
            matter: matter.into(),
            branches: whens.iter().map(|c| branch(c)).collect(),
            said: said.iter().map(|s| s.to_string()).collect(),
            at: Utc::now(),
            heard: 0,
            answering: true,
            held_told: false,
            id: 0,
        }
    }

    #[test]
    fn a_condition_runs_only_when_it_carries_the_mass_and_nothing_qualifies_it() {
        let met = read(json!({"b1": 0.95, "b2": 0.03, "rest": 0.02}), Some(0.04));
        assert_eq!((met.met, met.outcome), (Some(0), "met"));
        assert_eq!(read(json!({"b1": 0.7, "b2": 0.2, "rest": 0.1}), Some(0.04)).outcome, "below");
        // 对，不过预算砍一半 — agreement with a condition attached does not run the branch.
        assert_eq!(read(json!({"b1": 0.95, "b2": 0.03, "rest": 0.02}), Some(0.6)).outcome, "qualified");
        let rest = read(json!({"b1": 0.1, "b2": 0.1, "rest": 0.8}), Some(0.1));
        assert_eq!((rest.met, rest.outcome), (None, "rest"));
        assert_eq!(rest.on, vec![Some(0.9)]);
    }

    #[test]
    fn a_reply_that_cannot_be_read_runs_nothing_and_uses_nothing_up() {
        assert_eq!(read(json!({"b1": 0.99}), None).outcome, "error", "no `qualified`, no vouching");
        assert_eq!(read(json!({"b7": 0.99}), Some(0.0)).outcome, "error", "an option nobody asked");
        let timeout = Reading::of(&Err(Unread::Timeout), 2, 1, 0.9);
        assert_eq!((timeout.outcome, timeout.met), ("timeout", None));
        assert!(timeout.on.is_empty(), "nothing known about where it went, so nothing is used up");
        assert_eq!(Reading::of(&Ok(reply(vec![])), 0, 1, 0.9).outcome, "none asked");
    }

    #[test]
    fn fits_reads_the_heaviest_option() {
        let r = Ok(reply(vec![("fits1", choice(json!({"say": 0.2, "hold": 0.7, "drop": 0.1}))), ("fits2", choice(json!({"say": 0.9, "hold": 0.05, "drop": 0.05})))]));
        assert_eq!(fits_of(&r, 1), Some(Fits::Hold));
        assert_eq!(fits_of(&r, 2), Some(Fits::Say));
        assert_eq!(fits_of(&r, 3), None);
        assert_eq!(fits_of(&Err(Unread::Timeout), 1), None);
    }

    #[test]
    fn the_questions_ask_finished_and_fits_per_floor_branch_and_conditions_only_when_live() {
        let sets = vec![
            set("Attention 的复杂度", &["finished", "paused"], &[]),
            set("A 还是 B", &["同意 A", "选 B"], &[]),
            set("部署", &["finished", "要并进值守"], &[]),
        ];
        let asked = questions(&sets, true, false, true);
        assert_eq!(asked[FINISHED]["type"], "noul");
        assert!(asked["fits1"]["instructions"].as_str().unwrap().contains("Attention 的复杂度"));
        assert_eq!(asked["fits3"]["criteria"].as_object().unwrap().len(), 3, "say, hold, drop");
        assert!(asked.get("fits4").is_none(), "three floor branches");
        // The deploy set's condition waits for its answer to go; A/B's is live.
        let criteria = asked[WHICH]["criteria"].as_object().unwrap();
        assert_eq!(criteria.keys().collect::<Vec<_>>(), vec!["b1", "b2", "rest"]);
        assert_eq!(criteria["b1"], "A 还是 B — 同意 A");
        assert!(asked.get("on2").is_some() && asked.get("on3").is_none(), "only live matters are asked about");
        // No message at this stop: no condition questions. The wait ran out: no `finished`.
        let quiet = questions(&sets, false, true, false);
        // A stop with nothing prepared still asks whether they are done — a reply is being written.
        assert!(questions(&[], false, false, true).contains_key(FINISHED));
        assert!(questions(&[], false, false, false).is_empty());
        assert!(quiet.get(WHICH).is_none() && quiet.get(FINISHED).is_none());
        assert!(quiet.get("fits1").is_some());
        assert_eq!(condition_options(&sets), vec![(1, 0), (1, 1)]);
        assert_eq!(floor_options(&sets).len(), 3);
    }

    /// Every section a question is built from exists in the rubric — a heading missing there
    /// sends a question with no words in it, and nothing fails.
    #[test]
    fn every_question_has_its_words() {
        for heading in ["Frame", "Finished", "Fits finished", "Fits paused", "Say", "Hold", "Drop", "Which way", "Rest", "Qualified", "Took it up"] {
            let words = crate::identity::rubric_section(crate::identity::judges::PREPARED, heading);
            assert!(words.is_some_and(|w| !w.trim().is_empty()), "judges/prepared.md has no `## {heading}`");
        }
    }

    #[test]
    fn a_held_set_is_read_with_what_they_said_after_it_was_written() {
        let mut s = set("Attention", &["finished"], &[]);
        s.heard = 2;
        let lines = vec![(1, "旧的".to_string()), (2, "问了 O(N²)".to_string()), (3, "然后换来了更统一的结构".to_string())];
        let st = state(&[s.clone()], &[], &lines, &[]);
        assert!(st.contains("What the person said after this was written:\n> 然后换来了更统一的结构"), "{st}");
        let since = st.split("What the person said after this was written:").nth(1).unwrap().split("\n##").next().unwrap();
        assert!(!since.contains("问了 O(N²)"), "what it saw is not repeated as new: {st}");
        // What `finished` reads against when the stop itself carried nothing new.
        assert!(st.contains("## What the person said before this stop\n> 旧的\n> 问了 O(N²)\n> 然后换来了更统一的结构"), "{st}");
        assert!(st.contains("(nothing new — they have stopped)"));
        s.heard = 3;
        assert!(state(&[s], &[], &lines, &[]).contains("(written after everything the person has said)"));
    }

    #[tokio::test]
    async fn a_call_is_one_matter_checked_whole() {
        let dir = std::env::temp_dir();
        let (matter, ok) = parse(
            &json!({"matter": " A 还是 B ", "branches": [
                {"when": "finished", "actions": [{"do": "say", "text": "好，A。"}]},
                {"when": "agrees to A", "actions": [{"do": "say", "text": "开工"}]}
            ]}),
            &dir,
            400,
        )
        .await
        .unwrap();
        assert_eq!(matter, "A 还是 B");
        assert_eq!(ok[0].when, When::Finished);
        assert_eq!(ok[1].when, When::Condition("agrees to A".into()));
        assert_eq!(ok[0].actions, say("好，A。"));

        let refused = |args: Value| {
            let dir = dir.clone();
            async move { parse(&args, &dir, 400).await.unwrap_err() }
        };
        assert!(refused(json!({"branches": []})).await.contains("`matter`"));
        assert!(refused(json!({"matter": "x"})).await.contains("branches"));
        assert!(refused(json!({"matter": "x", "branches": [{"actions": []}]})).await.contains("no `when`"));
        let twice = json!({"matter": "x", "branches": [
            {"when": "finished", "actions": [{"do": "say", "text": "a"}]},
            {"when": "finished", "actions": [{"do": "say", "text": "b"}]}]});
        assert!(refused(twice).await.contains("one `finished` branch"));
        for empty in [json!({"when": "picks B"}), json!({"when": "picks B", "actions": []})] {
            let why = refused(json!({"matter": "x", "branches": [empty]})).await;
            assert!(why.contains("no actions") && why.contains("Nothing was prepared"), "{why}");
        }
        let long = refused(json!({"matter": "x", "branches": [{"when": "finished", "actions": [{"do": "say", "text": "字".repeat(401)}]}]})).await;
        assert!(long.contains("too long"), "{long}");
        assert!(refused(json!({"matter": "x", "branches": [{"when": "x", "actions": [{"do": "shell"}]}]})).await.contains("not an action"));
        assert!(refused(json!({"matter": "x", "branches": [{"when": "x", "actions": [{"tool": "hi_say"}]}]})).await.contains("`do`"));
        assert!(refused(json!({"matter": "x", "branches": [{"when": "x", "actions": [{"do": "show"}]}]})).await.contains("needs a `ref`"));
        assert!(refused(json!({"matter": "x", "branches": [{"when": "x", "actions": [{"do": "send_message", "to": "nobody-here", "message": "go"}]}]})).await.contains("nothing live"));
        let fan: Vec<Value> = (0..=MAX_BRANCHES).map(|i| json!({"when": format!("d{i}"), "actions": [{"do": "say", "text": "好"}]})).collect();
        assert!(refused(json!({"matter": "x", "branches": fan})).await.contains("a fan"));
    }

    fn matters(p: &Prepared) -> Vec<String> {
        p.lock().sets.iter().map(|s| s.matter.clone()).collect()
    }

    #[tokio::test]
    async fn matters_stand_side_by_side_and_one_prepared_again_is_replaced() {
        let p = Prepared::new(Observatory::new(None), None);
        p.set("A 还是 B", vec![branch("同意 A")], vec!["我倾向 A".into()], 0, true).await;
        p.set("VLX", vec![branch("finished")], vec![], 0, true).await;
        assert_eq!(matters(&p), vec!["A 还是 B", "VLX"]);
        // The same matter, spaced differently, is the same matter: replaced, and newest last.
        p.set("A还是B", vec![branch("选 B")], vec![], 0, true).await;
        assert_eq!(matters(&p), vec!["VLX", "A还是B"]);
        p.set("vlx", vec![], vec![], 0, true).await;
        assert_eq!(matters(&p), vec!["A还是B"], "an empty list clears that matter only");
    }

    #[tokio::test]
    async fn only_condition_matters_are_bounded() {
        let p = Prepared::new(Observatory::new(None), None);
        for m in ["一", "二", "三", "四"] {
            assert!(p.set(m, vec![branch("x")], vec![], 0, true).await.is_empty());
        }
        // A reply is not a bet on the future, and does not push one out.
        assert!(p.set("答", vec![branch("finished")], vec![], 0, true).await.is_empty());
        assert_eq!(p.set("五", vec![branch("x")], vec![], 0, true).await, vec!["一"], "past the matters kept");
        let wide: Vec<Branch> = (0..6).map(|i| branch(&format!("d{i}"))).collect();
        assert_eq!(p.set("六", wide, vec![], 0, true).await, vec!["二", "三"], "past the branches kept");
        assert_eq!(matters(&p), vec!["四", "答", "五", "六"]);
    }

    #[tokio::test]
    async fn a_reading_changes_only_what_it_read_and_it_all_outlives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let p = Prepared::new(Observatory::new(None), Some(dir.path()));
        p.set("答", vec![branch("finished"), branch("paused"), branch("要看数")], vec![], 3, true).await;
        p.set("A/B", vec![branch("同意 A")], vec![], 3, true).await;
        drop(p);

        let p = Prepared::new(Observatory::new(None), Some(dir.path()));
        assert_eq!(matters(&p), vec!["答", "A/B"], "kept on disk, in order");
        assert_eq!(p.lock().sets[0].heard, 3);
        let read = p.lock().sets.clone();
        // The answer went: its `finished` and `paused` are gone, its condition is now live.
        p.apply(&[(read[0].id, vec![When::Finished, When::Paused])], &[], &[]);
        assert_eq!(p.lock().sets[0].branches, vec![branch("要看数")]);
        assert!(p.lock().sets[0].conditions_live());
        // Prepared again while a reading ran: a different set, which a use-up of the old one leaves.
        p.set("A/B", vec![branch("选 B")], vec![], 3, true).await;
        p.apply(&[], &[read[1].id], &[]);
        assert_eq!(matters(&p), vec!["答", "A/B"]);
        drop(p);
        let p = Prepared::new(Observatory::new(None), Some(dir.path()));
        assert_eq!(p.lock().sets[1].branches[0].when, When::Condition("选 B".into()));
    }

    #[test]
    fn the_window_lists_what_is_ready_by_moment() {
        let p = Prepared::new(Observatory::new(None), None);
        assert_eq!(p.render(), "## What you have ready\n(nothing prepared)");
        p.lock().sets.push(set("Attention", &["finished", "paused"], &[]));
        let s = p.render();
        assert!(s.contains("### Attention — prepared "), "{s}");
        assert!(s.contains("- finished → say \"好\""), "{s}");
        assert!(s.contains("- paused → say \"好\""), "{s}");
        assert!(s.contains("no branches"), "how to clear one is said: {s}");
    }

    #[test]
    fn what_is_told_says_where_it_stopped_and_taking_it_empties_it() {
        let p = Prepared::new(Observatory::new(None), None);
        assert!(p.take_told().is_none());
        let ran = vec![
            ("show show plan/compare".to_string(), Err(Failed::judged("shown into their list, not in front of them".into()))),
        ];
        p.tell(told_ran("A/B", "finished", &ran), true);
        let told = p.take_told().unwrap();
        assert!(told.starts_with("## What happened to what you prepared\n"), "{told}");
        assert!(told.contains("\"A/B\" (finished) went:") && told.contains("It stopped there"), "{told}");
        assert!(p.take_told().is_none(), "told once");
        p.untake(told.clone());
        assert_eq!(p.take_told().unwrap(), told, "what could not be steered is owed again, whole");
    }

    #[test]
    fn cognition_is_told_the_two_arrivals_are_one_request() {
        let ok = Ran { condition: "agrees".into(), ran: vec![("send_message → cognition: \"按 A 开工\"".into(), Ok("delivered".into()))] };
        let c = ok.for_cognition();
        assert!(c.contains("already ran") && c.contains("按 A 开工\": delivered"), "{c}");
    }

    #[test]
    fn a_moment_round_trips_as_the_words_it_was_written_in() {
        for w in ["finished", "paused", "agrees to A"] {
            let json = serde_json::to_string(&When::parse(w)).unwrap();
            assert_eq!(serde_json::from_str::<When>(&json).unwrap(), When::parse(w));
        }
        let a = Action::Send { to: "cognition".parse().unwrap(), message: "go".into() };
        assert_eq!(serde_json::to_value(&a).unwrap()["do"], "send_message");
    }
}
