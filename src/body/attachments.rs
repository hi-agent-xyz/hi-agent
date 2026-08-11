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
//! **One consumer survives, and it is not a judgment.** A TTS span is synthesized to
//! frames that go out on the wire as they are made; with no speaker attached they
//! are spent and the person never learns it happened. So `open_tts` asks whether a
//! speaker is attached, at the instant it would open the span. That is a fact about
//! the wire, not a read of the room, and nothing above the host ever sees it.
//!
//! **Counts, not identities.** Several surfaces can watch at once — a window, a
//! popover, a phone — and the only question anyone asks is "is there a speaker
//! anywhere", never "which client". So this counts live connections per channel and
//! nothing about who holds them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// One output channel a client can subscribe to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum OutChannel {
    Text,
    Audio,
    View,
}

/// Shared attachment counts. Cloneable handle; counts move with guard lifetimes,
/// so a dropped connection un-counts itself.
#[derive(Clone, Default)]
pub struct Attachments {
    channels: Arc<Mutex<HashMap<OutChannel, usize>>>,
    mic: Arc<Mutex<usize>>,
}

impl Attachments {
    pub fn new() -> Self {
        Self::default()
    }

    /// Count one live out-channel subscriber until the returned guard drops.
    pub fn connect(&self, channel: OutChannel) -> Guard {
        *self.channels.lock().expect("attachments mutex poisoned").entry(channel).or_insert(0) += 1;
        Guard { channels: self.channels.clone(), key: channel }
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

/// Un-counts its out-channel subscriber on drop.
pub struct Guard {
    channels: Arc<Mutex<HashMap<OutChannel, usize>>>,
    key: OutChannel,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let mut map = self.channels.lock().expect("attachments mutex poisoned");
        if let Some(n) = map.get_mut(&self.key) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                map.remove(&self.key);
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
}
