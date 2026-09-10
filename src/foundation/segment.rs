//! Segmentation — aggregating a continuous signal into the coarser grain a
//! downstream consumer accepts. One mechanism, swappable cut policy.
//!
//! # One aggregator, two policies
//!
//! Both places we segment are the *same* operation: take a fine, continuous
//! stream and cut it into the coarser units the next stage wants. They differ
//! only in *where to cut* and in how the input behaves:
//!
//! - **[`Speech`]** — STT word-stream → **one message per finished thought**. The
//!   upstream revises the whole utterance until it closes it, so the cut waits for
//!   that close — and then for the sentence to actually be over, because the close
//!   fires on 800 ms of silence and a person hesitating makes plenty of it.
//! - **[`Terminator`]** — the agent's reply stream → sentences for TTS. The
//!   source is append-only (an LLM never un-says a token), so the cut is purely
//!   structural: a sentence terminator, mirroring the frontend `sentences.ts`.
//!
//! The buffer machinery is shared; the [`CutPolicy`] supplies the boundary rule.
//!
//! # The buffer model
//!
//! **Only committed text is ever cut.** A revisable source's current utterance is
//! held separately, and nothing in it can become a message.
//!
//! ```text
//!   locked                          utterance
//!   ┌────────────────────────────┐  ┌──────────────────────┐
//!   │ committed, not yet emitted │  │ still being revised  │
//!   └────────────────────────────┘  └──────────────────────┘
//!    ↑ what a cut is made from       ↑ preview only, plus proof
//!                                      somebody is still talking
//!
//!   tail() = locked + utterance          ← what the preview shows
//! ```
//!
//! - `observe(text, is_final=false)` replaces `utterance`. The text is not a
//!   candidate for anything; its *arrival* is the signal, because a source still
//!   revising is a speaker still going ([`Cut::quiet`]).
//! - `observe(text, is_final=true)` — also reachable as [`Segmenter::commit`] — is
//!   the authoritative text of the utterance it ends, and appends to `locked`.
//! - A cut drains chars off the front of `locked`; the rest stays for the next unit.
//! - [`Segmenter::flush`] is the sole exception, and the exception is safe: at
//!   stream end no revision can follow, so the trailing partial goes out too rather
//!   than being dropped.
//!
//! *This shape is a simplification, made 2026-09-10.* The buffer used to let a
//! guard cut into the revisable text, and then had to reconcile the final that
//! rewrote it — an edit-distance alignment of each revision against everything
//! already emitted, 50 lines, plus a `said` string and a `pending` string to feed
//! it. All of it existed to be precise about splicing text whose only real jobs are
//! *preview* and *barge-in*. Cutting committed text only deletes the problem: the
//! append becomes a plain `push_str`, the four strings become two, and the guard
//! that could only ever act on uncommitted text (a `max_segment` age cap) went with
//! them, since it could not do the job it documented.
//!
//! Time is injected into every method so a policy that uses it is
//! deterministically unit-testable with a synthetic clock. Policies that ignore
//! time (e.g. [`Terminator`]) are unaffected by the clock entirely.

use std::time::{Duration, Instant};

// -----------------------------------------------------------------------------
// CutPolicy — where to cut the tail
// -----------------------------------------------------------------------------

/// The undispatched tail and the facts a policy needs to judge a cut. `tail` is
/// already split into chars (all boundaries are char-aligned); a policy returns
/// the char count to emit, or `None` to keep waiting.
pub struct Cut<'a> {
    /// The undispatched suffix, char-aligned. **Committed text only** — a source's
    /// still-revisable tail never reaches a policy, so there is no such thing here
    /// as text that might be rewritten after it is cut.
    pub tail: &'a [char],
    /// How long since the source last committed text.
    pub since_commit: Duration,
    /// How long the source has been **silent** — no update of any kind, revisable
    /// ones included. For a speech source that is the difference between "they
    /// stopped" and "they are mid-sentence", which no other fact here can tell.
    pub quiet: Duration,
}

/// Decides where (if anywhere) to cut the current tail into a finished unit.
pub trait CutPolicy {
    fn boundary(&self, cut: &Cut) -> Option<usize>;
}

// -----------------------------------------------------------------------------
// Shared punctuation helpers
// -----------------------------------------------------------------------------

