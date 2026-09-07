//! The audio channel: inbound speech and outbound voice.
//!
//! "Audio is audio." The audio *input* channel carries audio bytes — observable
//! and playable the same way vision frames are. The transcript the agent reasons
//! over is a *derived* representation, so STT output is dispatched onto the **text**
//! channel (exactly like `POST /api/in/text`); the agent consumes text, while
//! `GET /api/in/audio` lets any client hear the raw audio.
//!
//! Inbound clip (`POST /api/in/audio`): the body bytes are audio; we save them
//! as a co-located `audio-<id>.<ext>` blob beside the conversation's day-log, publish
//! them on the inbound-audio broadcast (so `GET /api/in/audio` can play the
//! clip), transcribe via the configured STT capability
//! ([`crate::body::capabilities::stt`]), and feed the transcript into the same
//! reaction path that `POST /api/in/text` uses. The journal records a
//! `SignalIn { channel: Audio, body: <transcript>, media: Some(..) }` — the
//! agent reads text, while the media reference (sharing the blob's id) links
//! back to the audio this transcript was derived from.
//!
//! Inbound stream (`WS /api/in/audio/stream`): the client streams raw 16 kHz mono
//! 16-bit PCM as binary frames for the whole time the mic is open; the upstream
//! STT does the endpointing. There is no client-side VAD and nothing is sent back
//! on the socket — it is upload-only. Each frame is republished on the
//! inbound-audio broadcast (so `GET /api/in/audio` plays the live mic), and each
//! finalized sentence is dispatched as a text `SignalIn`. The agent sees no live
//! partials — a sentence reaches it once, settled — but rolling partials *are*
//! folded into the shared text appearance's `interim` field and echoed to the
//! inspector tap. The appearance update is the client-side barge-in trigger,
//! letting playback stop the instant speech is recognized.
//!
//! Observe (`GET /api/in/audio`): the live audio bytes for the conversation, one source
//! (mic stream or posted clip) per chunked response — the inbound mirror of
//! `GET /api/out/audio`. The `Start` event's mime tells the client how to decode
//! (`audio/pcm;rate=16000;channels=1` for the mic, the clip's own type for a POST).
//!
//! Outbound (`GET /api/out/audio`): subscriber to the reaction's `audio_out`
//! broadcast. A turn's speech arrives as a `Start`/`Frame`*/`End` run; this
//! handler blocks until a `Start` for the subscriber, then streams that turn's
//! frames as one chunked HTTP response until the matching `End`. The client
//! appends the bytes to a single sink and plays — one continuous utterance per
//! response, no per-clip reassembly. After the response closes the client re-GETs
//! for the next turn (same loop shape as the other channels).
//!
//! Capability gating: missing STT → 501 on POST/stream. Missing TTS → no audio
//! events are ever broadcast; GET /api/out/audio blocks forever (same long-poll
//! semantics as the other channels — the request is fine, the agent just never
//! speaks).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::ws::{Message as WsMessage, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

use crate::body::capabilities::stt::{self, Transcript};
use crate::body::capabilities::voiceprint;
use crate::mind::memory::layout::MediaSlot;
use crate::mind::memory::media;
use crate::mind::memory::people_vectors::{self, Candidate, Modality};
use crate::foundation::acoustics::{self, Room};
use crate::foundation::pcm;
use crate::foundation::server::headers::{AuthBearer, StreamHeader};
use crate::foundation::server::{AppState, AudioEvent, AudioInEvent};
use crate::foundation::segment::{Segmenter, Speech};
use crate::types::{Author, Channel, Content, Inbound, JournalEntry, Media, Message, Sender, SenderBasis};
use uuid::Uuid;

const DEFAULT_MIME: &str = "audio/wav";

/// Sample rate of the live mic, and samples per millisecond (16 kHz mono). Used to
/// map a diarized utterance's `[start_ms, end_ms]` onto the timeline buffer.
const SAMPLES_PER_MS: u64 = 16;

/// Minimum sliced span to voiceprint: ~1 s. Shorter turns ("嗯", "对") embed
/// poorly and pull clusters toward a noisy centroid, so they're skipped.
const VP_MIN_SPAN_SAMPLES: u64 = SAMPLES_PER_MS * 1_000;

/// Minimum sliced span to **keep** as a sample: ~2.5 s. A turn long enough to
/// identify from is not automatically one worth filing, and the two decisions were
/// one call until September 2026: every second-long "嗯 对" the mic caught became a
/// sample, and then a vote in every later match. That is how one person's gallery
/// reached its thousand-sample ceiling and two dozen fragments became two dozen
/// one-sample "people". Recognition still reads the short turns; only writing waits
/// for a real one.
const VP_ENROLL_MIN_SAMPLES: u64 = SAMPLES_PER_MS * 2_500;

/// How many of a diarized speaker's turns must name the same subject before the
/// stream calls them that. See [`SpeakerVoices::subject`].
const VOICE_TURNS_MIN: usize = 2;

/// What share of a turn somebody else may be talking across before the recording stops
/// being that speaker's voice at all — past this, it is not embedded, not matched, and
/// casts no vote.
///
/// **Overlap is a quantity and the two uses of it want different amounts.** A single
/// microphone hands back one waveform, so a turn another speaker crossed is a mix; but
/// three hundred milliseconds inside a four-second remark is a mix that is
/// overwhelmingly one person, and the vector it yields is *degraded*, not *somebody
/// else's*. The store already handles degraded evidence — a poorer match scores lower,
/// misses [`Modality::recognize_min`] or its margin, and comes back unplaced on its own
/// merits. Refusing to look at all would throw away the turns that still clear the bar,
/// and in a room where people talk over each other that is most of them.
///
/// What the score cannot handle is a genuine blend, which does not merely score lower —
/// **it can score nearest a third person**, and the margin rule is only a partial
/// defence. So heavy contamination is refused outright while light contamination is
/// left to the score.
///
/// **A guess.** Half is the point where "somebody talked across this" stops being an
/// interjection and starts being two people saying different things at once, but
/// nothing has measured where a `CAM++` embedding actually stops belonging to the
/// louder speaker. It fails in the tolerable direction: too high and some blends vote,
/// which the margin rule still has to survive; too low and a lively room recognizes
/// nobody, which is the failure that has no floor under it. See
/// [`acoustics::OVERLAP_JITTER_MS`] — the same recording measures both.
const OVERLAP_BLEND_SHARE: f32 = 0.5;

/// Seconds of **actual speech** a turn must hold before it is worth keeping as a
/// sample of somebody's voice — not seconds of recording, which is what
/// [`VP_ENROLL_MIN_SAMPLES`] bounds.
///
/// Measured over the 1278 voice samples this install had already stored: the median
/// holds **1.46 s** of speech inside 2.75 s of audio, and the worst hold 0.16 s — a
/// single syllable in a second of room. Neither duration nor signal-to-noise catches
/// those: 97% of them clear 15 dB, because a grunt in a quiet room is a fine ratio of
/// two levels. A person could not identify anybody from those clips either, and the
/// gallery they went into is the one that stopped being one person.
const ENROLL_MIN_VOICED_SECS: f32 = 1.5;

/// Words (or CJK characters) a turn must have been transcribed to before it is kept.
///
/// Two jobs. It is the only gate that can tell speech from a television, a cough or
/// music — the acoustics knew energy was there and the transcriber found no words in
/// it. And it is what asks for **a sentence rather than a phrase**: a speaker
/// embedding is only as good as the range of sounds it was taken from, and ten
/// characters is roughly 2.2 s of continuous speech against a median of 3.0 s among
/// the samples this install had already kept — a real remark, not "嗯对好的".
const ENROLL_MIN_UNITS: usize = 10;

/// How far a turn's speech must stand above the room it was spoken in to be kept.
/// **Scale-free** — a difference of two levels from one capture — so it means the same
/// on every microphone. Among the samples here that already clear every other gate the
/// tenth percentile is 25 dB and the floor 14 dB, so this takes the tail sitting
/// nearest the room: the muttered ones.
const ENROLL_MIN_SNR_DB: f32 = 20.0;

/// How far under the room's recent median a turn may be and still be kept. The other
/// half of "too quiet": someone who turned away, spoke from a doorway, or dropped
/// their voice is not quiet in absolute terms — which would mean something different
/// on every microphone — but quiet *for them, just now*
/// ([`acoustics::Reading::level_vs_room`]).
const ENROLL_QUIET_BELOW_DB: f32 = 8.0;

