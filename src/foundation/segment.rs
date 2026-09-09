//! Segmentation — aggregating a continuous signal into the coarser grain a
//! downstream consumer accepts. One mechanism, swappable cut policy.
//!
//! # One aggregator, two policies
//!
//! Both places we segment are the *same* operation: take a fine, continuous
//! stream and cut it into the coarser units the next stage wants. They differ
//! only in *where to cut* and in how the input behaves:
//!
//! - **[`Speech`]** — STT word-stream → **one message per finalized utterance**.
//!   The upstream revises the whole utterance until it closes it, so the cut waits
//!   for that close; size and age are guards against a speaker or a recognizer that
//!   never gets there.
//! - **[`Terminator`]** — the agent's reply stream → sentences for TTS. The
//!   source is append-only (an LLM never un-says a token), so the cut is purely
//!   structural: a sentence terminator, mirroring the frontend `sentences.ts`.
//!
//! The buffer machinery is shared; the [`CutPolicy`] supplies the boundary rule.
//!
//! # The buffer model
//!
//! The buffer holds **what has been heard and not yet emitted** — nothing else.
//! Emitting removes text from it rather than advancing a pointer over it, because
//! the source rewrites the very text a pointer would be counted against.
//!
//! ```text
//!   locked             utterance (whole, revisable)
//!   ┌───────────────┐  ┌──────────────────┬──────────┐
//!   │ committed,    │  │ already said     │ pending  │
//!   │ not yet out   │  └──────────────────┴──────────┘
//!   └───────────────┘   the seam is found by aligning the revision against
//!                       `said` — the text already emitted, kept as text
//!
//!   tail = locked + pending                     ← what a cut is made from
//! ```
//!
//! - `observe(text, is_final=false)` replaces `utterance` with the latest rolling
//!   text (a revisable source rewrites the whole utterance, not just its tail).
//! - `observe(text, is_final=true)` — also reachable as [`Segmenter::commit`] — is
//!   the authoritative text of *the utterance it ends*. Only the part of it that has
//!   not already gone out moves into `locked` ([`unemitted`]); appending it whole
//!   would repeat every word already emitted from its own partial. Append-only
//!   sources never emit before committing, so for them this is a plain append and
//!   the time-based rules never fire.
//! - A cut removes the emitted chars from the buffer; whatever follows stays for
//!   the next segment. Incomplete trailing words are never flushed just because an
//!   earlier part of the buffer was emitted.
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
    /// The undispatched suffix, char-aligned.
    pub tail: &'a [char],
    /// How many leading chars of `tail` are committed (definite) text, which can
    /// never be revised — punctuation there is settled the instant it is seen.
    pub committed: usize,
    /// How long the tail has been unchanged.
    pub since_change: Duration,
    /// How long the current undispatched segment has been accumulating.
    pub since_start: Duration,
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

/// Is this char part of the *words*, rather than of the punctuation and spacing a
/// revisable source rewrites freely between one revision and the next?
fn is_significant(c: char) -> bool {
    !c.is_whitespace() && !is_sentence_end(c) && !is_clause_boundary(c)
}

/// The part of `revision` that has not been emitted yet, given `said` — the text
/// already emitted out of an **earlier revision of the same utterance**.
///
/// The two are versions of the same words, so the question is where in `revision`
/// the ones already said end. It cannot be answered by counting: what a recognizer
/// changes when it revises is exactly what would have to hold still — it inserts
/// punctuation ("你好我是小明" → "你好，我是小明。"), drops fillers ("嗯，那我们" → "那我们"),
/// and corrects homophones. So `revision` is aligned against `said`: the prefix of
/// `revision` closest to `said` in edit distance is the part already said, and
/// everything after it is what is left to say. Comparison is over significant chars
/// only, and the remainder starts at a word — which is what stops a re-punctuated
/// final from shipping its newly added `。` as a message of its own.
///
/// Ties go to the shortest prefix, so a revision that merely re-punctuates what was
/// said returns everything genuinely new and nothing already shown. The alignment is
/// O(|said| · |revision|) over one utterance, computed once per revision.
fn unemitted<'a>(revision: &'a str, said: &str) -> &'a str {
    if said.trim().is_empty() {
        return revision;
    }
    // Significant chars of each side, with each `revision` char remembering where
    // in the original it starts, so a boundary found in word-space maps back.
    let rev: Vec<(usize, char)> =
        revision.char_indices().filter(|(_, c)| is_significant(*c)).collect();
    let old: Vec<char> = said.chars().filter(|c| is_significant(*c)).collect();
    if rev.is_empty() {
        return "";
    }

    // Edit distance from `old` to every prefix of `rev`; the best prefix is the one
    // already said. `row[j]` is the distance between `old[..i]` and `rev[..j]`.
    let mut row: Vec<usize> = (0..=rev.len()).collect();
    for (i, o) in old.iter().enumerate() {
        let mut next = vec![i + 1; rev.len() + 1];
        for j in 0..rev.len() {
            let substitute = row[j] + usize::from(*o != rev[j].1);
            next[j + 1] = substitute.min(row[j + 1] + 1).min(next[j] + 1);
        }
        row = next;
    }
    let cut = (0..=rev.len()).min_by_key(|j| (row[*j], *j)).unwrap_or(rev.len());

    let from = rev.get(cut).map(|(i, _)| *i).unwrap_or(revision.len());
    &revision[from..]
}

