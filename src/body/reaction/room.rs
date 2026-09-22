//! The room screen — whether a batch that is only room is anyone talking with the agent
//! (`docs/arch/host.md` § *The room screen*).
//!
//! A microphone in a room hears the whole room, so most of what reaches Reaction on `audio` and
//! `vision` is not for it: side talk, a child, a phone call, the car's navigation, a face walking
//! past the camera. Perception hands all of it up and must — nothing here drops a line. What this
//! decides is narrower: **whether one batch that is only room wakes Reaction now**, or waits, set
//! aside, for the next turn that something else drives.
//!
//! The question is System One's, one `noul`, worded in `src/identity/judges/room.md`. It is asked
//! **while the batch settles**, not after: every arrival that changes a room-only batch starts the
//! question again, so by the time the settle closes the answer for exactly that batch has usually
//! been in flight for the whole window. The settle is 700 ms and a call was measured at a p50 of
//! 580 ms, so in the ordinary case the screen adds nothing a person could notice.
//!
//! Every limit here is about not being able to do harm:
//!
//! - **only room is screened** — a batch carrying anything typed, handed, mailed or reported
//!   wakes Reaction exactly as before, and so does one the loop is retrying;
//! - **anything short of a clear answer wakes** — off, unconfigured, over budget, an error, or
//!   an answer that does not read as a probability: the batch goes to Reaction as it always did;
//! - **set aside is not dropped** — the lines ride into the next turn under their own heading
//!   ([`ASIDE_HEADING`]), so the one that was meant for the agent is still in front of it, and
//!   a "did you hear me?" wakes it with the original right there.
//!
//! **What this does not do is un-count a line at the mouth.** A room line that lands while a
//! turn is generating still makes that turn's words out of date ([`super::floor`]): it is
//! screened only when it becomes the next batch, which is after the refusal it caused. Doing
//! that too means screening each arrival on its own, which is a different question from the one
//! measured here.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::LoopInput;
use crate::body::capabilities::decision;
use crate::foundation::config::tunables;
use crate::mind::memory::{Memory, journal, snapshot};
use crate::types::{Channel, Content, JournalEntry};

/// The `app_settings` keys switching it off, choosing how long an answer may take, and where
/// the answer has to be for the batch to wake Reaction.
pub(super) const SWITCH_KEY: &str = "room_screen";
pub(super) const BUDGET_KEY: &str = "room_screen_budget_ms";
pub(super) const WAKE_AT_KEY: &str = "room_screen_wake_at";

/// A batch wakes Reaction when the answer puts at least this on someone talking with it.
///
/// **Chosen against 650 batches from 08-15 to 09-20, each labelled blind by a model reader**
/// (not by the person). At 0.30 it set aside 164 of the 322 that were nobody talking with the
/// agent, and 1 of the 297 that were — the opening line of a review, *"I'm looking at this
/// summary now"*, which the next line would have brought back. At 0.40 it set aside 71% of the
/// side talk and 11 of the 297. The cut was picked on the same set it was scored on, so those
/// numbers are the optimistic end.
const WAKE_AT: f64 = 0.30;

/// How long the answer may take, counted from when it was asked — which is usually the last
/// arrival, a whole settle before the batch closes. Past it the batch wakes Reaction.
///
/// Measured one call at a time from a dev machine through the managed gateway: p50 580 ms,
/// p90 1.06 s, the slowest of 39 at 1.96 s.
const BUDGET_MS: u64 = 1_500;

/// The key the one question is asked under.
const QUESTION: &str = "with_assistant";

/// How much of the recent conversation the question reads, and how long a line may be in it.
/// The measured wording read twenty-five lines, each clipped at 220 characters.
const CONTEXT_LINES: usize = 25;
const LINE_CHARS: usize = 220;

/// The heading set-aside room rides under in the turn that finally runs — before
/// `## New signals`, because it came first. `reaction.md` says what it is.
pub(super) const ASIDE_HEADING: &str = "## Heard around you\n";

/// How much set-aside room one turn carries, newest kept. A room that talks for an hour
/// without anyone addressing the agent is a lot of lines, and every one of them is in the log.
const ASIDE_CHARS: usize = 4_000;

