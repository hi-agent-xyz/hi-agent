//! Presence — how present the user is *to hi-agent*, built only from what the app
//! observes about its own surface and its own conversation. No system-level
//! probing (idle time, screen lock, other apps): those are privacy-ugly and, more
//! to the point, they'd measure "away from the keyboard" when what we care about
//! is "away from hi-agent" — someone typing in another app for an hour is, to us,
//! away.
//!
//! Presence is not one number and not a mode ladder; it's a point in three
//! **orthogonal** axes that combine freely (so the cases intersect — "eager but
//! screen-dark" is as real as "eager and voice-open"):
//!
//!   1. **Reach** (instantaneous) — which of the user's channels a message can
//!      land on right now: a *screen* (View out-channel), the *speaker* (Audio
//!      out-channel), and the *mic* (a live audio-in stream). Exactly observed via
//!      connection guards.
//!   2. **Expectation** (a decaying belief) — how much output the user is
//!      *awaiting*: rising when they repeatedly bring the window forward or when
//!      hi-agent owes a reply and time is passing; decaying, in the absence of any
//!      engagement, back toward *away*. Independent of which channels are open —
//!      someone can eagerly await a view with mic and speaker both off.
//!   3. **Posture** — whether a *voice conversation* is active (the mic is live),
//!      which is what licenses speaking aloud when no window is up.
//!
//! Everything here is **facts only** — a snapshot the mind reads. What to do about
//! it (report more, hold, work ahead) is the next layer's job, not this one's.
//!
//! With one exception, and it is an *event*, not a fact: **coming back**. Reaction is
//! told to hold an unprompted word for the person's return, which is only a promise
//! it can keep if returning wakes it. Every other presence change is polled — read
//! off the axes during a turn that was already happening — but a return happens
//! precisely when no turn is happening, so nothing would observe it. See
//! [`Presence::returns`].

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::Notify;


/// Recent-enough window over which repeated window activations read as "they keep
/// checking" — the eager signal. A couple of brings-to-front inside this span.
const ACTIVATION_WINDOW: Duration = Duration::from_secs(120);

/// How many activations within [`ACTIVATION_WINDOW`] count as eager.
const EAGER_ACTIVATIONS: usize = 2;

/// How long hi-agent may owe a reply before the user is read as actively waiting.
/// Short — past this they're expecting something and want progress exposed.
const OWED_EAGER: Duration = Duration::from_secs(30);

/// How long without any engagement (a message or a window activation) before the
/// user is read as away *from hi-agent*. A decayed-belief horizon: generous
/// enough that a working pause doesn't read as gone, short enough that a real
/// departure is noticed. Overrides a stale owed-reply — asked, then vanished for
/// this long, is away, not eager.
const AWAY_AFTER: Duration = Duration::from_secs(300);

/// Cap on retained activation timestamps per conversation (they're pruned by age anyway).
const MAX_ACTIVATIONS: usize = 16;

/// One output channel a client can subscribe to — the reach surfaces the agent
/// can push onto.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum OutChannel {
    Text,
    Audio,
    View,
}

/// How much output the user is awaiting — the decaying expectation axis.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Expectation {
    /// Actively waiting: repeatedly checking, or a reply is owed and overdue.
    Eager,
    /// Around and engaged, nothing overdue.
    Present,
    /// No sign of them for a while — stepped away from hi-agent.
    Away,
}

/// Which of the user's surfaces a message can land on right now — the three the
/// user thinks in: a window, the speaker, the mic.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Reach {
    /// A window is open — words or a view reach them on screen. (Either the
    /// words channel or the view channel being live means a window is up.)
    pub window: bool,
    /// The speaker is on — the agent can be heard aloud.
    pub speaker: bool,
    /// The mic is live — the agent can hear them (and a voice exchange is on).
    pub mic: bool,
}

/// A read of the conversation's presence across all three axes, taken at one instant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Snapshot {
    pub reach: Reach,
    pub expectation: Expectation,
    /// A voice conversation is active (mirrors `reach.mic`): licenses speaking
    /// aloud even with no window up.
    pub voice_posture: bool,
}