// -----------------------------------------------------------------------------
// Speech — STT word-stream → sentences (revisable source, time-aware)
// -----------------------------------------------------------------------------

/// Tunable thresholds for the [`Speech`] guards. The unit itself is not tunable:
/// it is where the speaker stopped.
#[derive(Debug, Clone, Copy)]
pub struct SegmenterConfig {
    /// Hard cap on undispatched chars before a forced cut (run-on guard).
    pub max_chars: usize,
    /// A finalized utterance must hold still this long before it leaves, so a
    /// recognizer that finalizes twice in quick succession — a correction, or two
    /// utterances in one frame — sends one message rather than two fragments.
    pub settle: Duration,
    /// A segment older than this is force-cut mid-utterance (monologue guard, for
    /// a recognizer whose endpoint never comes).
    pub max_segment: Duration,
}

impl Default for SegmenterConfig {
    fn default() -> Self {
        Self {
            max_chars: 96,
            settle: Duration::from_millis(150),
            max_segment: Duration::from_secs(20),
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
/// Two guards remain, and only they can split an utterance:
///
/// 1. **Size** — a tail at `max_chars` is force-cut, snapped back to the last
///    punctuation or clause mark, for a speaker who does not pause.
/// 2. **Age** — a segment older than `max_segment` is force-cut the same way, for a
///    recognizer whose endpoint never comes at all.
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

        // 1. A finalized utterance, whole. `committed >= n` means nothing in the
        //    buffer is still revisable — the recognizer has closed everything it
        //    holds — and the settle lets a second finalization in the same breath
        //    join it instead of arriving as its own fragment.
        if n > 0 && cut.committed >= n && cut.since_change >= self.cfg.settle {
            return Some(n);
        }

        // 2. Size — run-on guard. Snap back to the last phrase boundary so we emit
        //    a clean clause and keep the rest buffered.
        if n >= self.cfg.max_chars {
            return Some(snap_back(cut.tail, n));
        }

        // 3. Age — a recognizer that never calls the endpoint must not be able to
        //    hold a turn's words forever.
        if cut.since_start >= self.cfg.max_segment {
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
/// **Emitting removes text from the buffer.** It does not advance a pointer over
/// it. A pointer is what the buffer used to keep — `dispatched`, a char offset into
/// `locked + partial` — and it was wrong for the same reason the buffer has two
/// halves at all: a revisable source rewrites the very text the offset was counted
/// against. When a finalized utterance came back shorter than the partial it
/// replaced (recognizers drop fillers and re-punctuate), the offset landed past the
/// end and **the whole final was silently lost**, taking the head of the next
/// utterance with it; when it came back longer, the offset sliced a different string
/// and the residue shipped as its own one- or two-char message (`。`, `吧？`). Both
/// were live on 2026-09-09.
pub struct Segmenter<P> {
    policy: P,
    /// Finalized (definite) text that has not been emitted yet, in order.
    locked: String,
    /// The current utterance as the source last told it — revisable, whole, and
    /// including whatever part of it has already gone out.
    utterance: String,
    /// What has already been emitted out of the current utterance. Kept as text
    /// rather than a length because the next revision has to be aligned against it
    /// — see [`unemitted`].
    said: String,
    /// The part of `utterance` still to say: `unemitted(&utterance, &said)`, held
    /// so the alignment runs once per revision rather than once per tick.
    pending: String,
    /// When the current undispatched segment started accumulating.
    seg_start: Instant,
    /// When the undispatched tail last changed.
    last_change: Instant,
    /// Snapshot of the tail at `last_change`, to detect changes.
    last_tail: String,
}

impl<P: CutPolicy> Segmenter<P> {
    pub fn new(policy: P, now: Instant) -> Self {
        Self {
            policy,
            locked: String::new(),
            utterance: String::new(),
            said: String::new(),
            pending: String::new(),
            seg_start: now,
            last_change: now,
            last_tail: String::new(),
        }
    }

    /// Apply a stream update. `is_final` commits the text into the stable prefix;
    /// it does not itself cause a cut. Returns any units completed by this update.
    ///
    /// Both paths reconcile against what already went out ([`unemitted`]) rather
    /// than trusting the text to start where the last revision stopped. A final is
    /// the authoritative text of *the utterance it ends*, not an increment after it
    /// — so appending it whole would repeat every word already emitted from its own
    /// partial, and appending it blind would lose the words it rephrased.
    pub fn observe(&mut self, text: &str, is_final: bool, now: Instant) -> Vec<String> {
        if is_final {
            let rest = unemitted(text, &self.said).to_string();
            self.locked.push_str(&rest);
            self.utterance.clear();
            self.said.clear();
            self.pending.clear();
        } else {
            self.utterance = text.to_string();
            self.pending = unemitted(&self.utterance, &self.said).to_string();
        }
        self.cut(now)
    }

    /// Append finalized text — for sources that never revise their tail (e.g. an
    /// LLM token stream). Sugar for `observe(text, is_final = true, now)`.
    pub fn commit(&mut self, text: &str, now: Instant) -> Vec<String> {
        self.observe(text, true, now)
    }

    /// Time-driven check with no new text — drives the stability and max-segment
    /// cuts when the source has gone quiet. Call on a periodic tick.
    pub fn tick(&mut self, now: Instant) -> Vec<String> {
        self.cut(now)
    }

    /// Flush whatever undispatched text remains as a final unit (stream end).
    pub fn flush(&mut self) -> Option<String> {
        let tail = self.tail();
        let seg = tail.trim().to_string();
        self.take(tail.chars().count());
        if seg.is_empty() {
            return None;
        }
        Some(seg)
    }

    /// The heard-but-not-yet-emitted text — everything this segmenter is still
    /// holding. It is what the next cut will be made from, and it is exactly what
    /// the person has said that has not become a message yet, which is what a
    /// recognition preview is a preview *of*.
    pub fn tail(&self) -> String {
        let mut tail = self.locked.clone();
        tail.push_str(&self.pending);
        tail
    }

    /// Remove the first `n` chars of the tail from the buffer — which is what
    /// emitting them means. They come off `locked` first, then off the utterance's
    /// pending part, where removing them also records them as said so the next
    /// revision of that utterance can be aligned against them.
    fn take(&mut self, n: usize) {
        let from_locked = n.min(self.locked.chars().count());
        if from_locked > 0 {
            self.locked = self.locked.chars().skip(from_locked).collect();
        }
        let from_pending = n - from_locked;
        if from_pending > 0 {
            let said: String = self.pending.chars().take(from_pending).collect();
            self.said.push_str(&said);
            self.pending = self.pending.chars().skip(from_pending).collect();
        }
    }

    fn cut(&mut self, now: Instant) -> Vec<String> {
        let mut out = Vec::new();
        loop {
            let tail = self.tail();
            if tail.trim().is_empty() {
                // Nothing pending; reset the segment clock so the next unit is
                // timed from when it actually begins.
                if tail != self.last_tail {
                    self.last_tail = tail;
                    self.seg_start = now;
                    self.last_change = now;
                }
                break;
            }
            if tail != self.last_tail {
                if self.last_tail.trim().is_empty() {
                    self.seg_start = now; // a fresh segment just began
                }
                self.last_tail = tail.clone();
                self.last_change = now;
            }

            let chars: Vec<char> = tail.chars().collect();
            let ctx = Cut {
                committed: self.locked.chars().count(),
                since_change: now.duration_since(self.last_change),
                since_start: now.duration_since(self.seg_start),
                tail: &chars,
            };
            match self.policy.boundary(&ctx) {
                Some(b) if b > 0 => {
                    let seg: String = chars.iter().take(b).collect();
                    let seg = seg.trim().to_string();
                    self.take(b);
                    self.seg_start = now;
                    self.last_change = now;
                    self.last_tail = self.tail();
                    if !seg.is_empty() {
                        out.push(seg);
                    }
                    // Loop again: a backlog may hold more than one unit.
                }
                _ => break,
            }
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

    // ---- The two guards ------------------------------------------------------

    #[test]
    fn size_guard_cuts_a_runon_without_waiting_for_the_endpoint() {
        let (mut s, t0) = seg();
        let long: String = "字".repeat(100); // over max_chars (96), no punctuation
        let out = s.observe(&long, false, t0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].chars().count(), 100); // nothing to snap to → chop whole
    }

    #[test]
    fn size_guard_snaps_to_the_last_clause_and_buffers_the_rest() {
        let (mut s, t0) = seg();
        let text = format!("{}，{}", "啊".repeat(40), "哦".repeat(60)); // 101 chars
        let out = s.observe(&text, false, t0);
        assert_eq!(out.len(), 1);
        assert!(out[0].ends_with('，'));
        assert_eq!(out[0].chars().count(), 41); // 40 + the comma
        assert!(s.observe(&text, false, t0 + Duration::from_millis(50)).is_empty());
    }

    #[test]
    fn size_boundary_is_exact() {
        let (mut s, t0) = seg();
        assert!(s.observe(&"字".repeat(95), false, t0).is_empty());
        let out = s.observe(&"字".repeat(96), false, t0 + Duration::from_millis(10));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].chars().count(), 96);
    }

    #[test]
    fn age_guard_force_cuts_a_recognizer_that_never_closes() {
        let (mut s, t0) = seg();
        for i in 0..80 {
            let now = t0 + Duration::from_millis(i * 400);
            let out = s.observe(&"说".repeat((i as usize) + 1), false, now);
            if !out.is_empty() {
                assert!(now.duration_since(t0) >= Duration::from_secs(20));
                return;
            }
        }
        panic!("a monologue was never force-cut");
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
        assert!(s.observe("OK。next", true, t0).is_empty());
        assert_eq!(s.tick(t0 + SETTLED), vec!["OK。next"]);
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

    // ---- Reconciling a final against the partial it replaces -----------------
    //
    // Rarer now that only the guards emit un-final text, but unchanged in kind: the
    // three shapes below are what a recognizer does when it finalizes an utterance
    // a guard already emitted from, and each one broke the char offset the buffer
    // used to keep (all three observed live, 2026-09-09).

    #[test]
    fn a_final_that_drops_a_filler_keeps_the_rest_and_says_nothing_twice() {
        let (mut s, t0) = seg();
        // The size guard ships the first clause of a run-on…
        let head = format!("嗯，{}，", "那我们就这样定了".repeat(12)); // past max_chars
        let out = s.observe(&head, false, t0);
        assert_eq!(out.len(), 1);
        assert!(out[0].starts_with("嗯，"));
        // …and the final for that same utterance drops the "嗯" and re-punctuates.
        // Only the words not already said come through — the rephrased ones are not
        // repeated, and the tail the guard never reached is not lost.
        //
        // The offset this replaces counted into a string that was now one word
        // shorter, and shipped the residue at that position as its own message.
        let mut fin = head.trim_start_matches("嗯，").to_string();
        fin.push_str("好的。");
        assert!(s.observe(&fin, true, t0 + Duration::from_millis(200)).is_empty());
        let out = s.tick(t0 + Duration::from_millis(400));
        assert_eq!(out.len(), 1);
        assert!(out[0].ends_with("好的。"), "got {:?}", out[0]);
        assert!(!out[0].starts_with('。'), "a lone mark is not a message: {:?}", out[0]);
    }

    #[test]
    fn a_final_that_only_repunctuates_has_nothing_left_to_say() {
        let (mut s, t0) = seg();
        // A run-on with no punctuation, force-cut whole by the size guard.
        let long = "字".repeat(96);
        assert_eq!(s.observe(&long, false, t0).len(), 1);
        // Its final adds the period the partial never had. That period is not a
        // message; the words it ends were said a moment ago.
        assert!(s.observe(&format!("{long}。"), true, t0 + Duration::from_millis(200)).is_empty());
        assert!(s.tick(t0 + Duration::from_millis(400)).is_empty());
    }

    #[test]
    fn a_short_final_does_not_swallow_the_head_of_the_next_utterance() {
        let (mut s, t0) = seg();
        // The guard emits from the partial…
        let long = "字".repeat(96);
        assert_eq!(s.observe(&long, false, t0).len(), 1);
        // …and the final comes back SHORTER than what was already emitted.
        assert!(s
            .observe(&"字".repeat(90), true, t0 + Duration::from_millis(200))
            .is_empty());
        assert!(s.tick(t0 + Duration::from_millis(400)).is_empty());
        // The damage the offset did was not limited to that utterance: it stayed
        // past the end of the buffer, and every later utterance lost its head — for
        // as long as the session lasted.
        assert!(s.observe("到第二个大块。", true, t0 + Duration::from_secs(1)).is_empty());
        assert_eq!(s.tick(t0 + Duration::from_secs(1) + SETTLED), vec!["到第二个大块。"]);
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
