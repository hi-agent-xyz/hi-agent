//! Outbound retained log for `/out/text`, read by cursor.
//!
//! POST `/in/text` is fire-and-forget (`202`); the agent's reply streams back
//! out on GET `/out/text`. The original design broadcast each chunk over a
//! `tokio::broadcast`, which only delivers to receivers that already exist at
//! `send()` time. So a reply produced before the first GET — or in the
//! reconnect gap between two utterances — was dropped on the floor. The field
//! symptom was "send hi, nothing responds": the reply was produced and
//! journalled, but the client's GET re-subscribed milliseconds too late.
//!
//! **This is a retained log with a cursor per reader, not a queue that empties.**
//! There is one conversation and it can be watched from several places at once —
//! the desktop window, the menu-bar popover, a browser tab. A drain-and-delete
//! bus would let them steal each other's words: whichever GET landed first would
//! take the utterance and the others would never see it. Worse, it would do so
//! silently, and an unattended reader would spend a reply on nobody. So an
//! utterance is never removed by being read; it ages out of a bounded ring.
//!
//! A reader says where it is with `after` — the id of the last utterance it
//! received in full — and gets the next one. That is a *position*, not an
//! identity: the client knows what it has seen, which is the one thing about
//! this it is authoritative about. `None` means "start at the oldest I still
//! hold", so a client that has never connected still receives a reply produced
//! before it arrived. Each GET carries one utterance and closes — the spec's
//! body-close = end-of-utterance contract — and the id it just delivered comes
//! back on [`UTTERANCE_HEADER`] for the next request's `after`.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;

use axum::body::Bytes;
use futures::stream::{Stream, unfold};
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

/// Response header naming the utterance a `/out/text` body carries, so the
/// client can pass it back as the next request's `after` cursor.
pub const UTTERANCE_HEADER: &str = "X-HI-Utterance";

/// Response/request value naming the current server process's text log.
///
/// Utterance ids intentionally restart at zero on every boot. A cursor from a
/// previous process therefore has no meaning in the new log and must reset.
pub const TEXT_EPOCH_HEADER: &str = "X-HI-Text-Epoch";

/// Cap on retained utterances. Bounds growth when the agent produces output
/// nobody ever connects to read; the oldest are evicted first. Turns are
/// serial, so reaching this many means nobody has read in a long while.
const MAX_RETAINED: usize = 32;

/// Outbound `/out/text` retained log. Cloneable handle over shared state.
#[derive(Clone)]
pub struct TextBus {
    inner: Arc<Mutex<TextOut>>,
    epoch: Arc<String>,
}

#[derive(Default)]
struct TextOut {
    log: VecDeque<Utterance>,
    /// Pulsed whenever `log` changes (new chunk, new utterance, completion)
    /// so a parked reader re-checks.
    notify: Arc<Notify>,
    /// Monotonic utterance id.
    next_id: u64,
}

struct Utterance {
    id: u64,
    chunks: Vec<String>,
    complete: bool,
}

impl TextBus {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TextOut::default())),
            epoch: Arc::new(Uuid::now_v7().to_string()),
        }
    }

    pub fn epoch(&self) -> &str {
        self.epoch.as_str()
    }

    /// Discard a cursor unless it belongs to this process's retained log.
    pub fn normalize_after(&self, epoch: Option<&str>, after: Option<u64>) -> Option<u64> {
        (epoch == Some(self.epoch())).then_some(after).flatten()
    }

    /// Append a chunk of agent text. Starts a new utterance when the previous one
    /// has completed (or none exists). Empty chunks are dropped so they neither
    /// open an utterance nor emit empty body frames.
    pub async fn push_chunk(&self, text: String) {
        if text.is_empty() {
            return;
        }
        let mut out = self.inner.lock().await;

        let need_new = match out.log.back() {
            Some(u) => u.complete,
            None => true,
        };
        if need_new {
            while out.log.len() >= MAX_RETAINED {
                out.log.pop_front();
            }
            let id = out.next_id;
            out.next_id += 1;
            out.log.push_back(Utterance {
                id,
                chunks: Vec::new(),
                complete: false,
            });
        }
        if let Some(u) = out.log.back_mut() {
            u.chunks.push(text);
        }
        out.notify.notify_waiters();
    }

    /// Mark the open utterance complete. Readers streaming it close their HTTP
    /// bodies once they have drained the buffered chunks.
    pub async fn end_utterance(&self) {
        let mut out = self.inner.lock().await;
        if let Some(u) = out.log.back_mut() {
            u.complete = true;
        }
        out.notify.notify_waiters();
    }

    /// The id of the next utterance a reader positioned at `after` would receive,
    /// once one exists. Used by the handler to set [`UTTERANCE_HEADER`] before it
    /// starts streaming — the header has to be on the response, which is written
    /// before the body.
    ///
    /// Waits until such an utterance exists rather than returning `None`, so a
    /// long-poll opened into silence parks here instead of closing empty.
    pub async fn next_id_after(&self, after: Option<u64>) -> u64 {
        loop {
            let out = self.inner.lock().await;
            if let Some(u) = first_after(&out.log, after) {
                return u.id;
            }
            let notify = out.notify.clone();
            let notified = notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            drop(out);
            notified.await;
        }
    }

    /// A stream yielding the bytes of exactly one utterance — the first one after
    /// `after` — closing when that utterance ends. Reading does not consume it:
    /// every other reader still sees it, and it leaves only by ageing out of the
    /// ring.
    pub fn subscribe(&self, after: Option<u64>) -> impl Stream<Item = Result<Bytes, Infallible>> + use<> {
        struct Reader {
            inner: Arc<Mutex<TextOut>>,
            after: Option<u64>,
            bound: Option<u64>,
            cursor: usize,
        }

        let state = Reader {
            inner: self.inner.clone(),
            after,
            bound: None,
            cursor: 0,
        };

        unfold(state, |mut s| async move {
            let inner = s.inner.clone();
            loop {
                let out = inner.lock().await;

                if s.bound.is_none()
                    && let Some(u) = first_after(&out.log, s.after)
                {
                    s.bound = Some(u.id);
                    s.cursor = 0;
                }

                if let Some(id) = s.bound {
                    match out.log.iter().find(|u| u.id == id) {
                        Some(u) if s.cursor < u.chunks.len() => {
                            let chunk = u.chunks[s.cursor].clone();
                            s.cursor += 1;
                            drop(out);
                            return Some((Ok(Bytes::from(chunk)), s));
                        }
                        // Drained and done: close the body. The utterance stays in
                        // the log for every other reader.
                        Some(u) if u.complete => return None,
                        Some(_) => {}        // open, awaiting more chunks
                        None => return None, // aged out from under us
                    }
                }

                // Nothing to yield yet. Enroll on the notify *while still holding
                // the lock* so a `notify_waiters()` between here and the await
                // cannot be lost, then release the lock and park.
                let notify = out.notify.clone();
                let notified = notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                drop(out);
                notified.await;
            }
        })
    }
}