/// How old a set-aside line may be and still ride into a turn — the same window as the
/// recent tail a cold window carries ([`snapshot::RECENT_WINDOW_MIN`]). Side talk from before
/// lunch is not the room the next turn is in.
const ASIDE_FOR: Duration = Duration::from_secs(snapshot::RECENT_WINDOW_MIN as u64 * 60);

/// Whether one input is room: speech the microphone caught, or something a camera or
/// microphone perceived. Everything else was sent to the agent on purpose, or is the agent's
/// own machinery, and never waits on a screen.
pub(super) fn is_room(input: &LoopInput) -> bool {
    match input {
        LoopInput::Message(m) => matches!(m.content, Content::Speech { .. }),
        LoopInput::Observed(signal) => matches!(signal.channel, Channel::Audio | Channel::Vision),
        LoopInput::Worker(_) | LoopInput::Mail { .. } => false,
    }
}

/// Whether a batch is room and nothing else — the only kind the screen reads.
pub(super) fn only_room(batch: &[LoopInput]) -> bool {
    !batch.is_empty() && batch.iter().all(is_room)
}

/// Whether the screen reads at all: its switch is not `off`, and a decision provider is set.
fn enabled() -> bool {
    crate::body::legibility::enabled(SWITCH_KEY) && decision::available()
}

fn budget() -> Duration {
    crate::body::legibility::budget(BUDGET_KEY, BUDGET_MS)
}

fn wake_at() -> f64 {
    tunables::get(WAKE_AT_KEY)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|p| (0.0..=1.0).contains(p))
        .unwrap_or(WAKE_AT)
}

/// A question in flight about one batch, as long as that batch was when it was asked.
///
/// Dropping it cancels the call: the batch it was about has grown, and a new one is on its way.
pub(super) struct Pending {
    len: usize,
    asked: Instant,
    answer: JoinHandle<anyhow::Result<decision::Reply>>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.answer.abort();
    }
}

/// Ask about `batch` as it stands, unless the question in flight is already about exactly it.
/// Called at every arrival while a batch settles; a batch that is not only room cancels any
/// question and asks none, because it will wake Reaction whatever the answer.
pub(super) fn ask(pending: &mut Option<Pending>, memory: &Memory, batch: &[LoopInput]) {
    if pending.as_ref().is_some_and(|p| p.len == batch.len()) {
        return;
    }
    *pending = None;
    if !only_room(batch) || !enabled() {
        return;
    }
    let Some((setting, question)) = wording() else {
        return;
    };
    let lines: Vec<String> = batch.iter().filter_map(new_line).collect();
    let since = batch.iter().filter_map(arrived_at).min();
    let memory = memory.clone();
    let answer = tokio::spawn(async move {
        let recent = snapshot::build(&memory).await.map(|s| s.recent_entries).unwrap_or_default();
        let state = state(setting, &recent, since, &lines);
        let mut questions = Map::new();
        questions.insert(QUESTION.into(), json!({ "type": "noul", "instructions": question }));
        decision::ask(&Value::String(state), &questions, None).await
    });
    *pending = Some(Pending { len: batch.len(), asked: Instant::now(), answer });
}

/// Whether the closed batch wakes Reaction now. `false` only on a clear answer under the cut;
/// every other outcome is `true`, which is what the loop did before this existed.
pub(super) async fn wakes(pending: &mut Option<Pending>, memory: &Memory, batch: &[LoopInput]) -> bool {
    if !only_room(batch) {
        *pending = None;
        return true;
    }
    ask(pending, memory, batch);
    let Some(mut asked) = pending.take() else {
        return true;
    };
    let left = budget().saturating_sub(asked.asked.elapsed());
    let outcome = tokio::time::timeout(left, &mut asked.answer).await;
    let ms = asked.asked.elapsed().as_millis() as u64;
    let reply = match outcome {
        Ok(Ok(Ok(reply))) => reply,
        Ok(Ok(Err(err))) => {
            tracing::warn!(error = %format!("{err:#}"), lines = batch.len(), ms, "room screen: no answer; waking");
            return true;
        }
        Ok(Err(err)) => {
            tracing::warn!(error = %err, lines = batch.len(), ms, "room screen: the question died; waking");
            return true;
        }
        Err(_) => {
            tracing::info!(lines = batch.len(), ms, "room screen: over budget; waking");
            return true;
        }
    };
    let cut = wake_at();
    let Some(p) = with_assistant(&reply) else {
        tracing::warn!(model = %reply.model, lines = batch.len(), ms, "room screen: unreadable answer; waking");
        return true;
    };
    let wake = p >= cut;
    tracing::info!(p, cut, wake, model = %reply.model, lines = batch.len(), ms, "room screen");
    wake
}

