//! Prepared branches — `hi_prepare`, and what the person's next message does with it.
//!
//! **`hi_say` is the reply to this moment; `hi_prepare` is where the conversation may go
//! next.** One call is one choice: a few mutually exclusive branches, each a condition — where
//! the person's next message goes, in Reaction's own words — and the actions Reaction would
//! take there, written exactly as it would call them. When that message lands, one System One
//! call says which branch it met and whether it qualified it; a met branch runs at once, in
//! order, stopping at the first action that does not happen; and the message then drives an
//! ordinary turn, which is told what ran. See `docs/arch/agents.md` § *Prepared branches*.
//!
//! **Why this exists.** Everything Reaction does in answer to a message starts with a
//! generation, and a generation has a floor no prompt moves — 13.8 s to a first line, one
//! upstream request over ~100K tokens, nearly all of it fixed cost
//! (`docs/user-journeys/measuring.md`). The only way under it is to have decided before the
//! message came, and to spend the moment it comes only on recognizing which decision it was.
//! A generation cannot recognize in time and its self-reported confidence compares to
//! nothing; System One answers in well under a second with a number that means the same
//! thing on every call, so the cut here is a policy about how often a met branch may be wrong.
//!
//! What keeps it from doing harm:
//!
//! - **a miss runs nothing** and the turn runs as it does today; a timeout, an error or no
//!   System One is a miss, so the worst case is today's speed;
//! - **every action is one of Reaction's own verbs**, none of which reaches outside — words to
//!   the person, a view on their screen, a message to another rung. Anything outward is a
//!   worker's, behind Cognition's judgment, exactly as when the same message is handed down
//!   in a turn;
//! - **a set is good for one message**: that message resolves it, and anything that happens
//!   first voids it — another turn starting, a line sent after it. It lives in memory, so a
//!   restart drops it. None of these is a timer;
//! - **one switch** — `prepared_branches` = `off` prepares nothing; otherwise a met branch runs.
//!
//! **Built and unit-tested; never watched on a live turn** — journey 43 is the spec, and the
//! events it records (`branches_prepared` / `branches_resolved` / `branches_voided`) are the
//! count to read it by.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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

/// How much of the choice's mass a branch must carry to run, and how little `qualified` may.
/// A starting value: every reading's whole mass is recorded, so where the cut should sit is
/// read off what the readings actually were, not argued.
const THRESHOLD: f64 = 0.9;

/// How long the reading may take. System One answered the speech check's questions in p50
/// 0.53 s, p99 2.2 s on this install; a reading asked during the settle has most of this
/// already behind it by the time the batch closes. Past it the message takes today's path.
const BUDGET: Duration = Duration::from_secs(1);

/// More directions than this is a fan, not a guess — and System One loses accuracy on a
/// padded question as it does on a padded state.
const MAX_BRANCHES: usize = 8;

/// How long [`Prepared::settled`] waits for a reading before letting the turn run anyway: the
/// longest a batch can be held open, the reading's budget, and a second of slack. A reading
/// that outlives this is abandoned by nobody — it finishes and records — but no turn waits on it.
const SETTLED_WITHIN: Duration = Duration::from_secs(7);

/// The key the one `choice` is asked under; its branch options are `b1`, `b2`, … in order.
const WHICH: &str = "which";
/// The option meaning none of the branches.
const REST: &str = "rest";
/// The key the one `noul` is asked under.
const QUALIFIED: &str = "qualified";

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
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Say { text: String },
    Show { id: Option<String>, op: String, view_ref: Option<String>, source: String },
    Send { to: SessionSlug, message: String },
}

impl Action {
    /// The action as one line, for the event log and for the turn told what ran.
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

/// One direction: where their next message would go, and what to do there.
#[derive(Clone, Debug, PartialEq)]
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

/// Read a `hi_prepare` call's arguments into branches, **checking every action the way its own
/// tool would** — a line within `say_max`, a `ref` that resolves, a `to` that is live — so a
/// branch that could not run is found in the turn that can still fix it, not at the reply.
/// Any failure refuses the whole call, naming where: a choice with one option missing is a
/// different choice. An empty list is valid, and clears.
pub async fn parse(args: &Value, data_dir: &Path, say_max: usize) -> Result<Vec<Branch>, String> {
    let Some(list) = args.get("branches").and_then(Value::as_array) else {
        return Err("hi_prepare needs `branches`: a list of {condition, actions} — an empty list \
                    clears what is prepared"
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
                "branch {n} has no `condition` — say, in plain words, where their next message \
                 would go. Nothing was prepared."
            ));
        }
        let actions = match b.get("actions") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(list)) => {
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
                actions
            }
            Some(_) => {
                return Err(format!("branch {n}: `actions` is a list. Nothing was prepared."));
            }
        };
        branches.push(Branch { condition: condition.to_string(), actions });
    }
    Ok(branches)
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

