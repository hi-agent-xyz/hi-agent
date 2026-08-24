//! Public types — spec primitives plus journal records.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// -----------------------------------------------------------------------------
// Channel — the six spec channels
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// The text channel — typed input and the agent's worded replies. `alias`
    /// keeps journals written before the thought→text rename loadable.
    #[serde(alias = "thought")]
    Text,
    Vision,
    Audio,
    /// Handed artifacts — a file the user gives the agent (a contract, a passport
    /// scan), received by reference through an upload carrier. NOT a sense: the
    /// agent doesn't *perceive* a file, it is *handed* one; the bytes are kept
    /// verbatim and the signal says who handed over what.
    File,
    Touch,
    Smell,
    Taste,
    /// The agent's own presentation surface. Outbound it is what it put on the
    /// screen — like [`Channel::File`] not a sense: the agent doesn't perceive a
    /// view, it *shows* one. Recorded so a restart can tell what is already up
    /// (and so it doesn't show the same thing twice).
    ///
    /// **Inbound it is the person going to a view** — the band's history, a
    /// bookmark, or back to live. Addressed, so it carries the owner by default
    /// like `text` does; deliberately *not* turn-driving, because walking the band
    /// through five tiles must not produce five turns. It is read into the next
    /// turn's context instead, which is the moment it matters. See
    /// `docs/arch/stage.md#where-they-went-is-reported-the-cursor-still-is-not`.
    View,
    /// The host noticing the time: today a check-in coming due, and nothing else since
    /// Reaction's pulse was cut.
    /// Inbound, because it drives a turn exactly like an utterance does — but it
    /// came from no one, which is why it gets its own channel rather than being
    /// mixed into `text` where it would read as something the person said.
    /// Deliberately excluded from the "is anyone still here" activity scan;
    /// a heartbeat is not a conversation.
    Clock,
    /// Reports coming back from the agent's own delegated workers — cognition's
    /// answer, a worker's question, an interim finding. Inbound and turn-driving,
    /// from another of its own minds rather than from the person.
    Worker,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Text => "text",
            Channel::Vision => "vision",
            Channel::Audio => "audio",
            Channel::File => "file",
            Channel::Touch => "touch",
            Channel::Smell => "smell",
            Channel::Taste => "taste",
            Channel::View => "view",
            Channel::Clock => "clock",
            Channel::Worker => "worker",
        }
    }

    /// The channel's textual form for a prompt/journal line, suffixed with a
    /// `#stream` label when the signal came from a named stream
    /// (`audio#webcam`). The default stream (`None`) renders bare (`audio`), so
    /// single-stream output stays identical. The `#` notation lives only here.
    pub fn with_stream(self, stream: Option<&str>) -> String {
        match stream {
            Some(s) => format!("{}#{s}", self.as_str()),
            None => self.as_str().to_owned(),
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Error)]
#[error("unknown channel: {0}")]
pub struct ChannelParseError(pub String);

impl FromStr for Channel {
    type Err = ChannelParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "text" | "thought" => Ok(Channel::Text),
            "vision" => Ok(Channel::Vision),
            "audio" => Ok(Channel::Audio),
            "file" => Ok(Channel::File),
            "touch" => Ok(Channel::Touch),
            "smell" => Ok(Channel::Smell),
            "taste" => Ok(Channel::Taste),
            "view" => Ok(Channel::View),
            "clock" => Ok(Channel::Clock),
            "worker" => Ok(Channel::Worker),
            other => Err(ChannelParseError(other.to_owned())),
        }
    }
}

// -----------------------------------------------------------------------------
// Signal — one utterance on one channel
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub channel: Channel,
    pub body: String,
    /// The named stream this signal arrived on (`webcam`, `headset`), or `None`
    /// for the default stream. Carried so the reaction can tell concurrent
    /// sources of one channel apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
    pub ts: DateTime<Utc>,
}

// -----------------------------------------------------------------------------
// Origin — which mind produced a signal
// -----------------------------------------------------------------------------