fn is_sentence_end(c: char) -> bool {
    matches!(c, '。' | '！' | '？' | '!' | '?' | '.' | '…')
}

/// Clause-level marks — not sentence ends, but safe places to break a run-on so
/// a forced cut lands on a phrase boundary instead of mid-word.
fn is_clause_boundary(c: char) -> bool {
    matches!(c, '，' | '、' | '；' | '：' | ',' | ';' | ':')
}

/// When a size/time guard forces a cut, snap back to the last punctuation
/// (sentence end or clause mark) before `hard`, emitting up to there and leaving
/// the incomplete remainder buffered. Falls back to `hard` only when the tail
/// has no punctuation at all to break on.
fn snap_back(chars: &[char], hard: usize) -> usize {
    (0..hard)
        .rev()
        .find(|&i| is_sentence_end(chars[i]) || is_clause_boundary(chars[i]))
        .map(|i| i + 1)
        .unwrap_or(hard)
}

/// Does this text end a thought, rather than stop in the middle of one?
///
/// The recognizer punctuates as it closes an utterance, so the mark it left at the
/// end says which of the two happened: `。？！` is a sentence it considered
/// finished, while a trailing `，` — or no mark at all — is a person who paused
/// long enough for the endpoint to fire and then kept going.
///
/// Measured over the owner's own speech (2026-09-10, 22 messages from one 5-minute
/// briefing): every one of the 11 that ended in `，` or in no mark at all had more
/// sentence coming after it (`就是`, `然后这个`, `就是倒不在乎说一定要追求特别`), and
/// every one that ended in `。？！` read as a whole thing. **Every message that
/// ended a turn — the case where a reply is owed and latency is real — ended in
/// `。` or `？`**, which is why holding the others costs nothing that matters.
///
/// # It is a heuristic, and it is a cheap one to be wrong about
///
/// The recognizer's punctuation is a guess, so this is too. What makes it safe is
/// that **neither way of being wrong can lose a word or corrupt a message** — both
/// failures are bounded and one-off:
///
/// - It says *finished* when the speaker was not (a `。` mid-thought): one extra
///   message. Downstream batching usually puts the halves in the same turn anyway.
/// - It says *unfinished* when the turn was over (someone ends on `，`): that reply
///   waits out the hold instead of the settle. Late once, by design's own bound.
///
/// **Deleting it is the alternative, and the price is measured.** With no signal
/// there is one timer for every case, so the last message of *every* turn waits it
/// out. Same 222 s (`tests/speech_segmentation_replay.rs`): matching this policy's
/// raggedness needs a 2 s timer, which puts **2.10 s on every reply** against this
/// one's 0.15 s; at a latency that competes (0.5 s) it leaves 6 of 16 messages
/// ragged against 1 of 11. Roughly fifteen lines buy two seconds a turn.
///
/// So the failure to watch for is the *second* one above — noticeably waiting after
/// a sentence that ended on a comma. The fix for that is a smaller hold, not a
/// deleted signal.
fn ends_a_thought(chars: &[char]) -> bool {
    chars.iter().rev().find(|c| !c.is_whitespace()).is_some_and(|c| is_sentence_end(*c))
}

// -----------------------------------------------------------------------------
// Speech — STT word-stream → sentences (revisable source, time-aware)
// -----------------------------------------------------------------------------

/// Tunable thresholds for the [`Speech`] guards. The unit itself is not tunable:
/// it is where the speaker finished a thought.
#[derive(Debug, Clone, Copy)]
pub struct SegmenterConfig {
    /// Hard cap on undispatched chars before a forced cut (run-on guard).
    pub max_chars: usize,
    /// A finalized utterance that **ends a thought** must hold still this long
    /// before it leaves, so a recognizer that finalizes twice in quick succession —
    /// a correction, or two utterances in one frame — sends one message rather than
    /// two fragments.
    pub settle: Duration,
    /// How long a finalized utterance that does **not** end a thought is held for
    /// the rest of the sentence before it is given up on and sent as it stands.
    ///
    /// Measured from [`Cut::quiet`] — silence, not the last commit — because what it
    /// waits for is the speaker, and a speaker mid-sentence is revising a partial
    /// even when nothing has been committed for seconds. The recognizer has already
    /// spent `STREAM_END_WINDOW_MS` of silence to close the utterance, so the real
    /// silence before a dangling fragment goes out alone is that plus this.
    ///
    /// **[`DEFAULT_HOLD`](Self::DEFAULT_HOLD) is where the curve flattens, not a
    /// guess** — see that constant for the sweep.
    pub hold: Duration,
}