/// How much audio the timeline retains *before* the last consumed utterance end —
/// slack so a span whose diarized final lands slightly after its audio can still be
/// sliced. ~2 s.
const VP_PRUNE_MARGIN_SAMPLES: u64 = SAMPLES_PER_MS * 2_000;

/// Hard backstop on retained timeline samples (~60 s). The diarized second pass
/// lags the live stream by seconds; this is generous enough to cover that lag yet
/// bounds growth if the pass stalls. A span whose audio predates this window is
/// skipped (we never voiceprint the wrong audio) rather than mis-attributed.
const VP_RETAIN_CEILING_SAMPLES: u64 = SAMPLES_PER_MS * 60_000;

/// The live mic as an absolute-clock PCM buffer. `base` is the absolute sample
/// index of `buf[0]` (everything before it has been pruned); `consumed_end` is the
/// absolute index one past the last utterance span already voiceprinted. A diarized
/// utterance's `[start_ms, end_ms]` becomes absolute samples `start_ms*16 ..
/// end_ms*16`, sliced out of `buf` via `base`. This lets each speaker's *own* audio
/// be embedded even though the two-pass diarized finals lag the audio that spans
/// several speakers — the bug the old "take everything since the last final" had.
#[derive(Default)]
struct VpTimeline {
    buf: Vec<i16>,
    base: u64,
    consumed_end: u64,
}

impl VpTimeline {
    /// Absolute index one past the last buffered sample (== total samples ever
    /// pushed, since `base` accounts for everything dropped).
    fn end(&self) -> u64 {
        self.base + self.buf.len() as u64
    }

    /// Append freshly-decoded samples. Caller passes the count actually decoded
    /// (`pcm::le_i16` drops a trailing odd byte, so this must not assume bytes/2).
    fn push(&mut self, samples: &[i16]) {
        self.buf.extend_from_slice(samples);
    }

    /// Copy the samples covering absolute `[start, end)` — that speaker's own audio.
    /// `None` when `start` predates `base` (the audio was already pruned; we refuse
    /// to substitute other audio, which would re-introduce contamination) or when
    /// nothing remains after clamping `end` to what's buffered.
    fn slice(&self, start: u64, end: u64) -> Option<Vec<i16>> {
        if start < self.base {
            return None;
        }
        let lo = (start - self.base) as usize;
        let hi = (end.min(self.end()) - self.base) as usize;
        if hi <= lo || lo >= self.buf.len() {
            return None;
        }
        Some(self.buf[lo..hi.min(self.buf.len())].to_vec())
    }

    /// Drop audio no longer needed: keep a `margin` of samples before `consumed_end`
    /// and never retain more than `ceiling` trailing samples. When the unconsumed
    /// backlog itself exceeds `ceiling` (the second pass stalled), the ceiling wins
    /// and the stale audio is dropped — its later span then fails `slice` and is
    /// skipped rather than mis-sliced.
    fn prune(&mut self, margin: u64, ceiling: u64) {
        let keep_from = self
            .consumed_end
            .saturating_sub(margin)
            .max(self.end().saturating_sub(ceiling));
        if keep_from > self.base {
            let drop = ((keep_from - self.base) as usize).min(self.buf.len());
            self.buf.drain(..drop);
            self.base += drop as u64;
        }
    }
}

/// How much was said in a line, counting a CJK character and a run of letters or
/// digits as one unit each. Punctuation and spacing are not speech. A rough measure
/// deliberately — it separates "a sentence" from "one syllable" and nothing finer.
fn speech_units(text: &str) -> usize {
    let mut units = 0;
    let mut in_word = false;
    for c in text.chars() {
        let cjk = matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF);
        if cjk {
            units += 1;
            in_word = false;
        } else if c.is_alphanumeric() {
            if !in_word {
                units += 1;
            }
            in_word = true;
        } else {
            in_word = false;
        }
    }
    units
}

/// Whether a diarized turn is worth **keeping** as a sample of somebody's voice — a
/// far higher bar than the one for identifying from it, and deliberately so.
///
/// Recognition may work from whatever it gets; the answer is a sentence the agent can
/// revise a moment later. A gallery is what every future answer is graded against, so
/// a clip that nobody could identify anyone from does not belong in one no matter how
/// confidently it was matched. **The judgment is used and the sample is dropped** —
/// those were one decision until September 2026, which is how a gallery ends up
/// holding a thousand fragments.
fn worth_keeping(
    samples: usize,
    sound: &acoustics::Sound,
    said: &str,
    reading: &acoustics::Reading,
) -> bool {
    // **Any** overlap at all disqualifies a sample, where recognizing from one
    // tolerates a little ([`resolve_speaker`]). The asymmetry is this function's whole
    // subject: a judgment made off a slightly-blended turn is one sentence the agent
    // can revise, while a slightly-blended sample is graded against forever. Keeping
    // nothing costs one clip out of a gallery of two hundred.
    !reading.crosstalk()
        && samples as u64 >= VP_ENROLL_MIN_SAMPLES
        && sound.voiced_secs(samples) >= ENROLL_MIN_VOICED_SECS
        && speech_units(said) >= ENROLL_MIN_UNITS
        && sound.snr_db() >= ENROLL_MIN_SNR_DB
        && reading.level_vs_room.is_none_or(|d| d >= -ENROLL_QUIET_BELOW_DB)
}

/// Who each diarized speaker is, accumulated across their turns on this stream.
///
/// **One voiceprint is weak evidence, and the store says so.** Spontaneous speech
/// scores a genuine match not far above where a stranger sits — a short turn, the
/// room, an overlapping second voice all pull it down — so a name decided on one turn
/// is wrong often enough to put somebody else's words on a person's record. Turns are
/// cheap and a speaker keeps talking, so the answer is to wait for the second one. The
/// vendor's `speaker_id` already groups a person's turns within the session, which is
/// exactly the key to accumulate under; nothing here persists past the stream, and
/// nothing here writes to the people store.
#[derive(Default)]
struct SpeakerVoices {
    by_speaker: HashMap<String, SpeakerEvidence>,
}

/// One diarized speaker's turns: for each subject the store was willing to name, the
/// score of every turn that named them. Turns that named nobody — most of them, and
/// an ordinary outcome — are counted by their absence.
#[derive(Default)]
struct SpeakerEvidence {
    named: HashMap<String, Vec<f32>>,
    /// Subjects a **second sense** placed in the room on a turn that also matched
    /// them. One such turn settles this speaker on its own: the turns rule exists
    /// because one voiceprint is one piece of evidence, and a match the camera agrees
    /// with is two.
    corroborated: HashSet<String>,
}

impl SpeakerVoices {
    /// Record what one turn of `speaker_id` sounded like. `corroborated` is set when
    /// another sense independently placed that same person here — see
    /// [`Recognition::corroborated_by`].
    fn note(&mut self, speaker_id: &str, named: Option<&Candidate>, corroborated: bool) {
        let ev = self.by_speaker.entry(speaker_id.to_string()).or_default();
        if let Some(c) = named {
            ev.named.entry(c.subject.clone()).or_default().push(c.similarity);
            if corroborated {
                ev.corroborated.insert(c.subject.clone());
            }
        }
    }

    /// Who this speaker is, once their turns agree on somebody. [`VOICE_TURNS_MIN`]
    /// turns naming the same subject settle it; a single turn settles it only when it
    /// scored well enough to be worth filing as a sample — the store's own
    /// [`Modality::append_min`], borrowed rather than re-invented, because "solid
    /// enough to keep" and "solid enough to say out loud alone" are the same
    /// judgment. **A turn another sense agreed with also settles it alone** — the
    /// rule is there because one voiceprint is one piece of evidence, and that turn
    /// had two. Two subjects level on turns settle nothing: that is the speaker
    /// sounding like both, which is the honest answer and leaves them unplaced.
    fn subject(&self, speaker_id: &str) -> Option<String> {
        let ev = self.by_speaker.get(speaker_id)?;
        let mut ranked: Vec<(&String, usize, f32)> = ev
            .named
            .iter()
            .map(|(subject, scores)| {
                (subject, scores.len(), scores.iter().copied().fold(f32::MIN, f32::max))
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.1.cmp(&a.1).then(b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal))
        });
        let (subject, turns, best) = *ranked.first()?;
        if ranked.get(1).is_some_and(|second| second.1 == turns) {
            return None;
        }
        (turns >= VOICE_TURNS_MIN
            || best >= Modality::Voice.append_min()
            || ev.corroborated.contains(subject))
            .then(|| subject.clone())
    }
}

