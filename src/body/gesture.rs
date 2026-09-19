//! The attention gestures — ways the user pulls the agent's attention with one key,
//! all best-effort. **Which key is the vendor's decision, not this file's**: the right
//! ⌘ on macOS, the right Ctrl on Windows, each picked because the other one of that
//! pair is the everyday shortcut modifier there
//! ([`crate::body::capabilities::hotkey`]). This file says what a gesture *means*, and
//! that is the same everywhere:
//!
//! - **Single tap → open the chat popup:** a lone quick tap opens the menu-bar
//!   conversation popover ([`crate::body::capabilities::tray::open_chat`]).
//!   It is confirmed only after the double-tap window passes unpaired, so it never
//!   fires as the first half of a double-tap.
//! - **Double tap → "come and see this":** hands the agent a screenshot of
//!   the current screen. It is *not a new sense* — the screenshot lands exactly like a
//!   drag-dropped image (a handed file on the `file` channel) and wakes the mind
//!   (`foundation::server::files::receive_screenshot`).
//! - **Press and hold → continuous attention:** for as long as the key is
//!   held, the agent listens (native mic capture → the same audio ingest the browser mic
//!   uses); on release it stops. It cannot *look* while it listens — this bullet used to
//!   say it could, naming a `look` tool that was deleted with the screen capabilities —
//!   so seeing is the double tap's job and the two gestures are used together.
//!   The mic opens early (after a short capture threshold) and buffers a
//!   pre-roll, but that audio is only *processed* once the press also crosses the
//!   full hold threshold — so a genuine hold loses almost no leading speech while an
//!   accidental quick press opens nothing to process. No new processing path — the
//!   held speech rides the normal pipeline, carrying only a context note that it came
//!   from this headless gesture.
//!
//! **Two of the three reach further than their actions do.** Every recognizer runs on
//! every platform with a key tap, but the chat popup is an `NSPopover` and the
//! screenshot is a macOS grab, so on Windows a single tap and a double tap are
//! recognized and then land on nothing. Both wait on the same thing — a way for the
//! core to ask the shell that owns the window and the window server, which is
//! `WS /api/mechanisms` (`docs/arch/mechanisms.md`), dialed by no shell yet. The hold
//! needs no such thing: the mic is cpal in this process and the ear's open/shut state
//! is read back off `GET /api/listening`, so it works on Windows today.
//!
//! The OS tap only emits raw [`Edge`](crate::body::capabilities::hotkey::Edge)s; the
//! recognizers and the hold's threshold timer run here, on the runtime, against one
//! clock. Observing the keys needs the **Accessibility / Input Monitoring** grant on
//! macOS (Windows asks for nothing), the screenshot needs **Screen Recording**, and the
//! hold's mic needs **Microphone**; any missing grant just makes that part inert, never
//! fatal. On a platform with no key tap at all, [`install`] returns having done nothing.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::foundation::server::AppState;

use bytes::Bytes;
use std::collections::VecDeque;
use tokio::sync::mpsc;

