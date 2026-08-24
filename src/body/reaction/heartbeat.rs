//! The reflection ("sleep") pass — consolidate the raw frontier into
//! episodes and facets, cluster faces and voices, and fade what has gone cold.
//!
//! ## There is no session-swap here, on purpose — the agent compacts itself
//!
//! This module used to also own a **hot-swap**: count a session's accumulated
//! prompt+reply characters, and once past a ceiling ask it for a self-briefing and
//! reopen a replacement seeded with that briefing. It is **deleted**, and nothing
//! replaced it, because **the underlying agent already compacts its own context in
//! place** — automatically, near its real window, with far better information than we
//! have out here.
//!
//! We were never in a position to do this well:
//!
//! - **We cannot see the context.** `ContextBudget` counted characters *we* sent and
//!   received. It could not see the harness's own system prompt and tool schemas —
//!   the large majority of every request — so the number it thresholded on was a
//!   small, drifting fraction of the truth.
//! - **Summarize-and-reopen was a workaround for a gap the wire no longer has.** ACP
//!   offered `session/new`, `session/prompt`, `session/cancel`, `session/update` and no
//!   compaction method, so reopening was not the chosen design — it was the only move
//!   available from outside the boundary, and strictly lossier than what the agent does
//!   internally. The codex wire exposes `thread/compact/start` outright, which settles
//!   the argument rather than reopening it: compaction is the agent's, on its own
//!   history, and now even its own explicit verb. Nothing out here needs to imitate it.
//! - **The ceiling was wrong by more than an order of magnitude.** 48,000 chars is
//!   roughly 3% of a 1M-token window. In practice a conversation crossed it within one
//!   sitting, so an ordinary conversation was being summarized and restarted
//!   repeatedly, for nothing.
//! - **It fought the rungs being long-lived.** Cognition is long-lived so it can
//!   remember what it already arranged (`docs/arch/agents.md#session-lifetime-per-rung`).
//!   Swapping at 3% of the window threw that thread away and handed back a paragraph —
//!   the same forgetting the long-lived session exists to prevent, just slower.
//!
//! **So: context bounding is the underlying agent's job, and we do not duplicate it.**
//! If a future wire genuinely has no auto-compaction, the honest fix is to bound it in
//! that adapter where the real numbers are visible — not to re-add a character counter
//! up here that cannot see what it is counting.
//!
//! What still bounds a session from out here is failure, not size: a turn that errors
//! discards the session and the next one cold-opens (see the `Err` arms in
//! [`super::reaction_loop`] and [`super::cognition`]). The [log](super) remains the durable
//! backstop either way.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Duration, Utc};

use crate::foundation::codex::SessionOpts;
use crate::identity::Role;
use crate::body::capabilities::{face, voiceprint};
use crate::mind::memory::journal::after_cursor;
use crate::foundation::pcm;
use crate::mind::memory::{decay, episodes, facets, layout, people_vectors};
use crate::foundation::observatory::EventKind;
use crate::foundation::registry;
use crate::types::{Channel, JournalEntry, Origin};
use crate::foundation::vendors::ffmpeg_frame;

use super::Reaction;

/// Below this many unconsolidated signals, a reflection round is skipped — not
/// worth a whole session (and its subprocess spawn) to file a handful of lines;
/// they wait on the frontier for the next reflection tick.
const MIN_REFLECT_SIGNALS: usize = 4;

/// How many of a frontier's signals count toward [`MIN_REFLECT_SIGNALS`]: everything
/// except the host's own clock (see [`super::NON_ACTIVITY_CHANNELS`]). Those wakes stay
/// *in* the frontier — "then it was quiet for three hours" is worth settling — they
/// just may not be the reason a session opens. Otherwise a conversation left alone would
/// tick its way over the threshold on its own clock rows and reflect on nothing.
fn reflectable(tail: &[JournalEntry]) -> usize {
    tail.iter()
        .filter(|e| {
            let channel = crate::mind::memory::journal::entry_channel(e);
            !super::NON_ACTIVITY_CHANNELS.contains(&channel.as_str())
        })
        .count()
}

/// The two standing sentences of the consolidation prompt — the only text in it that
/// names tools rather than carrying data.
///
/// **Constants, and reachable from the tool layer's tests, because prose is where the
/// prefix keeps getting lost.** Both of these named their verbs bare
/// (`update_proactivity`, `keep_and_fade`, `image-text-to-text`) for as long as the
/// prefixed tools have existed, and no sweep of `src/identity/*.md` could see them: this
/// text is assembled here, not written in a prompt file. Naming them lets
/// `no_agent_facing_text_names_a_verb_without_its_prefix` read them without building a
/// `Frontier`.
pub(crate) const PROACTIVITY_HEADING: &str = "## Current proactivity.md (your read on what your words have earned — regenerate via `hi_update_proactivity` if any word of yours, asked for or not, landed or fell flat this stretch)\n";

