//! The floor: who is holding it, and therefore whether the voice may take it.
//!
//! Two questions live here, and they are the same question from opposite ends.
//!
//! # 1. May I speak right now?
//!
//! **The mouth is where turn-taking is decided, not the input queue.** The loop's
//! [settle](super::RESPONSE_SETTLE) batches arrivals so a turn is not spent per
//! word; it never knew whether the person was *done*, and it was never able to —
//! it counts finalized utterances, and a person mid-thought produces those
//! constantly. Measured on one live conversation: every gap between that
//! speaker's bursts was longer than the settle, so it coalesced nothing, and the
//! voice answered a half-finished sentence four times in twenty-five seconds.
//!
//! So `say` is gated here instead, against the room as it stands at the instant
//! the words are ready — up to nine seconds after the turn that composed them
//! began. Two conditions refuse, and they catch different failures:
//!
//! - [`Busy::Speaking`] — their voice is sounding right now. One mouth, one
//!   floor: it is theirs. Cheap, acoustic, and the only guard against starting
//!   on top of a sentence that has not finalized yet.
//! - [`Busy::Unheard`] — they said something after this turn's batch was frozen
//!   and the model never saw it. Exact: a counter against its value at turn
//!   start, no threshold and no guessing. This is the one that catches a reply
//!   released into a genuine two-second gap that was nonetheless written without
//!   the sentence carrying the person's actual point.
//!
//! **A refusal is not a queue.** Nothing is held, released later, or superseded:
//! the words are simply not said, and `say` answers with which of the two it was
//! ([`Spoken::NotSaid`](super::Spoken)). The retry is the next turn, which costs
//! nothing because it was already coming — whatever they said is in the queue and
//! drives one by itself. That is also why no "they stopped" signal is needed: to
//! stop talking they must have said a last thing, and that utterance is the wake.
//!
//! The starvation backstop is [`MAX_CONSECUTIVE_REFUSALS`]: a person who talks
//! through every generation must not be able to mute the agent for good.
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
//! **The gate above is what makes this half rare, and that is the point.** It
//! used to fire on people who had never interrupted anything: we started speaking
//! over someone who simply had not stopped, and then told the voice its words had
//! gone unheard — which invited it to say them again. Three consecutive turns in
//! the measured conversation carried that note, and none of them had been
//! interrupted.
//!
//! Everything here is estimate-grade on purpose: playback truth lives only in
//! the client, and we deliberately don't ask for it. The note says "about Ns
//! in" and hands over the full text — the mind judges, the way a person isn't
//! quite sure you caught their last sentence.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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
/// sounding — the width of [`Busy::Speaking`].
///
/// Deliberately short, and it is **not** the "are they finished thinking" number
/// that the settle kept trying to be. That question is answered exactly by
/// [`Busy::Unheard`], so this one only has to cover the beat between two partials
/// of the same breath. A long value here would buy nothing and would delay every
/// reply into a real silence.
const VOICE_ACTIVE_FOR: Duration = Duration::from_millis(900);

/// How many `say` calls may be refused in a row before one is let through.
///
/// The floor gate has no upper bound of its own: someone who says something
/// during every generation refuses every reply, and a person who talks steadily
/// for a minute would otherwise be met with total silence — which is a worse
/// failure than a slightly late line. So the backstop is the mechanical form of
/// what a person does when the wait grows: stop holding out for a clean opening
/// and take a small one. Reset by any utterance that lands.
const MAX_CONSECUTIVE_REFUSALS: u64 = 3;

