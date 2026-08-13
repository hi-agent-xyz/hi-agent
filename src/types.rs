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
    /// The agent's own presentation surface — what it put on the screen. Outbound
    /// only, and like [`Channel::File`] not a sense: the agent doesn't perceive a
    /// view, it *shows* one. Recorded so a restart can tell what is already up
    /// (and so it doesn't show the same thing twice).
    View,
    /// The host noticing the time: today a check-in coming due, and nothing else since
    /// the voice's pulse was cut.
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

/// Mechanical provenance: which mind produced a signal. NOT the speaker's
/// identity (that stays soft, inferred from content). Inbound human signals are
/// `Human`, the reaction's own articulation is `Reaction`, and delegated workers
/// (once they journal) are `Worker`.
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
// Media — the multimodal payload a signal carries (audio bytes, image, …)
// -----------------------------------------------------------------------------

/// A signal's media payload. The bytes live inside the signal's channel-day
/// folder on a wall-clock grid; this records the path (relative to that folder)
/// plus enough metadata that a reader needn't open the bytes to know what they
/// are. The signal's `body` stays the text surface (an STT transcript, a
/// caption).
#[derive(Debug, Clone, Serialize, Deserialize)]
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
// JournalEntry — the discriminated union written to the day-log
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JournalEntry {
    SignalIn {
        /// Stable, time-sortable id (uuidv7): the cursor + citation key, and the
        /// stem of any co-located media blob (`audio-<id>.mp3`).
        id: String,
        ts: DateTime<Utc>,
        channel: Channel,
        body: String,
        /// Named stream this signal arrived on, or absent for the default stream.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stream: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media: Option<Media>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin: Option<Origin>,
    },
    SignalOut {
        id: String,
        ts: DateTime<Utc>,
        channel: Channel,
        body: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media: Option<Media>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin: Option<Origin>,
    },
}

// -----------------------------------------------------------------------------
// ViewEnvelope — outbound agent-authored view module for the UI view slot
// -----------------------------------------------------------------------------

/// A view's one self-declared trait: whether it renders the conversation itself,
/// so the host's own conversation surface stands down.
///
/// This is all that survives of the old `Geometry` (`region` × `size`). Views are
/// full-bleed, one at a time, so there is no placement left to declare, and so
/// nothing for a view to get wrong by omission. The whole appearance history bore
/// that out: across every snapshot ever written, the nine non-`fill` regions were
/// never once used to compose two views deliberately — every multi-view state was
/// either stale accumulation or the host's condition layer over content. Placement
/// bought nothing and cost the bordered-card default that swallowed any view whose
/// builder forgot a sidecar.
///
/// Kept as a struct (not a bare `bool`) because it is what a `.geom.json` sidecar
/// deserializes into, and because the safe default is `false` — a view that forgets
/// to declare it simply gets the host's own conversation surface, which is the right
/// fallback rather than a silently mis-framed view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ViewTraits {
    /// This view renders the conversation itself, so the host draws neither the
    /// rail nor the collapsed pill (`docs/arch/stage.md`). Only declare it if the
    /// view actually renders the words — otherwise the person's speech goes
    /// invisible.
    ///
    /// The alias is the back-compat lever. It was `owns_captions` while the host's
    /// only rendering of the words was a caption band, and that name is already
    /// written into `.geom.json` sidecars on disk and into appearance snapshots.
    /// Both keep loading, and re-serialize under the name the rest of the system
    /// now uses.
    #[serde(default, alias = "owns_captions")]
    pub owns_conversation: bool,
}

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
    /// What the view declared about itself. Absent on `dismiss`; absent from
    /// snapshots written before this field existed, which reload as the default
    /// (host-owned captions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traits: Option<ViewTraits>,
    /// The ref `show` resolved this view from (`_builtin/tasks`, `deck/leader`) —
    /// the view's *durable* name.
    ///
    /// `module_url` is a content hash of the source **as it was when the view was
    /// shown**, and the compiled tree is a disposable cache, so it is the wrong
    /// thing to restore a screen from: edit the source (or ship a new binary that
    /// reseeds `_builtin/`) and the pinned hash keeps resolving — to the old view,
    /// forever. Carrying the ref lets the restore recompile what the view *is*
    /// now. `None` for an inline `source` view, which has no durable name and so
    /// can only ever be restored as the artifact it compiled to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_ref: Option<String>,
}
