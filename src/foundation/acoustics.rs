//! What a stretch of microphone audio *sounded like* — as distinct from what was
//! said in it.
//!
//! A person in a crowded room barely uses the words to work out who is talking to
//! them. They use physics: the near voice is loud and dry, the one across the room
//! arrives faint and smeared into the noise, and somebody speaking over somebody else
//! is not addressing either. None of that was available here. The PCM was in
//! [`crate::foundation::server::audio`]'s hands the whole time and went straight to
//! the recognizer; the mind downstream saw a sentence with a name on it and had no
//! way to tell a remark from the next table from one said into the mic.
//!
//! **Everything measured here is scale-free**, which is the whole reason it can be
//! trusted across machines. A microphone's absolute dBFS says as much about its gain
//! and its automatic level control as about the room, so an absolute "loud" threshold
//! would mean something different on every install. What survives that is a
//! *difference* of two levels — speech against the floor it sits on ([`Sound::snr_db`]),
//! this speaker against the others in the room ([`Distance`]) — and a fact from the
//! clock ([`Reading::crosstalk`]).
//!
//! And it stays evidence. Nothing here decides whether the agent was addressed, or
//! drops a signal for being faint: the perception layer stays mechanical and the
//! judgment is the mind's, one turn at a time
//! ([`docs/user-journeys/31-hear-the-room.md`]). What this module removes is the
//! excuse that there was nothing to judge with.

use std::collections::VecDeque;

/// Analysis frame: 20 ms at 16 kHz, the usual window for a level envelope — short
/// enough to sit inside a syllable, long enough that its RMS is not one glottal pulse.
const FRAME: usize = 320;

/// Level reported for a frame with no signal at all. Silence is negative infinity in
/// dB and a percentile over infinities is arithmetic nonsense, so digital silence
/// stops here.
const FLOOR_DBFS: f32 = -100.0;

/// Magnitude at/above which a sample counts as pinned against the rail.
const CLIP_LEVEL: u16 = 32_000;

/// Where the line between "somebody is talking" and "the room" goes for
/// [`Sound::voiced`]: this far up the stretch's own dynamic range, from its floor
/// toward its speech level. Relative for the usual reason — an absolute level would
/// mean something different on every microphone — and halfway because that is where
/// the gap between a syllable and the silence around it sits.
const VOICED_AT: f32 = 0.5;

/// The level of the speech itself: a high percentile of the frame envelope, i.e. the
/// loud part, not an average dragged down by the pauses between words.
const SPEECH_PCT: f32 = 0.90;

/// The floor the speech sits on: a low percentile, i.e. the room between the words.
const FLOOR_PCT: f32 = 0.10;

/// Speech this far above its own floor came through cleanly.
const CLEAR_SNR_DB: f32 = 20.0;

/// Speech this close to its own floor arrived buried — the room, a distance, a hand
/// over the mouth, a television. A guess until watched against real rooms.
const MUFFLED_SNR_DB: f32 = 12.0;

/// Within this of the loudest voice around, a speaker is one of the near ones.
const NEAR_DB: f32 = 6.0;

/// This far under the loudest voice around, a speaker is somewhere else in the room.
/// Guesses, like the two above.
const FAR_DB: f32 = 12.0;

/// The shortest overlap between two speakers' turns that counts as one of them
/// talking across the other. Below it, the two turns are treated as touching.
///
/// **The diarizer's turn boundaries are approximate**, and the overlap test is a
/// strict comparison of them: without a floor, two back-to-back turns whose edges
/// disagree by a few milliseconds read as an interruption. That cost nothing while
/// crosstalk only kept a sample out of a gallery; it costs an identity now that a
/// heavily-overlapped turn also stops being recognized
/// ([`crate::foundation::server::audio`]).
///
/// **A guess, and the one number here most worth measuring.** It has to sit above the
/// vendor's boundary jitter and below a real short interjection — "嗯", "对", a name
/// called across a room — and nothing has measured either end on this install. Too
/// low and a lively room stops recognizing anybody; too high and a genuine
/// interruption is filed as one person's clean speech. The measurement is the overlap
/// distribution over one real multi-party recording, which `resolve_speaker` logs at
/// debug for exactly this reason.
pub(crate) const OVERLAP_JITTER_MS: u64 = 150;

