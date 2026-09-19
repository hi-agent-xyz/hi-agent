//! Native microphone capture — raw 16 kHz mono PCM straight from the OS, no browser.
//!
//! Inbound speech normally arrives over the WebSocket from a page's `getUserMedia`.
//! This captures the mic **in-process** instead, so a headless gesture — the
//! press-and-hold-⌘ attention ([`crate::body::gesture`]) — can listen with no page open.
//! The frames it yields match the pipeline's contract exactly (16 kHz mono signed
//! 16-bit little-endian PCM), so they feed
//! [`crate::foundation::server::audio::ingest_pcm_stream`] the same as the browser mic.
//!
//! Like [`super::hotkey`] and [`super::tray`] the vendor is chosen at compile time
//! rather than configured; unlike them it is not the operating system but **cpal**,
//! which is CoreAudio and WASAPI behind one call
//! ([`crate::foundation::vendors::cpal_audio_capture`]). So "which platforms have a
//! mic" is a question about which ones we build cpal for, not about how many
//! implementations exist. On a platform this reports unavailable for, the gesture's
//! listen half is simply inert.

use bytes::Bytes;
use tokio::sync::mpsc;

/// A live capture. **Dropping it stops the mic** and ends the frame stream, which
/// lets a downstream [`ingest_pcm_stream`](crate::foundation::server::audio::ingest_pcm_stream)
/// finalize on its own.
///
/// The `cfg` here and in [`start`] is the same pair as the `cpal` dependency's own
/// target block in `Cargo.toml`, and has to be: cpal reaches further (ALSA, and more)
/// but a capture with no gesture to open it would be weight in the Docker image.
pub struct Capture {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    _vendor: crate::foundation::vendors::cpal_audio_capture::Capture,
}

/// Whether this build can capture the mic natively. Compile-time, not a permission
/// check — a macOS build still needs the **Microphone** grant for frames to flow, and
/// a Windows one still answers to the system-wide microphone privacy switch.
pub fn available() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// Start capturing the default input device as 16 kHz mono 16-bit PCM. Returns a
/// [`Capture`] (drop to stop) and the receiver of frames. Errs when capture is
/// unavailable on this platform or the device can't be opened.
pub fn start() -> anyhow::Result<(Capture, mpsc::Receiver<Bytes>)> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let (vendor, frames) = crate::foundation::vendors::cpal_audio_capture::start()?;
        Ok((Capture { _vendor: vendor }, frames))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        anyhow::bail!("native mic capture is not supported on this platform")
    }
}