impl SegmenterConfig {
    /// Clear of any spoken sentence, so the cap does not re-cut what the hold just
    /// merged. The longest single message in the 2026-09-10 recording was 96 chars,
    /// and that one was itself the cap firing.
    ///
    /// **It is not a cap on message length**, and a message longer than it is not a
    /// bug: a finished thought leaves whole (the 222 s replay's longest is 289 chars),
    /// because the rule that releases it is checked first. What this bounds is
    /// *unfinished* speech — how much of a run-on nobody has paused in the buffer will
    /// hold before cutting at a phrase boundary.
    pub const DEFAULT_MAX_CHARS: usize = 220;

    /// **Measured, with margin.** Swept over 222 s of the owner's own speech,
    /// replayed through the recognizer and cut with every value
    /// (`tests/speech_segmentation_replay.rs`, 358 frames, 16 finals):
    ///
    /// | hold | messages | median | longest | ragged | delay on the turn's last message |
    /// |---|---|---|---|---|---|
    /// | 0.15 s (= no hold) | 16 | 52 | 115 | 6/16 | 0.15 s |
    /// | 1 s | 16 | 52 | 115 | 6/16 | 0.15 s |
    /// | **2 s** | **11** | **45** | **289** | **1/11** | **0.15 s** |
    /// | 3 / 4 / 6 / 8 s | 11 | 45 | 289 | 1/11 | 0.15 s |
    ///
    /// The curve flattens at 2 s and **this is set one step past it on purpose**: the
    /// replay concatenates per-minute WAVs that are missing ~30 % of their wall clock
    /// (see `docs/user-journeys/gaps.md` #33), so it *removes pauses that happened* and
    /// therefore under-states how long a real speaker's mid-sentence gaps run. The knee
    /// it reports is a floor on the knee a person would need.
    ///
    /// And **the turn's last message landed at the same 0.15 s for every value** — the
    /// claim that waiting is free at the moment a reply is owed is measured, not
    /// argued: the utterance a person stops on ends in `。？！`, so it never enters the
    /// hold at all.
    pub const DEFAULT_HOLD: Duration = Duration::from_secs(3);

}

impl Default for SegmenterConfig {
    fn default() -> Self {
        Self {
            max_chars: Self::DEFAULT_MAX_CHARS,
            settle: Duration::from_millis(150),
            hold: Self::DEFAULT_HOLD,
        }
    }
}