/// How far back the room is remembered. Long enough that a second person who spoke a
/// few sentences ago still counts as being here; short enough that a room empties.
const WINDOW_MS: u64 = 60_000;

/// One stretch of audio as physics: how loud the speech was, what it sat on, and
/// whether it was pinned against the rail. Levels are dBFS — meaningful only as
/// differences (see the module note), which is why nothing here is called "loud".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sound {
    /// The speech itself — the [`SPEECH_PCT`] frame of the envelope.
    pub speech_dbfs: f32,
    /// The room between the words — the [`FLOOR_PCT`] frame.
    pub floor_dbfs: f32,
    /// Fraction of samples pinned at [`CLIP_LEVEL`]. A capture that is clipping is
    /// too hot to read levels from, and its voiceprint is distorted too.
    pub clipped: f32,
    /// Fraction of the stretch where somebody was actually talking, rather than
    /// where the room was ([`VOICED_AT`]).
    ///
    /// **This is not the signal-to-noise ratio and cannot be got from it.** A
    /// three-second clip holding one grunt has an excellent [`Self::snr_db`] — the
    /// grunt is loud and the rest is quiet, which is all a ratio of two levels can
    /// say. Measured over this install's stored voice samples, 97% clear 15 dB while
    /// the median holds only 1.46 seconds of actual speech; duration and SNR both
    /// pass the clips that nobody, machine or person, could identify anyone from.
    pub voiced: f32,
}

impl Sound {
    /// Seconds of actual speech in a stretch of `samples` at 16 kHz — the length that
    /// matters, as against how long the recording is.
    pub fn voiced_secs(&self, samples: usize) -> f32 {
        samples as f32 * self.voiced / 16_000.0
    }

    /// How far the speech stands above the room it was spoken in. **The scale-free
    /// one**: both terms come from the same capture through the same gain, so
    /// whatever the microphone did to one it did to the other.
    pub fn snr_db(&self) -> f32 {
        self.speech_dbfs - self.floor_dbfs
    }
}

/// Measure one stretch of 16 kHz mono 16-bit PCM — the live mic's format, and the
/// same slice the voiceprint is taken from. `None` for anything shorter than a single
/// frame, which is not a stretch of speech at all.
pub fn measure(pcm: &[i16]) -> Option<Sound> {
    if pcm.len() < FRAME {
        return None;
    }
    let mut levels: Vec<f32> = pcm.chunks_exact(FRAME).map(frame_dbfs).collect();
    levels.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let clipped =
        pcm.iter().filter(|s| s.unsigned_abs() >= CLIP_LEVEL).count() as f32 / pcm.len() as f32;
    let speech_dbfs = percentile(&levels, SPEECH_PCT);
    let floor_dbfs = percentile(&levels, FLOOR_PCT);
    let gate = floor_dbfs + (speech_dbfs - floor_dbfs) * VOICED_AT;
    let voiced = levels.iter().filter(|l| **l >= gate).count() as f32 / levels.len() as f32;
    Some(Sound { speech_dbfs, floor_dbfs, clipped, voiced })
}

/// RMS of one frame in dBFS, floored at [`FLOOR_DBFS`] so silence is a number.
fn frame_dbfs(frame: &[i16]) -> f32 {
    let sum: f64 = frame.iter().map(|s| (*s as f64 / 32_768.0).powi(2)).sum();
    let rms = (sum / frame.len() as f64).sqrt() as f32;
    if rms <= 0.0 {
        return FLOOR_DBFS;
    }
    (20.0 * rms.log10()).max(FLOOR_DBFS)
}