/// See [`PROACTIVITY_HEADING`].
pub(crate) const CONSOLIDATION_TOOLS: &str =
    "Consolidate these now. Use `count` against the single numbered frontier; \
     `hi_keep_and_fade` acts on the channel and day shown above; \
     `hi_image_text_to_text` takes the image ref shown beside a signal.";

/// The gathered frontier and context for the consolidated pass.
struct Frontier {
    tail: Vec<JournalEntry>,
    prior: Vec<String>,
    face_ids: HashMap<usize, Vec<String>>,
    voice_ids: HashMap<usize, Vec<String>>,
    pressure: Vec<decay::FadeDay>,
}

/// Consolidate the one conversation's unconsolidated frontier into episodes and
/// facets in a dedicated "sleep" pass. Reads the raw log after its
/// [`episodes::consolidation_cursor`], opens one reflection session, and drives
/// it to completion. Run from the global reflection clock (see
/// [`super::reflection`]). A crash leaves the frontier for the next tick.
pub(super) async fn consolidate(reaction: &Reaction, id: &registry::SessionSlug) {
    if let Err(err) = run_consolidation(reaction, id).await {
        // A pass already in flight when shutdown began fails because its child took
        // the process group's signal — expected, not a fault. Keep it out of the
        // WARN stream so a real consolidation failure stays visible.
        if reaction.inner.shutdown.is_triggered() {
            tracing::debug!(error = %format!("{err:#}"), "consolidation aborted by shutdown");
        } else {
            tracing::warn!(error = %format!("{err:#}"), "consolidation failed");
        }
    }
}

async fn run_consolidation(
    reaction: &Reaction,
    id: &registry::SessionSlug,
) -> anyhow::Result<()> {
    let data_dir = reaction.inner.memory.data_dir();

    // Gather the frontier; a pass is only worth opening when there is enough on it.
    // The cheap cursor+tail read gates the expensive face/voice clustering, so a
    // caught-up store costs almost nothing.
    let cursor = episodes::consolidation_cursor(data_dir).await?;
    let tail = after_cursor(data_dir, cursor.as_deref(), episodes::REFLECTION_TAIL_LIMIT).await?;
    if reflectable(&tail) < MIN_REFLECT_SIGNALS {
        tracing::debug!("consolidation skipped; not enough on the frontier");
        return Ok(());
    }
    // Prior episode gists give continue-vs-new context; faces and voices are
    // clustered mechanically so the prompt can show a stable id per detected person
    // to name. The old-store pressure lets the same pass tend the past, fading what
    // has gone cold.
    let prior = episodes::recent_gists(&reaction.inner.memory, 2).await.unwrap_or_default();
    let face_ids = cluster_faces(data_dir, &tail).await;
    let voice_ids = cluster_voices(data_dir, &tail).await;
    let pressure = decay::fade_pressure(data_dir, Utc::now()).await.unwrap_or_default();
    let frontier = Frontier { tail, prior, face_ids, voice_ids, pressure };

    tracing::info!(n = frontier.tail.len(), "reflection fired");

    // Forget ambient, one-off identity clusters — the video-night strangers and
    // passers-by that would otherwise bury the real people. Runs once per
    // consolidation, on the same reflection clock. The log names each cluster as it goes, since the deletion
    // itself leaves nothing behind to inspect.
    match people_vectors::sweep_forgettable(data_dir, Utc::now()).await {
        Ok(report) if !report.forgotten.is_empty() => {
            for v in &report.forgotten {
                tracing::info!(
                    subject = %v.subject,
                    samples = v.samples,
                    occasions = v.occasions,
                    "identity cluster forgotten (ambient, one-off, gone cold)",
                );
            }
            tracing::info!(
                examined = report.examined,
                forgotten = report.forgotten.len(),
                "cluster forgetting sweep",
            );
        }
        Ok(_) => {}
        Err(err) => tracing::warn!(error = %format!("{err:#}"), "cluster forgetting sweep failed"),
    }

    // The facet subject index is global — gathered once so the mind reuses a subject
    // instead of coining a near-duplicate.
    let subjects = facets::facet_subject_index(data_dir).await.unwrap_or_default();

    // The current proactivity read, folded into the prompt so the pass can
    // regenerate it from old-plus-new — the session has no cwd to read the file
    // itself, so it goes in (and back out through `update_proactivity`) like facets.
    let current_proactivity = crate::mind::memory::proactivity::read(data_dir).await.ok().flatten();

    let prompt = build_consolidation_prompt(&frontier, &subjects, current_proactivity.as_deref());
    // The same prompt a Reflection *mail* turn opens with — one self-contained file. It
    // was the role layer alone until `cd008a6`, then seed-plus-layer; it is now neither,
    // because `reflection.md` carries the whole thing.
    let system_prompt = crate::identity::reflection_prompt(data_dir).await;


    // **The pass runs under Reflection's standing id**, handed in by the loop that owns
    // it ([`super::reflection`]). It used to mint its own registration scoped to this
    // function, which made the rung addressable only *during* a pass and, worse, meant a
    // worker it dispatched outlived the session that asked — the report came back to an
    // address that had already been dropped. The note that used to sit here said exactly
    // that and pointed at a later item; this is that item.
    //
    // The standing reflection session has no conversation identity. Its role
    // header is enough for MCP routing.
    let session = reaction
        .inner
        .agent
        .session(
            Role::Reflection,
            Some(id.clone()),
            SessionOpts { system_prompt: Some(system_prompt), ..Default::default() },
        )
        .await?;
    reaction
        .inner
        .observatory
        .record(
            EventKind::SessionOpened {
                kind: Role::Reflection,
                id: session.id().to_string(),
            },
        )
        .await;

    let run = session.prompt(prompt).await?;
    run.wait().await?;

    tracing::info!("reflection finished");
    Ok(())
}