/// The oldest retained utterance strictly after `after`, or the oldest retained
/// one when `after` is `None`.
fn first_after(log: &VecDeque<Utterance>, after: Option<u64>) -> Option<&Utterance> {
    log.iter().find(|u| match after {
        Some(a) => u.id > a,
        None => true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    async fn collect_one(bus: &TextBus, after: Option<u64>) -> String {
        let mut s = Box::pin(bus.subscribe(after));
        let mut got = String::new();
        while let Some(Ok(b)) = s.next().await {
            got.push_str(std::str::from_utf8(&b).unwrap());
        }
        got
    }

    /// The bug this shape exists to prevent. Two readers on one conversation —
    /// a desktop window and a popover — must both get the reply. A queue that
    /// deleted on read would give it to whichever asked first and leave the
    /// other with nothing.
    #[tokio::test]
    async fn every_reader_sees_every_utterance() {
        let bus = TextBus::new();
        bus.push_chunk("在".into()).await;
        bus.end_utterance().await;

        assert_eq!(collect_one(&bus, None).await, "在");
        assert_eq!(collect_one(&bus, None).await, "在", "reading did not consume it");
    }

    /// A reader that says where it is gets the *next* utterance, not the one it
    /// already has — which is what stops a re-subscribe from looping on one line.
    #[tokio::test]
    async fn the_cursor_advances_past_what_was_delivered() {
        let bus = TextBus::new();
        bus.push_chunk("first".into()).await;
        bus.end_utterance().await;
        bus.push_chunk("second".into()).await;
        bus.end_utterance().await;

        assert_eq!(bus.next_id_after(None).await, 0);
        assert_eq!(collect_one(&bus, None).await, "first");
        assert_eq!(bus.next_id_after(Some(0)).await, 1);
        assert_eq!(collect_one(&bus, Some(0)).await, "second");
    }

    /// The original bug: a reply produced before anyone connected must still be
    /// there when they do.
    #[tokio::test]
    async fn a_reply_produced_into_an_empty_room_survives() {
        let bus = TextBus::new();
        bus.push_chunk("周报发出去了".into()).await;
        bus.end_utterance().await;
        assert_eq!(collect_one(&bus, None).await, "周报发出去了");
    }

    /// Chunks of one utterance stream in order and the body closes at its end.
    #[tokio::test]
    async fn one_get_carries_exactly_one_utterance() {
        let bus = TextBus::new();
        bus.push_chunk("a".into()).await;
        bus.push_chunk("b".into()).await;
        bus.end_utterance().await;
        bus.push_chunk("next".into()).await;
        bus.end_utterance().await;

        assert_eq!(collect_one(&bus, None).await, "ab");
    }

    /// Unbounded retention would be a leak when nobody ever reads.
    #[tokio::test]
    async fn the_log_is_bounded() {
        let bus = TextBus::new();
        for i in 0..(MAX_RETAINED + 5) {
            bus.push_chunk(format!("u{i}")).await;
            bus.end_utterance().await;
        }
        let out = bus.inner.lock().await;
        assert_eq!(out.log.len(), MAX_RETAINED);
    }

    #[test]
    fn a_cursor_from_another_process_resets() {
        let bus = TextBus::new();
        assert_eq!(bus.normalize_after(Some("old-process"), Some(999)), None);
        assert_eq!(bus.normalize_after(None, Some(999)), None);
        assert_eq!(bus.normalize_after(Some(bus.epoch()), Some(7)), Some(7));
    }

    #[test]
    fn each_process_gets_a_distinct_text_epoch() {
        let first = TextBus::new();
        let second = TextBus::new();
        assert_ne!(first.epoch(), second.epoch());
    }
}