/// The value at `p` through an ascending slice. Nearest-rank, not interpolated: these
/// feed a three-way verdict, and a tenth of a dB never decides one.
fn percentile(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return FLOOR_DBFS;
    }
    let idx = ((sorted.len() - 1) as f32 * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Where a speaker is, relative to everyone else the mic has heard lately.
///
/// **Relative on purpose.** "Far" as an absolute level cannot be stated without
/// knowing the microphone's gain; "12 dB under the person who is also in this room"
/// can. With only one voice around there is nothing to be relative to, so the answer
/// is [`Distance::Unplaced`] and the clarity carries what it can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Distance {
    /// As loud as the loudest voice around — one of the near ones.
    Near,
    /// Well under it: somewhere else in the room, or turned away.
    Far,
    /// Nothing to compare against, or in between.
    Unplaced,
}

/// Whether the speech arrived intact or buried in the room it crossed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clarity {
    Clear,
    /// Close to its own noise floor: distance, a bad angle, a room, a television.
    Muffled,
    /// Neither — and not worth saying.
    Ordinary,
}

/// One diarized turn's audio, with the span the vendor placed it at.
#[derive(Debug, Clone)]
pub struct Turn {
    /// The vendor's within-session speaker label.
    pub speaker: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub sound: Sound,
}

/// What the room was like when a turn landed in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// Distinct diarized speakers heard in the last [`WINDOW_MS`], this one included.
    /// A count from the clock and the vendor's labels — no judgment in it.
    pub voices: usize,
    pub distance: Distance,
    pub clarity: Clarity,
    /// This turn's speech level against the median of the room's recent turns, in dB.
    /// Negative means quieter than the room has lately been — someone who turned away,
    /// spoke from a doorway, or muttered.
    ///
    /// A difference, so the microphone's gain cancels; and against the *median of
    /// everything recently heard* rather than the loudest other speaker, so it still
    /// says something when one person is talking alone. `None` on the first turn of a
    /// stream, where there is nothing to be quieter than.
    pub level_vs_room: Option<f32>,
    /// How many milliseconds of this turn another speaker was talking across, summed
    /// over everyone who did. **A quantity, not a flag** — someone who put one word
    /// in and someone who talked over half the sentence are not the same event, and
    /// the callers that act on this need different amounts of it (`server::audio`:
    /// any overlap disqualifies the turn as a stored sample, but only a large share
    /// of it makes the recording stop being that speaker's voice).
    ///
    /// Overlaps shorter than [`OVERLAP_JITTER_MS`] count as zero: the vendor's turn
    /// boundaries are approximate, and a two-millisecond touch between back-to-back
    /// turns is the diarizer's arithmetic rather than anybody interrupting.
    pub overlap_ms: u64,
    /// This turn's own length, so a share can be taken without the caller having to
    /// have kept the turn. See [`Reading::overlap_share`].
    pub dur_ms: u64,
}

impl Reading {
    /// Whether anybody talked across this turn at all. Someone talking over someone
    /// else is not, in that moment, addressing either of them — but that is the
    /// mind's inference to draw, not this module's.
    pub fn crosstalk(&self) -> bool {
        self.overlap_ms > 0
    }

    /// What share of this turn somebody else was talking across, in `[0, 1]`. `0.0`
    /// for a turn with no length to speak of.
    ///
    /// **The share, not the milliseconds, is what says whether the recording is still
    /// one person's.** Three hundred milliseconds inside a four-second remark leaves a
    /// waveform that is overwhelmingly the speaker; the same three hundred inside a
    /// half-second one leaves a blend of two people. A caller weighing an embedding
    /// wants this; a caller deciding whether to keep a sample wants
    /// [`Self::crosstalk`], which does not forgive any of it.
    pub fn overlap_share(&self) -> f32 {
        if self.dur_ms == 0 {
            return 0.0;
        }
        (self.overlap_ms as f32 / self.dur_ms as f32).clamp(0.0, 1.0)
    }