/// Arm the gestures: from now on a double-tap of the attention key hands the agent a
/// screenshot, and a press-and-hold opens continuous attention, both in `conversation`.
/// Spawns the OS event-loop thread and the recognizer task and returns immediately.
/// Call once, after the reaction is running, from within the tokio runtime — it
/// captures the current runtime handle to drive the recognizers and the async
/// capture/ingest off the (blocking) event-loop thread.
///
/// A platform with no key tap is not an error and does not get a thread: [`install`]
/// says so once and returns, rather than spawning something whose only job is to fail.
pub fn install(state: Arc<AppState>) {
    use crate::body::capabilities::hotkey;

    if !hotkey::available() {
        tracing::info!("attention gestures: no key tap on this platform; nothing armed");
        return;
    }

    let handle = tokio::runtime::Handle::current();

    // Raw key edges flow from the OS tap thread to the async recognizer on the
    // runtime, which stamps them against its own clock (so the double-tap window
    // and the hold threshold share one clock with the timer below).
    let (edge_tx, edge_rx) = tokio::sync::mpsc::unbounded_channel::<hotkey::Edge>();
    // The follower carries a held session's state to whoever draws it: the menu-bar
    // item's single text field here, and `GET /api/listening` for a tray in another
    // process.
    let (attn_tx, attn_rx) = tokio::sync::mpsc::unbounded_channel::<AttnEvent>();
    handle.spawn(attention_follower(state.clone(), attn_rx));
    handle.spawn(recognizer_loop(state, edge_rx, attn_tx));

    let spawned = std::thread::Builder::new()
        .name("hotkey-gesture".to_string())
        .spawn(move || {
            let on_edge = move |e: hotkey::Edge| {
                // The tap callback must never block the OS run loop (macOS) or exceed
                // the low-level hook timeout (Windows); unbounded + non-blocking send.
                // A closed receiver (recognizer gone) drops the edge.
                let _ = edge_tx.send(e);
            };
            // Blocks on the OS run loop / message pump for the process's life.
            if let Err(e) = hotkey::listen(on_edge) {
                tracing::warn!(error = %format!("{e:#}"), "gesture: key listener unavailable; gestures disabled");
            }
        });
    match spawned {
        Ok(_) => tracing::info!(
            key = %hotkey::key_label(),
            conversation = %"the conversation",
            "attention gestures armed (single-tap → chat, double-tap → screenshot, press-hold → attention)"
        ),
        Err(e) => tracing::warn!(error = %e, "gesture: could not spawn listener thread; gestures disabled"),
    }
}

/// What the recognizer tells the attention follower: a committed hold began, or
/// it ended (release). Glance doesn't go through here — it's a self-contained
/// pulse on the tray icon.
enum AttnEvent {
    ListenStart,
    ListenStop,
}