/// Why the voice may not speak the words it just produced.
///
/// Both mean *not said*, and neither is an error. What separates them is what the
/// voice should make of it, which is why they are two arms and not a bool: one is
/// about the room being occupied, the other about the reply being out of date.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Busy {
    /// Their voice is sounding right now. The room is theirs.
    Speaking,
    /// They said something this turn never saw, so this reply was composed
    /// without it.
    Unheard,
}

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
/// (whose STT relay reports recognized speech and whose accepted lines bump
/// [`heard`](Self::note_heard)) and the reaction (whose sequencer stamps voice
/// spans, whose turns drain pending notes, and whose `say` asks this whether it
/// may speak at all).
#[derive(Clone)]
pub struct Floor {
    inner: Arc<Mutex<SpeechState>>,
    /// Latest reaction turn that actually started. Stored separately from the
    /// async speech state so inbound HTTP handlers can order a settled human
    /// line against a turn without waiting on a lock.
    latest_turn: Arc<AtomicU64>,
    /// Human lines accepted into the conversation, ever. Bumped as each one is
    /// handed to the voice's queue, **not** when the loop dequeues it — during a
    /// generation nothing dequeues, and "did they say something while I was
    /// thinking" is exactly the question.
    heard: Arc<AtomicU64>,
    /// [`heard`](Self::heard) as it stood when the running turn's batch was
    /// frozen. Everything above it is a line the model in flight has never seen.
    seen: Arc<AtomicU64>,
    /// Consecutive refusals, for [`MAX_CONSECUTIVE_REFUSALS`]. Reset by any
    /// utterance that lands.
    refused: Arc<AtomicU64>,
}