    /// The compact evidence note for the agent-facing transcript, e.g.
    /// ` ⟨room: 3 voices, this one far off and faint⟩` — the audio twin of
    /// `⟨voice: …⟩` and `⟨faces: …⟩`. `None` when there is nothing worth saying: one
    /// voice, near enough, clear enough. **The ordinary case says nothing**, so a note
    /// appearing at all means the room is not the simple one.
    pub fn note(&self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if self.voices > 1 {
            parts.push(format!("{} voices", self.voices));
        }
        let this = match (self.distance, self.clarity) {
            (Distance::Far, Clarity::Muffled) => Some("far off and faint"),
            (Distance::Far, _) => Some("far off"),
            (Distance::Near, Clarity::Muffled) => Some("close but faint"),
            (Distance::Near, _) if self.voices > 1 => Some("the closest"),
            (_, Clarity::Muffled) => Some("faint"),
            _ => None,
        };
        if let Some(this) = this {
            parts.push(if parts.is_empty() {
                this.to_string()
            } else {
                format!("this one {this}")
            });
        }
        if self.crosstalk() {
            parts.push("over someone else".to_string());
        }
        (!parts.is_empty()).then(|| format!(" ⟨room: {}⟩", parts.join(", ")))
    }
}

/// The room as the microphone has heard it over the last [`WINDOW_MS`] — every
/// speaker's recent turns, kept so a new turn can be read *against* them rather than
/// against a number baked in at compile time.
///
/// Bounded by the window and by the handful of people who can be in one room, so it
/// holds a few dozen turns at most. It lives and dies with one mic stream and is
/// never persisted: this is the room right now, not a record of it.
#[derive(Debug, Default)]
pub struct Room {
    turns: VecDeque<Turn>,
}

impl Room {
    /// Add one diarized turn and read the room back as it now stands, the newcomer
    /// included. Turns that have fallen out of the window are dropped first, on the
    /// newcomer's own clock — the diarized timeline, so nothing here depends on when
    /// the second pass got around to delivering it.
    pub fn note(&mut self, turn: Turn) -> Reading {
        let dur_ms = turn.end_ms.saturating_sub(turn.start_ms);
        let horizon = turn.end_ms.saturating_sub(WINDOW_MS);
        while self.turns.front().is_some_and(|t| t.end_ms < horizon) {
            self.turns.pop_front();
        }

        // How much of this turn somebody else was talking across, summed over
        // everyone who was. Measured rather than flagged, so a caller can tell one
        // interjected word from a sentence spoken over the top of another
        // ([`Reading::overlap_share`]).
        let overlap_ms: u64 = self
            .turns
            .iter()
            .filter(|t| t.speaker != turn.speaker)
            .map(|t| {
                let lo = t.start_ms.max(turn.start_ms);
                let hi = t.end_ms.min(turn.end_ms);
                let ms = hi.saturating_sub(lo);
                // Below the floor this is the diarizer's boundary arithmetic, not
                // somebody interrupting.
                if ms >= OVERLAP_JITTER_MS { ms } else { 0 }
            })
            .sum();

        // The loudest speaker in the window, by their own best turn — a speaker is
        // placed against the room's near voice, not against one shouted syllable.
        let loudest = self
            .turns
            .iter()
            .filter(|t| t.speaker != turn.speaker)
            .map(|t| t.sound.speech_dbfs)
            .fold(f32::NEG_INFINITY, f32::max);
        let distance = if loudest.is_finite() {
            let under = loudest - turn.sound.speech_dbfs;
            if under <= NEAR_DB {
                Distance::Near
            } else if under >= FAR_DB {
                Distance::Far
            } else {
                Distance::Unplaced
            }
        } else {
            Distance::Unplaced // alone in the window: nothing to be far from
        };

        // Against everything still in the window, this speaker's own earlier turns
        // included — the reference has to exist when only one person is talking.
        let mut levels: Vec<f32> = self.turns.iter().map(|t| t.sound.speech_dbfs).collect();
        let level_vs_room = (!levels.is_empty()).then(|| {
            levels.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            turn.sound.speech_dbfs - levels[levels.len() / 2]
        });

        let snr = turn.sound.snr_db();
        let clarity = if snr < MUFFLED_SNR_DB {
            Clarity::Muffled
        } else if snr >= CLEAR_SNR_DB {
            Clarity::Clear
        } else {
            Clarity::Ordinary
        };

        self.turns.push_back(turn);
        let mut voices: Vec<&str> = self.turns.iter().map(|t| t.speaker.as_str()).collect();
        voices.sort_unstable();
        voices.dedup();

        Reading {
            voices: voices.len(),
            distance,
            clarity,
            overlap_ms,
            dur_ms,
            level_vs_room,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `secs` of a sine at `amp` (0..1 of full scale) over a constant `noise` floor —
    /// a stand-in for a voice in a room, loud enough to measure and quiet enough to
    /// sit above something.
    fn speech(secs: f32, amp: f32, noise: f32) -> Vec<i16> {
        let n = (16_000.0 * secs) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / 16_000.0;
                // A slow envelope so the quiet frames are the gaps between "words".
                let gate = if (t * 3.0) % 1.0 < 0.6 { 1.0 } else { 0.0 };
                let v = amp * gate * (2.0 * std::f32::consts::PI * 200.0 * t).sin()
                    + noise * (2.0 * std::f32::consts::PI * 1_313.0 * t).sin();
                (v.clamp(-1.0, 1.0) * 32_767.0) as i16
            })
            .collect()
    }