/// Mechanical provenance: which *kind* of mind produced a signal — not which
/// person. Inbound human signals are `Human`, the reaction's own articulation is
/// `Reaction`, and delegated workers (once they journal) are `Worker`.
///
/// **Which person is [`Sender`], and it is decided at the boundary.** This used to
/// say the speaker's identity "stays soft, inferred from content", and that is
/// exactly the sentence that put one person's words on another person's facet: an
/// inferred name is indistinguishable from a verified one the moment it is written.
/// See [`docs/arch/signal-attribution.md`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Human,
    /// `alias` keeps journals written before the reactor-to-reaction rename loadable.
    #[serde(alias = "reactor")]
    Reaction,
    Worker,
    /// The host process itself — a deadline coming due. No mind produced it; the
    /// machinery did. Kept distinct from `Reaction` so a reader can tell what a rung
    /// emitted from what it simply received.
    Host,
}

// -----------------------------------------------------------------------------
// Sender — which person a signal came from, and how that was decided
// -----------------------------------------------------------------------------

/// How a signal's sender was arrived at. **The basis is the load-bearing half**: a
/// default that is *labelled* a default can be defeated by evidence later; a bare
/// name cannot be told apart from one that was verified, ever again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SenderBasis {
    /// The addressed-channel default — `text` and `file` are things somebody sent
    /// *to* the agent, and absent evidence otherwise that somebody is the owner.
    Owner,
    /// A face or voiceprint cluster matched. The subject may still be an opaque
    /// cluster id rather than a name; that is a person we can tell apart but cannot
    /// yet call anything.
    Cluster,
    /// The carrier said who sent it.
    Stated,
    /// Not grounded. **A complete answer, not a degraded one** — ambient capture is
    /// mostly unattributable, and an install with no declared owner has no default
    /// to fall back on either.
    Unknown,
}

/// Who an inbound signal came from. Absent entirely on machine channels (`clock`,
/// `worker`, `view`): those are not a person's *absence* but a person's
/// non-involvement, and a stretch made only of them must produce no person record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sender {
    /// The `people/` subject, without its dimension (`赵力`, or a cluster id like
    /// `7j2wa4r8`). `None` whenever [`SenderBasis::Unknown`] — someone sent this and
    /// we cannot say who.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub basis: SenderBasis,
}

impl Sender {
    /// The addressed-channel default when an owner is declared, else unattributed.
    /// The one constructor `text` and `file` ingress use, so the fallback cannot
    /// drift between them.
    pub fn owner_or_unknown(owner: Option<&str>) -> Self {
        match owner {
            Some(o) if !o.trim().is_empty() => {
                Self { subject: Some(o.trim().to_owned()), basis: SenderBasis::Owner }
            }
            _ => Self::unknown(),
        }
    }

    /// Someone sent this and we cannot say who.
    pub fn unknown() -> Self {
        Self { subject: None, basis: SenderBasis::Unknown }
    }

    /// Whether this names a person the record may act on. **Ungrounded senders are
    /// not people**: nothing may open a facet, dispatch a reader, or attach a
    /// `people/` subject on the strength of one.
    pub fn is_grounded(&self) -> bool {
        self.subject.is_some() && self.basis != SenderBasis::Unknown
    }

    /// How this reads on a frontier line. **The basis is shown, never just the
    /// name** — the settling pass has to be able to tell a default it may defeat
    /// from a recognition it should trust, and a bare name tells it neither.
    pub fn label(&self) -> String {
        match (&self.subject, self.basis) {
            (Some(s), SenderBasis::Owner) => format!("{s} (owner, by default)"),
            (Some(s), SenderBasis::Cluster) => format!("{s} (recognized)"),
            (Some(s), SenderBasis::Stated) => format!("{s} (stated)"),
            // Including `(None, _)` for any basis: a sender with no subject is
            // unattributed whatever claimed to ground it.
            _ => "unknown".to_string(),
        }
    }
}

// -----------------------------------------------------------------------------
// Media — the multimodal payload a signal carries (audio bytes, image, …)
// -----------------------------------------------------------------------------

