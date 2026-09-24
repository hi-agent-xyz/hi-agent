//! The floor: whether they are holding it, and what the running turn has seen of them.
//!
//! # 1. Are they still going?
//!
//! **Nothing is refused here any more.** This module used to gate every `say` at the instant its
//! words were ready — refused while their voice sounded, while they typed, or when a line had
//! landed that the turn had not seen — and a refused line was simply lost. What decides when a
//! line goes is now the floor reading in [`super::prepared`], which holds what Reaction prepared
//! and releases it at a stop it still fits (`docs/arch/host.md` § *The floor*). What stays here
//! are the facts that reading and the batching window stand on, none of them a judgment:
//!
//! - **their voice is sounding** ([`Floor::voice_active`]) — a recognized partial within
//!   [`VOICE_ACTIVE_FOR`]. The one guard against starting on top of a sentence that has not
//!   finalized, and the reason the batching window stays open;
//! - **they are typing** ([`Floor::typing_active`]) — the typed half of the same fact. A draft is
//!   not a barge-in, see [`Floor::note_typing`];
//! - **how many of their lines exist, and how many the running turn has seen**
//!   ([`Floor::heard`], [`Floor::seen`]) — a prepared set records what it was written against, so
//!   the reading can show it exactly what they said after.
//!
//! # 2. Did they talk over me?
//!
//! There is no interrupt signal anywhere on the wire, and the mind is never
//! cancelled — fix-forward holds with no exceptions. The client ducks its own
//! speaker reflexively when speech is recognized (it watches the shared text
//! appearance's rolling `interim`); the human's words then buffer and fold into
//! the next turn like any other signal. What this half adds is the speaker's
//! *self-knowledge*: a human knows "I'd been talking about ten seconds when she
//! cut in" from their own internal clock, not from a receipt. Same here — the
//! backend knows when a turn's voice started sounding and roughly how long the
//! reply takes to say, so when recognized speech arrives mid-sound it records a
//! pending note: which turn was cut, about how far in, and what the full reply
//! had been. The next turn's prompt carries that note as plain fact; how to fold
//! the unheard tail forward (drop, revise, mention) is the soul's judgment, not
//! the mechanism's (see `reaction.md`). The same barge-in also marks the cut turn
//! for flush ([`Floor::should_skip`]) so the output sequencer stops speaking and
//! typing its unheard tail rather than draining it over the human.
//!
//! **The floor holding lines until they stop is what makes this half rare, and that is
//! the point.** It used to fire on people who had never interrupted anything: we started
//! speaking over someone who simply had not stopped, and then told Reaction its words had
//! gone unheard — which invited it to say them again. Three consecutive turns in the
//! measured conversation carried that note, and none of them had been interrupted.
//!
//! Everything here is estimate-grade on purpose: playback truth lives only in
//! the client, and we deliberately don't ask for it. The note says "about Ns
//! in" and hands over the full text — the mind judges, the way a person isn't
//! quite sure you caught their last sentence.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;


/// How many finished turns' replies are retained for note resolution.
/// Playback lags synthesis by at most a reply or so; a barged turn is almost
/// always the latest finished one.
const RECENT_REPLIES: usize = 2;

/// Rough speaking rate for estimating how long a reply takes to sound —
/// Mandarin TTS runs ~5 chars/s and that's the dominant register here; mixed
/// or Latin text says more per char, which only makes the estimate generous
/// (we'd over-believe "still sounding", never under).
const SPEECH_MS_PER_CHAR: u64 = 200;

/// Playback starts a beat after synthesis begins (network + decode), so the
/// heard-portion estimate is shifted back by this much.
const PLAYBACK_START_LATENCY: Duration = Duration::from_millis(400);

/// Grace past the estimated reply duration during which incoming speech still
/// counts as a barge-in rather than a normal post-listen reply.
const STILL_SOUNDING_SLACK: Duration = Duration::from_secs(2);

/// How long after the last recognized partial their voice still counts as
/// sounding.
///
/// Deliberately short, and it is **not** the "are they finished thinking" number
/// that the settle kept trying to be. That question is the floor reading's
/// ([`super::prepared`]), so this one only has to cover the beat between two partials
/// of the same breath. A long value here would buy nothing and would delay every
/// reply into a real silence.
const VOICE_ACTIVE_FOR: Duration = Duration::from_millis(900);