    fn sound(speech_dbfs: f32, floor_dbfs: f32) -> Sound {
        Sound { speech_dbfs, floor_dbfs, clipped: 0.0, voiced: 0.5 }
    }

    fn turn(speaker: &str, start_ms: u64, end_ms: u64, sound: Sound) -> Turn {
        Turn { speaker: speaker.to_string(), start_ms, end_ms, sound }
    }

    #[test]
    fn a_clean_voice_stands_well_above_its_own_floor() {
        let s = measure(&speech(2.0, 0.5, 0.0005)).expect("two seconds is measurable");
        assert!(s.speech_dbfs > s.floor_dbfs);
        assert!(s.snr_db() > CLEAR_SNR_DB, "snr {}", s.snr_db());
        assert_eq!(s.clipped, 0.0);
    }

    #[test]
    fn a_voice_buried_in_the_room_does_not() {
        let s = measure(&speech(2.0, 0.02, 0.015)).expect("two seconds is measurable");
        assert!(s.snr_db() < MUFFLED_SNR_DB, "snr {}", s.snr_db());
    }

    #[test]
    fn gain_cancels_out_of_the_snr() {
        // The same room through a mic turned up 4x: every level moves, the difference
        // does not. This is the property the whole module rests on.
        let quiet = measure(&speech(2.0, 0.1, 0.002)).unwrap();
        let loud = measure(&speech(2.0, 0.4, 0.008)).unwrap();
        assert!(loud.speech_dbfs > quiet.speech_dbfs + 6.0, "the gain really did change");
        assert!((loud.snr_db() - quiet.snr_db()).abs() < 1.5, "{} vs {}", loud.snr_db(), quiet.snr_db());
    }

    #[test]
    fn a_pinned_capture_is_reported_as_clipping() {
        let s = measure(&speech(1.0, 2.0, 0.0)).unwrap();
        assert!(s.clipped > 0.2, "clipped {}", s.clipped);
    }

    #[test]
    fn a_grunt_in_a_long_pause_passes_every_level_test_and_still_says_almost_nothing() {
        // 0.4 s of speech inside 3 s of room. The levels look fine — that is the
        // point — and `voiced` is what notices.
        let mut pcm = speech(0.4, 0.5, 0.0008);
        pcm.extend(std::iter::repeat_n(0i16, 16_000 * 3).map(|_| {
            // a whisper of room tone, so the floor is a real floor
            0i16
        }));
        let s = measure(&pcm).unwrap();
        assert!(s.snr_db() > CLEAR_SNR_DB, "the levels are healthy: {}", s.snr_db());
        assert!(s.voiced < 0.2, "and almost none of it is speech: {}", s.voiced);
        assert!(s.voiced_secs(pcm.len()) < 0.8, "{}", s.voiced_secs(pcm.len()));
    }