/// What the person's window is doing — the one thing they'd describe in these
/// words, and the only presence fact the client *reports* rather than the host
/// deriving it.
///
/// Three states because two of them are invisible from anywhere else. Reach can
/// only say whether a channel is open, and the face drops its channels for
/// `Background` and `Closed` alike, so from the wire they are the same nothing.
/// They are not the same situation: **background is ambient and closed is an
/// act.** A window behind an editor may be glanced at in seconds; a window the
/// person shut is one they decided they were done with. That difference is worth
/// exactly one thing — how fast the conversation reads Away — which is why this feeds
/// [`Expectation`] instead of becoming a fourth axis.
///
/// Deliberately *not* projected and deliberately not in `say`'s answer: nothing
/// above the host learns a new vocabulary for this. It moves the expectation the
/// mind already reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum WindowState {
    /// Up and being looked at.
    #[default]
    Active,
    /// Open, but not being read — hidden, miniaturized, or covered by another
    /// window. They're still at the machine.
    Background,
    /// Shut. A deliberate act, so it reads Away at once rather than after
    /// [`AWAY_AFTER`] of a silence that already happened.
    Closed,
}

/// Engagement bookkeeping for the expectation axis. All timestamps are monotonic
/// `Instant`s; all-`None` means nobody has engaged yet.
#[derive(Default)]
struct Engagement {
    /// Last message or window activation — the recency that decays toward away.
    last_engaged: Option<Instant>,
    /// Recent window-activation instants (bring-to-front / became-foreground),
    /// pruned to [`ACTIVATION_WINDOW`].
    activations: VecDeque<Instant>,
    /// Set when a human message arrives with no reply yet delivered; the age of
    /// this is the "owed reply" clock. Cleared on delivery.
    owed_since: Option<Instant>,
    /// Last reported window state. Defaults to [`WindowState::Active`] before
    /// anything is reported: a client that says nothing is treated as present,
    /// the same keep-biased default the rest of this module takes.
    window: WindowState,
}

/// Shared presence state. Cloneable handle: reach counts move with guard
/// lifetimes (a dropped connection un-counts itself), and engagement is poked by
/// the signal sites.
///
/// **Counts, not identities.** Several surfaces can watch one conversation at
/// once — a window, a popover, a phone — and the question the mouth asks is "is
/// there a speaker anywhere I can be heard on", not "which client". So this
/// counts live connections per channel and nothing about who holds them.
#[derive(Clone, Default)]
pub struct Presence {
    /// Live out-channel subscriber counts (screen/speaker/words reach).
    channels: Arc<Mutex<HashMap<OutChannel, usize>>>,
    /// Live mic-in stream count (mic reach / voice posture).
    mic: Arc<Mutex<usize>>,
    /// Expectation-axis bookkeeping.
    engagement: Arc<Mutex<Engagement>>,
    /// The "they came back" notifier — see [`Presence::returns`].
    return_notifier: Arc<Notify>,
}

impl Presence {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- Reach: connection guards -------------------------------------------

    /// Count one live out-channel subscriber until the returned guard drops.
    /// Connecting is itself a light engagement (someone showed up), so it seeds
    /// the decay clock if nothing had.
    pub fn connect(&self, channel: OutChannel) -> PresenceGuard {
        *self.channels.lock().unwrap().entry(channel).or_insert(0) += 1;
        self.touch_engaged();
        PresenceGuard { channels: self.channels.clone(), key: channel }
    }

    /// Count one live mic-in stream until the returned guard drops. Held for the
    /// life of the audio-in WebSocket, so it doubles as the voice-posture signal.
    pub fn connect_mic(&self) -> MicGuard {
        *self.mic.lock().unwrap() += 1;
        self.touch_engaged();
        MicGuard { mic: self.mic.clone() }
    }

    fn live(&self, channel: OutChannel) -> bool {
        self.channels.lock().unwrap().get(&channel).copied().unwrap_or(0) > 0
    }

    fn mic_live(&self) -> bool {
        *self.mic.lock().unwrap() > 0
    }

    fn reach(&self) -> Reach {
        Reach {
            window: self.live(OutChannel::View) || self.live(OutChannel::Text),
            speaker: self.live(OutChannel::Audio),
            mic: self.mic_live(),
        }
    }