/// Assemble the consolidated reflection prompt: the global subject index, the
/// prior episode context, the old-media pressure, and one numbered frontier.
fn build_consolidation_prompt(
    frontier: &Frontier,
    subjects: &[String],
    current_proactivity: Option<&str>,
) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    if !subjects.is_empty() {
        s.push_str("## Subjects you already model (reuse these refs)\n");
        let _ = writeln!(s, "{}\n", subjects.join(", "));
    }
    render_frontier(&mut s, frontier);
    s.push('\n');
    // The current proactivity read goes in so the pass regenerates it from
    // old-plus-new (it can't read the file itself — no cwd). What to do with it
    // lives in reflection.md; this just carries the data.
    s.push_str(PROACTIVITY_HEADING);
    match current_proactivity {
        Some(c) if !c.trim().is_empty() => {
            s.push_str(c.trim());
            s.push_str("\n\n");
        }
        _ => s.push_str("(none yet)\n\n"),
    }
    s.push_str(CONSOLIDATION_TOOLS);
    s
}

/// Render the prior-episode context, old-media list, and unconsolidated
/// frontier as one numbered, oldest-first list. Image signals are marked
/// `⟨faces: <id>…⟩` when clustering placed faces, else with the still ref the
/// mind can inspect. Audio clips are marked `⟨voice: <id>…⟩` when voiceprint
/// clustering placed a speaker.
fn render_frontier(s: &mut String, g: &Frontier) {
    use std::fmt::Write as _;
    if !g.prior.is_empty() {
        s.push_str("## Your last episodes here (for continue-vs-new judgment)\n");
        for gist in &g.prior {
            let _ = writeln!(s, "- {}", gist.replace('\n', " "));
        }
        s.push('\n');
    }
    // Old-store pressure: consolidated days that still hold full media, heaviest
    // first. Only surface days weighty enough to be worth the mind's attention —
    // a cheap visibility gate, not a forgetting decision.
    let heavy: Vec<&decay::FadeDay> =
        g.pressure.iter().filter(|d| d.bytes >= FADE_SURFACE_FLOOR).collect();
    if !heavy.is_empty() {
        s.push_str(
            "## Older media still at full fidelity (all already settled — fade what's gone cold)\n",
        );
        for d in heavy {
            let eps = if d.episodes.is_empty() {
                String::new()
            } else {
                format!("  episodes: {}", d.episodes.join(", "))
            };
            let _ = writeln!(
                s,
                "- {} {}  ({}d old, {} events since, {}){}",
                d.channel.as_str(),
                d.date,
                d.age_days,
                d.episodes_since,
                human_bytes(d.bytes),
                eps
            );
        }
        s.push('\n');
    }
    s.push_str("## Unconsolidated signals (oldest first)\n");
    let cooccur = cooccurring_faces(&g.tail, &g.face_ids);
    for (i, e) in g.tail.iter().enumerate() {
        let mut line = render_signal(e);
        // **Who it came from, stated — never left for the pass to work out.** A
        // signal with no `⟨from: …⟩` is one the machinery raised (a clock wake, a
        // worker report): nobody sent it, so nobody is owed a person record for it.
        // An unattributed *person* signal says `unknown` out loud, because silence
        // here is what invited a guess in the first place.
        if let Some(sender) = crate::mind::memory::journal::entry_sender(e) {
            let _ = write!(line, " ⟨from: {}⟩", sender.label());
        }
        match g.face_ids.get(&i).filter(|v| !v.is_empty()) {
            Some(ids) => {
                let _ = write!(line, " ⟨faces: {}⟩", ids.join(", "));
            }
            None if is_image(e) => match still_ref(e) {
                Some(reff) => {
                    let _ = write!(line, " ⟨image — `image-text-to-text` ref: {reff}⟩");
                }
                None => line.push_str(" ⟨image⟩"),
            },
            None => {}
        }
        if let Some(ids) = g.voice_ids.get(&i).filter(|v| !v.is_empty()) {
            let _ = write!(line, " ⟨voice: {}⟩", ids.join(", "));
        }
        if let Some(faces) = cooccur.get(&i).filter(|v| !v.is_empty()) {
            if faces.len() == 1 {
                let _ = write!(line, " ⟨one face present: {}⟩", faces[0]);
            } else {
                let _ = write!(line, " ⟨faces present: {} (ambiguous)⟩", faces.join(", "));
            }
        }
        let _ = writeln!(s, "[{}] {}", i + 1, line);
    }
}