    #[test]
    fn someone_talking_steadily_is_mostly_voiced() {
        let pcm = speech(3.0, 0.4, 0.001);
        let s = measure(&pcm).unwrap();
        assert!(s.voiced > 0.45, "a 60%-duty envelope, got {}", s.voiced);
        assert!(s.voiced_secs(pcm.len()) > 1.5, "{}", s.voiced_secs(pcm.len()));
    }

    #[test]
    fn a_frame_of_silence_is_a_number_not_an_infinity() {
        let s = measure(&vec![0i16; 16_000]).unwrap();
        assert_eq!(s.speech_dbfs, FLOOR_DBFS);
        assert_eq!(s.snr_db(), 0.0);
    }

    #[test]
    fn too_short_to_be_a_stretch_of_speech() {
        assert!(measure(&[0i16; 100]).is_none());
    }

    #[test]
    fn a_speaker_who_drops_their_voice_is_measured_against_the_room_not_a_number() {
        let mut room = Room::default();
        // Nothing to compare the first turn with.
        assert_eq!(room.note(turn("0", 0, 2_000, sound(-20.0, -60.0))).level_vs_room, None);
        room.note(turn("0", 2_500, 4_500, sound(-20.0, -60.0)));
        // The same person, twelve decibels down: muttering, whoever's microphone it is.
        let r = room.note(turn("0", 5_000, 7_000, sound(-32.0, -60.0)));
        assert_eq!(r.level_vs_room, Some(-12.0));
        // And back to normal.
        let r = room.note(turn("0", 8_000, 10_000, sound(-20.0, -60.0)));
        assert_eq!(r.level_vs_room, Some(0.0));
    }

    #[test]
    fn one_speaker_alone_is_never_placed_far() {
        let mut room = Room::default();
        let r = room.note(turn("0", 0, 2_000, sound(-40.0, -70.0)));
        assert_eq!(r.voices, 1);
        assert_eq!(r.distance, Distance::Unplaced, "nothing to be far from");
        assert!(!r.crosstalk());
        assert_eq!(r.note(), None, "one clear voice is the ordinary case, and says nothing");
    }

    #[test]
    fn a_quiet_second_voice_reads_as_across_the_room() {
        let mut room = Room::default();
        room.note(turn("0", 0, 2_000, sound(-30.0, -60.0)));
        // 18 dB under the near voice, and only 10 dB over its own floor.
        let r = room.note(turn("1", 2_500, 4_000, sound(-48.0, -58.0)));
        assert_eq!(r.voices, 2);
        assert_eq!(r.distance, Distance::Far);
        assert_eq!(r.clarity, Clarity::Muffled);
        assert_eq!(r.note().as_deref(), Some(" ⟨room: 2 voices, this one far off and faint⟩"));
    }

    #[test]
    fn the_near_voice_is_named_as_the_close_one() {
        let mut room = Room::default();
        room.note(turn("1", 0, 2_000, sound(-48.0, -70.0)));
        let r = room.note(turn("0", 2_500, 4_000, sound(-30.0, -60.0)));
        assert_eq!(r.distance, Distance::Near);
        assert_eq!(r.note().as_deref(), Some(" ⟨room: 2 voices, this one the closest⟩"));
    }

