//! Speech-to-text capability — audio bytes in, text out.
//!
//! The capability is a module of free functions over a process-global,
//! once-initialized config: [`init`] resolves the vendor + credentials from the
//! config store into the global, [`available`] reports whether a provider is
//! configured, and [`transcribe`] / [`transcribe_streaming`] dispatch to it. The
//! config never appears in a signature — it is transparent to the caller. An
//! uninitialized global (or no stored key) means "not configured".

use std::sync::OnceLock;

use bytes::Bytes;
use tokio::sync::mpsc;

use crate::foundation::vendors::volcengine_stt;

/// One transcript update from a streaming recognition.
///
/// The upstream is two-pass: it emits a fast, rolling preliminary text
/// (`is_final = false`) that keeps changing as you speak, then a polished,
/// punctuated/ITN-corrected final (`is_final = true`) for the utterance.
#[derive(Debug, Clone)]
pub struct Transcript {
    pub text: String,
    pub is_final: bool,
    /// Vendor speaker-cluster label for a finalized utterance when diarization is
    /// on (two-pass mode) — e.g. `"0"`, `"1"`. `None` on rolling partials and when
    /// speaker info is off. It is a within-session cluster id, not a persistent
    /// identity; the caller resolves identity by voiceprinting the utterance audio.
    pub speaker_id: Option<String>,
    /// Diarized utterance spans carried on a final, one per speaker turn that
    /// finalized in this update (empty on partials and when diarization is off).
    /// Each gives a speaker label and the utterance's `[start_ms, end_ms]` from the
    /// stream start, so the caller can slice that speaker's *own* audio out of the
    /// live buffer and voiceprint it — instead of attributing a whole multi-speaker
    /// stretch to one label. Distinct from [`Self::speaker_id`], which names only
    /// the single turn the dispatched sentence belongs to.
    pub segments: Vec<DiarizedSpan>,
}

/// A finalized utterance's speaker and audio span (milliseconds from stream start),
/// the unit the voiceprint path slices and embeds per speaker.
#[derive(Debug, Clone)]
pub struct DiarizedSpan {
    pub speaker_id: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

enum Backend {
    /// Why nothing is configured, in the words a person could act on.
    Disabled(String),
    Volcengine(volcengine_stt::Config),
}

static BACKEND: OnceLock<Backend> = OnceLock::new();

/// The default wire when the source names none — the only STT impl today.
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
    pub model: Option<String>,
}

/// Resolve the STT backend into the process-global config, from **every provider the
/// source offers, best first**.
///
/// Transcription takes no model argument, so there is nothing for a caller to select
/// on: this holds the first provider whose wire it can actually speak, and the list is
/// what makes a second wire a match arm rather than a refactor.
///
/// A wire we have no impl for is skipped with a warning, never fatal — the broker names
/// wires in its own vocabulary (`volc-asr-stream-async`) and changes it without asking
/// us, and passing that id through cost a boot the first time it was tried. A provider
/// whose *config* won't build still is fatal: that is a broken setting, not an
/// unfamiliar name. Adding a vendor is a new `Backend` variant plus a match arm here.
/// Idempotent — the first init wins.
pub fn init(providers: Vec<ProviderSpec>) -> anyhow::Result<()> {
    let (chosen, skipped) = select(&providers);
    let backend = match chosen {
        Some(i) => Backend::Volcengine(volcengine_stt::Config::from_store(
            Some(&providers[i].api_key),
            providers[i].base_url.as_deref(),
            providers[i].model.as_deref(),
        )?),
        None if skipped.is_empty() => Backend::Disabled("set an STT key in Settings".to_string()),
        None => {
            tracing::warn!(wires = ?skipped, "no STT impl for any offered wire; speech input is off");
            Backend::Disabled(format!("no impl for the offered wire(s): {}", skipped.join(", ")))
        }
    };
    let _ = BACKEND.set(backend);
    Ok(())
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
        match p.wire.as_deref().map(str::trim).filter(|w| !w.is_empty()).unwrap_or(DEFAULT_WIRE) {
            "volcengine" => return (Some(i), skipped),
            other => skipped.push(other.to_string()),
        }
    }
    (None, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(wire: Option<&str>, key: &str) -> ProviderSpec {
        ProviderSpec {
            wire: wire.map(str::to_owned),
            api_key: key.into(),
            ..Default::default()
        }
    }

    /// The list is offered best-first, so the first wire we can speak wins — and a wire
    /// we cannot is stepped over rather than taken as the answer.
    #[test]
    fn the_first_speakable_wire_wins_and_the_rest_are_named() {
        let (chosen, skipped) =
            select(&[spec(Some("volc-asr-stream-async"), "k"), spec(Some("volcengine"), "k")]);
        assert_eq!(chosen, Some(1));
        assert_eq!(skipped, vec!["volc-asr-stream-async"]);

        // An empty wire means "the default", which is one we speak.
        assert_eq!(select(&[spec(None, "k")]).0, Some(0));
        assert_eq!(select(&[spec(Some("  "), "k")]).0, Some(0));
    }

    /// A keyless provider is an ordinary state (BYOK with nothing pasted yet), not a
    /// wire we failed to speak — it must not turn into a complaint about wires.
    #[test]
    fn a_keyless_provider_is_skipped_silently() {
        let (chosen, skipped) = select(&[spec(Some("volcengine"), "   ")]);
        assert_eq!(chosen, None);
        assert!(skipped.is_empty(), "nothing to report: there was no key to try");
    }
}

/// Whether a provider is configured. Callers gate on this and respond with
/// "capability unavailable" (e.g. 501) when it is false.
pub fn available() -> bool {
    matches!(BACKEND.get(), Some(Backend::Volcengine(_)))
}

/// Why the capability is off, for the error text at the point of use.
fn why_off() -> &'static str {
    match BACKEND.get() {
        Some(Backend::Disabled(reason)) => reason.as_str(),
        _ => "set an STT key in Settings",
    }
}

/// Transcribe `audio` (raw bytes) labeled with the given IANA mime type
/// (e.g. `audio/wav`, `audio/mpeg`). The vendor handles any format-specific
/// framing.
pub async fn transcribe(audio: Bytes, mime: &str) -> anyhow::Result<String> {
    match BACKEND.get() {
        Some(Backend::Volcengine(cfg)) => volcengine_stt::transcribe(cfg, audio, mime).await,
        _ => anyhow::bail!("STT not configured ({})", why_off()),
    }
}

/// Streaming transcription. Consumes 16 kHz mono 16-bit little-endian PCM
/// chunks from `audio_rx` until it closes, emitting incremental and final
/// [`Transcript`]s on `out` as the upstream produces them. Returns the final
/// transcript text. `out` send errors (receiver gone) are non-fatal;
/// recognition continues so the final can still be returned for journaling.
pub async fn transcribe_streaming(
    audio_rx: mpsc::Receiver<Bytes>,
    out: mpsc::Sender<Transcript>,
) -> anyhow::Result<String> {
    match BACKEND.get() {
        Some(Backend::Volcengine(cfg)) => {
            volcengine_stt::transcribe_streaming(cfg, audio_rx, out).await
        }
        _ => anyhow::bail!("STT not configured ({})", why_off()),
    }
}