/// Below this, a cold day's leftover media isn't worth surfacing to the mind — the
/// visibility gate on the old-store section. A cheap mechanical threshold on what
/// to *show*, never a decision about what to forget.
const FADE_SURFACE_FLOOR: u64 = 8 * 1024 * 1024;

/// A byte count as a short human string (`1.8 GB`) for the old-store list.
fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} B") } else { format!("{v:.1} {}", UNITS[i]) }
}

/// One frontier signal as a transcript line, reusing the snapshot's renderer so
/// the speaker glyph and channel formatting match what the reaction sees.
fn render_signal(e: &JournalEntry) -> String {
    use crate::mind::memory::snapshot::{Speaker, transcript_line};
    use crate::mind::memory::journal::{entry_body, entry_channel};
    match e {
        JournalEntry::Message { message, .. } if message.from.is_agent() => {
            transcript_line(Speaker::You, Channel::Text.as_str(), entry_body(e))
        }
        JournalEntry::Message { .. } => {
            transcript_line(Speaker::Them, entry_channel(e).as_str(), entry_body(e))
        }
        JournalEntry::Presentation { body, .. } => {
            transcript_line(Speaker::You, Channel::View.as_str(), body.as_str())
        }
        JournalEntry::Observation { channel, body, stream, .. } => {
            transcript_line(Speaker::Them, &channel.with_stream(stream.as_deref()), body.as_str())
        }
        // Machinery the agent's own rungs emitted is the agent's side of the row; a
        // worker reporting back is something that reached it. Rendering both as
        // inbound is how "spoke the reply aloud" showed up as the person talking.
        JournalEntry::Internal { channel, body, origin, .. } => {
            let who = match origin {
                // Only what a rung *emitted* is the agent's side. A deadline coming
                // due is `Host`, and it arrives at the agent like an utterance does.
                Some(Origin::Reaction) => Speaker::You,
                _ => Speaker::Them,
            };
            transcript_line(who, channel.as_str(), body.as_str())
        }
    }
}

/// Whether a frontier signal carries a still image — so the prompt can mark it
/// `⟨image⟩` even when face clustering found nothing or is unconfigured.
fn is_image(e: &JournalEntry) -> bool {
    crate::mind::memory::journal::entry_mime(e).is_some_and(|m| m.starts_with("image/"))
}

/// The ref for a still-image signal, in the grammar `image-text-to-text` resolves —
/// so reflection can look at the photo itself and fold what it shows into
/// episodes/facets, rather than indexing it blind. `None` for non-image or
/// media-less signals.
///
/// It takes the channel from the entry rather than assuming one. Consolidation sees
/// every channel, so the handed passport and the camera still both come through here;
/// while the ref omitted its channel, only the camera's resolved.
fn still_ref(e: &JournalEntry) -> Option<String> {
    if !crate::mind::memory::journal::entry_mime(e)?.starts_with("image/") {
        return None;
    }
    crate::mind::memory::journal::entry_media_ref(e)
}

