//! The reaction's outbound vocabulary — continuous channel signals, transport-free.
//!
//! The reaction is the mind; it must stay aligned to the human-channel model and
//! know nothing about whichever wire happens to carry it. So instead of building
//! HTTP-shaped events, it emits [`OutboundSignal`]s: "said this text", "this span
//! of speech", "show this view". A transport adapter (today the HTTP server)
//! binds these to a wire. Swap HTTP for WebSocket and only the adapter changes;
//! this vocabulary and the reaction are untouched.
//!
//! What is deliberately *absent* here is the tell: no `mime`/`Content-Type`, no
//! HTTP response framing, no body-close semantics. The one integer that remains,
//! `turn`, is the reaction's own cognition-turn id (it already tags journal and
//! logs); the adapter reuses it to keep one utterance's audio frames bound to one
//! response, but the reaction does not reason about responses.

use bytes::Bytes;

use crate::types::ViewEnvelope;

/// One continuous outbound signal on a channel. The
/// reaction's entire output surface in human-channel terms.
#[derive(Debug, Clone)]
pub enum OutboundSignal {
    /// A reaction turn has started. The text appearance uses this internal
    /// boundary to reject stale output if newer human input lands before the
    /// turn speaks. It carries no wire identity and changes no visible state.
    TextTurnStart { turn: u64 },
    /// A chunk of agent text on the /thought channel.
    Text { chunk: String },
    /// The boundary that settles the currently open /thought utterance.
    TextEnd,
    /// A span of synthesized speech begins; `codec` names the audio format
    /// (e.g. `audio/mpeg`). `turn` correlates this span's frames so the adapter
    /// can hold one response open for exactly one utterance.
    AudioBegin { turn: u64, codec: String },
    /// One frame of synthesized speech within the open span.
    AudioFrame { turn: u64, bytes: Bytes },
    /// The span of speech ends (synthesis finished, or the turn was cut short).
    AudioEnd { turn: u64 },
    /// An agent-authored view module to mount on the /view channel. `envelope`
    /// carries the compiled module URL; the binder broadcasts it to GET
    /// /api/out/view subscribers.
    View { envelope: ViewEnvelope },
}