/// A signal's media payload. The bytes live inside the signal's channel-day
/// folder on a wall-clock grid; this records the path (relative to that folder)
/// plus enough metadata that a reader needn't open the bytes to know what they
/// are. The signal's `body` stays the text surface (an STT transcript, a
/// caption).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Media {
    /// Path relative to the signal's channel-date folder, e.g. `09/16-45.mp3`
    /// (a one-off) or `output/09/11.mp3` (a streamed output minute).
    pub file: String,
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

// -----------------------------------------------------------------------------
// Message — one thing said between a person and the agent
// -----------------------------------------------------------------------------

/// One message in the conversation, in the one shape every part of the system
/// uses. **Minted once at the boundary and never rebuilt**: the ingress hands the
/// same value to the journal, to Reaction, and to the conversation, so the live
/// list and the one a restart replays cannot disagree.
///
/// Four fields, and the ones that used to be here and are not are the point.
/// `channel` moved to the journal envelope ([`JournalEntry::Message`]) because its
/// only remaining jobs were storage routing and per-sense fading; `role` collapsed
/// into [`Author`]; relevance is an [`Appraisal`] *about* a message rather than a
/// mutable field on one. See [`docs/arch/message.md`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Stable, time-sortable id (uuidv7): the cursor, the citation key, and what an
    /// [`Appraisal`] names when it says what it is about.
    pub id: String,
    pub ts: DateTime<Utc>,
    pub from: Author,
    pub content: Content,
}

/// Which end of the conversation a message came from.
///
/// **Total — every message answers it.** The pair this replaces (`role` beside an
/// optional sender) admitted `role: Agent` with a person attached, which is
/// nonsense, while collapsing the one case that *is* meaningful — a person nobody
/// placed — into the same absence the agent's own messages used.
///
/// Named `Author` rather than `From` only because `From` is in the prelude; the
/// field is `from`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Author {
    /// The agent itself. Carries no [`Sender`]: attribution answers which *person*
    /// something came from, and the agent is not one of the people it keeps.
    Agent,
    /// A person — named, or a cluster we can tell apart, or nobody we can place.
    /// `Person(Sender { subject: None, basis: Unknown })` is **a complete answer,
    /// not a degraded one** (`docs/arch/signal-attribution.md`).
    Person(Sender),
}

impl Author {
    /// The [`Sender`] behind a person's message, or `None` for the agent's own.
    pub fn sender(&self) -> Option<&Sender> {
        match self {
            Author::Agent => None,
            Author::Person(s) => Some(s),
        }
    }

    pub fn is_agent(&self) -> bool {
        matches!(self, Author::Agent)
    }
}

/// What was communicated. **One per message** — a caption and its photo are two
/// messages, ordered, which the turn queue's settle window puts in one turn.
///
/// `Text` and `Speech` are different facts, not a formatting distinction: typed
/// text is exactly what somebody wrote, a transcript is a machine's best guess at
/// what somebody said and it can be wrong. That distinction is the entire
/// remaining job of the `channel` field this replaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Content {
    /// Typed — what they wrote.
    Text(String),
    /// Recognized — the best guess at what they said. Never paired with
    /// [`Author::Agent`]: for the agent, text is the source and synthesis is a
    /// rendering of it, so its spoken audio is not a message.
    ///
    /// **The recording is part of this content, not an attachment to it.** A spoken
    /// message *is* audio and `text` is the derived view of it, so the two travel as
    /// one value — which is also what keeps a clip reachable for replay and for
    /// keepsakes when a day is faded. `None` when the words arrived without bytes
    /// kept (a live partial that settled, a transcript posted on its own).
    Speech {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        audio: Option<Media>,
    },
    /// Handed over, in either direction.
    File(FileRef),
}

impl Content {
    /// The words, for a reader that does not care how they arrived. `None` for a
    /// file, which has none — its name is [`FileRef::name`].
    pub fn text(&self) -> Option<&str> {
        match self {
            Content::Text(t) | Content::Speech { text: t, .. } => Some(t),
            Content::File(_) => None,
        }
    }

    /// The bytes behind this content, if any were kept — a spoken clip or a handed
    /// file. What [`crate::mind::memory::decay`] fades and what a replay reaches for.
    pub fn media_file(&self) -> Option<&str> {
        match self {
            Content::Speech { audio: Some(m), .. } => Some(&m.file),
            Content::File(f) => Some(&f.reff),
            _ => None,
        }
    }
}

