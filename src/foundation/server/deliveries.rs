//! Whether an arrival is a new one or the same act, sent again.
//!
//! A carrier that parks what it is sending on disk and retries it — the iPhone's
//! handed-drop queue, `app/apple/ios/Shared/HandedDrop.swift` — **cannot tell a
//! request that never landed from one that landed and whose answer never came
//! back**. It keeps the drop on either, because the other mistake is a file the
//! person believes they sent. So the duplicate is not a defect in the carrier; it
//! is the only safe thing it can do unaided, and the aid is a key.
//!
//! A carrier that means *this is the same act, retried* says so in
//! `Idempotency-Key`, and the same act keeps the same key across a retry, a crash
//! and a week in a tunnel. Nothing is inferred from the bytes: handing the same
//! file over twice on purpose is a thing people do, and only the carrier knows
//! which of the two happened. A request with no key is not a repeat — it is a
//! carrier that never retries, and it takes the path it always took.
//!
//! **Held in memory, so a restart forgets.** What forgetting costs is exactly the
//! duplicate this removes, and only inside the window between the core coming back
//! and the carrier's next attempt — the attempts made while it was down never
//! landed, so there is nothing about them to remember. The window that matters is
//! one app session: a foreground, a tap of "Try again", a retransmit on a
//! connection that went away.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::http::HeaderMap;

/// The header a carrier names its act in.
pub const HEADER: &str = "idempotency-key";

/// How long a landed delivery is remembered. Generous against the window it has to
/// cover (a person tapping "Try again", an app foregrounded again) because an entry
/// is a short string and a day of them is nothing.
const REMEMBER: Duration = Duration::from_secs(24 * 60 * 60);

/// Longest key taken seriously. Our own carriers send a UUID and a part name; a
/// header past this is not one of ours and is refused rather than stored.
const KEY_MAX: usize = 200;

/// The deliveries this core has taken, by the key their carrier named them with.
#[derive(Default)]
pub struct Deliveries {
    seen: Mutex<HashMap<String, Delivery>>,
}

enum Delivery {
    /// Being taken right now. Held so two attempts that overlap — a retransmit on a
    /// connection the client gave up on, while the first is still being read — do
    /// not both get through.
    InFlight,
    /// Taken, at this instant. Anything arriving under the same key is a repeat.
    Landed(Instant),
}

/// What a claim on an arrival's key came to.
pub enum Claim<'a> {
    /// Nobody has this one. The guard **holds the key only until it drops** — the
    /// arrival is remembered as landed if the caller says so with
    /// [`Claimed::landed`], and released otherwise, so a request that failed, or one
    /// whose connection died mid-body, can be sent again. A cancelled handler
    /// releases it too, which is the case a plain `insert` gets wrong.
    Take(Claimed<'a>),
    /// This act is already in the journal. Answer as the first one was answered.
    Landed,
    /// This act is being taken *right now*, by a request that has not finished.
    ///
    /// **Not answered as landed, and that is the whole reason this is a third case.**
    /// The request still reading may yet fail, and a carrier told "landed" discards
    /// its only copy — trading the duplicate this module exists to remove for a lost
    /// file, which is the worse of the two by the carrier's own reasoning. So it is
    /// refused, the carrier keeps the drop, and its next attempt finds either
    /// `Landed` or an empty slot.
    InFlight,
}

impl Deliveries {
    /// Take this arrival, or say who already has it.
    pub fn claim(&self, headers: &HeaderMap) -> Claim<'_> {
        let Some(key) = key_of(headers) else {
            // Unkeyed: nothing to remember and nothing to release.
            return Claim::Take(Claimed { deliveries: self, key: None, landed: false });
        };