/// Cut policy for a revisable speech transcript: **one finalized utterance is one
/// message.**
///
/// The unit is where the speaker stopped, and the recognizer's own VAD is what
/// reports it — 800 ms of trailing silence (`STREAM_END_WINDOW_MS` in
/// [`volcengine_stt`](crate::foundation::vendors::volcengine_stt)) marks the
/// utterance definite. Everything committed then leaves together.
///
/// **This used to cut on punctuation**, just after each sentence-ending mark, with
/// time only as a settle guard. That was a finer grain than anything downstream
/// wanted, and it was the wrong unit twice over:
///
/// - **Punctuation is a transcription boundary, not a conversational one.** The
///   recognizer marks a period wherever written Chinese would take one, so an
///   ordinary spoken turn arrived as a run of messages nobody would have sent
///   separately — `嗯。`, `然后呢？`, `时间嘛。`, `对吧？`. Measured over 164 s of real
///   speech (2026-09-09): 30 of 36 messages were cut by punctuation, median 18
///   chars, the lower quartile 6.
/// - **A partial's text is worse than its final's.** Cutting on punctuation meant
///   emitting words the moment they were heard, from the rolling partial; the
///   recognizer's second pass and ITN correction arrive later, with the final, and
///   were thrown away. Same 164 s: `刚才，刚才我边那你这个就放在右边` from the partial,
///   `刚才放在左边，那你这个就放在右边` from the final.
///
/// **Waiting costs nothing at the moment that matters.** The last message of a turn
/// lands at the same millisecond either way — the person stopped talking, so the
/// endpoint fires and everything settles — and that is the instant a reply is owed.
/// What waiting delays is only the *intermediate* text appearing on screen mid-turn,
/// and the recognition preview covers exactly that ([`Segmenter::tail`]).
///
/// # An endpoint is not the end of a thought
///
/// The recognizer's endpoint fires on 800 ms of silence, and a person thinking
/// mid-sentence produces that constantly. So a closed utterance is only *sometimes*
/// a finished thing to say, and the recognizer says which by how it punctuated:
/// [`ends_a_thought`]. One that does not is **kept**, and the next utterance is
/// appended to it, so a sentence somebody hesitated in the middle of arrives whole.
/// Measured on the owner's speech (2026-09-10): 11 of 22 messages were ragged this
/// way — 5 ending in `，`, 6 in no mark at all, several of them 2–4 chars (`就是`,
/// `然后这个`) — and every one of them had its own continuation delivered as a
/// separate message 1.4–2.8 s later.
///
/// **The reason this is affordable now is a change in what being early buys.** It
/// used to buy a faster reply, so the cut was made as early as it could be. It no
/// longer does: a reply composed off half a thought is refused at the mouth
/// ([`floor`](crate::body::reaction::floor)) and re-composed by the turn the rest of
/// the sentence drives. Measured the same day: **7 of 23 replies were generated and
/// then discarded**, 5 of them inside one briefing. Early bought no speech at all —
/// it bought wasted generations, and errands dispatched off half a request that
/// Cognition then had to cancel. So the cut can afford to be *late* and right.
///
/// Holding is bounded by `hold`, measured from the last frame of any kind. A speaker
/// who resumes resets it; one who genuinely stopped mid-sentence has the fragment
/// released as it stands.
///
/// **One guard remains**: a tail at `max_chars` is force-cut, snapped back to the
/// last punctuation or clause mark, for a speaker who does not pause. It has to be
/// well clear of a spoken sentence for the same reason the hold exists — a cap that
/// cuts what the hold just merged would put the raggedness straight back, one
/// boundary further along.
///
/// There was a second, an age cap for *a recognizer whose endpoint never comes*, and
/// it went on 2026-09-10 with the alignment machinery. Once a cut can only take
/// committed text, that guard has nothing to cut in the case it names — a recognizer
/// that never commits leaves the buffer empty — so it could not do its stated job.
/// What that failure mode gets instead is the preview, which keeps showing the
/// speaker their own words, and the final the upstream flushes at stream end.
#[derive(Debug, Clone, Copy, Default)]
pub struct Speech {
    cfg: SegmenterConfig,
}

impl Speech {
    pub fn new(cfg: SegmenterConfig) -> Self {
        Self { cfg }
    }
}

impl CutPolicy for Speech {
    fn boundary(&self, cut: &Cut) -> Option<usize> {
        let n = cut.tail.len();
        if n == 0 {
            return None;
        }

        // The whole rule. What the recognizer closed is only *sometimes* a finished
        // thing to say — the close fires on 800 ms of silence and a person hesitating
        // makes plenty of it — and how it punctuated the close says which. A finished
        // sentence leaves on the settle. One that stops mid-thought is kept for the
        // rest of it, and goes out as it stands only once the speaker has actually
        // been quiet, which is why the two waits read different clocks.
        let (wait, elapsed) = if ends_a_thought(cut.tail) {
            (self.cfg.settle, cut.since_commit)
        } else {
            (self.cfg.hold, cut.quiet)
        };
        if elapsed >= wait {
            return Some(n);
        }

        // The one guard: a run-on nobody paused in. Snap back to the last phrase
        // boundary so what leaves is a clean clause and the rest stays buffered.
        if n >= self.cfg.max_chars {
            return Some(snap_back(cut.tail, n));
        }

        None
    }
}

// -----------------------------------------------------------------------------
// Terminator — append-only reply stream → sentences (structural, time-free)
// -----------------------------------------------------------------------------

/// Cut policy for an append-only token stream (the agent's reply → TTS),
/// mirroring the frontend `sentences.ts`: CJK terminators (。！？) cut
/// immediately; Latin terminators (.!?…) cut only when followed by whitespace,
/// so decimals and abbreviations aren't broken. A terminator at the very end of
/// the tail waits for more text (or [`Segmenter::flush`]). Time is ignored.
///
/// This MUST stay in agreement with the frontend splitter: the spec requires the
/// text-fade and the TTS to cut at the same places.
#[derive(Debug, Clone, Copy, Default)]
pub struct Terminator;

