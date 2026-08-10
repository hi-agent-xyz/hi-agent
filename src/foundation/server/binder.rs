//! The transport adapter — binds the reaction's transport-free outbound
//! vocabulary to the HTTP wire.
//!
//! The reaction is the mind; it emits [`OutboundSignal`]s in human-channel terms
//! ("said this text", "this span of speech", "show this view") and knows
//! nothing about HTTP. Everything HTTP-shaped lives on this side of the seam:
//! the current-state projection of /out/text, the `Content-Type` and turn binding
//! of /out/audio, the broadcast of /out/view. This binder is the one place
//! that translates between the two, so swapping HTTP for another wire touches
//! only this file — the reaction and its vocabulary are untouched.
//!
//! It runs as a single task draining the reaction's outbound channel in order,
//! which keeps the mind's signals serialized exactly as it produced them.

use chrono::Utc;
use tokio::sync::{broadcast, mpsc};

use crate::body::reaction::OutboundSignal;
use crate::foundation::server::{AudioEvent, OutputEcho, TextAppearance, ViewBus, ViewEvent};
use crate::types::Channel;

/// Drain the reaction's outbound seam and bind each signal to its HTTP carrier.
/// Owns the producing halves of the wire-side broadcasts; runs until the reaction
/// drops `out_tx` (process teardown).
pub(crate) async fn bind_outbound(
    mut rx: mpsc::Receiver<OutboundSignal>,
    text: TextAppearance,
    audio_out: broadcast::Sender<AudioEvent>,
    views: ViewBus,
    view_out: broadcast::Sender<ViewEvent>,
    output_echo: broadcast::Sender<OutputEcho>,
) {
    while let Some(signal) = rx.recv().await {
        match signal {
            // /out/text is one backend-owned current state. A settled human line
            // supersedes any reaction turn that started before it, so trailing
            // output from that turn cannot reclaim the current appearance.
            OutboundSignal::TextTurnStart { turn } => {
                text.begin_reaction_turn(turn);
            }
            OutboundSignal::Text { chunk } => {
                // Echo a live copy for the inspector alongside the authoritative
                // current-state update.
                let _ = output_echo.send(OutputEcho {
                    channel: Channel::Text,
                    text: chunk.clone(),
                    is_final: false,
                    ts: Utc::now(),
                });
                text.push_agent_chunk(chunk);
            }
            OutboundSignal::TextEnd => {
                let _ = output_echo.send(OutputEcho {
                    channel: Channel::Text,
                    text: String::new(),
                    is_final: true,
                    ts: Utc::now(),
                });
                text.end_agent_utterance();
            }
            // /audio: one utterance's span is one chunked response. The codec
            // becomes the response's Content-Type, set before the first byte;
            // `turn` keeps a handler's response bound to a single utterance so a
            // later span's frames never bleed into an earlier response.
            OutboundSignal::AudioBegin { turn, codec } => {
                let _ = audio_out.send(AudioEvent::Start { turn, mime: codec });
            }
            OutboundSignal::AudioFrame { turn, bytes } => {
                let _ = audio_out.send(AudioEvent::Frame { turn, bytes });
            }
            OutboundSignal::AudioEnd { turn } => {
                let _ = audio_out.send(AudioEvent::End { turn });
            }
            // /view: fold the envelope into the retained appearance
            // state (what GET /out/view serves), and echo a non-draining copy
            // to the channel inspector's broadcast tap.
            OutboundSignal::View { envelope } => {
                let _ = view_out.send(ViewEvent {
                    envelope: envelope.clone(),
                    ts: Utc::now(),
                });
                views.apply(envelope).await;
            }
        }
    }
    tracing::info!("outbound binder: reaction seam closed; exiting");
}