/// How far a voice turn's window is padded, in seconds, when matching co-present
/// faces. Deliberately small: a camera "minute" is itself a ~60s interval, so the
/// overlap test already carries most of the slack; this just absorbs the seam
/// between a clip and a neighbouring frame. We are **loose on alignment, strict on
/// commitment** — co-occurrence is evidence for the mind, never an auto-bind.
const COOCCUR_TOLERANCE_SECS: i64 = 2;

/// For each Audio signal, the distinct face cluster ids whose vision interval
/// overlapped that voice turn's window — making "the same person, the same
/// moment" legible to the mind. This is the binding substrate from the design:
/// humans tie a voice to a face by *correlation within a tolerant window*, not by
/// a shared clock, so we match by **interval overlap** (each side is `[ts, ts +
/// duration]`, the voice side padded by [`COOCCUR_TOLERANCE_SECS`]) rather than
/// timestamp equality. The count is the ambiguity cue: exactly one face over a
/// turn is near-certain evidence it is the speaker; several means the mind must
/// judge (or wait for a clearer moment). We only surface the evidence — the
/// cross-sense bind stays the mind's call (`merge_people`), per
/// [[project-people-recognition-design]].
fn cooccurring_faces(
    tail: &[JournalEntry],
    face_ids: &HashMap<usize, Vec<String>>,
) -> HashMap<usize, Vec<String>> {
    let tol = Duration::seconds(COOCCUR_TOLERANCE_SECS);

    // The time interval each face-bearing vision signal covered: `[ts, ts + dur]`
    // (a still is a point; a camera minute spans its duration).
    let faces_at: Vec<(DateTime<Utc>, DateTime<Utc>, &[String])> = face_ids
        .iter()
        .filter_map(|(&i, ids)| {
            let e = tail.get(i)?;
            if ids.is_empty() {
                return None;
            }
            let ts = crate::mind::memory::journal::entry_ts(e);
            let media = crate::mind::memory::journal::entry_media(e);
            Some((ts, ts + media_dur(media), ids.as_slice()))
        })
        .collect();
    if faces_at.is_empty() {
        return HashMap::new();
    }

    let mut out: HashMap<usize, Vec<String>> = HashMap::new();
    for (j, e) in tail.iter().enumerate() {
        if crate::mind::memory::journal::entry_channel(e) != Channel::Audio {
            continue;
        }
        let ts = crate::mind::memory::journal::entry_ts(e);
        let media = crate::mind::memory::journal::entry_media(e);
        let win_start = ts - tol;
        let win_end = ts + media_dur(media) + tol;
        let mut seen: Vec<String> = Vec::new();
        for (f_start, f_end, ids) in &faces_at {
            // Two intervals overlap iff each starts no later than the other ends.
            if win_start <= *f_end && *f_start <= win_end {
                for id in *ids {
                    if !seen.contains(id) {
                        seen.push(id.clone());
                    }
                }
            }
        }
        if !seen.is_empty() {
            out.insert(j, seen);
        }
    }
    out
}

/// A media payload's duration as a [`Duration`], or zero when absent (a still, or
/// a media-less live-mic turn) — those are treated as instantaneous points.
fn media_dur(media: Option<&crate::types::Media>) -> Duration {
    media
        .and_then(|m| m.duration_ms)
        .map(|ms| Duration::milliseconds(ms as i64))
        .unwrap_or_else(Duration::zero)
}