impl CutPolicy for Terminator {
    fn boundary(&self, cut: &Cut) -> Option<usize> {
        let chars = cut.tail;
        for (i, &c) in chars.iter().enumerate() {
            if matches!(c, '。' | '！' | '？') {
                return Some(i + 1);
            }
            if matches!(c, '.' | '!' | '?' | '…') {
                if let Some(&next) = chars.get(i + 1) {
                    if next.is_whitespace() {
                        return Some(i + 1);
                    }
                }
            }
        }
        None
    }
}

// -----------------------------------------------------------------------------
// Segmenter — the shared buffer machinery
// -----------------------------------------------------------------------------

/// Stateful segmenter. Feed it rolling stream updates; it returns completed
/// units to dispatch, cut by its [`CutPolicy`]. Time is injected so a
/// time-aware policy is deterministically testable.
///
/// **Nothing revisable is ever emitted, so nothing emitted can be revised.** That
/// one invariant is what keeps this small, and it is worth knowing what it replaced.
///
/// The buffer used to cut across both halves, and then had to answer *where in this
/// rewrite do the words I already sent end?* Twice. First with a char offset into
/// `locked + partial`, which a recognizer's rewrite invalidated: a final shorter
/// than the partial it replaced (fillers dropped, re-punctuated) put the offset past
/// the end and **lost the whole final**, taking the head of every later utterance
/// with it; a longer one sliced a different string and shipped the residue as its own
/// one-char message (`。`, `吧？`). Both were live on 2026-09-09. Then with an
/// edit-distance alignment of each revision against the emitted text, which was
/// correct — and 50 lines answering a question that only exists because the cut was
/// allowed to reach text the source had not finished writing.
///
/// It is not allowed to any more.
pub struct Segmenter<P> {
    policy: P,
    /// Committed text that has not been emitted yet, in order. **This is the
    /// buffer** — the only thing a cut is ever made from.
    locked: String,
    /// The source's current revisable utterance, whole, as it last told it.
    ///
    /// **It is never a candidate for emission.** It exists for the two things a
    /// still-changing transcript is genuinely good for: showing the speaker their
    /// own words as a preview ([`Segmenter::tail`]) and proving somebody is talking
    /// (its arrival stamps `quiet`). Letting its text become a *message* is what the
    /// buffer used to do, and it cost an edit-distance alignment of every revision
    /// against everything already emitted — 50 lines whose only job was to
    /// reconcile a guard's cut with the rewrite that followed it. Cutting only
    /// committed text deletes the problem instead of solving it.
    utterance: String,
    /// When the source last committed text — the clock a settled sentence reads.
    last_commit: Instant,
    /// When the source last said anything at all, revisable updates included — the
    /// clock the hold reads, because it is waiting for the *speaker*, not for the
    /// recognizer to commit.
    last_frame: Instant,
}

impl<P: CutPolicy> Segmenter<P> {
    pub fn new(policy: P, now: Instant) -> Self {
        Self {
            policy,
            locked: String::new(),
            utterance: String::new(),
            last_commit: now,
            last_frame: now,
        }
    }

    /// Apply a stream update. `is_final` appends the text to the buffer; a revisable
    /// update only replaces the preview. Neither itself forces a cut.
    ///
    /// A final is the authoritative text of the utterance it ends, and since nothing
    /// is ever emitted before it, this is a plain append — no reconciling against
    /// what already went out, because nothing did.
    pub fn observe(&mut self, text: &str, is_final: bool, now: Instant) -> Vec<String> {
        self.last_frame = now;
        if is_final {
            self.locked.push_str(text);
            self.utterance.clear();
            self.last_commit = now;
        } else {
            self.utterance = text.to_string();
        }
        self.cut(now)
    }

    /// Append finalized text — for sources that never revise their tail (e.g. an
    /// LLM token stream). Sugar for `observe(text, is_final = true, now)`.
    pub fn commit(&mut self, text: &str, now: Instant) -> Vec<String> {
        self.observe(text, true, now)
    }

    /// Time-driven check with no new text — drives the settle and hold once the
    /// source has gone quiet. Call on a periodic tick.
    pub fn tick(&mut self, now: Instant) -> Vec<String> {
        self.cut(now)
    }