/// How long after the last keystroke they still count as composing.
///
/// Deliberately much wider than [`VOICE_ACTIVE_FOR`], because the two bridge
/// different rhythms. Speech partials arrive every few hundred milliseconds for
/// as long as the breath lasts, so 900ms only has to cover the beat between two
/// partials of one sentence. Keystrokes stop for as long as the person is
/// thinking, re-reading, or fixing a typo — pauses of a second or two happen
/// inside a sentence somebody is very much still writing. A window this wide is
/// affordable here and was not there, because typing costs nothing to wait out:
/// nobody is mid-sound, so the reply is late rather than trampled.
const TYPING_ACTIVE_FOR: Duration = Duration::from_secs(3);

/// One inferred barge-in, held until the next turn folds it into its prompt.
#[derive(Debug)]
pub struct Interruption {
    pub turn: u64,
    /// Estimated portion of the reply that had sounded when they broke in.
    pub heard_ms: u64,
    /// The cut turn's full spoken reply, when known. `None` while the turn is
    /// still in progress (back-filled by [`Floor::end_turn`]).
    pub reply: Option<String>,
}

#[derive(Default)]
struct SpeechState {
    /// When speech was last recognized from them — every rolling partial, final
    /// or not. The floor's own clock: `now - last_voice < VOICE_ACTIVE_FOR`
    /// means they are still mid-breath.
    ///
    /// Partials rather than finalized utterances, because that is the whole
    /// point: a final lands after the words are over, and the settle's mistake
    /// was measuring the wrong one.
    last_voice: Option<Instant>,
    /// When a keystroke last landed in an unsent draft. The typed counterpart of
    /// [`last_voice`](Self::last_voice), kept as its own field rather than folded
    /// into it because the two decay at different widths and only one of them can
    /// imply a barge-in.
    last_typing: Option<Instant>,
    /// The latest voice span: which turn, and when its audio started
    /// flowing. Stamped by the sequencer when it opens a turn's TTS.
    audio: Option<(u64, Instant)>,
    /// Last few finished turns' replies, newest last: `(turn, reply)`.
    recent: VecDeque<(u64, String)>,
    /// The unconsumed barge-in note, if any. First wins; duplicates are noise.
    pending: Option<Interruption>,
    /// Turn marked for flush by a barge-in: the sequencer drops this turn's
    /// remaining `say`/`show` output so the unheard tail isn't spoken or typed
    /// out. Monotonic turn ids mean a stale value never matches a later turn, so
    /// it's simply overwritten by the next barge-in — no explicit reset.
    flush_turn: Option<u64>,
}

/// Shared floor state. Created once in `lib.rs`, cloned into the HTTP front
/// (whose STT relay reports recognized speech, whose composers report keystrokes,
/// and whose accepted lines bump [`heard`](Self::note_heard)) and the reaction
/// (whose sequencer stamps voice spans, whose turns drain pending notes, and whose
/// `say` asks this whether it may speak at all).
#[derive(Clone)]
pub struct Floor {
    inner: Arc<Mutex<SpeechState>>,
    /// Latest reaction turn that actually started. Stored separately from the
    /// async speech state so inbound HTTP handlers can order a settled human
    /// line against a turn without waiting on a lock.
    latest_turn: Arc<AtomicU64>,
    /// Whether that turn was started by something they said. Read by `show`, because what
    /// a turn they started puts up is the answer to them and always takes the screen.
    answering: Arc<AtomicBool>,
    /// Human lines accepted into the conversation, ever. Bumped as each one is
    /// handed to Reaction's queue, **not** when the loop dequeues it — during a
    /// generation nothing dequeues, and "did they say something while I was
    /// thinking" is exactly the question.
    heard: Arc<AtomicU64>,
    /// Whether a model turn is running: set as one starts, cleared as it ends. What is prepared
    /// outside one — the warm-up and the seed, which prime a session and answer nobody — must
    /// not reach the person, and the floor would release it.
    in_turn: Arc<AtomicBool>,
    /// [`heard`](Self::heard) as it stood when the running turn's batch was
    /// frozen. Everything above it is a line the model in flight has never seen.
    seen: Arc<AtomicU64>,
}