/// Recognize the speaker of a single-voice clip: the compact evidence note to append
/// to the agent-facing transcript, e.g. ` ⟨voice: 老王 ~0.82⟩`, **and the subject it
/// named** when the store was willing to name one — so the signal carries a grounded
/// [`Sender`] instead of leaving its own text the only place that says who was
/// talking. `None` for the subject means the voice was heard and not
/// placed, which is a complete answer.
///
/// **The note and the sender do not use the same bar, and that is the point.** The
/// note is prose the mind reads and may weigh however it likes — soft evidence, at
/// whatever strength the match had. The sender is a field nothing downstream
/// re-decides, so it takes [`Modality::append_min`] rather than the weaker
/// [`Modality::recognize_min`] a match needs to be worth mentioning.
///
/// The reason is that a clip only ever gets **one** look. The live mic can afford the
/// lower bar because it accumulates: two turns naming the same subject settle it, and
/// a lone turn there already has to clear `append_min` on its own
/// ([`SpeakerVoices::subject`]). A clip has no second turn and never will — the
/// sender is decided when the signal is delivered and is not revised afterwards — so
/// it is exactly the one-turn case, and it answers to the one-turn bar. One rule for
/// both paths: **evidence that stands alone has to be strong; evidence that repeats
/// may be weak.**
///
/// The audio twin of the vision channel's `face_note`. Returns `None` outright when
/// voiceprint is unconfigured, the clip can't be decoded/embedded, or the clip is
/// diarized into multiple speakers (a single blended embedding would be misleading —
/// the labeled transcript already attributes the turns). Best-effort: the signal
/// stands regardless.
async fn voice_note(
    bytes: &Bytes,
    mime: &str,
    transcript: &str,
    data_dir: &std::path::Path,
) -> Option<(String, Option<String>)> {
    if !voiceprint::available() {
        return None;
    }
    // A diarized, multi-speaker clip ("说话人0：…") is not one voice; skip rather
    // than embed a blend of several speakers into one misleading sample.
    if transcript.starts_with("说话人") {
        return None;
    }
    let samples = pcm::to_i16_16k_mono(bytes, mime).ok().filter(|s| !s.is_empty())?;
    let embedding = match voiceprint::embed(samples).await {
        Ok(e) => e,
        Err(err) => {
            tracing::warn!(error = %format!("{err:#}"), "voiceprint embed failed");
            return None;
        }
    };
    let seen = people_vectors::recognize(data_dir, Modality::Voice, &embedding).await.ok()?;
    let (who, subject) = match seen.named() {
        Some(c) => (
            format!("{} ~{:.2}", c.subject, c.similarity),
            (c.similarity >= Modality::Voice.append_min()).then(|| c.subject.clone()),
        ),
        None => ("unfamiliar".to_string(), None),
    };
    Some((format!(" ⟨voice: {who}⟩"), subject))
}

/// Format of the live mic stream: raw 16 kHz mono signed 16-bit little-endian PCM.
/// Carried on the inbound-audio `Start` so a listener knows how to decode it.
const PCM_MIME: &str = "audio/pcm;rate=16000;channels=1";

#[derive(Debug, Serialize)]
struct PostAudioAck {
    transcript: String,
    media_path: String,
}

pub async fn post_audio(
    State(state): State<Arc<AppState>>,
    StreamHeader(stream): StreamHeader,
    AuthBearer(auth): AuthBearer,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if !stt::available() {
        return (
            StatusCode::NOT_IMPLEMENTED,
            "audio capability not configured (set an STT key in Settings)\n",
        )
            .into_response();
    }

    if body.is_empty() {
        return (StatusCode::BAD_REQUEST, "audio body is empty\n").into_response();
    }

    let mime = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_else(|| DEFAULT_MIME.to_string());
    let ext = mime_to_ext(&mime);

    tracing::info!(
        auth = ?auth,
        mime = %mime,
        bytes = body.len(),
        "POST /audio"
    );

    // The signal's id (uuidv7) also names its co-located blob; `ts` places both
    // in the same day-folder. Generate them before storing so the two agree.
    let ts = Utc::now();
    let id = Uuid::now_v7().to_string();

    // 1. Persist the raw bytes so we can replay/audit and so the log has a
    //    stable reference. We do this before STT so a transcription failure
    //    still leaves the audio on disk.
    let media_path = match media::store_blob(&state.data_dir, Channel::Audio, ts, MediaSlot::InputOneOff, ext, &body).await {
        Ok(f) => f,
        Err(err) => {
            tracing::error!(error = %format!("{err:#}"), "failed to persist incoming audio");
            return (StatusCode::INTERNAL_SERVER_ERROR, "audio store failed\n").into_response();
        }
    };

    // 2. Publish the clip on the inbound-audio channel as one source, so any
    //    `GET /api/in/audio` listener can play it. Bytes are refcounted, so with
    //    no listener this is a cheap drop.
    let turn = state.audio_in_turn.fetch_add(1, Ordering::Relaxed);
    let _ = state.audio_in.send(AudioInEvent::Start {
        turn,
        mime: mime.clone(),
    });
    let _ = state.audio_in.send(AudioInEvent::Frame {
        turn,
        bytes: body.clone(),
    });
    let _ = state.audio_in.send(AudioInEvent::End { turn });

    // Keep the raw bytes for voiceprint before STT consumes them below.
    let vp_bytes = body.clone();

    // 3. Transcribe. Errors surface as 502 — the upstream provider failed.
    let transcript = match stt::transcribe(body, &mime).await {
        Ok(t) => t,
        Err(err) => {
            crate::foundation::energy_state::note_402_error(&state.data_dir, &err);
            tracing::warn!(error = %format!("{err:#}"), media_path = %media_path, "STT transcribe failed");
            return (
                StatusCode::BAD_GATEWAY,
                format!("transcription failed: {err}\n"),
            )
                .into_response();
        }
    };

    // Empty transcript = the clip held no recognizable speech. The upstream
    // cannot distinguish silence from un-transcribable sound, so we don't try:
    // there's nothing to journal or dispatch. Return a benign ack (the raw
    // audio is already persisted for audit) — the SPA reads the empty
    // transcript and drops back to idle rather than treating it as a failure.
    if transcript.trim().is_empty() {
        tracing::info!(media_path = %media_path, "audio clip held no speech");
        let ack = PostAudioAck { transcript: String::new(), media_path };
        return (StatusCode::ACCEPTED, axum::Json(ack)).into_response();
    }

    // 4. The transcript is text: dispatch it onto the text channel exactly like a
    //    typed line. The agent reads text; the audio stays on the audio channel.
    //    The clip's (ts, id, media) ride along so the journal entry links back to
    //    the stored blob by the shared id.
    let media = Media {
        file: media_path.clone(),
        mime: mime.clone(),
        duration_ms: None,
        width: None,
        height: None,
    };
    // Fold a voiceprint recognition note into the agent-facing transcript (who is
    // speaking), the way the vision path folds in recognized faces. The ack keeps
    // the raw transcript so the SPA caption isn't cluttered with the evidence tag.
    let mut delivered = transcript.clone();
    let mut speaker = None;
    if let Some((note, matched)) = voice_note(&vp_bytes, &mime, &transcript, &state.data_dir).await {
        delivered.push_str(&note);
        speaker = matched;
    }
    if !deliver_transcript(&state, stream, &delivered, Some((ts, id, media)), speaker).await {
        return (StatusCode::SERVICE_UNAVAILABLE, "inbound channel closed\n").into_response();
    }

    let ack = PostAudioAck { transcript, media_path };
    (StatusCode::ACCEPTED, axum::Json(ack)).into_response()
}

#[derive(Debug, Deserialize)]
pub struct StreamParams {
    /// The named source, same role as `X-HI-Stream` on the POST path.
    /// Absent or empty means the default stream.
    stream: Option<String>,
}

/// `GET /api/in/audio/stream` — continuous inbound speech over a WebSocket.
///
/// Upload-only: the client streams raw 16 kHz mono 16-bit PCM as binary frames
/// for the whole time the mic is open; the upstream STT does the endpointing.
/// There is no client-side VAD and nothing is sent back on the socket. Each frame
/// is republished on the inbound-audio broadcast so `GET /api/in/audio` plays the
/// live mic; each finalized sentence is dispatched on the text channel (the path
/// `POST /api/in/audio` uses for its transcript).
pub async fn get_audio_stream(
    State(state): State<Arc<AppState>>,
    Query(params): Query<StreamParams>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    if !stt::available() {
        return (
            StatusCode::NOT_IMPLEMENTED,
            "audio capability not configured (set an STT key in Settings)\n",
        )
            .into_response();
    }
    let stream = params.stream.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    tracing::info!(stream = ?stream, "WS /api/in/audio/stream opened");
    ws.on_upgrade(move |socket| stream_audio_in(state, stream, socket))
}