        let mut seen = self.seen.lock().unwrap();
        seen.retain(|_, delivery| match delivery {
            Delivery::InFlight => true,
            Delivery::Landed(at) => at.elapsed() < REMEMBER,
        });
        match seen.get(&key) {
            Some(Delivery::Landed(_)) => Claim::Landed,
            Some(Delivery::InFlight) => Claim::InFlight,
            None => {
                seen.insert(key.clone(), Delivery::InFlight);
                Claim::Take(Claimed { deliveries: self, key: Some(key), landed: false })
            }
        }
    }
}

/// One arrival's hold on its key, for as long as taking it lasts.
pub struct Claimed<'a> {
    deliveries: &'a Deliveries,
    /// `None` for an unkeyed arrival, which this guard exists only to not get in the
    /// way of.
    key: Option<String>,
    landed: bool,
}

impl Claimed<'_> {
    /// The arrival is in the journal and in the conversation. Said before answering
    /// the carrier, because what the key promises is "sending this again changes
    /// nothing", and that is only true once it is durable.
    pub fn landed(&mut self) {
        self.landed = true;
    }
}

impl Drop for Claimed<'_> {
    fn drop(&mut self) {
        let Some(key) = self.key.take() else {
            return;
        };
        let mut seen = self.deliveries.seen.lock().unwrap();
        if self.landed {
            seen.insert(key, Delivery::Landed(Instant::now()));
        } else {
            seen.remove(&key);
        }
    }
}

fn key_of(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(HEADER)?.to_str().ok()?.trim();
    if raw.is_empty() {
        return None;
    }
    if raw.len() > KEY_MAX {
        tracing::warn!(len = raw.len(), "idempotency key too long; treating the arrival as unkeyed");
        return None;
    }
    Some(raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyed(key: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(HEADER, key.parse().unwrap());
        headers
    }

    fn take<'a>(deliveries: &'a Deliveries, headers: &HeaderMap) -> Claimed<'a> {
        match deliveries.claim(headers) {
            Claim::Take(claimed) => claimed,
            Claim::Landed => panic!("expected an untaken key, found a landed one"),
            Claim::InFlight => panic!("expected an untaken key, found one in flight"),
        }
    }

    #[test]
    fn a_landed_delivery_is_taken_once() {
        let deliveries = Deliveries::default();
        let mut first = take(&deliveries, &keyed("drop-1.0"));
        first.landed();
        drop(first);
        assert!(matches!(deliveries.claim(&keyed("drop-1.0")), Claim::Landed), "the same act again");
        assert!(matches!(deliveries.claim(&keyed("drop-1.1")), Claim::Take(_)), "a different one");
    }

    /// The whole point of releasing on drop: a send that failed must be sendable
    /// again, or the key turns a lost file into a permanently lost one.
    #[test]
    fn a_delivery_that_did_not_land_can_be_sent_again() {
        let deliveries = Deliveries::default();
        drop(take(&deliveries, &keyed("drop-2.0")));
        assert!(matches!(deliveries.claim(&keyed("drop-2.0")), Claim::Take(_)), "retry after a failure");
    }

    /// A second attempt while the first is still being read is refused, **not** told
    /// it landed: the first may still fail, and a carrier that believes it landed
    /// throws away the only copy.
    #[test]
    fn an_attempt_over_one_still_reading_is_refused_not_absolved() {
        let deliveries = Deliveries::default();
        let first = take(&deliveries, &keyed("drop-3.0"));
        assert!(matches!(deliveries.claim(&keyed("drop-3.0")), Claim::InFlight));
        // The first one fails, so the key is free and the thing can still be sent.
        drop(first);
        assert!(matches!(deliveries.claim(&keyed("drop-3.0")), Claim::Take(_)));
    }

    #[test]
    fn an_unkeyed_arrival_is_never_a_repeat() {
        let deliveries = Deliveries::default();
        let mut first = take(&deliveries, &HeaderMap::new());
        first.landed();
        drop(first);
        assert!(matches!(deliveries.claim(&HeaderMap::new()), Claim::Take(_)), "nothing was named");
    }
}
