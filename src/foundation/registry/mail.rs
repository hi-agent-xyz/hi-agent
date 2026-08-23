//! What one session said to another — kept long enough to be read back.
//!
//! [`Registry::send`](super::Registry::send) is the one agent-to-agent verb, and until now
//! it was **write-only from the outside**: the text went into the recipient's mailbox, was
//! taken out by [`take_pending`](super::Registry::take_pending), rendered into that
//! session's next prompt, and after that the only trace of it was buried in two different
//! frame logs — as an `hi_send_message` tool call in the sender's, and as a paragraph of
//! the prompt in the recipient's. So "what have these two been saying to each other" was a
//! question you answered by reading two transcripts side by side and matching text by eye.
//!
//! This is the small ring that answers it directly. Every delivered `send` is appended
//! here, and [`Registry::traffic_between`](super::Registry::traffic_between) reads back one
//! pair's half of it, both directions, oldest first — which is what the arrow between two
//! cards on the sessions page opens.
//!
//! **In memory, capped, and not the record.** Same split the roster already makes against
//! the frame log ([`super::Registry::recent`]): the durable copy is the wire log, which is
//! verbatim and unpruned, and this is the working set a page poll can afford to read. A
//! restart empties it.
//!
//! It used to empty the roster it draws arrows between at the same instant, so there was no
//! arrow left pointing at a history this could have kept. **That is no longer true.** An
//! errand a stop interrupted is reopened under its own slug and its own owner
//! ([`crate::body::reaction::reopen_interrupted`]), so the arrow between those two cards is
//! back on the page with nothing behind it, and clicking it opens an empty exchange for a
//! pair that has been talking for an hour. What it costs is a reader's confidence in the
//! page, not a message — both sides still hold the exchange in their own threads, and the
//! wire log has every frame of it. Restoring the ring from the frame logs at boot is the fix
//! and is not built.
//!
//! Nothing refused is recorded. `NotPermitted` and `Unknown` never reached a mailbox, so
//! they are not communication between the two sessions; they are a sender's mistake, and
//! the place that shows them is the sender's own transcript, where the tool call and its
//! error sit together.

use chrono::{DateTime, Utc};

use super::SessionSlug;

/// How many delivered messages the ring keeps, process-wide.
///
/// Sized for reading, not for retention: a pair's conversation over a working afternoon is
/// tens of messages, and what a reader wants from the arrow is the recent stretch of it.
pub const KEPT: usize = 300;

/// The longest single message the ring stores, in characters.
///
/// A worker's report can be a page. Storing the whole of every one of them makes the
/// working set unbounded in the one dimension the count above does not cover, so a long
/// message is clipped here and says so. The whole of it is in the recipient's frame log,
/// in the prompt it became.
pub const TEXT_CHARS: usize = 4_000;

/// One delivered message, as the ring keeps it.
#[derive(Debug, Clone)]
pub struct Sent {
    pub at: DateTime<Utc>,
    pub from: SessionSlug,
    pub to: SessionSlug,
    pub text: String,
    /// Whether `text` is the whole of what was sent, or the first [`TEXT_CHARS`] of it.
    pub clipped: bool,
}

impl Sent {
    pub fn new(from: SessionSlug, to: SessionSlug, text: &str) -> Self {
        let kept: String = text.chars().take(TEXT_CHARS).collect();
        let clipped = kept.len() < text.len();
        Self { at: Utc::now(), from, to, text: kept, clipped }
    }

    /// Whether this message is one of the two directions between `a` and `b`.
    ///
    /// Unordered on purpose: an arrow joins two sessions, and a conversation read from one
    /// end only is not a conversation. `a == b` matches nothing — a session cannot
    /// `send` to itself through the switchboard, and answering as though it could would
    /// draw its own mail as a dialogue.
    pub fn between(&self, a: &SessionSlug, b: &SessionSlug) -> bool {
        a != b && ((&self.from == a && &self.to == b) || (&self.from == b && &self.to == a))
    }
}

/// Append `sent`, dropping the oldest once the ring is full.
pub fn push(ring: &mut std::collections::VecDeque<Sent>, sent: Sent) {
    if ring.len() == KEPT {
        ring.pop_front();
    }
    ring.push_back(sent);
}