    /// Flush everything still held as one last unit (stream end).
    ///
    /// **This is the one place the revisable utterance is emitted**, and it is safe
    /// here for the reason it is unsafe everywhere else: the stream is over, so no
    /// revision can follow to disagree with it. Losing a trailing partial would be
    /// the alternative, and it is the half-sentence somebody just said.
    pub fn flush(&mut self) -> Option<String> {
        let all = self.tail();
        self.locked.clear();
        self.utterance.clear();
        let seg = all.trim().to_string();
        (!seg.is_empty()).then_some(seg)
    }

    /// Everything heard and not yet sent: the buffer plus the utterance still being
    /// revised. That is what a recognition preview is a preview *of* — and it is
    /// deliberately **not** what a cut is made from, which is `locked` alone.
    pub fn tail(&self) -> String {
        let mut tail = self.locked.clone();
        tail.push_str(&self.utterance);
        tail
    }

    /// Cut as many units off the buffer as the policy will give, then stop.
    ///
    /// The buffer is committed text only, so emitting is a plain `drain` of the
    /// front — there is no seam to find and nothing left behind that a later
    /// revision could contradict.
    fn cut(&mut self, now: Instant) -> Vec<String> {
        let mut out = Vec::new();
        while !self.locked.trim().is_empty() {
            let chars: Vec<char> = self.locked.chars().collect();
            let ctx = Cut {
                tail: &chars,
                since_commit: now.duration_since(self.last_commit),
                quiet: now.duration_since(self.last_frame),
            };
            let Some(b) = self.policy.boundary(&ctx).filter(|b| *b > 0) else { break };
            self.locked = chars.iter().skip(b).collect();
            let seg: String = chars.iter().take(b).collect();
            let seg = seg.trim().to_string();
            if !seg.is_empty() {
                out.push(seg);
            }
            // Loop again: a backlog may hold more than one unit.
        }
        out
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod speech_tests {
    use super::*;

    fn seg() -> (Segmenter<Speech>, Instant) {
        let t0 = Instant::now();
        (Segmenter::new(Speech::default(), t0), t0)
    }

    /// The settle is short, and only a tick can carry it; every test that expects a
    /// finalized utterance to leave advances past it.
    const SETTLED: Duration = Duration::from_millis(200);

    // ---- The unit: one finalized utterance -----------------------------------

    #[test]
    fn a_finalized_utterance_is_one_message_however_many_sentences_it_holds() {
        let (mut s, t0) = seg();
        // Three sentences, one utterance. The recognizer closed it once, so it is
        // one thing the person said and it arrives as one message.
        assert!(s.observe("你好。最近怎么样？我这边挺好的。", true, t0).is_empty());
        assert_eq!(s.tick(t0 + SETTLED), vec!["你好。最近怎么样？我这边挺好的。"]);
    }

    #[test]
    fn a_partial_never_cuts_however_much_punctuation_it_grows() {
        let (mut s, t0) = seg();
        // Punctuation in a rolling partial is a transcription boundary, not the end
        // of what somebody is saying — and the words are not final either.
        assert!(s.observe("你好。", false, t0).is_empty());
        assert!(s.observe("你好。最近", false, t0 + Duration::from_millis(400)).is_empty());
        assert!(s.tick(t0 + Duration::from_secs(3)).is_empty());
        // It leaves when the recognizer closes it — in its corrected form.
        assert!(s.observe("你好，最近怎么样？", true, t0 + Duration::from_secs(4)).is_empty());
        assert_eq!(s.tick(t0 + Duration::from_secs(4) + SETTLED), vec!["你好，最近怎么样？"]);
    }

    #[test]
    fn two_utterances_finalized_together_leave_as_one_message() {
        // The settle exists for this: a recognizer that closes twice in one breath
        // sends one message, not two fragments a person never separated.
        let (mut s, t0) = seg();
        assert!(s.observe("嗯。", true, t0).is_empty());
        assert!(s.observe("那就这样。", true, t0 + Duration::from_millis(50)).is_empty());
        assert_eq!(s.tick(t0 + SETTLED), vec!["嗯。那就这样。"]);
    }

    #[test]
    fn a_new_utterance_after_one_settled_is_its_own_message() {
        let (mut s, t0) = seg();
        assert!(s.observe("你好。", true, t0).is_empty());
        assert_eq!(s.tick(t0 + SETTLED), vec!["你好。"]);
        assert!(s.observe("在吗", false, t0 + Duration::from_secs(1)).is_empty());
        assert!(s.observe("在吗？", true, t0 + Duration::from_secs(2)).is_empty());
        assert_eq!(s.tick(t0 + Duration::from_secs(2) + SETTLED), vec!["在吗？"]);
    }

    // ---- Stopping mid-thought ------------------------------------------------

    /// Past the hold, so a fragment the speaker never finished has been given up on.
    const HELD: Duration = SegmenterConfig::DEFAULT_HOLD.saturating_add(Duration::from_millis(100));

    #[test]
    fn an_utterance_that_stops_mid_thought_is_not_a_message_yet() {
        let (mut s, t0) = seg();
        // The endpoint fired on a hesitation, so the recognizer closed a sentence it
        // punctuated with a comma. That is 800 ms of silence, not a finished thought.
        assert!(s.observe("我可能更多希望鞋好一点吧，", true, t0).is_empty());
        assert!(s.tick(t0 + SETTLED).is_empty(), "the settle must not release it");
    }

    #[test]
    fn a_held_fragment_and_its_continuation_are_one_message() {
        let (mut s, t0) = seg();
        // Exactly the 2026-09-10 shape: an 85-char clause closed on `，`, and its own
        // ending delivered 2.05 s later as a 14-char message of its own.
        assert!(s.observe("我可能更多希望鞋好一点吧，", true, t0).is_empty());
        let t1 = t0 + Duration::from_millis(900);
        assert!(s.observe("就是倒不在乎说一定要", false, t1).is_empty());
        let t2 = t1 + Duration::from_millis(600);
        assert!(s.observe("就是倒不在乎说一定要追求特别贵的。", true, t2).is_empty());
        assert_eq!(
            s.tick(t2 + SETTLED),
            vec!["我可能更多希望鞋好一点吧，就是倒不在乎说一定要追求特别贵的。"]
        );
    }

    #[test]
    fn a_speaker_who_resumes_keeps_resetting_the_hold() {
        let (mut s, t0) = seg();
        assert!(s.observe("然后这个", true, t0).is_empty());
        // Frames keep arriving inside the hold, so it never expires…
        let mut now = t0;
        for i in 1..8 {
            now = t0 + Duration::from_millis(i * 900);
            assert!(
                s.observe(&format!("鞋的这个要求{}", "啊".repeat(i as usize)), false, now).is_empty(),
                "released at {i}"
            );
        }
        // …and the whole sentence lands as one message when it is finally closed.
        assert!(s.observe("鞋的这个要求你清楚了吗？", true, now).is_empty());
        assert_eq!(s.tick(now + SETTLED), vec!["然后这个鞋的这个要求你清楚了吗？"]);
    }

    #[test]
    fn a_speaker_who_stops_mid_sentence_still_gets_their_words_out() {
        let (mut s, t0) = seg();
        assert!(s.observe("就是", true, t0).is_empty());
        assert!(s.tick(t0 + Duration::from_secs(1)).is_empty());
        assert_eq!(s.tick(t0 + HELD), vec!["就是"]);
    }

    #[test]
    fn a_thought_that_ends_is_not_delayed_by_the_hold() {
        // The load-bearing half: the message that ends a turn is the one a reply is
        // owed on, and it still leaves on the settle.
        let (mut s, t0) = seg();
        assert!(s.observe("这鞋值不值得我去淘宝上搜一下，对吧？", true, t0).is_empty());
        assert_eq!(s.tick(t0 + SETTLED), vec!["这鞋值不值得我去淘宝上搜一下，对吧？"]);
    }

    // ---- Revisable text is never cut -----------------------------------------

    /// Read from the config rather than repeated, so raising the cap moves the
    /// guard tests with it instead of quietly turning them into hold tests.
    const CAP: usize = SegmenterConfig::DEFAULT_MAX_CHARS;

    #[test]
    fn a_partial_is_never_cut_however_far_past_the_cap_it_runs() {
        // The invariant the whole buffer is built on, and the one that deleted the
        // alignment machinery: a rolling partial is preview text, so no matter how
        // long it grows none of it may become a message — the next revision is free
        // to rewrite every character of it.
        let (mut s, t0) = seg();
        let mut now = t0;
        for i in 1..=4 {
            now = t0 + Duration::from_secs(i);
            assert!(s.observe(&"字".repeat(CAP * i as usize), false, now).is_empty());
        }
        assert!(s.tick(now + Duration::from_secs(30)).is_empty(), "no clock may cut it either");
        // It becomes cuttable the instant the recognizer commits it, and not before.
        let out = s.observe(&"字".repeat(CAP + 4), true, now);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].chars().count(), CAP + 4); // nothing to snap to → chop whole
    }

