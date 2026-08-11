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
    /// One thing the agent said — a whole `say` call, which is a whole message.
    ///
    /// `id` and `ts` are the ones it was journaled under, carried here so the
    /// message in the conversation and the entry in the log share a key. Nothing
    /// downstream mints its own.
    Text {
        id: String,
        ts: chrono::DateTime<chrono::Utc>,
        text: String,
    },
    /// The end of an utterance, for the observability tap only. The conversation
    /// needs no such boundary: a message is complete when it is appended.
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