/// Mechanically cluster the faces in the frontier's vision signals: for each one,
/// detect+embed and [`people_vectors::assign`] every salient face to the people
/// store (append to a near cluster, or mint a fresh id). Returns, per tail index,
/// the cluster ids the faces landed in — the stable handles the reflection prompt
/// shows so the mind can name a face, even a first-time one. Covers both posted
/// stills and camera-stream minutes (one keyframe decoded out of the video, the
/// same frame the perceive-time note used). No-op (empty) when the face capability
/// is unconfigured; a per-signal failure is logged and skipped.
async fn cluster_faces(
    data_dir: &Path,
    tail: &[JournalEntry],
) -> HashMap<usize, Vec<String>> {
    let mut out: HashMap<usize, Vec<String>> = HashMap::new();
    if !face::available() {
        return out;
    }
    for (i, e) in tail.iter().enumerate() {
        if crate::mind::memory::journal::entry_channel(e) != Channel::Vision {
            continue;
        }
        let Some(m) = crate::mind::memory::journal::entry_media(e) else {
            continue;
        };
        let ts_val = crate::mind::memory::journal::entry_ts(e);
        let ts = &ts_val;
        let is_image = m.mime.starts_with("image/");
        let is_video = m.mime.starts_with("video/");
        if !is_image && !is_video {
            continue;
        }
        let path = layout::channel_day_dir(data_dir, Channel::Vision, *ts).join(&m.file);
        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(err) => {
                tracing::warn!(error = %err, "cluster: reading vision media failed");
                continue;
            }
        };
        // A still is ready for the face pipeline; a camera minute needs one
        // keyframe decoded out first (the same path the perceive-time note takes).
        let image: bytes::Bytes = if is_video {
            match ffmpeg_frame::first_frame(bytes.into()).await {
                Ok(frame) => frame,
                Err(err) => {
                    tracing::warn!(error = %format!("{err:#}"), "cluster: keyframe extraction failed");
                    continue;
                }
            }
        } else {
            bytes.into()
        };
        // Clone for detection (it consumes the bytes); keep `image` to crop the
        // recognized faces out of for previews.
        let faces = match face::detect_and_embed(image.clone()).await {
            Ok(f) => f,
            Err(err) => {
                tracing::warn!(error = %format!("{err:#}"), "cluster: face detect failed");
                continue;
            }
        };
        for f in faces.iter().filter(|f| salient(f)) {
            // The crop is the sample's canonical media; without it there's no 1:1
            // pair to store, so skip this face rather than enroll a bare vector.
            let jpg = match face::crop_to_jpeg(image.as_ref(), f.bbox, 0.3) {
                Ok(jpg) => jpg,
                Err(err) => {
                    tracing::warn!(error = %format!("{err:#}"), "cluster: face crop failed");
                    continue;
                }
            };
            match people_vectors::assign(data_dir, people_vectors::Modality::Face, &f.embedding, &jpg, "jpg").await {
                Ok(id) => {
                    out.entry(i).or_default().push(id);
                }
                Err(err) => tracing::warn!(error = %format!("{err:#}"), "cluster: assign failed"),
            }
        }
    }
    out
}

/// Mechanically cluster the voices in the frontier's audio clips: for each clip
/// that carries persisted audio, decode it, embed a voiceprint, and
/// [`people_vectors::assign`] it to the people store (append to a near cluster, or
/// mint a fresh id). Returns, per tail index, the cluster ids — the audio twin of
/// [`cluster_faces`], so the mind can name a voice the same way it names a face.
/// No-op (empty) without the voiceprint capability. Only clips have media here;
/// live-mic utterances are media-less and are clustered inline on the stream.
async fn cluster_voices(
    data_dir: &Path,
    tail: &[JournalEntry],
) -> HashMap<usize, Vec<String>> {
    let mut out: HashMap<usize, Vec<String>> = HashMap::new();
    if !voiceprint::available() {
        return out;
    }
    for (i, e) in tail.iter().enumerate() {
        if crate::mind::memory::journal::entry_channel(e) != Channel::Audio {
            continue;
        }
        let Some(m) = crate::mind::memory::journal::entry_media(e) else {
            continue;
        };
        let ts_val = crate::mind::memory::journal::entry_ts(e);
        let ts = &ts_val;
        let body = crate::mind::memory::journal::entry_body(e);
        // A diarized, multi-speaker clip ("说话人0：…") is not one voice; embedding
        // the blend into a single sample would contaminate a cluster. Mirror the
        // hear-time guard in `voice_note` and skip it — the labeled transcript
        // already attributes the turns.
        if body.starts_with("说话人") {
            continue;
        }
        let path = layout::channel_day_dir(data_dir, Channel::Audio, *ts).join(&m.file);
        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(err) => {
                tracing::warn!(error = %err, "cluster: reading audio failed");
                continue;
            }
        };
        let samples = match pcm::to_i16_16k_mono(&bytes, &m.mime) {
            Ok(s) if !s.is_empty() => s,
            Ok(_) => continue,
            Err(err) => {
                tracing::warn!(error = %format!("{err:#}"), "cluster: audio decode failed");
                continue;
            }
        };
        let embedding = match voiceprint::embed(samples).await {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!(error = %format!("{err:#}"), "cluster: voiceprint embed failed");
                continue;
            }
        };
        // The clip is the sample's canonical media, stored 1:1 with its voiceprint.
        let ext = std::path::Path::new(&m.file).extension().and_then(|e| e.to_str()).unwrap_or("wav");
        match people_vectors::assign(data_dir, people_vectors::Modality::Voice, &embedding, &bytes, ext).await {
            Ok(id) => {
                out.entry(i).or_default().push(id);
            }
            Err(err) => tracing::warn!(error = %format!("{err:#}"), "cluster: voice assign failed"),
        }
    }
    out
}