/// A file in the conversation: where the bytes are, what they are, and what a
/// person calls it.
///
/// **The name is a field.** It used to be deliberately absent, on the ground that
/// it was already inside the message text and a second copy would let the live path
/// and the journal-seeded path disagree. Both halves of that reason are gone: a
/// file message has no prose to hide a name in, and one canonical [`Message`] is
/// what removes the disagreement. Both consumers need it — the renderer draws it
/// under the thumbnail, the prompt builder writes "they handed you passport.jpg".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRef {
    /// [`crate::mind::memory::media::signal_ref`] — channel-qualified, so it is a
    /// path rather than a path plus a guess. `GET /api/media/<ref>`.
    #[serde(rename = "ref")]
    pub reff: String,
    pub mime: String,
    pub name: String,
}

/// What crosses the boundary inward, on its way to Reaction.
///
/// Two kinds, because they are two kinds: somebody said something, or something was
/// perceived. Both reach the mind and both can drive a turn — a face appearing is
/// context for the next thing said — but only one of them is conversation, and
/// keeping them apart here is what stops a camera frame from needing a `Sender` and
/// a channel filter to be told from a sentence.
#[derive(Debug, Clone)]
pub enum Inbound {
    Message(Message),
    Observed(Signal),
}

// -----------------------------------------------------------------------------
// Appraisal — Reaction's judgement about a message
// -----------------------------------------------------------------------------

/// What Reaction concluded about a message, keyed to the message it concluded it
/// about.
///
/// **Separate from [`Message`] because a message is immutable.** Relevance is
/// computed after the message is journaled, so storing it on the message would mean
/// a field authoritative in memory and always null on disk, rebuilt at read time by
/// folding update records — the live-versus-replay divergence coming back through
/// the one mutable field. It is a judgement *about* a message, in the same category
/// as an observation, and it lives alongside rather than inside.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Appraisal {
    pub message_id: String,
    pub ts: DateTime<Utc>,
    pub relevance: f32,
}

// -----------------------------------------------------------------------------
// JournalEntry — the discriminated union written to the day-log
// -----------------------------------------------------------------------------

/// One line in a channel-day log. **Four kinds, and only one of them is
/// conversation** — the split is what stopped a view, a face and a check-in from
/// being told apart by filtering on a channel field the message no longer carries.
///
/// Lines written before this shape (`signal_in` / `signal_out`) are read through
/// [`crate::mind::memory::journal::StoredLine`] and classified on their way in.
/// Nothing is rewritten on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JournalEntry {
    /// Human ↔ agent communication. `channel` is the storage routing key — which
    /// day-log this line lives in, and which subtree [`crate::mind::memory::decay`]
    /// fades as a unit — and is deliberately *not* part of the [`Message`].
    Message { channel: Channel, message: Message },
    /// A view put up, replaced or dismissed. Replayed to restore the screen after a
    /// restart, which is why it is neither conversation nor machinery. Always on
    /// [`Channel::View`].
    Presentation { id: String, ts: DateTime<Utc>, body: String },
    /// Ambient perception — a face seen, a room gone quiet, the person walking the
    /// view band. Nobody said anything.
    Observation {
        id: String,
        ts: DateTime<Utc>,
        channel: Channel,
        body: String,
        /// Named stream this arrived on (`webcam`, `headset`), or absent for the
        /// default. Carried so concurrent sources of one channel stay apart.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stream: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media: Option<Media>,
        /// Who was perceived, when that was decided at the boundary. Absent is a
        /// real answer and is **never backfilled** from content.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sender: Option<Sender>,
    },
    /// The machinery: a check-in coming due, a worker's report, mail between rungs.
    /// Journaled because without it the log shows a turn's output with nothing that
    /// could have caused it, and a restart cannot tell the turn happened at all.
    Internal {
        id: String,
        ts: DateTime<Utc>,
        channel: Channel,
        body: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin: Option<Origin>,
    },
}