async fn stream_audio_in(
    state: Arc<AppState>,
    stream: Option<String>,
    mut socket: axum::extract::ws::WebSocket,
) {
    // Hold a mic guard for the life of the socket. It counts the open mic and
    // nothing more: it used to double as a "voice conversation is active" posture
    // that presence read, and there is no longer anything reading it that way.
    let _mic = state.attachments.connect_mic();
    // The WS is just one source of PCM frames. Forward its binary frames into the
    // shared ingest; when the socket closes the sender drops, the ingest sees the
    // stream end, and it finalizes. A browser mic carries no source tag.
    let (tx, rx) = mpsc::channel::<Bytes>(64);
    let pump = tokio::spawn(async move {
        while let Some(msg) = socket.recv().await {
            match msg {
                Ok(WsMessage::Binary(b)) => {
                    if tx.send(b).await.is_err() {
                        break;
                    }
                }
                Ok(WsMessage::Close(_)) | Err(_) => break,
                Ok(_) => {}
            }
        }
    });

    ingest_pcm_stream(state, stream, None, rx).await;
    pump.abort();
    tracing::info!("WS /api/in/audio/stream closed");
}

/// Ingest a stream of raw 16 kHz mono 16-bit PCM frames as live inbound speech,
/// from any source — the browser mic over a WebSocket ([`stream_audio_in`]) or a
/// native capture (the press-hold-⌘ attention gesture). Frames flow through the
/// same path regardless: republished on the inbound-audio broadcast (so
/// `GET /api/in/audio` plays the live mic), persisted on the wall-clock-minute grid,
/// fed to streaming STT, segmented into sentences, voiceprint-tagged, and dispatched
/// on the audio channel. Returns when the `frames` sender drops (the source closed).
///
/// `source_tag`, when present, rides the **first** delivered sentence as a context
/// note (e.g. press-hold attention is headless, screen-aware) using the same `⟨…⟩`
/// convention as the voiceprint tag — so the mind knows where this speech came from
/// without any branch downstream. The browser mic passes `None`.
pub async fn ingest_pcm_stream(
    state: Arc<AppState>,
    stream: Option<String>,
    source_tag: Option<String>,
    mut frames: mpsc::Receiver<Bytes>,
) {
    // PCM source → STT; Transcripts STT → dispatch. Bounded so a stalled upstream
    // exerts backpressure rather than buffering unboundedly.
    let (audio_tx, audio_rx) = mpsc::channel::<Bytes>(64);
    let (tr_tx, mut tr_rx) = mpsc::channel::<Transcript>(64);

    let mut stt_task = tokio::spawn(async move { stt::transcribe_streaming(audio_rx, tr_tx).await });

    // Live-mic voiceprint: who is speaking, from the vendor's diarized segments.
    // The PCM pump (below) appends raw samples to `timeline` (an absolute-clock
    // buffer); when a diarized utterance finalizes, the out task slices *that
    // speaker's own* audio by the utterance's `[start_ms, end_ms]`, measures how it
    // sounded, embeds it, and asks the people store who it sounds like — accumulating
    // that under `speaker_id` until this speaker's turns agree on somebody.
    //
    // The timeline is armed whether or not voiceprints are: slicing a speaker's own
    // audio is what the room reading needs too, and that runs on any install with a
    // microphone. Only the identity half waits on the model.
    let voiceprints_on = voiceprint::available();
    let timeline: Arc<Mutex<VpTimeline>> = Arc::new(Mutex::new(VpTimeline::default()));
    let voices: Arc<Mutex<SpeakerVoices>> = Arc::new(Mutex::new(SpeakerVoices::default()));

    // An explicit Segmenter — not the upstream's silence flag — decides where the
    // continuous word-stream is cut into sentences for the agent. A periodic tick
    // drives the time-based cut rules when the speaker has gone quiet. Each
    // finalized sentence is delivered on the text channel; there are no partials.
    let relay_state = state.clone();
    let relay_stream = stream.clone();
    let relay_pcm = timeline.clone();
    let relay_voices = voices.clone();
    let relay_tag = source_tag.clone();
    let out_task = tokio::spawn(async move {
        let mut seg = Segmenter::new(Speech::default(), Instant::now());
        let mut ticker = tokio::time::interval(Duration::from_millis(150));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The speaker of the latest diarized final, and the (diarized speaker, who we
        // called them) pair the last tag stated — so we mark turn changes, not every
        // line, and re-mark when a voice we could not place acquires a name.
        let mut current_speaker: Option<String> = None;
        let mut last_tagged: Option<(String, String)> = None;
        // The room the mic is in, the reading from the newest diarized turn waiting
        // for a sentence to ride out on, and the last reading actually stated.
        let mut room = Room::default();
        let mut pending_room: Option<String> = None;
        let mut last_room: Option<String> = None;
        // The source note rides the first sentence only.
        let mut source_noted = false;
        loop {
            let cuts = tokio::select! {
                msg = tr_rx.recv() => match msg {
                    Some(t) => {
                        // Publish every rolling partial as the conversation's one
                        // pending line and echo it to observers (`final:false`).
                        // The interim update is the duck trigger: the client
                        // stops playback hundreds of ms before a sentence settles.
                        // The same moment is reported to the barge-in registry,
                        // whose own clock decides whether the agent's voice was
                        // probably still sounding (→ "what went unheard" note).
                        if !t.is_final && !t.text.trim().is_empty() {
                            relay_state.note_interim(Channel::Text, &t.text);
                            relay_state.floor.note_speech(tokio::time::Instant::now()).await;
                        }
                        // A diarized utterance just finalized. Each segment names a
                        // speaker and its `[start_ms, end_ms]`; slice that speaker's
                        // *own* audio out of the timeline (not the whole stretch
                        // since the last final, which spans several speakers because
                        // the diarized second pass lags) and resolve each off-thread.
                        if t.is_final {
                            if !t.segments.is_empty() {
                                let sliced: Vec<(acoustics::Turn, Vec<i16>)> = {
                                    let mut tl = relay_pcm.lock().unwrap();
                                    t.segments
                                        .iter()
                                        .filter_map(|sp| {
                                            let start = sp.start_ms.saturating_mul(SAMPLES_PER_MS);
                                            let end = sp.end_ms.saturating_mul(SAMPLES_PER_MS);
                                            if end <= tl.consumed_end {
                                                return None; // already handled (re-sent span)
                                            }
                                            let pcm = tl.slice(start, end);
                                            tl.consumed_end = tl.consumed_end.max(end);
                                            let pcm =
                                                pcm.filter(|p| p.len() as u64 >= VP_MIN_SPAN_SAMPLES)?;
                                            let sound = acoustics::measure(&pcm)?;
                                            Some((
                                                acoustics::Turn {
                                                    speaker: sp.speaker_id.clone(),
                                                    start_ms: sp.start_ms,
                                                    end_ms: sp.end_ms,
                                                    sound,
                                                },
                                                pcm,
                                            ))
                                        })
                                        .collect()
                                };
                                // A final naming one speaker is that speaker's
                                // words; one naming several has no per-speaker text
                                // to hand out, so nothing from it is ever kept.
                                let alone = t
                                    .segments
                                    .first()
                                    .map(|f| t.segments.iter().all(|sp| sp.speaker_id == f.speaker_id))
                                    .unwrap_or(false);
                                let said = alone.then(|| t.text.clone());
                                for (turn, pcm) in sliced {
                                    let speaker = turn.speaker.clone();
                                    let sound = turn.sound;
                                    // What the room was like when this turn landed. The
                                    // note is the empty string for the ordinary room —
                                    // not news, but still a change worth remembering —
                                    // and the reading also says whether anyone was
                                    // talking across this turn, which decides whether
                                    // it can be kept.
                                    let reading = room.note(turn);
                                    pending_room =
                                        Some(reading.note().clone().unwrap_or_default());
                                    if voiceprints_on {
                                        resolve_speaker(
                                            &relay_state,
                                            relay_voices.clone(),
                                            speaker,
                                            pcm,
                                            sound,
                                            said.clone(),
                                            reading,
                                        );
                                    }
                                }
                            }
                            // Tag the dispatched sentence with the last finalized
                            // speaker — the turn the Segmenter is about to emit.
                            if let Some(spk) = t.speaker_id.clone() {
                                current_speaker = Some(spk);
                            }
                        }
                        seg.observe(&t.text, t.is_final, Instant::now())
                    }
                    None => break, // STT session ended
                },
                _ = ticker.tick() => seg.tick(Instant::now()),
            };
            for sentence in cuts {
                // Who the voiceprint placed this sentence with, if anyone. It rides
                // the signal as its sender on EVERY line — that is a field, and a
                // field costs nothing to repeat.
                let mut line = sentence;
                let speaker = current_speaker
                    .as_ref()
                    .and_then(|spk| relay_voices.lock().unwrap().subject(spk));
                // The tag in the *text* still marks turn changes only: it is prose
                // the mind reads, and a name restated on every sentence is noise
                // there (a 1:1 chat shows it once; a multi-party one marks each
                // handoff). What it marks is the **diarized speaker** changing, not
                // the name — so a handoff between two people neither of whom can be
                // placed still reads as a handoff instead of one long stretch by
                // nobody. A voice that was unfamiliar and later gets placed re-marks,
                // because that is news.
                if let Some(spk) = &current_speaker {
                    let mark = (
                        spk.clone(),
                        speaker.clone().unwrap_or_else(|| "unfamiliar".to_string()),
                    );
                    if last_tagged.as_ref() != Some(&mark) {
                        line.push_str(&format!(" ⟨voice: {}⟩", mark.1));
                        last_tagged = Some(mark);
                    }
                }
                // The room note rides a sentence only when the picture changed —
                // the same discipline as the voice mark. A condition restated on every
                // line stops being information, and "the room is ordinary" was never
                // information in the first place: it renders empty, and only the change
                // back out of it is worth a mark.
                if let Some(note) = pending_room.take()
                    && last_room.as_deref() != Some(note.as_str())
                {
                    line.push_str(&note);
                    last_room = Some(note);
                }
                if let Some(tag) = &relay_tag
                    && !source_noted
                {
                    line.push_str(&format!(" ⟨{tag}⟩"));
                    source_noted = true;
                }
                deliver_transcript(&relay_state, relay_stream.clone(), &line, None, speaker).await;
            }
        }
        // Flush any trailing words as a final sentence when the session ends.
        if let Some(sentence) = seg.flush() {
            let mut line = sentence;
            let speaker = current_speaker
                .as_ref()
                .and_then(|spk| relay_voices.lock().unwrap().subject(spk));
            if let Some(tag) = &relay_tag
                && !source_noted
            {
                line.push_str(&format!(" ⟨{tag}⟩"));
            }
            deliver_transcript(&relay_state, relay_stream.clone(), &line, None, speaker).await;
        }
    });

    // One source is one inbound-audio source: its frames carry a shared `turn` so a
    // `GET /api/in/audio` listener stays bound to this mic alone.
    let turn = state.audio_in_turn.fetch_add(1, Ordering::Relaxed);
    let mut started = false;

    // Persist the live mic on a wall-clock-minute grid: PCM accumulates per
    // minute and flushes to `audio/<date>/<HH>/<MM>.wav` at each rollover (and
    // at close). The bytes are the raw signal; utterance lines (journaled by
    // `deliver_transcript`) stay media-less and correlate to a minute by ts.
    let mut cap_minute: Option<String> = None;
    let mut cap_ts = Utc::now();
    let mut cap_buf: Vec<u8> = Vec::new();

    // Pump inbound PCM until the source closes or the STT session ends. The STT
    // side is awaited *alongside* the frames rather than discovered through a
    // failed send, because a session can die while no frame is coming: the
    // upstream ends one that goes 8 s without a packet, which is exactly what a
    // capture that quietly stopped producing looks like. Learning that on the
    // next frame means a stalled capture holds the socket open against a dead
    // session indefinitely — no transcription, nothing logged, and the client
    // (which reopens on close) never told to start over. Ending here closes the
    // socket, and the reconnect brings up a fresh session.
    let mut stt_ended: Option<Result<anyhow::Result<String>, tokio::task::JoinError>> = None;
    loop {
        let b = tokio::select! {
            frame = frames.recv() => match frame {
                Some(b) => b,
                None => break,
            },
            joined = &mut stt_task => {
                stt_ended = Some(joined);
                break;
            }
        };
        // Republish the raw PCM for `GET /api/in/audio` listeners. The `Start`
        // (carrying the format) precedes the first frame.
        if !started {
            started = true;
            let _ = state.audio_in.send(AudioInEvent::Start {
                turn,
                mime: PCM_MIME.to_owned(),
            });
        }
        let _ = state.audio_in.send(AudioInEvent::Frame {
            turn,
            bytes: b.clone(),
        });
        // Fold the frame into the current minute's WAV buffer, flushing the
        // completed minute when the wall clock rolls over.
        let now = Utc::now();
        let minute = now.format("%Y-%m-%dT%H:%M").to_string();
        match &cap_minute {
            Some(m) if *m != minute => {
                flush_mic_minute(&state, cap_ts, &cap_buf).await;
                cap_buf.clear();
                cap_minute = Some(minute);
                cap_ts = now;
            }
            None => {
                cap_minute = Some(minute);
                cap_ts = now;
            }
            _ => {}
        }
        cap_buf.extend_from_slice(&b);
        // Feed the per-speaker timeline (same raw 16 kHz mono PCM). Push the samples
        // actually decoded (le_i16 drops a trailing odd byte, so a byte-derived
        // clock would drift), then prune audio the out task has already consumed —
        // bounded so a stalled diarized pass can't grow it.
        {
            let samples = pcm::le_i16(&b);
            let mut tl = timeline.lock().unwrap();
            tl.push(&samples);
            tl.prune(VP_PRUNE_MARGIN_SAMPLES, VP_RETAIN_CEILING_SAMPLES);
        }
        if audio_tx.send(b).await.is_err() {
            break;
        }
    }

    // Close the inbound-audio source so listeners end their current response.
    if started {
        let _ = state.audio_in.send(AudioInEvent::End { turn });
    }
    // Flush the final, partial minute of mic audio.
    if !cap_buf.is_empty() {
        flush_mic_minute(&state, cap_ts, &cap_buf).await;
    }

    // Closing the audio side lets the STT session flush its last utterance —
    // unless it already ended above, in which case its outcome is in hand and the
    // handle must not be polled again.
    drop(audio_tx);
    let joined = match stt_ended {
        Some(joined) => Some(joined),
        None => tokio::time::timeout(Duration::from_secs(5), stt_task).await.ok(),
    };
    match joined {
        Some(Err(err)) => tracing::warn!(error = %err, "audio ingest STT task panicked"),
        Some(Ok(Err(err))) => {
            // A 402 here means the managed account is out of energy (STT draws the
            // same budget) — raise the out-of-energy hint now, without waiting for the
            // next balance poll. No-op in BYOK / for non-402 STT failures.
            crate::foundation::energy_state::note_402_error(&state.data_dir, &err);
            tracing::warn!(error = %format!("{err:#}"), "audio ingest STT ended");
        }
        None => tracing::warn!("audio ingest STT did not finalize in time"),
        _ => {}
    }
    out_task.abort();
}

/// Deliver one finalized transcript on the **audio** channel — journal it, echo
/// it to conversation observers (settled), and hand it to the reaction. The transcript is
/// the signal's text surface (`body`); the modality stays `audio` so its bytes,
/// when present, land under `audio/`. The reaction reads `body` regardless. For a
/// posted clip, `clip` carries the `(ts, id, media)` of the stored audio blob so
/// the journal entry references it; the live mic passes `None` for now (its bytes
/// are persisted by wall-clock minute rather than per utterance), so the journal
/// records no direct `media` ref.
///
/// `speaker` is the subject a voiceprint **matched**, when one did — never a guess
/// and never a name read out of the words. Everything else is unattributed.
async fn deliver_transcript(
    state: &AppState,
    stream: Option<String>,
    text: &str,
    clip: Option<(DateTime<Utc>, String, Media)>,
    speaker: Option<String>,
) -> bool {
    let (ts, id, media) = match clip {
        Some((ts, id, media)) => (ts, id, Some(media)),
        None => (Utc::now(), Uuid::now_v7().to_string(), None),
    };
    crate::foundation::channel_log::inbound(Channel::Audio, text);
    // **Ambient — never the owner default.** A microphone picks up whoever is in
    // range: the person, someone else in the room, a television. Only voiceprint
    // clustering may answer who spoke, and when it has (`speaker`), the answer is
    // recorded as `cluster` — the basis `docs/arch/signal-attribution.md` reserves
    // for a face or voiceprint match. Until it does, unknown is the true one, and it
    // stays unknown rather than borrowing the last name the room produced.
    let sender = match &speaker {
        Some(subject) => Sender { subject: Some(subject.clone()), basis: SenderBasis::Cluster },
        None => Sender::unknown(),
    };
    let _ = &stream;
    let message = Message {
        id,
        ts,
        from: Author::Person(sender),
        content: Content::Speech { text: text.to_owned(), audio: media },
    };
    let entry = JournalEntry::Message { channel: Channel::Audio, message: message.clone() };
    if let Err(err) = state.memory.journal.append(entry).await {
        tracing::error!(error = %format!("{err:#}"), "journal append failed; accepting signal anyway");
    }
    // Append before dispatching inward. A spoken line is a message like a typed
    // one, so it rides the text channel into the conversation (a display concern);
    // the journal above keeps it on `Audio`, where it was actually heard.
    state.note_message(Channel::Text, message.clone());
    if let Err(err) = state.inbound.send(Inbound::Message(message)).await {
        tracing::error!(error = %err, "inbound channel closed");
        return false;
    }
    true
}

/// Resolve a diarized speaker's identity off the hot path, in two decisions that used
/// to be one call.
///
/// **Identifying is a read.** Embed the utterance's PCM, ask the people store who it
/// sounds like, and hand that to [`SpeakerVoices`], which decides across this
/// speaker's turns whether they can be named at all.
///
/// **Keeping it is a separate, much stricter decision** — [`worth_keeping`], plus a
/// store that can place it confidently or not at all. `said` is what the transcriber
/// made of this turn, and `None` means the utterance it came from held more than one
/// speaker, which is never kept and never has text that belongs to one person.
/// `reading` is the room this turn landed in — how much of it anybody talked across,
/// and how it stood against how loudly the room has lately been talking. A turn more
/// than [`OVERLAP_BLEND_SHARE`] spoken over returns here without being embedded at
/// all: identifying somebody from a blend of two voices is not weak evidence, it is
/// evidence about nobody.
///
/// Detached and best-effort: a failure leaves the speaker unplaced, which is an
/// ordinary state rather than an error. Unlike clips and stills, the live mic persists
/// no per-utterance media for the reflection pass to re-derive, so both decisions
/// happen inline here.
fn resolve_speaker(
    state: &Arc<AppState>,
    voices: Arc<Mutex<SpeakerVoices>>,
    speaker_id: String,
    pcm: Vec<i16>,
    sound: acoustics::Sound,
    said: Option<String>,
    reading: acoustics::Reading,
) {
    if pcm.is_empty() {
        return;
    }
    // The measurement behind [`OVERLAP_BLEND_SHARE`] and
    // [`acoustics::OVERLAP_JITTER_MS`]: what the overlaps in a real multi-party
    // recording actually look like. Both numbers are guesses until this has been read
    // off one, and nothing else in the process records it.
    tracing::debug!(
        speaker = %speaker_id,
        overlap_ms = reading.overlap_ms,
        dur_ms = reading.dur_ms,
        share = reading.overlap_share(),
        "diarized turn overlap"
    );
    if reading.overlap_share() >= OVERLAP_BLEND_SHARE {
        // Half this turn is somebody else. The embedding would be a blend belonging to
        // neither speaker, and a blend does not just score badly — it can score nearest
        // a third person. Heard, and deliberately not measured.
        return;
    }
    let data_dir = state.data_dir.clone();
    // Who the camera has in frame right now, when it is exactly one person and the
    // store has a name or an id for them. Read here rather than after the embedding,
    // so it is the room as it stood nearest the turn itself.
    //
    // **This is up to 2.5 s stale on arrival and up to 8 s stale on departure** — the
    // presence lane's still cadence and its leave grace. That is the accuracy of "who
    // is in the room", which is what it is being asked, and not of "who spoke".
    let on_camera = state
        .face_presence
        .lock()
        .expect("face_presence mutex poisoned")
        .alone()
        .map(str::to_owned);
    // A playable WAV of this turn, built before the PCM is consumed by `embed`, so a
    // kept sample carries an audible preview of the live-mic voice (the stream stores
    // no per-utterance clip otherwise). Built only for a turn actually worth keeping.
    let keep = said.filter(|t| worth_keeping(pcm.len(), &sound, t, &reading));
    let wav = keep.as_ref().map(|_| {
        let pcm_bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
        pcm16_mono_16k_to_wav(&pcm_bytes)
    });
    tokio::spawn(async move {
        let embedding = match voiceprint::embed(pcm).await {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!(error = %format!("{err:#}"), "live voiceprint embed failed");
                return;
            }
        };
        let seen = match people_vectors::recognize(&data_dir, Modality::Voice, &embedding).await {
            Ok(seen) => seen,
            Err(err) => {
                tracing::warn!(error = %format!("{err:#}"), "live voice match failed");
                return;
            }
        };
        // Two senses beat one margin. When the only person on camera is also this
        // recognition's top candidate, a tie the voice alone could not break is broken
        // — nothing is added to the score, and no name the store did not already
        // propose can appear this way. Face is the sense that gets to do this and not
        // the reverse: its floor is measured to sit in a gap with no overlap, while
        // voice's is a guess in the region where the two distributions meet.
        let corroborated = on_camera
            .as_deref()
            .and_then(|who| seen.corroborated_by(who))
            .cloned();
        let named = seen.named().or(corroborated.as_ref());
        voices.lock().unwrap().note(&speaker_id, named, corroborated.is_some());

        let (Some(wav), Some(said)) = (wav, keep) else {
            return; // heard and weighed, and that is all this turn was good for
        };
        let subject = match seen.filing() {
            people_vectors::Filing::Append(subject) => subject,
            people_vectors::Filing::Mint => people_vectors::mint_id(),
            people_vectors::Filing::Unplaceable => return,
        };
        match people_vectors::enroll(
            &data_dir, &subject, Modality::Voice, &embedding, &wav, "wav", Some(&said),
        )
        .await
        {
            // `None` is a full gallery that this turn could not earn a place in —
            // ordinary once somebody has been heard a couple of hundred times, and the
            // reason a gallery stops drifting toward whoever else is in the room.
            Ok(_) => {}
            Err(err) => tracing::warn!(error = %format!("{err:#}"), "live voice enroll failed"),
        }
    });
}