    #[test]
    fn overlapping_spans_are_crosstalk_and_touching_ones_are_not() {
        let mut room = Room::default();
        room.note(turn("0", 0, 3_000, sound(-30.0, -60.0)));
        let r = room.note(turn("1", 2_000, 4_000, sound(-32.0, -62.0)));
        assert!(r.crosstalk());
        assert_eq!(r.overlap_ms, 1_000, "a second of the two-second turn was spoken over");
        assert_eq!(r.overlap_share(), 0.5);

        let mut room = Room::default();
        room.note(turn("0", 0, 3_000, sound(-30.0, -60.0)));
        assert!(!room.note(turn("1", 3_000, 4_000, sound(-32.0, -62.0))).crosstalk(), "back to back");
    }

    /// The floor exists because the vendor's turn boundaries are approximate. Without
    /// it, two turns whose edges disagree by a few milliseconds read as an
    /// interruption — and since a heavily-overlapped turn is no longer recognized
    /// from, that arithmetic would cost people their identity in a busy room.
    #[test]
    fn a_boundary_that_disagrees_by_a_hair_is_not_an_interruption() {
        let mut room = Room::default();
        room.note(turn("0", 0, 3_000, sound(-30.0, -60.0)));
        let r = room.note(turn("1", 3_000 - (OVERLAP_JITTER_MS - 1), 5_000, sound(-32.0, -62.0)));
        assert_eq!(r.overlap_ms, 0);
        assert!(!r.crosstalk());
    }

    /// One word put in over a long remark is not the same event as a sentence spoken
    /// across it, and the share is what tells them apart.
    #[test]
    fn an_interjection_and_a_talk_over_are_told_apart_by_share() {
        let mut room = Room::default();
        room.note(turn("0", 0, 4_300, sound(-30.0, -60.0)));
        let brief = room.note(turn("1", 4_000, 8_000, sound(-32.0, -62.0)));
        assert_eq!(brief.overlap_ms, 300);
        assert!(brief.overlap_share() < 0.1, "300ms inside a four-second turn");

        let mut room = Room::default();
        room.note(turn("0", 0, 4_000, sound(-30.0, -60.0)));
        let over = room.note(turn("1", 1_000, 5_000, sound(-32.0, -62.0)));
        assert_eq!(over.overlap_ms, 3_000);
        assert!(over.overlap_share() > 0.5, "three of its four seconds");
    }

    /// Overlaps with different people add up: two speakers each crossing a third of a
    /// turn leave a third of it clean.
    #[test]
    fn overlaps_with_two_people_are_summed() {
        let mut room = Room::default();
        room.note(turn("0", 0, 1_000, sound(-30.0, -60.0)));
        room.note(turn("1", 2_000, 3_000, sound(-30.0, -60.0)));
        let r = room.note(turn("2", 0, 3_000, sound(-30.0, -60.0)));
        assert_eq!(r.overlap_ms, 2_000);
    }

    #[test]
    fn a_speaker_does_not_talk_over_themselves() {
        let mut room = Room::default();
        room.note(turn("0", 0, 3_000, sound(-30.0, -60.0)));
        // The same label re-sent across an overlapping span is one person, not two.
        assert!(!room.note(turn("0", 2_000, 4_000, sound(-30.0, -60.0))).crosstalk());
    }

    #[test]
    fn the_room_empties_when_nobody_speaks_for_a_while() {
        let mut room = Room::default();
        room.note(turn("0", 0, 2_000, sound(-30.0, -60.0)));
        let r = room.note(turn("1", 3_000, 5_000, sound(-31.0, -61.0)));
        assert_eq!(r.voices, 2);
        // Two minutes later the first speaker has fallen out of the window.
        let r = room.note(turn("1", 120_000, 122_000, sound(-31.0, -61.0)));
        assert_eq!(r.voices, 1, "only the one still talking");
        assert_eq!(r.distance, Distance::Unplaced);
    }

    #[test]
    fn a_faint_lone_voice_still_says_so() {
        let mut room = Room::default();
        let r = room.note(turn("0", 0, 2_000, sound(-55.0, -60.0)));
        assert_eq!(r.voices, 1);
        assert_eq!(r.clarity, Clarity::Muffled);
        assert_eq!(r.note().as_deref(), Some(" ⟨room: faint⟩"));
    }
}
