//! Version numbers for the stores a review surface reads, so a read can **park until its
//! own store moves** instead of asking again on a clock.
//!
//! Every other live thing in this server is already an event. The appearance is a versioned
//! whole-state long-poll ([`view_bus::ViewBus::wait_state`](super::view_bus::ViewBus::wait_state));
//! agent activity is a `watch` behind SSE ([`activity`](super::activity)); a view rewritten on
//! disk reaches the screen through a filesystem watcher ([`view_watch`](super::view_watch)),
//! whose own note states the rule this module applies to the ledger: *it is an event, not a
//! tick — the watcher costs nothing until a file is written.*
//!
//! The review surfaces were the exception. Ten of them re-read their whole store every few
//! seconds for as long as somebody had the page open, which costs the same whether anything
//! changed or not, and is *still* up to one period stale at the moment it matters. A version
//! to park on is better on both counts at once: nothing on the wire while nothing happens,
//! and the change arrives when it happens rather than by the next tick.
//!
//! **Nothing here knows what a store contains.** It counts, and it wakes people who are
//! waiting; what a version means is the handler's business, and what fills it in is whatever
//! watches that store — see [`facet_watch`](super::facet_watch).

use std::collections::HashMap;
use std::time::Duration;

use tokio::sync::{Mutex, Notify};

/// How long a parked read waits before answering "nothing yet, ask again".
///
/// **A long-poll that only ever answers is a long-poll a proxy kills.** Off-box these run
/// through something that gives up on a request producing no bytes — the observed failures
/// were 524s at around thirty seconds — and a request killed at the edge tells the client
/// nothing about whether it was ever registered. So this answers first, well under any such
/// limit, with the one fact the client cannot infer: that it is still current.
pub const WAIT: Duration = Duration::from_secs(25);

/// One counter per store, and one place to park on any of them.
///
/// A single [`Notify`] for every store rather than one each: a woken reader re-checks its own
/// version under the lock and parks again if it was somebody else's store that moved, which
/// costs a lock and a compare. Splitting them would buy that back at the price of a registry
/// of channels to keep in step with the stores that exist.
#[derive(Default)]
pub struct StoreVersions {
    inner: Mutex<HashMap<String, u64>>,
    notify: Notify,
}

impl StoreVersions {
    /// Where `store` stands now. `0` for a store nothing has ever written.
    pub async fn version(&self, store: &str) -> u64 {
        self.inner.lock().await.get(store).copied().unwrap_or(0)
    }

    /// Say that `store` changed, and wake everyone parked on anything.
    pub async fn bump(&self, store: &str) {
        {
            let mut map = self.inner.lock().await;
            *map.entry(store.to_owned()).or_insert(0) += 1;
        }
        self.notify.notify_waiters();
    }

    /// `store`'s version as soon as it stops agreeing with `since`, or `None` if it still
    /// agrees [`WAIT`] later.
    ///
    /// **Disagreement, not advance.** A client that comes back holding a version *higher*
    /// than ours has outlived a restart — the counters start at zero again — and waiting for
    /// a counter to climb past it would park that client forever while the store changed
    /// under it. Any disagreement means the same thing here: what you have is not what we
    /// have, come and read.
    pub async fn wait(&self, store: &str, since: u64, within: Duration) -> Option<u64> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let map = self.inner.lock().await;
            let current = map.get(store).copied().unwrap_or(0);
            if current != since {
                return Some(current);
            }
            // Enrol on the notify *while still holding the lock*, so a bump landing between
            // the check above and the park below cannot be missed. Same order as `ViewBus`.
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            drop(map);
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_reader_parks_until_its_own_store_moves() {
        let versions = std::sync::Arc::new(StoreVersions::default());
        assert_eq!(versions.version("tasks").await, 0);

        // Somebody else's store does not wake this one through.
        let parked = tokio::spawn({
            let versions = versions.clone();
            async move { versions.wait("tasks", 0, Duration::from_secs(5)).await }
        });
        tokio::task::yield_now().await;
        versions.bump("skills").await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!parked.is_finished(), "a skills write is not a tasks change");

        versions.bump("tasks").await;
        assert_eq!(parked.await.unwrap(), Some(1));
    }

    #[tokio::test]
    async fn a_store_that_did_not_move_answers_rather_than_hanging() {
        let versions = StoreVersions::default();
        versions.bump("tasks").await;
        let answer = versions.wait("tasks", 1, Duration::from_millis(80)).await;
        assert_eq!(answer, None, "still current, and said so");
    }

    /// The version a client kept across a restart is higher than the one the process starts
    /// with. Waiting for the counter to climb past it would park that client for good.
    #[tokio::test]
    async fn a_client_from_before_a_restart_is_told_to_re_read() {
        let versions = StoreVersions::default();
        let answer = versions.wait("tasks", 41, Duration::from_millis(80)).await;
        assert_eq!(answer, Some(0), "disagreement is the signal, not advance");
    }
}