    /// What can land right now. Public because the mouth needs it at
    /// the moment of emission, not just at prompt-render time: the speaker is the
    /// one surface where an unheard word is *spent* rather than kept.
    pub fn reachable(&self) -> Reach {
        self.reach()
    }

    // ---- Expectation: engagement pokes --------------------------------------

    /// The window was brought forward / became foreground — the strongest eager
    /// signal ("they keep checking"). Reported first-party by the web face's own
    /// visibility/focus events.
    ///
    /// Returns `true` when this activation is a **return**: presence read
    /// [`Expectation::Away`] until this instant, and nothing was owed. That is the
    /// edge [`Presence::returns`] fires on.
    pub fn note_activation(&self) -> bool {
        self.note_window(WindowState::Active)
    }

    /// The face reports what its window is doing. [`WindowState::Active`] is an
    /// activation — the existing "they're checking on you" edge, and the only one
    /// that can be a return. The other two are the window going away, told apart
    /// because *how* it went away is the whole information: `Closed` is a decision
    /// and reads Away at once; `Background` is ambient and lets the ordinary
    /// [`AWAY_AFTER`] decay do its work, because someone reading in the next window
    /// over has not gone anywhere.
    ///
    /// Returns `true` only for a return (see [`Presence::note_activation`]).
    pub fn note_window(&self, state: WindowState) -> bool {
        if state != WindowState::Active {
            self.engagement.lock().unwrap().window = state;
            return false;
        }
        let now = Instant::now();
        let returned = {
            let e = &mut *self.engagement.lock().unwrap();
            // Classified against the state *before* this activation lands — after it,
            // presence reads Present by construction and the edge would be invisible.
            // That includes the window state: coming back from `Closed` is *the* return
            // to catch, and clearing it first would be the edge erasing its own cause.
            let was_away = matches!(classify_at(e, now), Expectation::Away);
            e.window = WindowState::Active;
            // A reply already owed means a turn is coming for that reason; waking a
            // second time for "they're back" would double-answer the same arrival.
            // (They typed, which pokes `note_activity`, and the page reports focus at
            // about the same moment — a race with no fixed order.)
            let idle = e.owed_since.is_none();
            e.last_engaged = Some(now);
            e.activations.push_back(now);
            while e.activations.len() > MAX_ACTIVATIONS {
                e.activations.pop_front();
            }
            was_away && idle
        };
        if returned {
            // Fires at most once per absence, and needs no debounce state to do it:
            // the line above sets `last_engaged`, so the conversation cannot read Away again
            // until another full `AWAY_AFTER` of silence. A page refresh — disconnect
            // then immediate reconnect — is two activations seconds apart, and only
            // the first can possibly be a return.
            self.return_notifier.notify_waiters();
        }
        returned
    }

    /// A human message arrived — refresh engagement and start the owed-reply
    /// clock if nothing is owed yet.
    pub fn note_activity(&self) {
        let now = Instant::now();
        let e = &mut *self.engagement.lock().unwrap();
        e.last_engaged = Some(now);
        e.owed_since.get_or_insert(now);
    }

    /// A turn finished delivering — clear the owed-reply clock. A no-op when
    /// nothing was owed.
    pub fn note_delivered(&self) {
        self.engagement.lock().unwrap().owed_since = None;
    }

    /// Seed the decay clock on first contact without starting an owed-reply.
    fn touch_engaged(&self) {
        self.engagement.lock().unwrap().last_engaged.get_or_insert(Instant::now());
    }

    fn expectation(&self, now: Instant) -> Expectation {
        classify_at(&self.engagement.lock().unwrap(), now)
    }

    /// A handle the reaction loop parks on, pulsed the moment presence goes from
    /// *away* to the person actually being back.
    ///
    /// **Only [`Presence::note_activation`] fires it**, and that is the whole care in
    /// the design. A reconnecting out-channel looks like an arrival and is not one:
    /// `/api/out/text` is a long-lived state subscription that reconnects after
    /// transport failures, so `connect` proves a surface exists, never that a
    /// person is in front of it. The attention lane is first-party and reports
    /// exactly one thing — this page just became visible or regained focus — which is
    /// a human hand.
    ///
    /// **Drop, don't queue** ([`docs/arch/core.md`] clock rule 3): `notify_waiters`
    /// stores no permit, so a return that fires while the loop is mid-turn is
    /// discarded rather than delivered late. That is the wanted behaviour — a turn
    /// in progress is already talking to them.
    pub fn returns(&self) -> Arc<Notify> {
        self.return_notifier.clone()
    }