impl Default for Floor {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SpeechState::default())),
            latest_turn: Arc::new(AtomicU64::new(u64::MAX)),
            answering: Arc::new(AtomicBool::new(false)),
            heard: Arc::new(AtomicU64::new(0)),
            seen: Arc::new(AtomicU64::new(0)),
            in_turn: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Floor {
    pub fn new() -> Self {
        Self::default()
    }

    /// One more line from the person has been accepted for Reaction. Called at
    /// the moment it enters the queue, which is the moment it becomes something
    /// a turn already in flight cannot have seen.
    pub fn note_heard(&self) {
        self.heard.fetch_add(1, Ordering::Release);
    }

    /// How many of their lines have been accepted, ever.
    pub fn heard(&self) -> u64 {
        self.heard.load(Ordering::Acquire)
    }

    /// How many of them the running turn has: its batch, and what was steered into it. A set it
    /// prepares is read against the lines after this.
    pub fn seen(&self) -> u64 {
        self.seen.load(Ordering::Acquire)
    }

    /// Whether the running turn was started by something they said.
    pub fn answering(&self) -> bool {
        self.answering.load(Ordering::Acquire)
    }

    /// Are they mid-breath right now?
    ///
    /// Read by two callers with different stakes. The floor reading asks so nothing
    /// goes out on top of them. The loop's batching window asks so it does not spend a
    /// generation — and dispatch an errand — on a third of a sentence: a `say` can be
    /// held, but the thinking and the hand-down a turn already did cannot be taken back. Measured, that cost was two overlapping errands from
    /// one question whose batch closed 0.8s early.
    pub async fn voice_active(&self, now: Instant) -> bool {
        self.inner
            .lock()
            .await
            .last_voice
            .is_some_and(|at| now.saturating_duration_since(at) < VOICE_ACTIVE_FOR)
    }

    /// Record a reaction turn at the point it starts, before its prompt can
    /// produce output. This is internal ordering only; it never reaches a wire.
    ///
    /// It also records what this turn has seen: its batch, and nothing after. A line steered
    /// in reaches the model only at its next step, which the host cannot see — so it is never
    /// counted as seen, and a set the turn prepares is read against it at the stop. Called
    /// once the batch is assembled — after the settle has drained — so the count
    /// matches exactly what went into the prompt.
    ///
    /// `answering` is whether the batch carried something they said, as opposed to a
    /// worker's report, mail or the boot wake — see [`show_from`](Self::show_from).
    pub fn note_turn_started(&self, turn: u64, answering: bool) {
        self.in_turn.store(true, Ordering::Release);
        self.answering.store(answering, Ordering::Release);
        self.latest_turn.store(turn, Ordering::Release);
        self.seen.store(self.heard.load(Ordering::Acquire), Ordering::Release);
    }

    /// The model turn is over; anything prepared from here until the next one starts answers
    /// nobody.
    pub fn note_turn_ended(&self) {
        self.in_turn.store(false, Ordering::Release);
    }

    /// Whether a model turn is running — the only place a prepared set is for someone.
    pub fn in_turn(&self) -> bool {
        self.in_turn.load(Ordering::Acquire)
    }

    /// The running turn as the screen reads it, or `None` before any turn has started.
    pub fn show_from(&self) -> Option<crate::foundation::server::view_bus::ShowFrom> {
        self.latest_turn_started().map(|turn| crate::foundation::server::view_bus::ShowFrom {
            turn,
            answering: self.answering.load(Ordering::Acquire),
        })
    }

    /// Latest started reaction turn, if any.
    pub fn latest_turn_started(&self) -> Option<u64> {
        match self.latest_turn.load(Ordering::Acquire) {
            u64::MAX => None,
            turn => Some(turn),
        }
    }

    /// A turn's voice just started flowing. Called by the sequencer as it opens
    /// the turn's TTS span; the latest span wins.
    pub async fn audio_began(&self, turn: u64, now: Instant) {
        let mut inner = self.inner.lock().await;
        inner.audio = Some((turn, now));
    }

    /// The turn closed with `reply` as its full spoken text. Caches the reply
    /// for note resolution and back-fills a pending note that cut into this
    /// very turn.
    pub async fn end_turn(&self, turn: u64, reply: &str) {
        let mut inner = self.inner.lock().await;
        let state = &mut *inner;
        let reply = reply.trim();
        if reply.is_empty() {
            return; // a silent turn can't be barged into
        }
        if let Some(p) = state.pending.as_mut() {
            if p.turn == turn && p.reply.is_none() {
                p.reply = Some(reply.to_owned());
            }
        }
        state.recent.push_back((turn, reply.to_owned()));
        while state.recent.len() > RECENT_REPLIES {
            state.recent.pop_front();
        }
    }

    /// Recognized human speech just arrived (any rolling
    /// partial). Stamps the floor as theirs, and — if our own clock says the last
    /// reply is probably still sounding — records the barge-in note; otherwise
    /// this is a normal post-listen reply and no note is recorded. Cheap and
    /// idempotent — called for every partial, only the first mid-sound one lands.
    pub async fn note_speech(&self, now: Instant) {
        let mut inner = self.inner.lock().await;
        let state = &mut *inner;
        // Before every early return below: the floor half of this cares only that
        // a voice was heard, never whether it was worth a barge-in note.
        state.last_voice = Some(now);
        if state.pending.is_some() {
            return; // first barge-in wins
        }
        let Some((turn, began)) = state.audio else { return };
        let elapsed = now.saturating_duration_since(began);

        let reply = state.recent.iter().find(|(t, _)| *t == turn).map(|(_, r)| r.clone());
        let est = reply.as_deref().map(estimated_speech_duration);
        // A turn with no cached reply hasn't closed yet — it is mid-speech by
        // definition. A closed turn is "still sounding" while our clock sits
        // inside its estimated spoken length (plus slack).
        let still_sounding = match est {
            None => true,
            Some(d) => elapsed < d + STILL_SOUNDING_SLACK,
        };
        if !still_sounding {
            return;
        }

        let mut heard = elapsed.saturating_sub(PLAYBACK_START_LATENCY);
        if let Some(d) = est {
            heard = heard.min(d);
        }
        state.flush_turn = Some(turn);
        state.pending = Some(Interruption { turn, heard_ms: heard.as_millis() as u64, reply });
        tracing::info!(turn, heard_ms = heard.as_millis() as u64, "barge-in inferred (speech while voice sounding)");
    }

    /// A keystroke landed in a draft that has not been sent. Stamps the floor as
    /// theirs for [`TYPING_ACTIVE_FOR`] and does nothing else.
    ///
    /// **The "and does nothing else" is the point**, and is why this is not a call
    /// into [`note_speech`](Self::note_speech) with a different clock. That one also
    /// infers a barge-in from the TTS span, which is right for a voice — sound over
    /// sound means words went unheard — and wrong for keystrokes, which trample
    /// nothing. Someone quietly starting to type while the agent talks is listening,
    /// not interrupting; giving that the interruption treatment would tell Reaction
    /// its last reply had been cut off and flush the rest of the turn's output.
    pub async fn note_typing(&self, now: Instant) {
        self.inner.lock().await.last_typing = Some(now);
    }

    /// Their draft became a line: they are no longer composing it.
    ///
    /// Called when a typed line is accepted, and needed because the stamp outlives
    /// the send by design — [`TYPING_ACTIVE_FOR`] is three seconds, so without this
    /// the reply *to the line they just sent* would be held as though they were still
    /// writing for the rest of that window.
    pub async fn note_sent(&self) {
        self.inner.lock().await.last_typing = None;
    }

    /// Are they mid-draft right now?
    ///
    /// Read by the same two callers as [`voice_active`](Self::voice_active), for the
    /// same two reasons: the mouth so it does not answer half a thought, and the
    /// loop's batching window so a generation and its errands are not spent on one.
    pub async fn typing_active(&self, now: Instant) -> bool {
        self.inner
            .lock()
            .await
            .last_typing
            .is_some_and(|at| now.saturating_duration_since(at) < TYPING_ACTIVE_FOR)
    }

    /// Mark `turn` for flush directly, without an audio span. Used when the mind
    /// is reorganized mid-turn (new human input lands while the prompt is still in
    /// flight): in the thinking phase no TTS has started, so `note_speech` never
    /// fired and nothing marked the turn — but we still want the sequencer to drop
    /// any say/show beats this now-abandoned turn emits before it's cancelled.
    /// `flush_turn` is monotonic-per-turn, so this never collides with a later
    /// reorganized pass's id (self-clearing, no reset).
    pub async fn mark_flush(&self, turn: u64) {
        self.inner.lock().await.flush_turn = Some(turn);
    }

    /// Take (and clear) the pending note, for the next prompt.
    pub async fn take_pending(&self) -> Option<Interruption> {
        self.inner.lock().await.pending.take()
    }

    /// Whether the sequencer should abandon `turn`'s remaining output — a
    /// barge-in landed on it. Unlike [`take_pending`] (consumed once by the next
    /// prompt), this stays set so every trailing beat of the cut turn is skipped.
    pub async fn should_skip(&self, turn: u64) -> bool {
        let inner = self.inner.lock().await;
        inner.flush_turn == Some(turn)
    }
}