// -----------------------------------------------------------------------------
// ViewEnvelope — outbound agent-authored view module for the UI view slot
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewOp {
    /// Mount a new view under `id`.
    Show,
    /// Swap the module mounted under an existing `id` in place. Reusing the id
    /// is the continuity lever — the client keeps the slot, so a `motion`-tagged
    /// element animates rather than popping.
    Replace,
    /// Remove the view mounted under `id`.
    Dismiss,
}

/// One view event delivered to the browser over GET /api/out/view. `module_url`
/// points at the compiled ESM module (`/views/_compiled/<hash>.mjs`) the client
/// dynamically imports and mounts under `id` in the view slot. For
/// `op = dismiss` only `id` is meaningful. A view persists until the agent
/// dismisses (or replaces) it — there is no auto-expiry; lifetime is the
/// reaction's decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewEnvelope {
    pub id: String,
    pub op: ViewOp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module_url: Option<String>,
    /// The ref `show` resolved this view from (`factory/tasks`, `deck/leader`) —
    /// the view's *durable* name.
    ///
    /// `module_url` is a content hash of the source **as it was when the view was
    /// shown**, and the compiled tree is a disposable cache, so it is the wrong
    /// thing to restore a screen from: edit the source (or ship a new binary that
    /// reseeds `factory/`) and the pinned hash keeps resolving — to the old view,
    /// forever. Carrying the ref lets the restore recompile what the view *is*
    /// now. `None` for an inline `source` view, which has no durable name and so
    /// can only ever be restored as the artifact it compiled to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_ref: Option<String>,
}

#[cfg(test)]
mod sender_tests {
    use super::*;

    /// Every line already in the log predates attribution. It must still load, and it
    /// must load as *unattributed* — never as anybody. There is no backfill: who sent
    /// those signals is not recoverable, and inventing it is the whole failure.
    #[test]
    fn a_pre_attribution_journal_line_still_loads_and_names_nobody() {
        let line = r#"{"kind":"signal_in","id":"019ffabf-32e6-7c92-bbbe-9216732ef264",
            "ts":"2026-08-13T10:51:02.246936Z","channel":"text",
            "body":"show me the sessions view","origin":"human"}"#;
        let entry = crate::mind::memory::journal::classify_line(line).expect("old lines still parse");
        let JournalEntry::Message { message, .. } = entry else {
            panic!("a typed line is a message, whenever it was written");
        };
        assert!(message.from.sender().is_none_or(|s| s.subject.is_none()), "an old line names nobody");
        assert_eq!(message.content.text(), Some("show me the sessions view"));
    }

    /// The absent field must stay absent on the way back out, so a machine-channel
    /// entry and a pre-attribution one both keep reading as "no sender" rather than
    /// gaining a null that later code could misread as a person.
    #[test]
    fn no_sender_serializes_to_no_field() {
        let entry = crate::mind::memory::journal::legacy_signal_in("1".into(), Utc::now(), Channel::Clock, "check-in due".to_string(), None, None, Some(Origin::Host), None);
        let json = serde_json::to_string(&entry).unwrap();
        assert!(!json.contains("sender"), "{json}");
    }

    #[test]
    fn a_declared_owner_grounds_an_addressed_signal() {
        let s = Sender::owner_or_unknown(Some("赵力"));
        assert!(s.is_grounded());
        assert_eq!(s.basis, SenderBasis::Owner);
        assert_eq!(s.subject.as_deref(), Some("赵力"));
    }

    /// A subject with `Unknown` behind it is not a person to act on. Belt and braces:
    /// nothing constructs this today, and if something ever does it must not qualify.
    #[test]
    fn an_unknown_basis_is_never_grounded_even_with_a_subject() {
        let s = Sender { subject: Some("赵力".into()), basis: SenderBasis::Unknown };
        assert!(!s.is_grounded());
        assert_eq!(s.label(), "unknown");
    }

    #[test]
    fn a_recognized_cluster_reads_as_recognized() {
        let s = Sender { subject: Some("7j2wa4r8".into()), basis: SenderBasis::Cluster };
        assert!(s.is_grounded());
        assert_eq!(s.label(), "7j2wa4r8 (recognized)");
    }
}