/// What a set is read against, kept from the moment it was prepared.
struct Set {
    branches: Vec<Branch>,
    /// What the agent had said since the person last wrote — what their next message answers.
    said: Vec<String>,
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
    set: Option<Set>,
    /// The reading in flight, if any: further messages of the same batch go to it.
    reading: Option<mpsc::UnboundedSender<Message>>,
    /// What the last reading ran, for the turn its message drives.
    ran: Option<Ran>,
    runner: Option<Runner>,
}

/// The one set of prepared branches, and the reading of the message that resolves it.
pub struct Prepared {
    state: std::sync::Mutex<State>,
    observatory: Observatory,
    /// `true` while no reading is in flight. The loop waits on it before a turn, so the turn a
    /// message drives always starts after that message's reading — never beside it.
    idle: watch::Sender<bool>,
}

impl Prepared {
    pub fn new(observatory: Observatory) -> Self {
        let (idle, _) = watch::channel(true);
        Self { state: std::sync::Mutex::new(State::default()), observatory, idle }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Where a met branch's actions go. Called once, by the loop standing up.
    pub(super) fn attach(&self, beats: mpsc::Sender<Beat>, said: Arc<AtomicU64>, from: SessionSlug) {
        self.lock().runner = Some(Runner { beats, said, from });
    }

    /// Replace the set. An empty list clears it.
    pub(super) async fn set(&self, branches: Vec<Branch>, said: Vec<String>) {
        let directions = branches.iter().map(Branch::direction).collect();
        self.lock().set = (!branches.is_empty()).then_some(Set { branches, said });
        self.observatory.record(EventKind::BranchesPrepared { directions }).await;
    }

    /// Void whatever is set, recording why. Nothing set, nothing recorded.
    pub(super) async fn void(&self, reason: &str) {
        let had = self.lock().set.take().is_some();
        if had {
            tracing::info!(reason, "prepared branches voided");
            self.observatory.record(EventKind::BranchesVoided { reason: reason.to_string() }).await;
        }
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
}

/// What to do about one message from the person, decided under the lock.
enum OnMessage {
    Nothing,
    Void(&'static str),
    Read(Set, mpsc::UnboundedReceiver<Message>),
}

/// A message from the person landed. It joins a reading in flight; otherwise, if a set is
/// waiting, a reading of it starts.
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
        } else {
            match st.set.take() {
                None => OnMessage::Nothing,
                Some(_) if !enabled() => OnMessage::Void("prepared branches are switched off"),
                Some(set) => {
                    let (tx, rx) = mpsc::unbounded_channel();
                    st.reading = Some(tx);
                    st.ran = None;
                    OnMessage::Read(set, rx)
                }
            }
        }
    };
    match next {
        OnMessage::Nothing => {}
        OnMessage::Void(reason) => {
            prepared.observatory.record(EventKind::BranchesVoided { reason: reason.to_string() }).await;
        }
        OnMessage::Read(set, rx) => {
            prepared.idle.send_replace(false);
            tokio::spawn(read(reaction.clone(), set, message.clone(), rx));
        }
    }
}

