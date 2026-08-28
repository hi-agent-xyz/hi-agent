//! Capabilities — the stable interface layer.
//!
//! Each capability (STT, TTS, vision, image generation, video generation) is an
//! **independent module of free functions** over a process-global,
//! once-initialized config. A capability function reads its global, picks the
//! configured vendor, and dispatches; the config is transparent to the caller
//! and never appears in a signature. The vendor impls live under
//! [`crate::foundation::vendors`].
//!
//! The capabilities are deliberately independent: no shared-vendor umbrella, no
//! cross-capability references. A vendor that happens to back several
//! capabilities is configured separately for each.
//!
//! [`init`] is the composition root for the keyed capabilities — it sequences
//! each one's own `init`, threading in every provider configured for it (from the
//! credential store, else its `.env` key). A key that won't build a config fails fast
//! at startup rather than as an error at first use; a wire no impl can speak is
//! skipped with a warning, because that list is the broker's to write, not ours.
//! The two recognition capabilities (voiceprint, face) are not env-configured — they run pinned local ONNX models that [`init_recognition`]
//! auto-provisions on first run (see [`crate::foundation::models`]), so they have no provider
//! toggle and nothing for the operator to set.
//!
//! [`accessibility`], [`audio_capture`], [`desktop_context`], [`hotkey`],
//! [`input`], [`screencast`], and [`tray`] are the exceptions to the env-config
//! pattern: their vendor is the operating system, selected at compile time, so they
//! have no `init` and do not appear in the composition root. [`view_render`] is the
//! same shape with a provisioned rather than compile-time vendor: its browser is
//! resolved lazily on first render (system, else a pinned managed build), so it
//! too has no `init` and nothing for the operator to set.

use crate::foundation::models;

pub mod accessibility;
pub mod audio_capture;
pub mod bundle;
pub mod desktop_context;
pub mod face;
pub mod hotkey;
pub mod image_gen;
pub mod input;
pub mod screencast;
pub mod stt;
pub mod tray;
pub mod tts;
pub mod video_gen;
pub mod view_render;
pub mod vision;
pub mod voiceprint;

/// Initialize the keyed capabilities (STT, TTS, vision, image/video gen) from the
/// credentials in effect for the current mode — the user's BYOK keys, or the
/// broker-minted bundle (xiaoyuanzhu) — falling back to `.env` per vendor. The
/// recognition capabilities are provisioned separately by [`init_recognition`].
///
/// **Every capability takes a list of providers — one per wire its source offers.**
/// A task is served over several wires (`text-to-image` over both `openai-images` and
/// `openai-responses`), and which HTTP shapes we can actually speak is knowledge that
/// lives in the capability, so the choice is made there rather than upstream. Each
/// capability maps the credential vocabulary into its own `ProviderSpec` — no shared
/// spec type across capabilities, by the same rule as everything else here.
///
/// Where the caller names a model (image, video), the model chooses the wire and all
/// of them stay live at once. Where it does not (STT, TTS, vision), the capability
/// holds the first wire it can speak and logs the rest.
pub fn init(creds: &crate::foundation::credentials::Credentials) -> anyhow::Result<()> {
    use crate::foundation::credentials::VendorKey;
    let eff = creds.effective();
    let none: &[VendorKey] = &[];
    let (stt_wires, tts_wires, vision_wires, image_wires, video_wires) = match eff.as_ref() {
        Some(e) => (e.stt, e.tts, e.vision, e.image, e.video),
        None => (none, none, none, none, none),
    };

    stt::init(
        stt_wires
            .iter()
            .map(|v| stt::ProviderSpec {
                wire: v.wire_opt().map(str::to_owned),
                base_url: v.base_url_opt().map(str::to_owned),
                api_key: v.key_opt().unwrap_or_default().to_owned(),
                model: v.model_opt().map(str::to_owned),
            })
            .collect(),
    )?;
    tts::init(
        tts_wires
            .iter()
            .map(|v| tts::ProviderSpec {
                wire: v.wire_opt().map(str::to_owned),
                base_url: v.base_url_opt().map(str::to_owned),
                api_key: v.key_opt().unwrap_or_default().to_owned(),
            })
            .collect(),
    )?;
    vision::init(
        vision_wires
            .iter()
            .map(|v| vision::ProviderSpec {
                wire: v.wire_opt().map(str::to_owned),
                base_url: v.base_url_opt().map(str::to_owned),
                api_key: v.key_opt().unwrap_or_default().to_owned(),
                model: v.model_opt().map(str::to_owned),
            })
            .collect(),
    )?;
    image_gen::init(
        image_wires
            .iter()
            .map(|v| image_gen::ProviderSpec {
                wire: v.wire_opt().map(str::to_owned),
                base_url: v.base_url_opt().map(str::to_owned),
                api_key: v.key_opt().unwrap_or_default().to_owned(),
                carrier: v.carrier_opt().map(str::to_owned),
                default_model: v.model_opt().map(str::to_owned),
                models: v
                    .models
                    .iter()
                    .map(|m| image_gen::ModelInfo {
                        name: m.name.clone(),
                        quality: m.quality,
                        speed: m.speed,
                        price: m.price,
                    })
                    .collect(),
            })
            .collect(),
    )?;
    video_gen::init(
        video_wires
            .iter()
            .map(|v| video_gen::ProviderSpec {
                wire: v.wire_opt().map(str::to_owned),
                base_url: v.base_url_opt().map(str::to_owned),
                api_key: v.key_opt().unwrap_or_default().to_owned(),
                default_model: v.model_opt().map(str::to_owned),
                models: v
                    .models
                    .iter()
                    .map(|m| video_gen::ModelInfo {
                        name: m.name.clone(),
                        quality: m.quality,
                        speed: m.speed,
                        price: m.price,
                    })
                    .collect(),
            })
            .collect(),
    )?;
    Ok(())
}

/// Provision and load the local recognition models (voiceprint + face) — pinned
/// ONNX fetched into the OS cache on first run, reused thereafter. **Best-effort
/// and never fatal**: if a model can't be provisioned or loaded (offline first
/// run, mirror down, a bad pin), that capability stays disabled for this launch
/// and the agent runs without it — the same as any unconfigured capability. The
/// failure is logged server-side; there is nothing for the operator to fix.
///
/// The two models the face capability needs are fetched concurrently; voiceprint
/// runs alongside. Already-cached runs are effectively instant.
pub async fn init_recognition() {
    let (voice, scrfd, arcface) = tokio::join!(
        models::ensure(&models::CAMPLUS),
        models::ensure(&models::SCRFD),
        models::ensure(&models::ARCFACE),
    );

    match voice {
        Ok(path) => {
            if let Err(err) = voiceprint::init(path).await {
                tracing::error!(error = %format!("{err:#}"), "voiceprint model loaded but failed to init; capability disabled");
            }
        }
        Err(err) => tracing::warn!(error = %format!("{err:#}"), "voiceprint model unavailable; capability disabled"),
    }

    match (scrfd, arcface) {
        (Ok(s), Ok(a)) => {
            if let Err(err) = face::init(s, a).await {
                tracing::error!(error = %format!("{err:#}"), "face models loaded but failed to init; capability disabled");
            }
        }
        (s, a) => {
            let err = s.err().or(a.err()).map(|e| e.to_string()).unwrap_or_default();
            tracing::warn!(error = %err, "face models unavailable; capability disabled");
        }
    }
}