impl Default for Floor {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SpeechState::default())),
            latest_turn: Arc::new(AtomicU64::new(u64::MAX)),
            heard: Arc::new(AtomicU64::new(0)),
            seen: Arc::new(AtomicU64::new(0)),
            refused: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Floor {
    pub fn new() -> Self {
        Self::default()
    }

    /// One more line from the person has been accepted for the voice. Called at
    /// the moment it enters the queue, which is the moment it becomes something
    /// a turn already in flight cannot have seen.
    pub fn note_heard(&self) {
        self.heard.fetch_add(1, Ordering::Release);
    }

    /// May the voice say what it just produced? `Ok(())`, or which of the two
    /// refusals applies.
    ///
    /// **Order matters and is not arbitrary.** `Speaking` is checked first
    /// because it is the ruder of the two to get wrong: talking over a live voice
    /// is audible, where speaking a slightly stale line is only unhelpful. A caller
    /// that is refused should say nothing and let the next turn carry it.
    pub async fn may_speak(&self, now: Instant) -> Result<(), Busy> {
        let speaking = {
            let inner = self.inner.lock().await;
            inner
                .last_voice
                .is_some_and(|at| now.saturating_duration_since(at) < VOICE_ACTIVE_FOR)
        };
        let unheard = self.heard.load(Ordering::Acquire) > self.seen.load(Ordering::Acquire);
        let busy = match (speaking, unheard) {
            (true, _) => Busy::Speaking,
            (false, true) => Busy::Unheard,
            (false, false) => {
                self.refused.store(0, Ordering::Relaxed);
                return Ok(());
            }
        };
        // The backstop, counted here rather than by the caller so that "how many
        // in a row" cannot disagree between the two conditions.
        if self.refused.fetch_add(1, Ordering::Relaxed) >= MAX_CONSECUTIVE_REFUSALS {
            self.refused.store(0, Ordering::Relaxed);
            tracing::info!(?busy, "floor still busy after {MAX_CONSECUTIVE_REFUSALS} refusals; speaking anyway");
            return Ok(());
        }
        tracing::info!(?busy, "not said — the floor is theirs");
        Err(busy)
    }

    /// Record a reaction turn at the point it starts, before its prompt can
    /// produce output. This is internal ordering only; it never reaches a wire.
    ///
    /// It also freezes what this turn is allowed to have seen: everything heard
    /// from here on is a line the generation about to run cannot account for, and
    /// [`may_speak`](Self::may_speak) refuses on it. Called once the batch is
    /// assembled — after the settle has drained — so the count matches exactly
    /// what went into the prompt.
    pub fn note_turn_started(&self, turn: u64) {
        self.latest_turn.store(turn, Ordering::Release);
        self.seen.store(self.heard.load(Ordering::Acquire), Ordering::Release);
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

    /// Nobody is speaking and nothing has arrived unseen: the ordinary case, and
    /// the one that must stay cheap and quiet.
    #[tokio::test]
    async fn a_free_floor_is_the_voice_s_to_take() {
        let floor = Floor::new();
        assert_eq!(floor.may_speak(Instant::now()).await, Ok(()));
    }

    /// The 11:10 failure: the reply was ready while their voice was still going.
    /// Nothing had finalized, so no counter had moved — only the partials say so.
    #[tokio::test]
    async fn their_voice_still_sounding_refuses() {
        let floor = Floor::new();
        let t0 = Instant::now();
        floor.note_speech(t0).await;
        assert_eq!(floor.may_speak(t0 + ms(300)).await, Err(Busy::Speaking));
        // And it lapses on its own once they actually stop.
        assert_eq!(floor.may_speak(t0 + VOICE_ACTIVE_FOR + ms(1)).await, Ok(()));
    }

    /// The 11:18 failure, which the acoustic half cannot see: the room had been
    /// quiet for over a second, but two sentences had landed since this turn's
    /// batch was frozen and the reply was written without them.
    #[tokio::test]
    async fn a_line_the_turn_never_saw_refuses_even_in_a_quiet_room() {
        let floor = Floor::new();
        floor.note_heard(); // their first sentence
        floor.note_turn_started(1); // ...which this turn was built from
        floor.note_heard(); // and then they kept going, mid-generation
        assert_eq!(floor.may_speak(Instant::now()).await, Err(Busy::Unheard));
    }

    /// The turn driven by what they said last is allowed to answer it. Without
    /// this the gate would refuse every reply forever, since every turn is caused
    /// by a line that was heard before it started.
    #[tokio::test]
    async fn the_next_turn_is_built_from_what_it_refused_over() {
        let floor = Floor::new();
        floor.note_heard();
        floor.note_turn_started(1);
        floor.note_heard();
        assert_eq!(floor.may_speak(Instant::now()).await, Err(Busy::Unheard));
        floor.note_turn_started(2); // the next turn's batch carries that line
        assert_eq!(floor.may_speak(Instant::now()).await, Ok(()));
    }

    /// Someone who talks through every generation must not be able to mute the
    /// agent for good — the backstop takes a small opening rather than holding
    /// out for a clean one.
    #[tokio::test]
    async fn the_backstop_takes_a_small_opening() {
        let floor = Floor::new();
        floor.note_turn_started(1);
        floor.note_heard();
        for _ in 0..MAX_CONSECUTIVE_REFUSALS {
            assert_eq!(floor.may_speak(Instant::now()).await, Err(Busy::Unheard));
        }
        assert_eq!(floor.may_speak(Instant::now()).await, Ok(()), "backstop");
        // ...and having spent it, the gate is strict again.
        assert_eq!(floor.may_speak(Instant::now()).await, Err(Busy::Unheard));
    }

    /// An utterance that lands resets the count, so the backstop measures a run of
    /// refusals rather than a lifetime total.
    #[tokio::test]
    async fn a_landed_utterance_resets_the_run() {
        let floor = Floor::new();
        let t0 = Instant::now();
        floor.note_speech(t0).await;
        assert_eq!(floor.may_speak(t0 + ms(100)).await, Err(Busy::Speaking));
        assert_eq!(floor.may_speak(t0 + VOICE_ACTIVE_FOR + ms(1)).await, Ok(()));
        // Two more refusals is a fresh run, not one short of the backstop.
        floor.note_speech(t0 + VOICE_ACTIVE_FOR + ms(2)).await;
        for _ in 0..MAX_CONSECUTIVE_REFUSALS {
            assert_eq!(
                floor.may_speak(t0 + VOICE_ACTIVE_FOR + ms(3)).await,
                Err(Busy::Speaking)
            );
        }
    }

    /// Both conditions at once reports the ruder one: talking over a live voice is
    /// audible, where a stale line is only unhelpful.
    #[tokio::test]
    async fn a_live_voice_outranks_a_stale_reply() {
        let floor = Floor::new();
        let t0 = Instant::now();
        floor.note_turn_started(1);
        floor.note_heard();
        floor.note_speech(t0).await;
        assert_eq!(floor.may_speak(t0 + ms(100)).await, Err(Busy::Speaking));
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
        reg.note_turn_started(41);
        assert_eq!(clone.latest_turn_started(), Some(41));
    }
}