/// Persist one wall-clock minute of live mic PCM as a WAV under
/// `audio/<date>/<HH>/<MM>.wav`. Best-effort: a failure is logged, never fatal.
async fn flush_mic_minute(state: &AppState, ts: DateTime<Utc>, pcm: &[u8]) {
    let wav = pcm16_mono_16k_to_wav(pcm);
    if let Err(err) =
        media::store_blob(&state.data_dir, Channel::Audio, ts, MediaSlot::InputStream, "wav", &wav).await
    {
        tracing::warn!(error = %format!("{err:#}"), "persisting mic minute failed");
    }
}

/// Wrap raw 16 kHz mono signed-16-bit-LE PCM (the live mic format, [`PCM_MIME`])
/// in a canonical 44-byte WAV header so the minute file is independently
/// playable.
fn pcm16_mono_16k_to_wav(pcm: &[u8]) -> Vec<u8> {
    const SAMPLE_RATE: u32 = 16_000;
    const CHANNELS: u16 = 1;
    const BITS: u16 = 16;
    let byte_rate = SAMPLE_RATE * CHANNELS as u32 * (BITS as u32 / 8);
    let block_align = CHANNELS * (BITS / 8);
    let data_len = pcm.len() as u32;
    let mut w = Vec::with_capacity(44 + pcm.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVE");
    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    w.extend_from_slice(&1u16.to_le_bytes()); // audio format = PCM
    w.extend_from_slice(&CHANNELS.to_le_bytes());
    w.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    w.extend_from_slice(&byte_rate.to_le_bytes());
    w.extend_from_slice(&block_align.to_le_bytes());
    w.extend_from_slice(&BITS.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    w.extend_from_slice(pcm);
    w
}

/// `GET /api/in/audio` — the live audio bytes on this conversation, one source per
/// long-poll. The inbound mirror of [`get_out_audio`].
pub async fn get_in_audio(
    State(state): State<Arc<AppState>>,
    AuthBearer(auth): AuthBearer,
) -> impl IntoResponse {
    let mut rx = state.audio_in.subscribe();

    tracing::info!(auth = ?auth, "GET /api/in/audio long-poll opened");

    // Block until a source for this subscriber starts. `Start` carries the mime,
    // which must be set before any body byte; Frame/End seen before a Start (we
    // subscribed mid-source) are skipped — the client re-polls and catches the
    // next source cleanly.
    let (turn, mime) = loop {
        match rx.recv().await {
            Ok(event) => {
                if let AudioInEvent::Start { turn, mime, .. } = event {
                    break (turn, mime);
                }
            }
            Err(RecvError::Lagged(n)) => {
                tracing::warn!(missed = n, "inbound-audio subscriber lagged");
                continue;
            }
            Err(RecvError::Closed) => {
                return (StatusCode::SERVICE_UNAVAILABLE, "broadcast closed\n").into_response();
            }
        }
    };

    // Stream this source's frames as a chunked body until its `End`. Frames from
    // any other source are filtered out, so a response stays bound to the
    // single source it opened on.
    let stream = futures::stream::unfold((rx, turn), |(mut rx, turn)| async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if event.turn() != turn {
                        continue;
                    }
                    match event {
                        AudioInEvent::Frame { bytes, .. } => {
                            return Some((
                                Ok::<Bytes, std::convert::Infallible>(bytes),
                                (rx, turn),
                            ));
                        }
                        AudioInEvent::End { .. } => return None,
                        AudioInEvent::Start { .. } => continue,
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!(missed = n, "inbound-audio subscriber lagged mid-source");
                    continue;
                }
                Err(RecvError::Closed) => return None,
            }
        }
    });

    let mut response = Body::from_stream(stream).into_response();
    if let Ok(val) = HeaderValue::from_str(&mime) {
        response.headers_mut().insert(CONTENT_TYPE, val);
    }
    response
}