    // ---- Read ----------------------------------------------------------------

    /// Presence across all three axes, right now.
    pub fn snapshot(&self) -> Snapshot {
        let reach = self.reach();
        Snapshot {
            reach,
            expectation: self.expectation(Instant::now()),
            voice_posture: reach.mic,
        }
    }

    /// The conversation's presence as human-model facts for the mind's prompt — **the
    /// expectation, and only the expectation**. Facts only.
    ///
    /// Reach deliberately does not appear here. It is binary and per-channel, so a
    /// tool call can *answer* it: `say` reports where the words actually landed, read
    /// at the instant of emission rather than off a snapshot taken when the turn
    /// began. A projected reach is a second, staler copy of the same fact, and the
    /// two disagree exactly when it matters — a turn can outlive the window that
    /// started it. So nothing above the host asks whether it can be heard; it speaks
    /// and reads the answer.
    ///
    /// Expectation cannot be learned that way and stays. It is graded rather than
    /// binary (eager / around / away) and it is true even when every channel is
    /// open, because it shapes *how much* to say rather than *whether* — a failed
    /// send can never teach it, since there was no failure to read.
    pub fn render(&self) -> String {
        render_expectation(self.expectation(Instant::now())).to_owned()
    }
}

/// One conversation's expectation at `now`, read off its raw engagement record. Split out
/// so the return edge can classify the state *before* an activation is applied,
/// against the same rule the prompt-render path uses.
fn classify_at(e: &Engagement, now: Instant) -> Expectation {
    let engaged_ago = e.last_engaged.map(|t| now.saturating_duration_since(t));
    let recent_activations = e
        .activations
        .iter()
        .filter(|t| now.saturating_duration_since(**t) < ACTIVATION_WINDOW)
        .count();
    let owed_age = e.owed_since.map(|t| now.saturating_duration_since(t));
    classify(engaged_ago, recent_activations, owed_age, e.window)
}

/// Pure expectation policy, extracted so the fusion is unit-testable without a
/// clock. Away wins over a stale owed-reply (asked, then gone = away, not eager).
///
/// A shut window is Away immediately and outranks everything, owed reply included.
/// Waiting out [`AWAY_AFTER`] for it would be answering a question they already
/// answered: the decay exists to *infer* absence from silence, and closing the
/// window states it. `Background` gets no such shortcut and is deliberately absent
/// from this function — someone reading in the window in front of ours has not
/// left, and treating "not looking right now" as "gone" is the over-eager read
/// that would make the agent go quiet on a person sitting right there.
fn classify(
    engaged_ago: Option<Duration>,
    recent_activations: usize,
    owed_age: Option<Duration>,
    window: WindowState,
) -> Expectation {
    if window == WindowState::Closed {
        return Expectation::Away;
    }
    if engaged_ago.is_some_and(|d| d >= AWAY_AFTER) {
        return Expectation::Away;
    }
    let eager =
        recent_activations >= EAGER_ACTIVATIONS || owed_age.is_some_and(|a| a >= OWED_EAGER);
    if eager {
        Expectation::Eager
    } else {
        Expectation::Present
    }
}

fn render_expectation(e: Expectation) -> &'static str {
    match e {
        Expectation::Eager => {
            "They're actively waiting on you right now — checking in, or expecting a reply."
        }
        Expectation::Present => "They're around.",
        Expectation::Away => {
            "No sign of them for a while — they've stepped away from hi-agent (no messages, \
             no checking in)."
        }
    }
}

/// Un-counts its out-channel subscriber on drop.
pub struct PresenceGuard {
    channels: Arc<Mutex<HashMap<OutChannel, usize>>>,
    key: OutChannel,
}

