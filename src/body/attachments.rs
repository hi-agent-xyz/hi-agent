//! What is attached — which out-channels currently have a live subscriber, and
//! whether a mic is open.
//!
//! **This is what is left of presence, and the shrinkage is the design.** There used
//! to be three axes here — reach, a decaying *expectation* (eager / around / away),
//! and a voice posture — derived from open channels, window activations, and how
//! recently the person had engaged. Reaction was told all of it every turn and was
//! expected to decide whether to speak into a room that might be empty.
//!
//! It was removed because the derivation could not work. An open channel answers
//! *is a window subscribed*, which was never the same question as *are you reading*:
//! a window behind an editor, a tab left open on another desk, and a person leaning
//! in are the same subscription, and no amount of decay separates them. Everything
//! downstream inherited that error in both directions at once — the agent went quiet
//! on someone sitting right there, and talked to an empty desk, from the same signal.
//!
//! What made it load-bearing was that words did not keep: text was one current
//! appearance slot, so speaking into an empty room threw the words away and
//! withholding them was the lesser loss. Text is now an append-only
//! [conversation](crate::foundation::server::transcript), so a message said to
//! nobody is a message waiting. There is nothing left to protect, so there is
//! nothing left to detect.
//!
//! **Two consumers survive, and neither is a judgment.** Both are facts about the wire
//! read by the host at one instant, and nothing above the host ever sees either.
//!
//! - A TTS span is synthesized to frames that go out on the wire as they are made; with
//!   no speaker attached they are spent and the person never learns it happened. So
//!   `open_tts` asks whether a speaker is attached, at the instant it would open the span.
//! - A show asks whether **every window let the conversation go** for a while since the
//!   view in front of them went up ([`back_from_away`](Attachments::back_from_away)). The
//!   face holds `/api/out/text` only while its window is attended — hidden, minimized and
//!   (on macOS) fully covered all drop it — so a stretch with no text subscriber anywhere
//!   is a stretch nobody was reading. The error this inherits runs one way only: a window
//!   left open on an unwatched desk reads as someone still there, and the cost of that is
//!   a new view waiting in their list rather than taking the screen
//!   (`docs/arch/stage.md` § *A show leaves a page being read alone*).
//!
//! **Counts, not identities.** Several surfaces can watch at once — a window, a
//! popover, a phone — and the only question anyone asks is "is there a speaker
//! anywhere", never "which client". So this counts live connections per channel and
//! nothing about who holds them.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};

/// One output channel a client can subscribe to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum OutChannel {
    Text,
    Audio,
    View,
}

/// How many stretches with no reader are remembered. The one question asked of them
/// looks back a few minutes, so this only has to outlast a burst of reconnects.
const GAPS_KEPT: usize = 32;

/// Stretches in which no window held the conversation open.
#[derive(Debug)]
struct Away {
    /// When the current stretch began, while there is no text subscriber at all.
    /// A process starts with nobody attached, so it starts away.
    since: Option<DateTime<Utc>>,
    /// Finished stretches, oldest first, `(began, ended)`.
    gaps: VecDeque<(DateTime<Utc>, DateTime<Utc>)>,
}

impl Default for Away {
    fn default() -> Self {
        Self { since: Some(Utc::now()), gaps: VecDeque::new() }
    }
}

/// Shared attachment counts. Cloneable handle; counts move with guard lifetimes,
/// so a dropped connection un-counts itself.
#[derive(Clone, Default)]
pub struct Attachments {
    channels: Arc<Mutex<HashMap<OutChannel, usize>>>,
    mic: Arc<Mutex<usize>>,
    away: Arc<Mutex<Away>>,
}

impl Attachments {
    pub fn new() -> Self {
        Self::default()
    }

    /// Count one live out-channel subscriber until the returned guard drops.
    pub fn connect(&self, channel: OutChannel) -> Guard {
        let mut map = self.channels.lock().expect("attachments mutex poisoned");
        let n = map.entry(channel).or_insert(0);
        *n += 1;
        if channel == OutChannel::Text && *n == 1 {
            self.away.lock().expect("away mutex poisoned").returned(Utc::now());
        }
        Guard { attachments: self.clone(), key: channel }
    }

    /// Count one live mic-in stream until the returned guard drops.
    pub fn connect_mic(&self) -> MicGuard {
        *self.mic.lock().expect("mic mutex poisoned") += 1;
        MicGuard { mic: self.mic.clone() }
    }

    /// Is there a speaker anywhere the agent can be heard on?
    ///
    /// Read at the moment a TTS span would open — not once per process and not off
    /// a snapshot taken when a turn began, because a turn can outlive the window
    /// that started it, and someone unplugging headphones mid-conversation should
    /// stop being spoken to on the next `say` with no state to reconcile.
    pub fn speaker_attached(&self) -> bool {
        self.live(OutChannel::Audio)
    }