/// `GET /api/out/audio` — the agent's voice, one turn per long-poll.
pub async fn get_out_audio(
    State(state): State<Arc<AppState>>,
    AuthBearer(auth): AuthBearer,
) -> impl IntoResponse {
    let mut rx = state.audio_out.subscribe();
    // A held audio long-poll = their ears are on; counted while we wait for a turn.
    let _attached = state.attachments.connect(crate::body::attachments::OutChannel::Audio);

    tracing::info!(auth = ?auth, "GET /api/out/audio long-poll opened");

    // Opening this long-poll is a presence signal: warm the conversation up so its
    // process + session + upstream cache are hot before the first utterance.
    state.warm();

    // Block until a turn for this subscriber starts. `Start` carries the mime,
    // which must be set before any body byte; Frame/End seen before a Start
    // (we subscribed mid-turn) are skipped — the client re-polls and catches
    // the next turn cleanly.
    let (turn, mime) = loop {
        match rx.recv().await {
            Ok(event) => {
                if let AudioEvent::Start { turn, mime, .. } = event {
                    break (turn, mime);
                }
            }
            Err(RecvError::Lagged(n)) => {
                tracing::warn!(missed = n, "audio subscriber lagged");
                continue;
            }
            Err(RecvError::Closed) => {
                return (StatusCode::SERVICE_UNAVAILABLE, "broadcast closed\n").into_response();
            }
        }
    };

    // Stream this turn's frames as a chunked body until its `End`. Frames from
    // any other turn are filtered out, so a response stays bound to the
    // single turn it opened on.
    let stream = futures::stream::unfold(
        (rx, turn, false),
        |(mut rx, turn, done)| async move {
            if done {
                return None;
            }
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if event.turn() != turn {
                            continue;
                        }
                        match event {
                            AudioEvent::Frame { bytes, .. } => {
                                return Some((
                                    Ok::<Bytes, std::convert::Infallible>(bytes),
                                    (rx, turn, false),
                                ));
                            }
                            AudioEvent::End { .. } => return None,
                            AudioEvent::Start { .. } => continue,
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        tracing::warn!(missed = n, "audio subscriber lagged mid-turn");
                        continue;
                    }
                    Err(RecvError::Closed) => return None,
                }
            }
        },
    );

    let mut response = Body::from_stream(stream).into_response();
    if let Ok(val) = HeaderValue::from_str(&mime) {
        response.headers_mut().insert(CONTENT_TYPE, val);
    }
    response
}

