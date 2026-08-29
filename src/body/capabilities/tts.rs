//! Text-to-speech capability — text in, streamed audio out.
//!
//! Synthesis is a *streaming session*, not a one-shot call: [`start`] opens a
//! session, the caller pushes text incrementally as the agent produces it, and
//! audio frames stream back as they are synthesized. One session spans a whole
//! turn, so the audio is one continuous stream rather than a sequence of
//! per-sentence clips — the brain consolidates, the client just plays.
//!
//! The capability is a module of free functions over a process-global,
//! once-initialized config: [`init`] resolves the vendor from the config store,
//! [`available`] reports whether a provider is configured, and [`start`]
//! dispatches to it. The config never appears in a signature.

use std::path::Path;
use std::sync::OnceLock;

use bytes::Bytes;
use tokio::sync::mpsc;

use crate::foundation::vendors::volcengine_tts;

/// A live synthesis session. Feed text via [`text`](Self::text) as it becomes
/// available; drop the sender to signal end-of-input (the provider flushes any
/// trailing audio and closes). Drain [`frames`](Self::frames) for the audio
/// bytes; the receiver closes when synthesis ends or the session errors.
///
/// Every frame shares the same [`mime`](Self::mime) — it is fixed for the life
/// of the session and known the moment the session opens, so the HTTP layer can
/// set `Content-Type` before the first byte.
pub struct TtsStream {
    /// IANA mime type for every frame in this stream, e.g. `audio/mpeg`.
    pub mime: String,
    /// Push text to be spoken. Send each chunk as it arrives; dropping the
    /// sender signals that no more text is coming.
    pub text: mpsc::Sender<String>,
    /// Synthesized audio frames, in order. Closes when synthesis completes.
    pub frames: mpsc::Receiver<Bytes>,
}

enum Backend {
    /// Why nothing is configured, in the words a person could act on.
    Disabled(String),
    Volcengine(volcengine_tts::Config),
}

static BACKEND: OnceLock<Backend> = OnceLock::new();

/// The default wire when the source names none — the only TTS impl today.
const DEFAULT_WIRE: &str = "volcengine";

/// One provider offered for this capability, as the credential store or the broker
/// describes it. Its own type, not one shared with the other capabilities: a vendor
/// that happens to back several is configured separately for each.
#[derive(Debug, Clone, Default)]
pub struct ProviderSpec {
    /// Wire id, in the source's own vocabulary. `None`/empty → [`DEFAULT_WIRE`].
    pub wire: Option<String>,
    pub base_url: Option<String>,
    pub api_key: String,
}

/// Resolve the TTS backend into the process-global config, from **every provider the
/// source offers, best first**.
///
/// Synthesis takes no model argument, so there is nothing for a caller to select on:
/// this holds the first provider whose wire it can actually speak, and the list is what
/// makes a second wire a match arm rather than a refactor.
///
/// A wire we have no impl for is skipped with a warning, never fatal — the broker names
/// wires in its own vocabulary and changes it without asking us, and a menu we half
/// understand must not cost a boot. A provider whose *config* won't build still is
/// fatal: that is a broken setting, not an unfamiliar name. Adding a vendor is a new
/// `Backend` variant plus a match arm here. Idempotent — the first init wins.
pub fn init(providers: Vec<ProviderSpec>) -> anyhow::Result<()> {
    let (chosen, skipped) = select(&providers);
    let backend = match chosen {
        Some(i) => Backend::Volcengine(volcengine_tts::Config::from_store(
            Some(&providers[i].api_key),
            providers[i].base_url.as_deref(),
        )?),
        None if skipped.is_empty() => Backend::Disabled("set a TTS key in Settings".to_string()),
        None => {
            tracing::warn!(wires = ?skipped, "no TTS impl for any offered wire; speech output is off");
            Backend::Disabled(format!("no impl for the offered wire(s): {}", skipped.join(", ")))
        }
    };
    let _ = BACKEND.set(backend);
    Ok(())
}

/// Whether an explicit wire id names something [`volcengine_tts`] can speak.
///
/// Loose on purpose, for the reason spelled out in [`crate::body::capabilities::stt`]:
/// the broker names the **protocol** (`volc-tts-bidirectional`) where this capability
/// names the **vendor** (`volcengine`), and that bidirectional V3 socket is exactly the
/// one impl we have.
fn speakable(wire: &str) -> bool {
    let w = wire.trim().to_ascii_lowercase();
    w.contains("volc") || w.contains("bytedance")
}

/// The provider to use, and the wires passed over on the way to it.
///
/// Split out of [`init`] so the rule can be tested: the backend is a write-once process
/// global, and a test that had to install one could only ever run first, alone.
fn select(providers: &[ProviderSpec]) -> (Option<usize>, Vec<String>) {
    let mut skipped = Vec::new();
    for (i, p) in providers.iter().enumerate() {
        if p.api_key.trim().is_empty() {
            continue;
        }
        let wire =
            p.wire.as_deref().map(str::trim).filter(|w| !w.is_empty()).unwrap_or(DEFAULT_WIRE);
        if speakable(wire) {
            return (Some(i), skipped);
        }
        skipped.push(wire.to_string());
    }
    (None, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list is offered best-first, so the first wire we can speak wins — and a wire
    /// we cannot is stepped over rather than taken as the answer.
    #[test]
    fn the_first_speakable_wire_wins_and_the_rest_are_named() {
        let spec = |wire: Option<&str>, key: &str| ProviderSpec {
            wire: wire.map(str::to_owned),
            api_key: key.into(),
            ..Default::default()
        };
        let (chosen, skipped) =
            select(&[spec(Some("some-new-tts"), "k"), spec(Some("volcengine"), "k")]);
        assert_eq!(chosen, Some(1));
        assert_eq!(skipped, vec!["some-new-tts"]);
        assert_eq!(select(&[spec(None, "k")]).0, Some(0), "no wire named → the default");
        assert!(select(&[spec(Some("volcengine"), " ")]).1.is_empty(), "no key, no complaint");
        assert_eq!(
            select(&[spec(Some("volc-tts-bidirectional"), "k")]).0,
            Some(0),
            "the broker's own spelling of the socket we speak"
        );
    }
}

/// Whether a provider is configured.
pub fn available() -> bool {
    matches!(BACKEND.get(), Some(Backend::Volcengine(_)))
}

/// Why the capability is off, for the error text at the point of use.
fn why_off() -> &'static str {
    match BACKEND.get() {
        Some(Backend::Disabled(reason)) => reason.as_str(),
        _ => "set a TTS key in Settings",
    }
}

/// Open a streaming synthesis session. Returns once the session is ready to
/// accept text; synthesis is driven by pushing text and draining frames.
pub async fn start(data_dir: &Path) -> anyhow::Result<TtsStream> {
    match BACKEND.get() {
        Some(Backend::Volcengine(cfg)) => volcengine_tts::start(cfg, data_dir).await,
        _ => anyhow::bail!("TTS not configured ({})", why_off()),
    }
}