/// Carry a held-attention session to everything that draws it.
///
/// Two things, on **two different clocks**, and the difference is the point:
///
/// - **The ear** ([`AppState::listening`]) is open exactly while the mic is — it opens
///   on `ListenStart` and shuts on `ListenStop`, the moment the key comes up. That is
///   what `GET /api/listening` reports, so a tray in another process draws the truth
///   about the microphone and nothing else.
/// - **The menu-bar strip** is a place to read, so it lingers. macOS has one text field
///   beside the status item, and this puts the live transcript in it while the user
///   speaks and then the agent's reply — in sequence, because there is only the one
///   field. The reply usually lands *after* release, so `ListenStop` starts a short
///   dwell that each reply chunk extends, and only then does the item collapse back to
///   icon-only.
///
/// Reading a reply for three seconds is not the mic still being open, and a tray that
/// said so would be lying about the one thing it is there to say. Windows has no text
/// beside a notification icon and so has nothing to linger for.
///
/// Spawned once and fed `AttnEvent`s by the recognizer; the text comes off the same
/// non-draining echoes the channel inspector uses ([`AppState::input_echo`] /
/// [`AppState::output_echo`]).
async fn attention_follower(
    state: Arc<AppState>,
    mut events: tokio::sync::mpsc::UnboundedReceiver<AttnEvent>,
) {
    use crate::body::capabilities::tray;
    use crate::types::Channel;
    use tokio::sync::broadcast::error::RecvError;

    // How long the strip lingers after release so the reply can show; each reply
    // chunk pushes it out again.
    const DWELL: Duration = Duration::from_millis(3500);
    // Keep the menu-bar item from ballooning past the notch — show only the tail.
    const MAX_CHARS: usize = 48;
    fn tail(s: &str) -> String {
        let chars: Vec<char> = s.chars().collect();
        if chars.len() <= MAX_CHARS {
            s.to_string()
        } else {
            let cut: String = chars[chars.len() - MAX_CHARS..].iter().collect();
            format!("…{cut}")
        }
    }

    let mut input_rx = state.input_echo.subscribe();
    let mut output_rx = state.output_echo.subscribe();
    let mut active = false;
    let mut reply = String::new();
    let mut deadline: Option<Instant> = None;

    loop {
        let dwell = async {
            match deadline {
                Some(d) => tokio::time::sleep_until(tokio::time::Instant::from_std(d)).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            ev = events.recv() => match ev {
                None => break, // recognizer gone
                Some(AttnEvent::ListenStart) => {
                    active = true;
                    reply.clear();
                    deadline = None;
                    // Re-subscribe so we start from the live edge (no stale lines).
                    input_rx = state.input_echo.subscribe();
                    output_rx = state.output_echo.subscribe();
                    state.listening.set(true);
                    tray::set_listening(true);
                    tray::set_text("聆听…");
                }
                Some(AttnEvent::ListenStop) => {
                    // The ear shuts now; the strip stays up long enough to read.
                    state.listening.set(false);
                    if active {
                        deadline = Some(Instant::now() + DWELL);
                    }
                }
            },
            r = input_rx.recv(), if active => match r {
                Ok(echo) => {
                    if echo.channel == Channel::Text {
                        tray::set_text(&tail(&echo.text)); // listen phase
                    }
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => {}
            },
            r = output_rx.recv(), if active => match r {
                Ok(echo) => {
                    if echo.channel == Channel::Text {
                        reply.push_str(&echo.text); // speak phase
                        tray::set_text(&tail(&reply));
                        if deadline.is_some() {
                            deadline = Some(Instant::now() + DWELL);
                        }
                    }
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => {}
            },
            _ = dwell, if deadline.is_some() => {
                active = false;
                reply.clear();
                deadline = None;
                tray::set_text("");
                tray::set_listening(false);
            }
        }
    }
}

/// Drive the recognizers off the edge stream. Double-tap and hold run side by side
/// over the one key: a quick second press fires a glance; a single press still down
/// opens attention in two stages — past a short capture threshold the mic opens and
/// buffers a pre-roll, past the full threshold that pre-roll is committed to live
/// processing — and its release closes it. The two are kept from colliding — a
/// completed double-tap (or any other key) disarms a pending hold (discarding a
/// buffering pre-roll), and committing to a hold cancels a half-formed double-tap.
async fn recognizer_loop(
    state: Arc<AppState>,
    mut edges: tokio::sync::mpsc::UnboundedReceiver<crate::body::capabilities::hotkey::Edge>,
    attn_tx: tokio::sync::mpsc::UnboundedSender<AttnEvent>,
) {
    use crate::body::capabilities::hotkey::{self, Edge, GestureEvent};

    let start = Instant::now();
    let mut dt = hotkey::DoubleTap::new(hotkey::DEFAULT_WINDOW);
    let mut hold = hotkey::Hold::new(hotkey::DEFAULT_CAPTURE, hotkey::DEFAULT_HOLD);
    let mut tap = hotkey::SingleTap::new(hotkey::DEFAULT_WINDOW);
    let mut session: Option<MicSession> = None;

    // Per-press bookkeeping for the single-tap recognizer: the current press's
    // down-time, and whether — since it went down — the mic opened (a hold, not a
    // tap), a chord broke it, or it completed a double-tap. A release is a *clean
    // quick tap* (the single-tap candidate) only when none of these is true.
    let mut down_at: u64 = 0;
    let mut captured_since_down = false;
    let mut chorded_since_down = false;
    let mut glanced_since_down = false;

    loop {
        // Sleep until the earliest pending threshold elapses — a hold's capture/hold
        // threshold, or a single-tap's confirmation window; if none is pending, wait
        // forever (only an edge can wake us). Rebuilt each iteration so it tracks the
        // current pending press.
        let deadline = match (hold.next_deadline(), tap.next_deadline()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let tick = async {
            match deadline {
                Some(d) => {
                    let now = start.elapsed().as_millis() as u64;
                    tokio::time::sleep(Duration::from_millis(d.saturating_sub(now))).await;
                }
                None => std::future::pending::<()>().await,
            }
        };

        tokio::select! {
            edge = edges.recv() => {
                let Some(edge) = edge else { break }; // tap thread gone
                let t = start.elapsed().as_millis() as u64;
                match edge {
                    Edge::Down => {
                        down_at = t;
                        captured_since_down = false;
                        chorded_since_down = false;
                        glanced_since_down = false;
                        // A new press is either a fresh tap or the second half of a
                        // double-tap — either way an earlier tap is no longer "lone".
                        tap.cancel();
                        hold.on_down(t);
                        if dt.on_down(t) {
                            // A completed double-tap: glance, and make sure this same
                            // press can't also become a hold.
                            dt.on_other_input();
                            hold.cancel();
                            glanced_since_down = true;
                            glance(&state);
                        }
                    }
                    Edge::Up => {
                        // Release: end a committed attention, or discard a still-buffering
                        // pre-roll (released before the hold threshold). `on_up`
                        // clears the recognizer's pending press either way.
                        let _ = hold.on_up(t);
                        stop_attention(&attn_tx, &mut session);
                        // A clean quick tap — released before the mic ever opened, no
                        // chord, and not a double-tap's second press — is a single-tap
                        // candidate: confirm it after the double-tap window (so it can't
                        // be the first half of a double-tap) before it opens the chat.
                        if !captured_since_down && !chorded_since_down && !glanced_since_down {
                            tap.arm(down_at);
                        }
                    }
                    Edge::Other => {
                        dt.on_other_input();
                        hold.cancel();
                        // A chord (⌘C, Ctrl+C) breaks a half-formed hold: drop a buffering
                        // pre-roll, but leave an already-committed attention running.
                        discard_capture(&mut session);
                        chorded_since_down = true;
                        tap.cancel();
                    }
                }
            }
            _ = tick => {
                let t = start.elapsed().as_millis() as u64;
                match hold.poll(t) {
                    // Stage 1: open the mic and start buffering, no processing yet.
                    Some(GestureEvent::CaptureStart) => {
                        captured_since_down = true;
                        arm_capture(&state, &mut session);
                    }
                    // Stage 2: commit the pre-roll to live processing. Cancels a
                    // half-formed double-tap so a later tap doesn't pair with this press.
                    Some(GestureEvent::HoldStart) => {
                        dt.on_other_input();
                        commit_capture(&attn_tx, &mut session);
                    }
                    _ => {}
                }
                // A single tap whose confirmation window elapsed unpaired opens the
                // menu-bar chat popup — best-effort, and a no-op when no tray is up *or*
                // when there is no in-process popover to open, which is every platform
                // but macOS. See this module's header for what that waits on.
                if tap.poll(t) {
                    crate::body::capabilities::tray::open_chat();
                }
            }
        }
    }

    // Tap thread ended — make sure we aren't left attending.
    stop_attention(&attn_tx, &mut session);
}

/// Capture the screen and hand it to the agent (the double-tap gesture). Flashes the
/// tray first as an instant ack of the *gesture* (before the async capture), then
/// spawns the capture + carrier ingest so a slow grab never stalls the recognizer.
///
/// **macOS only, and deliberately so far.** This is the one screen grab that survived
/// the capabilities deletion, because it is not the agent looking — it is the person
/// handing over a picture, which lands on the `file` channel like a dropped image.
/// Windows has no twin yet: writing one means either a second grab in the engine or a
/// `screen.grab` over `WS /api/mechanisms`, and that is the shell's call to make when
/// it dials. Until then a Windows double-tap is recognized and says so in the log
/// rather than silently doing nothing.
fn glance(state: &Arc<AppState>) {
    crate::body::capabilities::tray::flash();
    #[cfg(target_os = "macos")]
    {
        let state = state.clone();
        tokio::spawn(async move {
            match crate::foundation::vendors::macos_screencast::grab_screen_png().await {
                Ok(png) => {
                    if let Err(e) = crate::foundation::server::files::receive_screenshot(&state, &png).await {
                        tracing::warn!(error = %e, "gesture: handing screenshot to the agent failed");
                    }
                }
                Err(e) => tracing::warn!(
                    error = %format!("{e:#}"),
                    "gesture: screen capture failed (Screen Recording permission?)"
                ),
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = state;
        tracing::info!("gesture: double-tap recognized, but this platform has no screen grab to hand over");
    }
}

/// The context note that rides the first utterance of a held-attention session, so the
/// mind knows this speech is live and headless — spoken at a machine with no page open,
/// not typed.
///
/// It used to end "look at their screen if you need to". **It cannot**: `hi_look` and
/// every other screen capability were deleted, so that sentence invited a tool call
/// with nothing behind it. What the person can do instead is the double tap, which
/// hands over a picture — and that is what this now says.
fn attention_tag() -> String {
    format!(
        "live attention: the user is holding the {key} and talking to you right now, with no \
         page open — this is speech, not typing, and they are waiting. Respond as the \
         conversation warrants. You cannot see their screen; if you need to, ask them to \
         double-tap the same key, which hands you a picture of it.",
        key = crate::body::capabilities::hotkey::key_label()
    )
}

/// The control message that promotes a buffering capture into live processing.
enum Ctrl {
    /// Stop buffering the pre-roll and start feeding the audio ingest — committing
    /// this press to continuous attention.
    Commit,
}

/// A held-attention session, in one of two states the host moves through: first a
/// *buffering* capture (mic open, pre-roll accumulating, `committed == false`), then —
/// once the press crosses the hold threshold — a *committed* one feeding the audio
/// ingest. Dropping it stops the mic; the pump task then either discards the pre-roll
/// (released/chorded before commit) or lets the ingest finalize (committed).
struct MicSession {
    /// Dropping this stops the mic and ends the frame stream the pump reads.
    _capture: crate::body::capabilities::audio_capture::Capture,
    /// Send `Ctrl::Commit` to promote the buffered pre-roll into live processing.
    ctrl: mpsc::Sender<Ctrl>,
    /// Whether this press has crossed the hold threshold (pre-roll promoted to ingest).
    committed: bool,
}

/// Stage 1 — open the mic and start buffering a pre-roll, *without* processing it yet.
/// Called when a press crosses the short capture threshold; minimizing the lost
/// leading audio while the press is still being confirmed as a hold. If it goes on to
/// cross the hold threshold the host calls [`commit_capture`]; otherwise the pre-roll
/// is dropped ([`discard_capture`] / [`stop_attention`]) and no transcript is produced.
/// Idempotent (a re-entrant capture is ignored) and best-effort — no mic, no attention.
fn arm_capture(state: &Arc<AppState>, session: &mut Option<MicSession>) {
    if session.is_some() {
        return; // already capturing
    }
    if !crate::body::capabilities::audio_capture::available() {
        tracing::warn!("press-hold attention: native mic capture unavailable; nothing to listen with");
        return;
    }
    match crate::body::capabilities::audio_capture::start() {
        Ok((capture, frames)) => {
            let (ctrl_tx, ctrl_rx) = mpsc::channel::<Ctrl>(1);
            tokio::spawn(pump_capture(state.clone(), frames, ctrl_rx));
            *session = Some(MicSession { _capture: capture, ctrl: ctrl_tx, committed: false });
            tracing::info!("press-hold attention: capturing (mic open, buffering pre-roll)");
        }
        Err(e) => tracing::warn!(
            error = %format!("{e:#}"),
            "press-hold attention: mic capture failed (Microphone permission?)"
        ),
    }
}

/// Stage 2 — commit a buffering capture to continuous attention: tell the pump to flush
/// the pre-roll and start feeding the audio ingest. Called when the press crosses the
/// hold threshold. No-op if not capturing or already committed.
fn commit_capture(
    attn_tx: &tokio::sync::mpsc::UnboundedSender<AttnEvent>,
    session: &mut Option<MicSession>,
) {
    if let Some(s) = session.as_mut()
        && !s.committed
    {
        // Capacity-1 channel, freshly created and sent on once — a send only fails if
        // the pump is already gone (mic stopped), in which case there's nothing to commit.
        if s.ctrl.try_send(Ctrl::Commit).is_ok() {
            s.committed = true;
            // The follower lights the icon colour and shows the transcript/reply text.
            let _ = attn_tx.send(AttnEvent::ListenStart);
            tracing::info!("press-hold attention: listening (processing)");
        }
    }
}

/// Drop a still-buffering capture — mic closes, pre-roll discarded — for a chord that
/// breaks a pending hold. A press that has already *committed* is left attending: a key
/// pressed during attention must not drop it.
fn discard_capture(session: &mut Option<MicSession>) {
    if session.as_ref().is_some_and(|s| !s.committed) {
        session.take();
        tracing::info!("press-hold attention: discarded (chord broke the hold)");
    }
}

/// Close the session: drop the capture (mic stops). If it had committed, the detached
/// ingest sees the stream end and finalizes its last utterance; if it was only
/// buffering, the pre-roll is discarded and no transcript is produced. No-op when not
/// attending.
fn stop_attention(
    attn_tx: &tokio::sync::mpsc::UnboundedSender<AttnEvent>,
    session: &mut Option<MicSession>,
) {
    if let Some(s) = session.take() {
        if s.committed {
            // The follower keeps the strip up through the reply, then settles back.
            let _ = attn_tx.send(AttnEvent::ListenStop);
            tracing::info!("press-hold attention: released (mic closed)");
        } else {
            tracing::info!("press-hold attention: discarded (released before hold)");
        }
    }
}

/// The buffering/forwarding task behind a [`MicSession`]. Until it receives
/// `Ctrl::Commit` it accumulates incoming PCM as a bounded pre-roll; on commit it
/// spawns the same audio ingest the browser mic uses, flushes the pre-roll into it,
/// then forwards live frames. When the capture is dropped (mic stops) the frame stream
/// ends and the task exits — finalizing the ingest if it had committed (its `tx` drops,
/// ending the stream), or dropping the pre-roll if it had not.
async fn pump_capture(
    state: Arc<AppState>,
    mut frames: mpsc::Receiver<Bytes>,
    mut ctrl: mpsc::Receiver<Ctrl>,
) {
    // Cap the pre-roll so an unexpectedly long buffering window can't grow unbounded.
    // In practice it only spans the capture→hold gap (~300 ms); this is a safety rail.
    const MAX_PREROLL_CHUNKS: usize = 20; // ~2 s at 100 ms/chunk
    let mut preroll: VecDeque<Bytes> = VecDeque::new();
    let mut out: Option<mpsc::Sender<Bytes>> = None;
    let mut ctrl_open = true;

    loop {
        tokio::select! {
            ctrl_msg = ctrl.recv(), if ctrl_open => {
                match ctrl_msg {
                    Some(Ctrl::Commit) => {
                        let (tx, rx) = mpsc::channel::<Bytes>(64);
                        tokio::spawn(crate::foundation::server::audio::ingest_pcm_stream(
                            state.clone(),
                            None,
                            Some(attention_tag()),
                            rx,
                        ));
                        let mut gone = false;
                        for chunk in preroll.drain(..) {
                            if tx.send(chunk).await.is_err() {
                                gone = true; // ingest already gone
                                break;
                            }
                        }
                        if gone {
                            break;
                        }
                        out = Some(tx);
                        ctrl_open = false; // committed; no further control expected
                    }
                    None => ctrl_open = false, // session dropped without committing
                }
            }
            frame = frames.recv() => {
                match frame {
                    Some(b) => match &out {
                        Some(tx) => {
                            if tx.send(b).await.is_err() {
                                break; // ingest gone
                            }
                        }
                        None => {
                            preroll.push_back(b);
                            while preroll.len() > MAX_PREROLL_CHUNKS {
                                preroll.pop_front();
                            }
                        }
                    },
                    None => break, // capture dropped → mic stopped
                }
            }
        }
    }
    // On exit: a committed `out` (tx) drops here, ending the ingest's stream so it
    // finalizes; an uncommitted `preroll` drops, discarding the buffered audio.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tag_names_the_key_this_build_actually_listens_to() {
        let tag = attention_tag();
        assert!(
            tag.contains(crate::body::capabilities::hotkey::key_label()),
            "a person told to hold the wrong key holds the wrong key: {tag}"
        );
    }

    #[test]
    fn the_tag_does_not_offer_a_screen_the_agent_cannot_see() {
        // `hi_look` is deleted. This guards the specific sentence that outlived it —
        // a prompt naming a tool that is not there is the failure mode this repo has
        // shipped green before.
        let tag = attention_tag().to_lowercase();
        assert!(tag.contains("cannot see their screen"));
        assert!(!tag.contains("look at their screen if you need to"));
    }
}