fn mime_to_ext(mime: &str) -> &'static str {
    match mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "audio/wav" | "audio/wave" | "audio/x-wav" => "wav",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/ogg" | "audio/opus" => "ogg",
        "audio/flac" => "flac",
        "audio/aac" | "audio/x-aac" => "aac",
        "audio/m4a" | "audio/x-m4a" | "audio/mp4" => "m4a",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_canonical_16k_mono_16bit() {
        let pcm = vec![0u8; 320]; // 0.01s of silence
        let wav = pcm16_mono_16k_to_wav(&pcm);
        let u16le = |i: usize| u16::from_le_bytes([wav[i], wav[i + 1]]);
        let u32le = |i: usize| u32::from_le_bytes([wav[i], wav[i + 1], wav[i + 2], wav[i + 3]]);

        assert_eq!(wav.len(), 44 + pcm.len());
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(u32le(4), 36 + pcm.len() as u32); // RIFF chunk size
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(u32le(16), 16); // fmt chunk size
        assert_eq!(u16le(20), 1); // PCM
        assert_eq!(u16le(22), 1); // mono
        assert_eq!(u32le(24), 16_000); // sample rate
        assert_eq!(u16le(34), 16); // bits per sample
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32le(40), pcm.len() as u32); // data size
    }

    #[test]
    fn timeline_slices_a_spans_own_samples() {
        let mut tl = VpTimeline::default();
        tl.push(&(0..100).collect::<Vec<i16>>());
        // Absolute [10, 20) → exactly those samples.
        assert_eq!(tl.slice(10, 20).unwrap(), (10..20).collect::<Vec<i16>>());
        // End clamps to what's buffered.
        assert_eq!(tl.slice(95, 999).unwrap(), (95..100).collect::<Vec<i16>>());
    }

    #[test]
    fn timeline_maps_through_base_after_prune_and_refuses_dropped_audio() {
        let mut tl = VpTimeline::default();
        tl.push(&(0..100).collect::<Vec<i16>>());
        tl.consumed_end = 50;
        tl.prune(0, 1_000_000); // margin 0, huge ceiling → drop everything below consumed_end
        assert_eq!(tl.base, 50);
        // Absolute coordinates still map correctly through `base`.
        assert_eq!(tl.slice(60, 70).unwrap(), (60..70).collect::<Vec<i16>>());
        // Audio before `base` was pruned: refuse rather than substitute other audio.
        assert!(tl.slice(10, 20).is_none());
    }

    #[test]
    fn timeline_prune_caps_at_ceiling_even_when_unconsumed() {
        let mut tl = VpTimeline::default();
        tl.push(&vec![0i16; 1000]);
        // consumed_end stays 0 (second pass stalled), ceiling 400 → keep last 400.
        tl.prune(0, 400);
        assert_eq!(tl.base, 600);
        assert_eq!(tl.buf.len(), 400);
    }

    #[test]
    fn timeline_slice_is_none_on_empty_or_inverted_range() {
        let tl = VpTimeline::default();
        assert!(tl.slice(0, 10).is_none()); // nothing buffered yet
        let mut tl = VpTimeline::default();
        tl.push(&vec![1i16; 10]);
        assert!(tl.slice(5, 5).is_none()); // zero-width
    }

    #[test]
    fn one_second_gate_threshold_is_16k_samples() {
        // The out_task drops sliced spans shorter than this before voiceprinting.
        assert_eq!(VP_MIN_SPAN_SAMPLES, 16_000);
        let short: Vec<i16> = vec![0; 8_000]; // 0.5 s
        assert!((short.len() as u64) < VP_MIN_SPAN_SAMPLES);
    }

    /// An unremarkable room: nobody talking across, nothing to be quieter than.
    fn quiet_room() -> acoustics::Reading {
        acoustics::Reading {
            voices: 1,
            distance: acoustics::Distance::Unplaced,
            clarity: acoustics::Clarity::Clear,
            overlap_ms: 0,
            dur_ms: 4_000,
            level_vs_room: None,
        }
    }

    /// A turn of `secs` whose speech occupies `voiced` of it.
    fn turn(secs: f32, voiced: f32) -> (usize, acoustics::Sound) {
        let n = (16_000.0 * secs) as usize;
        (n, acoustics::Sound { speech_dbfs: -20.0, floor_dbfs: -60.0, clipped: 0.0, voiced })
    }

    #[test]
    fn speech_units_counts_characters_and_words_and_not_punctuation() {
        assert_eq!(speech_units("嗯"), 1);
        assert_eq!(speech_units("嗯，对。"), 2);
        assert_eq!(speech_units("帮我看看明天会不会下雨"), 11);
        assert_eq!(speech_units("ok, sure — see you at five"), 6);
        assert_eq!(speech_units("… ,,, !!"), 0, "no speech in punctuation");
        assert_eq!(speech_units(""), 0);
    }

    #[test]
    fn a_long_recording_of_almost_nothing_is_not_worth_keeping() {
        // Four seconds of audio holding 0.4 s of speech: past every length bar, and
        // its levels are healthy, which is exactly why neither of those catches it.
        let (n, sound) = turn(4.0, 0.1);
        assert!(sound.snr_db() > 20.0, "the levels look fine");
        assert!(!worth_keeping(n, &sound, "嗯 对 好 的 是 吧 哦 呀 嘛 啦", &quiet_room()));
    }

    #[test]
    fn a_turn_with_no_words_in_it_is_not_worth_keeping() {
        // A television, a cough, music: energy for two seconds, nothing transcribed.
        let (n, sound) = turn(4.0, 0.6);
        assert!(!worth_keeping(n, &sound, "", &quiet_room()), "no words, whatever the acoustics say");
        assert!(!worth_keeping(n, &sound, "嗯。", &quiet_room()), "one syllable is not a voice sample");
    }

    /// A remark long enough to be worth keeping — eleven characters.
    const SENTENCE: &str = "帮我看看明天会不会下雨";

    #[test]
    fn a_real_sentence_at_a_normal_pace_is_kept() {
        let (n, sound) = turn(4.0, 0.6);
        assert!(worth_keeping(n, &sound, SENTENCE, &quiet_room()));
    }

    #[test]
    fn a_turn_somebody_talked_across_is_never_kept() {
        // Everything else about it is ideal. The waveform still holds two voices and
        // its embedding belongs to neither of them.
        let (n, sound) = turn(4.0, 0.6);
        assert!(worth_keeping(n, &sound, SENTENCE, &quiet_room()));
        // Any overlap at all keeps a sample out, however small a share of the turn it
        // was — the bar for keeping does not forgive what the bar for recognizing does.
        let mut over = quiet_room();
        over.overlap_ms = acoustics::OVERLAP_JITTER_MS;
        assert!(over.overlap_share() < OVERLAP_BLEND_SHARE, "still recognizable from");
        assert!(!worth_keeping(n, &sound, SENTENCE, &over), "but never kept");
    }

    #[test]
    fn muttering_is_not_kept_and_neither_kind_of_quiet_needs_an_absolute_level() {
        // Close to the room it was spoken in: the words are there, the voice is not.
        let (n, mut near_floor) = turn(4.0, 0.6);
        near_floor.floor_dbfs = -35.0; // 15 dB of headroom, under the 20 the gate wants
        assert!(near_floor.snr_db() < ENROLL_MIN_SNR_DB);
        assert!(!worth_keeping(n, &near_floor, SENTENCE, &quiet_room()));

        // Or plain quieter than this speaker has just been — a doorway, a turned head.
        let (n, sound) = turn(4.0, 0.6);
        let mut hushed = quiet_room();
        hushed.level_vs_room = Some(-12.0);
        assert!(!worth_keeping(n, &sound, SENTENCE, &hushed));
        hushed.level_vs_room = Some(-3.0);
        assert!(worth_keeping(n, &sound, SENTENCE, &hushed), "a little under is still speech");
    }

    #[test]
    fn a_phrase_is_not_a_sentence() {
        let (n, sound) = turn(4.0, 0.6);
        assert!(!worth_keeping(n, &sound, "好的没问题", &quiet_room()), "five characters");
        assert!(worth_keeping(n, &sound, "好的没问题我等下过去看一眼", &quiet_room()));
    }

    #[test]
    fn a_short_turn_is_still_identified_from_but_never_kept() {
        // One second, densely spoken: enough to voiceprint (VP_MIN_SPAN_SAMPLES),
        // never enough to become part of who somebody is.
        let (n, sound) = turn(1.0, 0.9);
        assert!(n as u64 >= VP_MIN_SPAN_SAMPLES, "we do embed it");
        assert!(!worth_keeping(n, &sound, "明天会不会下雨,谢谢啦", &quiet_room()), "and we do not keep it");
    }

    #[test]
    fn keeping_a_sample_takes_a_longer_turn_than_hearing_one() {
        assert!(VP_ENROLL_MIN_SAMPLES > VP_MIN_SPAN_SAMPLES);
        let a_second_of_speech: u64 = 16_000;
        assert!(a_second_of_speech >= VP_MIN_SPAN_SAMPLES, "long enough to identify from");
        assert!(a_second_of_speech < VP_ENROLL_MIN_SAMPLES, "not long enough to keep");
    }

    /// A candidate as `people_vectors::recognize` would return it.
    fn named(subject: &str, similarity: f32) -> Candidate {
        Candidate { subject: subject.to_string(), similarity, support: 3, coherence: 0.8 }
    }

    #[test]
    fn one_middling_turn_names_nobody() {
        let mut voices = SpeakerVoices::default();
        // Above the store's floor — it was willing to say a name — but nowhere near
        // solid enough to stand on its own.
        voices.note("0", Some(&named("赵力", 0.47)), false);
        assert_eq!(voices.subject("0"), None);
    }

    #[test]
    fn a_second_turn_agreeing_settles_it() {
        let mut voices = SpeakerVoices::default();
        voices.note("0", Some(&named("赵力", 0.47)), false);
        voices.note("0", Some(&named("赵力", 0.46)), false);
        assert_eq!(voices.subject("0").as_deref(), Some("赵力"));
    }

    #[test]
    fn one_turn_solid_enough_to_file_stands_alone() {
        let mut voices = SpeakerVoices::default();
        voices.note("0", Some(&named("赵力", Modality::Voice.append_min() + 0.05)), false);
        assert_eq!(voices.subject("0").as_deref(), Some("赵力"));
    }

    /// A weak turn the camera agreed with settles on its own. One voiceprint at 0.47
    /// is one piece of evidence and waits for a second turn; the same turn with the
    /// only person on camera being that same person is two, and does not.
    #[test]
    fn a_turn_a_second_sense_agreed_with_does_not_wait_for_another() {
        let mut voices = SpeakerVoices::default();
        voices.note("0", Some(&named("赵力", 0.47)), true);
        assert_eq!(voices.subject("0").as_deref(), Some("赵力"));
    }

    /// Corroboration settles *which* subject, never *whether* there is one. A turn
    /// the store declined to name is not rescued by anybody being on camera.
    #[test]
    fn corroboration_never_invents_a_subject_the_store_did_not_propose() {
        let mut voices = SpeakerVoices::default();
        voices.note("0", None, true);
        assert_eq!(voices.subject("0"), None);
    }

    #[test]
    fn turns_split_between_two_people_name_neither() {
        let mut voices = SpeakerVoices::default();
        voices.note("0", Some(&named("赵力", 0.48)), false);
        voices.note("0", Some(&named("赵君宁", 0.47)), false);
        assert_eq!(voices.subject("0"), None, "sounding like both is not being either");
        // A third turn breaks the tie.
        voices.note("0", Some(&named("赵力", 0.46)), false);
        assert_eq!(voices.subject("0").as_deref(), Some("赵力"));
    }

    #[test]
    fn speakers_are_weighed_apart_and_an_unheard_one_is_nobody() {
        let mut voices = SpeakerVoices::default();
        voices.note("0", Some(&named("赵力", 0.47)), false);
        voices.note("1", Some(&named("赵力", 0.47)), false);
        assert_eq!(voices.subject("0"), None, "one turn each, not two for either");
        assert_eq!(voices.subject("2"), None, "a speaker with no turns is unplaced");
    }

    #[test]
    fn turns_that_named_nobody_are_kept_as_nobody() {
        let mut voices = SpeakerVoices::default();
        for _ in 0..5 {
            voices.note("0", None, false);
        }
        assert_eq!(voices.subject("0"), None, "five unplaced turns do not add up to a person");
    }
}