    #[test]
    fn size_guard_snaps_to_the_last_clause_and_buffers_the_rest() {
        let (mut s, t0) = seg();
        // Just past the cap, so the remainder left buffered is under it and the
        // guard fires exactly once.
        let head = CAP / 2;
        let text = format!("{}，{}", "啊".repeat(head), "哦".repeat(CAP - head));
        let out = s.observe(&text, true, t0);
        assert_eq!(out.len(), 1);
        assert!(out[0].ends_with('，'));
        assert_eq!(out[0].chars().count(), head + 1); // the clause plus its comma
        // What is left is under the cap and ends mid-thought, so it waits for the
        // hold rather than trailing out behind the clause it was cut from.
        assert!(s.tick(t0 + Duration::from_millis(50)).is_empty());
        assert_eq!(s.tick(t0 + HELD).len(), 1);
    }

    #[test]
    fn size_boundary_is_exact() {
        let (mut under, t0) = seg();
        assert!(under.observe(&"字".repeat(CAP - 1), true, t0).is_empty());
        let (mut at, _) = seg();
        let out = at.observe(&"字".repeat(CAP), true, t0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].chars().count(), CAP);
    }

    #[test]
    fn whitespace_only_input_never_cuts() {
        let (mut s, t0) = seg();
        assert!(s.observe("   ", false, t0).is_empty());
        assert!(s.tick(t0 + Duration::from_secs(30)).is_empty());
    }

    #[test]
    fn multibyte_boundaries_are_char_aligned() {
        let (mut s, t0) = seg();
        assert!(s.observe("OK。next 就这样。", true, t0).is_empty());
        assert_eq!(s.tick(t0 + SETTLED), vec!["OK。next 就这样。"]);
    }

    #[test]
    fn flush_emits_pending_tail_then_nothing() {
        let (mut s, t0) = seg();
        assert!(s.observe("还没说完", false, t0).is_empty());
        assert_eq!(s.flush(), Some("还没说完".to_string()));
        assert_eq!(s.flush(), None);
    }

    #[test]
    fn flush_is_none_when_empty() {
        let (mut s, _t0) = seg();
        assert_eq!(s.flush(), None);
    }

    #[test]
    fn the_tail_is_what_has_not_become_a_message_yet() {
        // What the recognition preview shows: heard, not yet settled. It never
        // repeats a line that is already in the conversation above it.
        let (mut s, t0) = seg();
        assert!(s.observe("你好", false, t0).is_empty());
        assert_eq!(s.tail(), "你好");
        assert!(s.observe("你好，我是小明。", true, t0 + Duration::from_millis(500)).is_empty());
        assert_eq!(s.tail(), "你好，我是小明。");
        assert_eq!(
            s.tick(t0 + Duration::from_millis(700)),
            vec!["你好，我是小明。"]
        );
        assert_eq!(s.tail(), "");
    }
}

#[cfg(test)]
mod terminator_tests {
    use super::*;

    fn seg() -> (Segmenter<Terminator>, Instant) {
        let t0 = Instant::now();
        (Segmenter::new(Terminator, t0), t0)
    }

    #[test]
    fn splits_latin_on_terminator_plus_space() {
        let (mut s, t0) = seg();
        assert_eq!(s.commit("Hello world", t0), Vec::<String>::new());
        assert_eq!(s.commit(". How are", t0), vec!["Hello world.".to_string()]);
        assert_eq!(s.flush(), Some("How are".to_string()));
    }

    #[test]
    fn does_not_split_decimals() {
        let (mut s, t0) = seg();
        assert_eq!(s.commit("pi is 3.14 today", t0), Vec::<String>::new());
    }

    #[test]
    fn splits_cjk_immediately() {
        let (mut s, t0) = seg();
        assert_eq!(
            s.commit("你好。最近怎么样？", t0),
            vec!["你好。".to_string(), "最近怎么样？".to_string()]
        );
    }
}