    /// Is a mic stream open? Used by the audio carrier itself, never to infer
    /// anything about the person.
    pub fn mic_attached(&self) -> bool {
        *self.mic.lock().expect("mic mutex poisoned") > 0
    }

    /// The last moment every window had let the conversation go for at least `min` —
    /// now, if that is true at this moment — or `None` if it has not happened since
    /// the process started.
    ///
    /// A stretch shorter than `min` is not an absence: a reconnect, a glance at another
    /// app, a window briefly covered are all someone who never stopped reading.
    pub fn back_from_away(&self, min: Duration) -> Option<DateTime<Utc>> {
        self.away.lock().expect("away mutex poisoned").back_from(min, Utc::now())
    }

    fn live(&self, channel: OutChannel) -> bool {
        self.channels
            .lock()
            .expect("attachments mutex poisoned")
            .get(&channel)
            .copied()
            .unwrap_or(0)
            > 0
    }
}

impl Away {
    fn left(&mut self, at: DateTime<Utc>) {
        self.since.get_or_insert(at);
    }

    fn returned(&mut self, at: DateTime<Utc>) {
        if let Some(began) = self.since.take() {
            self.gaps.push_back((began, at));
            while self.gaps.len() > GAPS_KEPT {
                self.gaps.pop_front();
            }
        }
    }

    fn back_from(&self, min: Duration, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if self.since.is_some_and(|began| now - began >= min) {
            return Some(now);
        }
        self.gaps.iter().rev().find(|(began, ended)| *ended - *began >= min).map(|(_, ended)| *ended)
    }
}

/// Un-counts its out-channel subscriber on drop.
pub struct Guard {
    attachments: Attachments,
    key: OutChannel,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let mut map = self.attachments.channels.lock().expect("attachments mutex poisoned");
        if let Some(n) = map.get_mut(&self.key) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                map.remove(&self.key);
                if self.key == OutChannel::Text {
                    self.attachments.away.lock().expect("away mutex poisoned").left(Utc::now());
                }
            }
        }
    }
}

/// Un-counts its mic-in stream on drop.
pub struct MicGuard {
    mic: Arc<Mutex<usize>>,
}

impl Drop for MicGuard {
    fn drop(&mut self) {
        let mut n = self.mic.lock().expect("mic mutex poisoned");
        *n = n.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_follow_guard_lifetimes() {
        let a = Attachments::new();
        assert!(!a.speaker_attached());
        let g1 = a.connect(OutChannel::Audio);
        let g2 = a.connect(OutChannel::Audio);
        assert!(a.speaker_attached());
        drop(g1);
        assert!(a.speaker_attached(), "one surface left is still a speaker");
        drop(g2);
        assert!(!a.speaker_attached());
    }

    #[test]
    fn the_mic_has_its_own_count() {
        let a = Attachments::new();
        assert!(!a.mic_attached());
        let g = a.connect_mic();
        assert!(a.mic_attached());
        drop(g);
        assert!(!a.mic_attached());
    }

    /// A window open with no audio output is not a speaker. This is the one
    /// distinction the module exists to make, and conflating the channels here
    /// would silently resurrect the gate's central mistake in miniature.
    #[test]
    fn a_window_without_audio_is_not_a_speaker() {
        let a = Attachments::new();
        let _text = a.connect(OutChannel::Text);
        let _view = a.connect(OutChannel::View);
        assert!(!a.speaker_attached());
    }

    fn at(minute: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_800_000_000, 0).unwrap() + Duration::minutes(minute)
    }

    /// Only a stretch with nobody attached for at least the asked length counts, and
    /// the answer is when the latest such stretch ended.
    #[test]
    fn a_short_gap_is_not_an_absence_and_a_long_one_is_dated_by_its_end() {
        let mut away = Away { since: None, gaps: VecDeque::new() };
        away.left(at(0));
        away.returned(at(10));
        away.left(at(20));
        away.returned(at(20) + Duration::seconds(5));
        assert_eq!(away.back_from(Duration::minutes(2), at(30)), Some(at(10)));
        assert_eq!(away.back_from(Duration::minutes(15), at(30)), None);
    }

    /// Away right now, long enough, is away *now* — the answer a show compares with the
    /// moment the view in front of them went up.
    #[test]
    fn nobody_attached_long_enough_is_away_now() {
        let mut away = Away { since: None, gaps: VecDeque::new() };
        away.left(at(0));
        assert_eq!(away.back_from(Duration::minutes(2), at(1)), None);
        assert_eq!(away.back_from(Duration::minutes(2), at(5)), Some(at(5)));
    }

    /// Two windows are one reader: closing one of them is not leaving.
    #[test]
    fn one_window_closing_while_another_holds_is_not_leaving() {
        let a = Attachments::new();
        let first = a.connect(OutChannel::Text);
        let second = a.connect(OutChannel::Text);
        drop(first);
        assert!(a.away.lock().unwrap().since.is_none());
        drop(second);
        assert!(a.away.lock().unwrap().since.is_some());
    }
}