/// The answer, when it is the probability the question asked for.
fn with_assistant(reply: &decision::Reply) -> Option<f64> {
    match reply.answers.get(QUESTION)? {
        decision::Answer::Noul { p } if (0.0..=1.0).contains(p) => Some(*p),
        _ => None,
    }
}

/// The two sections of `room.md` that are sent. `None` if either is gone — then nothing is
/// asked and every batch wakes Reaction.
fn wording() -> Option<(String, String)> {
    let part = |heading: &str| crate::identity::rubric_section(crate::identity::judges::ROOM, heading);
    Some((part("Setting")?.to_string(), part("Question")?.to_string()))
}

/// What the question reads: the setting, the conversation before the batch with each line's
/// age, when the agent last spoke, and the batch.
fn state(setting: String, recent: &[JournalEntry], since: Option<DateTime<Utc>>, new: &[String]) -> String {
    let since = since.unwrap_or_else(Utc::now);
    let before: Vec<&JournalEntry> =
        recent.iter().filter(|e| journal::entry_ts(e) < since).collect();
    let ago = |e: &JournalEntry| (since - journal::entry_ts(e)).num_seconds().max(0);
    let transcript: Vec<String> = before
        .iter()
        .filter_map(|e| context_line(e).map(|line| format!("{}s ago {line}", ago(e))))
        .collect();
    let transcript = &transcript[transcript.len().saturating_sub(CONTEXT_LINES)..];
    let spoke = match before.iter().rev().find(|e| from_agent(e)) {
        Some(e) => format!("The assistant last spoke {} seconds ago.", ago(e)),
        None => format!("The assistant has not spoken in the last {} minutes.", snapshot::RECENT_WINDOW_MIN),
    };
    let transcript = if transcript.is_empty() { "(nothing yet)".to_string() } else { transcript.join("\n") };
    format!(
        "{setting}\n\nRecent transcript, oldest first:\n{transcript}\n\n{spoke}\n\nNew lines just captured:\n{}",
        new.join("\n")
    )
}

fn from_agent(e: &JournalEntry) -> bool {
    matches!(e, JournalEntry::Message { message, .. } if message.from.is_agent())
}

/// One line of the conversation before the batch, in the question's own tags. Only what was
/// said and seen: the agent's machinery and what it put on screen are not the room.
fn context_line(e: &JournalEntry) -> Option<String> {
    match e {
        JournalEntry::Message { message, .. } if message.from.is_agent() => {
            Some(format!("[assistant] {}", clip(journal::entry_body(e))))
        }
        JournalEntry::Message { channel, message } => {
            let said = match message.from.sender().and_then(|s| s.subject.as_deref()) {
                Some(who) => format!("⟨voice: {who}⟩ {}", journal::entry_body(e)),
                None => journal::entry_body(e).to_string(),
            };
            Some(format!("{} {}", tag(*channel), clip(&said)))
        }
        JournalEntry::Observation { channel, body, .. } => Some(format!("{} {}", tag(*channel), clip(body))),
        JournalEntry::Presentation { .. } | JournalEntry::Internal { .. } => None,
    }
}

/// One line of the batch, in the question's own tags.
fn new_line(input: &LoopInput) -> Option<String> {
    match input {
        LoopInput::Message(m) => {
            let Content::Speech { text, .. } = &m.content else {
                return None;
            };
            let said = match m.from.sender().and_then(|s| s.subject.as_deref()) {
                Some(who) => format!("⟨voice: {who}⟩ {text}"),
                None => text.clone(),
            };
            Some(format!("{} {}", tag(Channel::Audio), clip(&said)))
        }
        LoopInput::Observed(signal) => Some(format!("{} {}", tag(signal.channel), clip(&signal.body))),
        LoopInput::Worker(_) | LoopInput::Mail { .. } => None,
    }
}