/// Rough wall-clock length of `reply` spoken aloud. Estimate-grade by design.
fn estimated_speech_duration(reply: &str) -> Duration {
    Duration::from_millis(reply.chars().count() as u64 * SPEECH_MS_PER_CHAR)
}

/// Render a pending note as a prompt section. Facts only — how to fold the
/// unheard tail forward is the soul's guidance, not the mechanism's.
pub(super) fn render_interruption(i: &Interruption) -> String {
    let secs = (i.heard_ms as f64 / 1000.0).round() as u64;
    match &i.reply {
        Some(reply) => format!(
            "## Interrupted\nThey started speaking about {secs}s into your last reply, and your \
             voice cut out there. You had been saying: \"{reply}\" — assume what came after \
             roughly that point went unheard."
        ),
        None => format!(
            "## Interrupted\nThey started speaking about {secs}s into your last reply, and your \
             voice cut out there; the rest of what you were saying went unheard."
        ),
    }
}

#[cfg(test)]
mod floor_tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// The same predicate the floor reading and the batching window ask, and it has to
    /// answer *no* on a silent start — otherwise a first utterance would hold the window
    /// open against nothing.
    #[tokio::test]
    async fn a_silent_room_is_never_audibly_talking() {
        let floor = Floor::new();
        assert!(!floor.voice_active(Instant::now()).await);
        let t0 = Instant::now();
        floor.note_speech(t0).await;
        assert!(floor.voice_active(t0 + ms(300)).await, "mid-breath");
        assert!(!floor.voice_active(t0 + VOICE_ACTIVE_FOR + ms(1)).await, "they stopped");
    }

    /// The typed half: a draft moving is them still going, and it lapses on its own,
    /// since an abandoned draft sends no signal.
    #[tokio::test]
    async fn a_draft_still_being_written_is_them_still_going() {
        let floor = Floor::new();
        let t0 = Instant::now();
        floor.note_typing(t0).await;
        assert!(floor.typing_active(t0 + ms(300)).await);
        assert!(!floor.typing_active(t0 + TYPING_ACTIVE_FOR + ms(1)).await);
    }

    /// The stamp outlives the send by three seconds, so without [`Floor::note_sent`]
    /// the line they just sent would read as a draft still being written.
    #[tokio::test]
    async fn sending_the_line_ends_the_draft_it_came_from() {
        let floor = Floor::new();
        let t0 = Instant::now();
        floor.note_typing(t0).await;
        floor.note_sent().await;
        assert!(!floor.typing_active(t0 + ms(50)).await);
    }

    /// Keystrokes must not manufacture a barge-in. Someone typing while the agent
    /// talks is listening, and telling Reaction it had been cut off would invite
    /// it to say the whole reply again.
    #[tokio::test]
    async fn typing_through_a_reply_is_not_an_interruption() {
        let floor = Floor::new();
        let t0 = Instant::now();
        floor.end_turn(7, "a reply long enough to still be sounding").await;
        floor.audio_began(7, t0).await;
        floor.note_typing(t0 + ms(500)).await;
        assert!(floor.take_pending().await.is_none(), "no note");
        assert!(!floor.should_skip(7).await, "and the turn is not flushed");
    }

    /// What a turn has seen is its batch — never a line still in the queue, and never one
    /// steered in, which the model reaches only at its next step.
    #[tokio::test]
    async fn a_turn_sees_its_batch_and_nothing_after() {
        let floor = Floor::new();
        floor.note_heard();
        floor.note_turn_started(1, true);
        assert_eq!((floor.heard(), floor.seen()), (1, 1));
        floor.note_heard();
        floor.note_heard();
        assert_eq!(floor.seen(), 1, "two lines it has not been handed at a step it can see");
        assert!(floor.answering());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[tokio::test]
    async fn speech_mid_reply_records_a_note_with_text() {
        let reg = Floor::new();
        let t0 = Instant::now();
        reg.audio_began(7, t0).await;
        // 100 chars ≈ 20s spoken; turn closed (synthesis done) while audio plays on.
        reg.end_turn(7, &"字".repeat(100)).await;
        reg.note_speech(t0 + secs(5)).await;
        let p = reg.take_pending().await.expect("note recorded");
        assert_eq!(p.turn, 7);
        assert!(p.reply.is_some());
        // ~5s elapsed minus the playback-start beat.
        assert!((4_000..=5_000).contains(&p.heard_ms), "heard_ms = {}", p.heard_ms);
        assert!(reg.take_pending().await.is_none(), "take drains");
    }

    #[tokio::test]
    async fn speech_after_reply_finished_is_not_a_barge_in() {
        let reg = Floor::new();
        let t0 = Instant::now();
        reg.audio_began(7, t0).await;
        reg.end_turn(7, &"字".repeat(10)).await; // ≈2s spoken
        reg.note_speech(t0 + secs(10)).await; // long after it finished
        assert!(reg.take_pending().await.is_none());
    }

    #[tokio::test]
    async fn turn_still_in_progress_counts_as_sounding_and_backfills() {
        let reg = Floor::new();
        let t0 = Instant::now();
        reg.audio_began(3, t0).await;
        // No end_turn yet — mid-generation. Speech arrives:
        reg.note_speech(t0 + secs(2)).await;
        // The turn then closes with its full reply.
        reg.end_turn(3, "what was said in full").await;
        let p = reg.take_pending().await.expect("note recorded");
        assert_eq!(p.turn, 3);
        assert_eq!(p.reply.as_deref(), Some("what was said in full"));
    }

    #[tokio::test]
    async fn first_barge_in_wins() {
        let reg = Floor::new();
        let t0 = Instant::now();
        reg.audio_began(1, t0).await;
        reg.note_speech(t0 + secs(1)).await;
        reg.note_speech(t0 + secs(3)).await; // later partials are noise
        let p = reg.take_pending().await.expect("note recorded");
        assert!(p.heard_ms <= 1_000, "heard_ms = {}", p.heard_ms);
    }

    #[tokio::test]
    async fn speech_with_no_voice_span_records_nothing() {
        let reg = Floor::new();
        reg.note_speech(Instant::now()).await;
        assert!(reg.take_pending().await.is_none());
    }

    #[tokio::test]
    async fn heard_estimate_is_capped_at_reply_length() {
        let reg = Floor::new();
        let t0 = Instant::now();
        reg.audio_began(5, t0).await;
        reg.end_turn(5, &"字".repeat(10)).await; // ≈2s spoken
        // Speech lands inside the slack window, past the estimated end.
        reg.note_speech(t0 + secs(3)).await;
        let p = reg.take_pending().await.expect("note recorded");
        assert!(p.heard_ms <= 2_000, "heard_ms = {}", p.heard_ms);
    }

    #[tokio::test]
    async fn barge_in_marks_its_turn_for_skip() {
        let reg = Floor::new();
        let t0 = Instant::now();
        reg.audio_began(4, t0).await;
        reg.note_speech(t0 + secs(1)).await;
        assert!(reg.should_skip(4).await);
        // A later turn is unaffected — monotonic ids never collide with a stale flag.
        assert!(!reg.should_skip(5).await);
        // Draining the note for the prompt does not un-flush the turn.
        let _ = reg.take_pending().await;
        assert!(reg.should_skip(4).await);
    }

    #[tokio::test]
    async fn no_barge_in_no_skip() {
        let reg = Floor::new();
        let t0 = Instant::now();
        reg.audio_began(4, t0).await;
        assert!(!reg.should_skip(4).await);
    }

    #[tokio::test]
    async fn mark_flush_marks_turn_for_skip_without_an_audio_span() {
        let reg = Floor::new();
        // No audio_began / note_speech — the thinking-phase reorg case.
        reg.mark_flush(9).await;
        assert!(reg.should_skip(9).await);
        // A later (reorganized) pass with a fresh id is unaffected.
        assert!(!reg.should_skip(10).await);
    }

    #[test]
    fn latest_started_turn_is_shared_without_entering_the_wire() {
        let reg = Floor::new();
        let clone = reg.clone();
        assert_eq!(clone.latest_turn_started(), None);
        reg.note_turn_started(41, true);
        assert_eq!(clone.latest_turn_started(), Some(41));
    }
}