/// Skip incidental/background faces: require a confident detection and a face big
/// enough (in original-image pixels) to embed reliably.
fn salient(f: &crate::body::capabilities::face::DetectedFace) -> bool {
    let w = (f.bbox[2] - f.bbox[0]).max(0.0);
    let h = (f.bbox[3] - f.bbox[1]).max(0.0);
    f.score >= 0.6 && w >= 50.0 && h >= 50.0
}

#[cfg(test)]
mod frontier_tests {
    use super::*;

    fn on(channel: Channel) -> JournalEntry {
        crate::mind::memory::journal::legacy_signal_in("x".into(), Utc::now(), channel, String::new(), None, None, None, None)
    }

    #[test]
    fn clock_wakes_alone_never_reach_the_threshold() {
        let tail: Vec<JournalEntry> =
            (0..MIN_REFLECT_SIGNALS * 3).map(|_| on(Channel::Clock)).collect();
        assert_eq!(
            reflectable(&tail), 0,
            "a conversation left alone must not reflect on its own clock"
        );
    }

    #[test]
    fn real_signals_count_even_with_clock_wakes_mixed_in() {
        let tail = vec![
            on(Channel::Clock),
            on(Channel::Text),
            on(Channel::Clock),
            on(Channel::Worker),
            on(Channel::View),
        ];
        assert_eq!(reflectable(&tail), 3);
    }

    use crate::types::Sender;

    fn from(channel: Channel, body: &str, sender: Option<Sender>) -> JournalEntry {
        crate::mind::memory::journal::legacy_signal_in("x".into(), Utc::now(), channel, body.into(), None, None, None, sender)
    }

    fn frontier_of(tail: Vec<JournalEntry>) -> String {
        let mut s = String::new();
        render_frontier(
            &mut s,
            &Frontier {
                tail,
                prior: Vec::new(),
                face_ids: HashMap::new(),
                voice_ids: HashMap::new(),
                pressure: Vec::new(),
            },
        );
        s
    }

    /// The default is shown *as* a default. A pass that cannot tell an assumption
    /// from a recognition has no way to prefer evidence over it — which is how a
    /// guess became indistinguishable from a fact in the first place.
    #[test]
    fn an_owner_default_renders_with_its_basis() {
        let s = frontier_of(vec![from(
            Channel::Text,
            "save this blog",
            Some(Sender::owner_or_unknown(Some("赵力"))),
        )]);
        assert!(s.contains("⟨from: 赵力 (owner, by default)⟩"), "{s}");
    }

    /// Unattributed says so out loud. Silence is what invited the guess.
    #[test]
    fn an_ungrounded_sender_says_unknown_rather_than_nothing() {
        let s = frontier_of(vec![from(Channel::Audio, "…someone talking", Some(Sender::unknown()))]);
        assert!(s.contains("⟨from: unknown⟩"), "{s}");
    }

    /// A machine channel is not an unknown person — it is *no* person, and the
    /// absence of the mark is how the pass tells those two apart.
    #[test]
    fn machine_channels_carry_no_from_mark_at_all() {
        let s = frontier_of(vec![
            from(Channel::Clock, "check-in due", None),
            from(Channel::Worker, "worker 3 reported", None),
        ]);
        assert!(!s.contains("⟨from:"), "machine traffic has no sender: {s}");
    }

    /// A name in a body is a topic. Nothing may promote it to a sender — this is the
    /// exact shape that put one person's words on a colleague's facet.
    #[test]
    fn a_name_in_the_body_never_becomes_the_sender() {
        let s = frontier_of(vec![from(
            Channel::Text,
            "rewrite xuwenhan's basketball note for the team",
            Some(Sender::owner_or_unknown(Some("赵力"))),
        )]);
        assert!(s.contains("⟨from: 赵力 (owner, by default)⟩"), "{s}");
        assert!(!s.contains("⟨from: xuwenhan"), "a mentioned name is not a sender: {s}");
    }

    /// With no owner declared there is nothing to default to, and the honest answer
    /// is unknown rather than the nearest plausible person.
    #[test]
    fn no_declared_owner_leaves_addressed_signals_unattributed() {
        for owner in [None, Some(""), Some("   ")] {
            let sender = Sender::owner_or_unknown(owner);
            assert!(!sender.is_grounded(), "{owner:?} must not ground a sender");
            assert_eq!(sender.label(), "unknown");
        }
    }
}