/// Read the person's message against the set, run the branch it met, and record it all.
///
/// **The question is asked as soon as the message lands, and the answer is used only once the
/// batch has closed** — the same settle a turn waits out, held open while they are still
/// talking or typing, because a second message is exactly what turns 同意。 into 同意。不过…. A
/// message that joins the batch asks again over the whole of it. So on a typed message the
/// reading is mostly hidden inside a window the turn waits through anyway, and a miss costs
/// nothing.
async fn read(reaction: Reaction, set: Set, first: Message, mut more: mpsc::UnboundedReceiver<Message>) {
    let arrived = Instant::now();
    let floor = reaction.inner.floor.clone();
    let questions = Arc::new(questions(&set.branches));
    let mut messages = vec![first];
    let mut asking = ask(&set.said, &messages, &questions);
    // **A microphone hears the whole room**, and a line nobody spoke to the agent is not their
    // next message: it must neither run a branch — 行，就这样 said across the dinner table —
    // nor use the set up. So a batch that is only room asks the room screen's own question
    // beside this one ([`super::room`]), at every arrival, as the loop does. It cannot wait
    // for the loop's answer: while a turn is running the loop screens nothing, and that is
    // exactly when a spoken reply lands.
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
                    asking = ask(&set.said, &messages, &questions);
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
    // Side talk: nothing runs, and the set waits for the message that is theirs. Anything
    // short of a clear answer from the screen reads as someone talking with the agent, the
    // way the loop reads it — the branch question below still has to be met.
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
                direction: None,
                p: None,
                qualified: None,
                mass: None,
                model: None,
                decided_ms: arrived.elapsed().as_millis() as u64,
                first_action_ms: None,
                ran: Vec::new(),
            })
            .await;
        {
            let mut st = reaction.inner.prepared.lock();
            st.reading = None;
            if st.set.is_none() {
                st.set = Some(set);
            }
        }
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
    let reading = Reading::of(&answer, set.branches.len(), threshold());
    let mut outcome = reading.outcome.to_string();
    let mut ran: Vec<(String, Result<String, String>)> = Vec::new();
    let mut first_action_ms = None;
    if let Some(i) = reading.met {
        let branch = &set.branches[i];
        let now = Instant::now();
        let runner = reaction.inner.prepared.lock().runner.clone();
        if branch.actions.is_empty() {
            outcome = "met, nothing prepared".into();
        } else if floor.voice_active(now).await || floor.typing_active(now).await {
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
    if let Some(err) = &reading.error {
        tracing::warn!(error = %err, "prepared branches: the reading failed; the message takes the ordinary path");
    }
    let direction = reading.met.map(|i| set.branches[i].condition.clone());
    tracing::info!(
        outcome = %outcome,
        p = ?reading.p,
        qualified = ?reading.qualified,
        decided_ms,
        ran = ran.len(),
        "prepared branches: their message read"
    );
    reaction
        .inner
        .observatory
        .record(EventKind::BranchesResolved {
            message: render_messages(&messages),
            outcome,
            direction,
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
        })
        .await;
    let direction = reading.met.map(|i| set.branches[i].condition.clone());
    {
        let mut st = reaction.inner.prepared.lock();
        st.reading = None;
        st.ran = match (direction, ran.is_empty()) {
            (Some(condition), false) => Some(Ran { condition, ran }),
            _ => None,
        };
    }
    reaction.inner.prepared.idle.send_replace(true);
}

/// Ask the questions about the messages so far, inside the budget. `None` when nothing is
/// configured to ask — then the reading is a miss before it begins.
fn ask(
    said: &[String],
    messages: &[Message],
    questions: &Arc<Map<String, Value>>,
) -> Option<JoinHandle<anyhow::Result<decision::Reply>>> {
    if !decision::available() {
        return None;
    }
    let state = Value::String(state(said, messages));
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

/// A reply read: which branch was met, if any, and every number that decided it.
#[derive(Debug, Default, PartialEq)]
struct Reading {
    met: Option<usize>,
    outcome: &'static str,
    p: Option<f64>,
    qualified: Option<f64>,
    mass: Option<Value>,
    model: Option<String>,
    error: Option<String>,
}

impl Reading {
    /// A branch is met when its option carries at least `tau` of the choice's mass and
    /// `qualified` carries at most `1 − tau`. A reply that cannot be read runs nothing.
    fn of(answer: &Result<decision::Reply, Unread>, branches: usize, tau: f64) -> Self {
        let reply = match answer {
            Ok(reply) => reply,
            Err(Unread::Unavailable) => return Self { outcome: "unavailable", ..Self::default() },
            Err(Unread::Timeout) => return Self { outcome: "timeout", ..Self::default() },
            Err(Unread::Error(e)) => {
                return Self { outcome: "error", error: Some(e.clone()), ..Self::default() };
            }
        };
        let model = Some(reply.model.clone());
        let qualified = match reply.answers.get(QUALIFIED) {
            Some(decision::Answer::Noul { p }) => Some(*p),
            _ => None,
        };
        let Some(decision::Answer::Choice { probabilities: Some(Value::Object(mass)), .. }) =
            reply.answers.get(WHICH)
        else {
            return Self { outcome: "error", model, qualified, error: Some("no choice in the reply".into()), ..Self::default() };
        };
        let whole = Some(Value::Object(mass.clone()));
        let best = mass
            .iter()
            .filter_map(|(option, p)| Some((option.as_str(), p.as_f64()?)))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        let Some((option, p)) = best else {
            return Self { outcome: "error", model, qualified, mass: whole, error: Some("an empty choice".into()), ..Self::default() };
        };
        let read = |outcome, met| Self { met, outcome, p: Some(p), qualified, mass: whole.clone(), model: model.clone(), error: None };
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

/// The two questions, in `judges/prepared.md`'s words, with one option per branch.
fn questions(branches: &[Branch]) -> Map<String, Value> {
    let rubric = crate::identity::judges::PREPARED;
    let part = |heading: &str| crate::identity::rubric_section(rubric, heading).unwrap_or_default();
    let frame = part("Frame");
    let mut criteria = Map::new();
    for (i, b) in branches.iter().enumerate() {
        criteria.insert(format!("b{}", i + 1), Value::from(b.condition.clone()));
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
    asked
}

/// The state a reading is about: what the agent said since the person last wrote, then what
/// they wrote. Nothing else — System One loses accuracy on a padded state.
fn state(said: &[String], messages: &[Message]) -> String {
    let mut s = String::from("## What the assistant said\n");
    if said.is_empty() {
        s.push_str("(nothing since the person last spoke)\n");
    }
    for line in said {
        s.push_str(&format!("< {}\n", line.replace('\n', "\n  ")));
    }
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
                        let beat = Beat::Show { id: id.clone(), op: op.clone(), source, view_ref: view_ref.clone() };
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

/// A branch that ran: where the message went, and each action with what became of it.
pub(super) struct Ran {
    condition: String,
    ran: Vec<(String, Result<String, String>)>,
}

impl Ran {
    /// As the turn the message drives reads it. Facts only: what to do about it is
    /// `reaction.md`'s.
    pub(super) fn for_reaction(&self) -> String {
        format!(
            "\n## What your prepared branch did\nTheir message went where you had prepared for: \
             \"{}\". Before this turn began, that branch ran, in order:\n{}",
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

    fn reply(which: Value, qualified: Option<f64>) -> decision::Reply {
        let mut answers = BTreeMap::new();
        answers.insert(
            WHICH.to_string(),
            decision::Answer::Choice { choice: String::new(), probabilities: Some(which), confidence: None },
        );
        if let Some(p) = qualified {
            answers.insert(QUALIFIED.to_string(), decision::Answer::Noul { p });
        }
        decision::Reply { model: "jev-1.13.0".into(), answers, usage: decision::Usage::default() }
    }

    fn read(which: Value, qualified: Option<f64>) -> Reading {
        Reading::of(&Ok(reply(which, qualified)), 2, 0.9)
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
    }

    #[test]
    fn a_reply_that_cannot_be_read_runs_nothing() {
        assert_eq!(read(json!({"b1": 0.99}), None).outcome, "error", "no `qualified`, no vouching");
        assert_eq!(read(json!({"b7": 0.99}), Some(0.0)).outcome, "error", "an option nobody asked");
        assert_eq!(read(json!({}), Some(0.0)).outcome, "error");
        assert_eq!(Reading::of(&Err(Unread::Timeout), 2, 0.9).outcome, "timeout");
        assert_eq!(Reading::of(&Err(Unread::Unavailable), 2, 0.9).outcome, "unavailable");
        for r in [read(json!({"b1": 0.99}), None), Reading::of(&Err(Unread::Timeout), 2, 0.9)] {
            assert_eq!(r.met, None);
        }
    }

    #[test]
    fn options_are_the_branches_in_order_and_the_rest() {
        let branches = vec![
            Branch { condition: "同意 A、没加条件".into(), actions: vec![] },
            Branch { condition: "选 B".into(), actions: vec![] },
        ];
        let asked = questions(&branches);
        let criteria = asked[WHICH]["criteria"].as_object().unwrap();
        assert_eq!(criteria.keys().collect::<Vec<_>>(), vec!["b1", "b2", "rest"]);
        assert_eq!(criteria["b1"], "同意 A、没加条件");
        assert!(!criteria["rest"].as_str().unwrap().is_empty(), "the rubric's Rest is read");
        assert_eq!(asked[QUALIFIED]["type"], "noul");
        assert!(asked[WHICH]["instructions"].as_str().unwrap().contains("Which of these directions"));
        assert_eq!(branch_index("b1"), Some(0));
        assert_eq!(branch_index("b0"), None);
        assert_eq!(branch_index("rest"), None);
    }

    #[tokio::test]
    async fn a_call_is_one_choice_checked_whole() {
        let dir = std::env::temp_dir();
        let ok = parse(
            &json!({"branches": [
                {"condition": "agrees to A", "actions": [{"tool": "hi_say", "text": "好，A。"}]},
                {"condition": "picks B"}
            ]}),
            &dir,
            400,
        )
        .await
        .unwrap();
        assert_eq!(ok.len(), 2);
        assert_eq!(ok[0].actions, vec![Action::Say { text: "好，A。".into() }]);
        assert!(ok[1].actions.is_empty(), "a direction with nothing ready is a valid option");

        assert!(parse(&json!({"branches": []}), &dir, 400).await.unwrap().is_empty(), "empty clears");

        let refused = |args: Value| {
            let dir = dir.clone();
            async move { parse(&args, &dir, 400).await.unwrap_err() }
        };
        assert!(refused(json!({})).await.contains("branches"));
        assert!(refused(json!({"branches": [{"actions": []}]})).await.contains("no `condition`"));
        let long = refused(json!({"branches": [{"condition": "x", "actions": [{"tool": "hi_say", "text": "字".repeat(401)}]}]})).await;
        assert!(long.contains("too long") && long.contains("Nothing was prepared"), "{long}");
        assert!(refused(json!({"branches": [{"condition": "x", "actions": [{"tool": "hi_prepare"}]}]})).await.contains("cannot prepare"));
        assert!(refused(json!({"branches": [{"condition": "x", "actions": [{"tool": "shell"}]}]})).await.contains("cannot be prepared"));
        assert!(refused(json!({"branches": [{"condition": "x", "actions": [{"tool": "hi_show"}]}]})).await.contains("needs a `ref`"));
        assert!(refused(json!({"branches": [{"condition": "x", "actions": [{"tool": "hi_send_message", "to": "nobody-here", "message": "go"}]}]})).await.contains("nothing live"));
        let fan: Vec<Value> = (0..=MAX_BRANCHES).map(|i| json!({"condition": format!("d{i}")})).collect();
        assert!(refused(json!({"branches": fan})).await.contains("a fan"));
    }

    #[tokio::test]
    async fn a_set_is_replaced_whole_and_voided_once() {
        let prepared = Prepared::new(Observatory::new(None));
        let one = vec![Branch { condition: "agrees".into(), actions: vec![] }];
        prepared.set(one.clone(), vec!["我倾向 A".into()]).await;
        assert_eq!(prepared.lock().set.as_ref().map(|s| s.branches.clone()), Some(one));
        prepared.set(vec![], vec![]).await;
        assert!(prepared.lock().set.is_none(), "an empty list clears");

        prepared.set(vec![Branch { condition: "agrees".into(), actions: vec![] }], vec![]).await;
        prepared.void("another turn started").await;
        assert!(prepared.lock().set.is_none());
        prepared.void("again").await; // nothing set: nothing recorded, nothing panics
        assert_eq!(prepared.observatory.event_count().await, 4, "three prepared, one voided");
    }

    #[test]
    fn what_ran_says_where_it_stopped() {
        let stopped = Ran {
            condition: "想看两边的数".into(),
            ran: vec![(
                "hi_show show plan/compare".to_string(),
                Err("not shown — `ref` plan/compare: no such view".to_string()),
            )],
        };
        let s = stopped.for_reaction();
        assert!(s.contains("## What your prepared branch did"));
        assert!(s.contains("想看两边的数"));
        assert!(s.contains("It stopped there"));
        let ok = Ran {
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