impl Drop for PresenceGuard {
    fn drop(&mut self) {
        let mut map = self.channels.lock().unwrap();
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
        let mut n = self.mic.lock().unwrap();
        *n = n.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Reach ----

    #[test]
    fn out_channel_counts_follow_guard_lifetimes() {
        let p = Presence::new();
        assert!(!p.live(OutChannel::View));
        let g1 = p.connect(OutChannel::View);
        let g2 = p.connect(OutChannel::View);
        assert!(p.live(OutChannel::View));
        drop(g1);
        assert!(p.live(OutChannel::View));
        drop(g2);
        assert!(!p.live(OutChannel::View));
    }

    #[test]
    fn mic_liveness_follows_its_guard() {
        let p = Presence::new();
        assert!(!p.reach().mic);
        let g = p.connect_mic();
        assert!(p.reach().mic);
        assert!(p.snapshot().voice_posture);
        drop(g);
        assert!(!p.reach().mic);
    }

    #[test]
    fn reach_reports_each_surface_independently() {
        let p = Presence::new();
        let _v = p.connect(OutChannel::View);
        let r = p.reach();
        assert!(r.window && !r.speaker && !r.mic);
    }

    #[test]
    fn a_words_only_client_still_counts_as_a_window() {
        // A text-reply reader has a window up even with no rendered view.
        let p = Presence::new();
        let _t = p.connect(OutChannel::Text);
        assert!(p.reach().window);
    }

    // ---- Expectation policy (pure) ----

    #[test]
    fn never_engaged_reads_present_not_away() {
        // A bare connection with nothing observed yet is neutral, not gone.
        assert_eq!(classify(None, 0, None, WindowState::Active), Expectation::Present);
    }

    #[test]
    fn stale_engagement_is_away() {
        assert_eq!(classify(Some(AWAY_AFTER), 0, None, WindowState::Active), Expectation::Away);
        assert_eq!(classify(Some(Duration::from_secs(600)), 0, None, WindowState::Active), Expectation::Away);
    }

    #[test]
    fn repeated_activation_is_eager() {
        assert_eq!(classify(Some(Duration::from_secs(2)), 2, None, WindowState::Active), Expectation::Eager);
    }

    #[test]
    fn overdue_owed_reply_is_eager() {
        assert_eq!(classify(Some(Duration::from_secs(1)), 0, Some(OWED_EAGER), WindowState::Active), Expectation::Eager);
    }

    #[test]
    fn owed_but_fresh_is_present_not_eager() {
        // A reply owed for only a moment isn't yet "waiting".
        assert_eq!(classify(Some(Duration::from_secs(1)), 0, Some(Duration::from_secs(5)), WindowState::Active), Expectation::Present);
    }

    #[test]
    fn away_overrides_a_long_owed_reply() {
        // Asked, then vanished: staleness wins over the (huge) owed age.
        assert_eq!(
            classify(Some(Duration::from_secs(3600)), 0, Some(Duration::from_secs(3600)), WindowState::Active),
            Expectation::Away
        );
    }

    // ---- The three window states ----

    /// Shutting the window states the absence the decay exists to infer, so it
    /// takes effect at once — even a second after they last typed, and even with a
    /// reply owed. It outranks both, which is the whole reason it is worth
    /// reporting separately from `Background`.
    #[test]
    fn a_closed_window_reads_away_immediately() {
        assert_eq!(
            classify(Some(Duration::from_secs(1)), 0, None, WindowState::Closed),
            Expectation::Away
        );
        assert_eq!(
            classify(Some(Duration::from_secs(1)), 2, Some(OWED_EAGER), WindowState::Closed),
            Expectation::Away,
            "shutting the window outranks both the eager signals — they left mid-wait"
        );
    }

    /// Backgrounding is ambient — reading in the window in front of ours is not
    /// leaving — so it must NOT shortcut to Away. Getting this wrong makes the
    /// agent go quiet on someone sitting right there.
    #[test]
    fn a_backgrounded_window_still_reads_present() {
        assert_eq!(
            classify(Some(Duration::from_secs(1)), 0, None, WindowState::Background),
            Expectation::Present
        );
    }

    /// The two non-active states are the same to reach and different to
    /// expectation. That asymmetry is the point of having three.
    #[test]
    fn background_and_closed_diverge_only_on_expectation() {
        let p = Presence::new();
        p.note_activity(); // just spoke — nothing stale
        p.note_delivered();

        p.note_window(WindowState::Background);
        assert_eq!(p.snapshot().expectation, Expectation::Present);

        p.note_window(WindowState::Closed);
        assert_eq!(p.snapshot().expectation, Expectation::Away);

        // Neither touched reach: the channels are what say whether anything lands,
        // and nothing here opened or closed one.
        assert!(!p.reachable().window);
    }

    /// Opening a shut window is the return edge, and it must survive the state
    /// being cleared on the way in — the arrival cannot erase its own cause.
    #[test]
    fn reopening_a_closed_window_is_a_return() {
        let p = Presence::new();
        p.note_activity();
        p.note_delivered();
        p.note_window(WindowState::Closed);
        assert!(p.note_window(WindowState::Active), "reopening is a return");
        assert_eq!(p.snapshot().expectation, Expectation::Present);
    }

    // ---- Expectation through the store ----

    #[test]
    fn fresh_activity_reads_present() {
        let p = Presence::new();
        p.note_activity();
        // Just messaged: engaged and owed, but not yet overdue → present.
        assert_eq!(p.snapshot().expectation, Expectation::Present);
    }

    #[test]
    fn delivered_clears_the_owed_clock() {
        let p = Presence::new();
        p.note_activity();
        p.note_delivered();
        // With nothing owed and no activations, they're just present.
        assert_eq!(p.snapshot().expectation, Expectation::Present);
    }

    #[test]
    fn two_activations_read_eager() {
        let p = Presence::new();
        p.note_activation();
        p.note_activation();
        assert_eq!(p.snapshot().expectation, Expectation::Eager);
    }

    /// The projection carries the expectation and *not* the reach: reach is what a
    /// `say` call answers, and a second copy in the window would be the staler of
    /// the two. An open channel must not put anything about channels in the prompt.
    #[test]
    fn render_states_expectation_only() {
        let p = Presence::new();
        let _v = p.connect(OutChannel::View);
        let _a = p.connect(OutChannel::Audio);
        let out = p.render();
        assert_eq!(out, render_expectation(Expectation::Present));
        for leaked in ["screen", "window", "speaker", "mic", "Open to them"] {
            assert!(!out.contains(leaked), "reach leaked into the projection: {leaked:?}");
        }
    }

    // ---- The return edge ----

    /// Backdate the last engagement past [`AWAY_AFTER`] so it reads Away without a
    /// five-minute test. Reaches into the private state on purpose: the alternative
    /// is a clock injection that only this handful of tests would use.
    fn make_away(p: &Presence) {
        let stale = Instant::now()
            .checked_sub(AWAY_AFTER + Duration::from_secs(1))
            .expect("monotonic clock younger than AWAY_AFTER — machine booted seconds ago?");
        *p.engagement.lock().unwrap() =
            Engagement { last_engaged: Some(stale), ..Default::default() };
    }

    #[test]
    fn activation_after_an_absence_is_a_return() {
        let p = Presence::new();
        make_away(&p);
        assert_eq!(p.snapshot().expectation, Expectation::Away);
        assert!(p.note_activation(), "coming back to a cold conversation is a return");
        assert_eq!(p.snapshot().expectation, Expectation::Present);
    }

    #[test]
    fn activation_while_they_are_already_here_is_not_a_return() {
        let p = Presence::new();
        p.note_activation();
        assert!(!p.note_activation(), "they never left");
    }

    #[test]
    fn a_page_refresh_fires_the_edge_at_most_once() {
        // Disconnect + immediate reconnect is two activations seconds apart. The
        // first may be a genuine return; the second must not re-fire, or every
        // reload would wake the conversation.
        let p = Presence::new();
        make_away(&p);
        assert!(p.note_activation());
        assert!(!p.note_activation(), "the reload half must not fire again");
    }

    #[test]
    fn an_owed_reply_suppresses_the_return() {
        // They typed: `note_activity` starts the owed clock and the message drives a
        // turn on its own. The page reporting focus at the same moment must not
        // schedule a second one.
        let p = Presence::new();
        make_away(&p);
        p.note_activity();
        assert!(!p.note_activation(), "their message is already the reason to turn");
    }

    /// One conversation, one return handle — so a waiter parked before an absence
    /// is the same waiter the activation notifies.
    #[test]
    fn the_return_handle_is_stable() {
        let p = Presence::new();
        assert!(Arc::ptr_eq(&p.returns(), &p.returns()));
    }

}