#[cfg(test)]
mod cooccur_tests {
    use super::*;
    use crate::types::Media;
    use chrono::TimeZone;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn vision(ts: DateTime<Utc>, dur_ms: Option<u64>) -> JournalEntry {
        crate::mind::memory::journal::legacy_signal_in("v".into(), ts, Channel::Vision, String::new(), None, Some(Media {
                file: "f".into(),
                mime: "image/jpeg".into(),
                duration_ms: dur_ms,
                width: None,
                height: None,
            }), None, None)
    }

    fn audio(ts: DateTime<Utc>, dur_ms: Option<u64>) -> JournalEntry {
        crate::mind::memory::journal::legacy_signal_in("a".into(), ts, Channel::Audio, "hi".to_string(), None, dur_ms.map(|ms| Media {
                file: "f".into(),
                mime: "audio/mp3".into(),
                duration_ms: Some(ms),
                width: None,
                height: None,
            }), None, None)
    }

    fn faces(pairs: &[(usize, &str)]) -> HashMap<usize, Vec<String>> {
        let mut m: HashMap<usize, Vec<String>> = HashMap::new();
        for (i, id) in pairs {
            m.entry(*i).or_default().push((*id).to_string());
        }
        m
    }

    #[test]
    fn sole_face_overlapping_a_voice_turn_is_one_face() {
        // A still at t=0, a (media-less) live-mic turn at t=1 — within tolerance.
        let tail = vec![vision(at(0), None), audio(at(1), None)];
        let c = cooccurring_faces(&tail, &faces(&[(0, "ff32ce3w")]));
        assert_eq!(c.get(&1).map(Vec::as_slice), Some(["ff32ce3w".to_string()].as_slice()));
    }

    #[test]
    fn two_distinct_faces_in_window_are_ambiguous() {
        let tail = vec![vision(at(0), None), vision(at(1), None), audio(at(1), None)];
        let c = cooccurring_faces(&tail, &faces(&[(0, "aaa"), (1, "bbb")]));
        let got = c.get(&2).unwrap();
        assert_eq!(got.len(), 2);
        assert!(got.contains(&"aaa".to_string()) && got.contains(&"bbb".to_string()));
    }

    #[test]
    fn the_same_face_across_frames_counts_once() {
        let tail = vec![vision(at(0), None), vision(at(1), None), audio(at(1), None)];
        let c = cooccurring_faces(&tail, &faces(&[(0, "aaa"), (1, "aaa")]));
        assert_eq!(c.get(&2).map(Vec::as_slice), Some(["aaa".to_string()].as_slice()));
    }

    #[test]
    fn a_face_outside_the_window_does_not_co_occur() {
        let tail = vec![vision(at(0), None), audio(at(100), None)];
        let c = cooccurring_faces(&tail, &faces(&[(0, "aaa")]));
        assert!(c.get(&1).is_none());
    }

    #[test]
    fn a_camera_minute_overlaps_a_voice_turn_within_it() {
        // A 60s vision minute from t=0; a voice turn at t=30 overlaps even though
        // the minute's start is well before the turn — interval overlap, not equality.
        let tail = vec![vision(at(0), Some(60_000)), audio(at(30), Some(2_000))];
        let c = cooccurring_faces(&tail, &faces(&[(0, "aaa")]));
        assert_eq!(c.get(&1).map(Vec::len), Some(1));
    }

    fn frontier(tail: Vec<JournalEntry>, face_ids: HashMap<usize, Vec<String>>) -> Frontier {
        Frontier {
            tail,
            prior: Vec::new(),
            face_ids,
            voice_ids: HashMap::new(),
            pressure: Vec::new(),
        }
    }

    #[test]
    fn prompt_annotates_a_sole_co_occurring_face() {
        let tail = vec![vision(at(0), None), audio(at(1), None)];
        let g = frontier(tail, faces(&[(0, "ff32ce3w")]));
        let p = build_consolidation_prompt(&g, &[], None);
        assert!(p.contains("⟨one face present: ff32ce3w⟩"), "prompt was:\n{p}");
    }

    #[test]
    fn global_subjects_appear_once_above_the_groups() {
        let a = frontier(vec![audio(at(0), None), audio(at(1), None), audio(at(2), None), audio(at(3), None)], HashMap::new());
        let p = build_consolidation_prompt(&a, &["people/alice".into(), "places/office".into()], None);
        assert_eq!(p.matches("Subjects you already model").count(), 1, "prompt was:\n{p}");
        assert!(p.contains("people/alice, places/office"), "prompt was:\n{p}");
    }
}