fn arrived_at(input: &LoopInput) -> Option<DateTime<Utc>> {
    match input {
        LoopInput::Message(m) => Some(m.ts),
        LoopInput::Observed(signal) => Some(signal.ts),
        LoopInput::Worker(_) | LoopInput::Mail { .. } => None,
    }
}

fn tag(channel: Channel) -> String {
    match channel {
        Channel::Audio => "[heard]".into(),
        Channel::Vision => "[camera]".into(),
        Channel::Text => "[typed]".into(),
        other => format!("[{}]", other.as_str()),
    }
}

fn clip(s: &str) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= LINE_CHARS {
        s
    } else {
        let mut out: String = s.chars().take(LINE_CHARS).collect();
        out.push('…');
        out
    }
}

/// Room the screen set aside, each line with when it was set aside.
pub(super) type Aside = Vec<(Instant, String)>;

/// Keep a set-aside batch's lines for the next turn.
pub(super) fn set_aside(aside: &mut Aside, rendered: &str) {
    let now = Instant::now();
    aside.retain(|(at, _)| now.duration_since(*at) <= ASIDE_FOR);
    aside.extend(rendered.lines().map(|line| (now, line.to_owned())));
}

/// The set-aside room as a turn carries it, or nothing when there is none: only what is
/// younger than [`ASIDE_FOR`], and over [`ASIDE_CHARS`] the oldest lines are cut, and the
/// section says how many.
pub(super) fn render_aside(aside: &Aside, now: Instant) -> String {
    let lines: Vec<&str> = aside
        .iter()
        .filter(|(at, _)| now.duration_since(*at) <= ASIDE_FOR)
        .map(|(_, line)| line.as_str())
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    let mut kept = 0;
    let mut chars = 0;
    for line in lines.iter().rev() {
        let len = line.chars().count() + 1;
        if kept > 0 && chars + len > ASIDE_CHARS {
            break;
        }
        chars += len;
        kept += 1;
    }
    let cut = lines.len() - kept;
    let mut s = String::from(ASIDE_HEADING);
    if cut > 0 {
        s.push_str(&format!("({cut} earlier lines set aside are cut here; they are in the log.)\n"));
    }
    for line in &lines[cut..] {
        s.push_str(line);
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Author, Message, Sender, Signal};

    fn heard(text: &str, at: DateTime<Utc>) -> LoopInput {
        LoopInput::Message(Message {
            id: uuid::Uuid::now_v7().to_string(),
            ts: at,
            from: Author::Person(Sender::unknown()),
            content: Content::Speech { text: text.into(), audio: None },
            task: None,
        })
    }

    fn typed(text: &str) -> LoopInput {
        LoopInput::Message(Message {
            id: uuid::Uuid::now_v7().to_string(),
            ts: Utc::now(),
            from: Author::Person(Sender::unknown()),
            content: Content::Text(text.into()),
            task: None,
        })
    }

    fn seen(body: &str) -> LoopInput {
        LoopInput::Observed(Signal { channel: Channel::Vision, body: body.into(), stream: None, ts: Utc::now() })
    }

    /// Room is what the microphone and the camera caught. A typed line, a file, mail or a
    /// report in the batch means somebody sent the agent something, and that batch wakes it.
    #[test]
    fn only_a_batch_of_nothing_but_room_is_screened() {
        let now = Utc::now();
        assert!(only_room(&[heard("吃饭了", now), seen("糯米 appeared on camera.")]));
        assert!(!only_room(&[heard("吃饭了", now), typed("在吗")]));
        assert!(!only_room(&[typed("在吗")]));
        assert!(!only_room(&[]), "an empty batch is not room; it is nothing");
        assert!(!only_room(&[LoopInput::Mail { mail: vec![] }]));
    }

    /// The conversation it reads stops where the batch starts — the batch's own lines are
    /// already in the log, and reading them twice as context would be reading the answer.
    #[test]
    fn the_state_reads_what_came_before_the_batch_and_says_when_the_agent_spoke() {
        let now = Utc::now();
        let entry = |secs: i64, from: Author, text: &str| JournalEntry::Message {
            channel: Channel::Audio,
            message: Message {
                id: uuid::Uuid::now_v7().to_string(),
                ts: now - chrono::Duration::seconds(secs),
                from,
                content: Content::Speech { text: text.into(), audio: None },
                task: None,
            },
        };
        let recent = vec![
            entry(40, Author::Person(Sender::unknown()), "帮我查一下明天天气"),
            entry(30, Author::Agent, "明天晴"),
            entry(0, Author::Person(Sender::unknown()), "谢谢"),
        ];
        let state = state("SETTING".into(), &recent, Some(now), &["[heard] 谢谢".into()]);
        assert!(state.starts_with("SETTING\n\nRecent transcript, oldest first:\n40s ago [heard] 帮我查一下明天天气\n"), "{state}");
        assert!(state.contains("30s ago [assistant] 明天晴"), "{state}");
        assert!(state.contains("The assistant last spoke 30 seconds ago."), "{state}");
        assert_eq!(state.matches("谢谢").count(), 1, "the batch is not its own context: {state}");
        assert!(state.ends_with("New lines just captured:\n[heard] 谢谢"), "{state}");
    }

    #[test]
    fn a_quiet_log_is_said_rather_than_left_blank() {
        let state = state("S".into(), &[], None, &["[camera] 糯米 appeared on camera.".into()]);
        assert!(state.contains("(nothing yet)"), "{state}");
        assert!(state.contains("has not spoken in the last 30 minutes"), "{state}");
    }

    /// Both sections the screen sends are on the page; without either it asks nothing.
    #[test]
    fn the_wording_is_on_the_page() {
        let (setting, question) = wording().expect("room.md has a Setting and a Question");
        assert!(setting.contains("[heard]") && setting.contains("seconds before"), "{setting}");
        assert!(question.contains("WITH the assistant"), "{question}");
        assert!(!setting.contains("never sent"), "the editor's note is not sent");
    }

    #[test]
    fn only_a_probability_is_an_answer() {
        let reply = |answer: decision::Answer| decision::Reply {
            model: "jev-1.13.0".into(),
            answers: [(QUESTION.to_string(), answer)].into_iter().collect(),
            usage: Default::default(),
        };
        assert_eq!(with_assistant(&reply(decision::Answer::Noul { p: 0.12 })), Some(0.12));
        assert_eq!(with_assistant(&reply(decision::Answer::Noul { p: 1.7 })), None);
        let other = decision::Answer::Other { kind: "future".into(), raw: json!({}) };
        assert_eq!(with_assistant(&reply(other)), None);
    }

    /// Unconfigured — which is every test process — wakes on everything and asks nothing.
    #[tokio::test]
    async fn with_no_provider_every_batch_wakes() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::open(dir.path()).await.unwrap();
        let batch = [heard("吃饭了", Utc::now())];
        let mut pending = None;
        ask(&mut pending, &memory, &batch);
        assert!(pending.is_none());
        assert!(wakes(&mut pending, &memory, &batch).await);
    }

    #[test]
    fn set_aside_room_keeps_the_newest_and_says_what_it_cut() {
        let now = Instant::now();
        assert_eq!(render_aside(&Vec::new(), now), "");
        let mut aside = Aside::new();
        set_aside(&mut aside, ">/audio 吃饭了\n");
        assert_eq!(render_aside(&aside, now), format!("{ASIDE_HEADING}>/audio 吃饭了\n"));
        let many: Aside = (0..400).map(|n| (now, format!(">/audio 第{n}句闲聊"))).collect();
        let long = render_aside(&many, now);
        assert!(long.contains("earlier lines set aside are cut here"), "{long}");
        assert!(long.ends_with(">/audio 第399句闲聊\n"), "the newest stays");
        assert!(!long.contains("第0句"), "the oldest goes");
        assert!(long.chars().count() < ASIDE_CHARS + 200);
    }

    /// Side talk from before the last half hour is not the room the next turn is in.
    #[test]
    fn set_aside_room_ages_out_with_the_recent_window() {
        let now = Instant::now();
        let Some(old) = now.checked_sub(ASIDE_FOR + Duration::from_secs(1)) else {
            return; // a clock too young to hold a line that old
        };
        let aside: Aside = vec![(old, ">/audio 早上的闲聊".into()), (now, ">/audio 刚才的闲聊".into())];
        let text = render_aside(&aside, now);
        assert!(text.contains("刚才的闲聊") && !text.contains("早上的闲聊"), "{text}");
        assert_eq!(render_aside(&vec![(old, ">/audio 早上的闲聊".into())], now), "");
    }
}
