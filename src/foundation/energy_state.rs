//! Process-wide "is the managed account out of energy right now?" flag.
//!
//! In xiaoyuanzhu (managed) mode the whole account draws on one shared budget, so
//! **any** 402 means the same thing — the LLM (songguo), STT/TTS, or vision all
//! signal "out of energy" — and a later positive balance is the recovery signal.
//! This flag collects those signals so the web app can raise the out-of-energy hint
//! the instant we notice, from whichever source noticed first. It also broadcasts the
//! two lifecycle edges to the live agent loops:
//!   - [`EnergyEvent::Pause`] — an observed managed 402.
//!   - [`EnergyEvent::Resume`] — a fetched balance with energy again.
//! A zero balance never preflights or suppresses a call: providers run normally until
//! one actually returns 402. This matters when the serving layer has an override or the
//! balance cache lags reality.
//! In BYOK a 402 is the user's own vendor account, not our energy — so [`note_402`]
//! is a no-op there. The `/api/account/energy` handler reads [`is_out`]. One process,
//! one account, one event bus — a global keeps the wiring trivial.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use crate::foundation::credentials::{Credentials, Mode};
use tokio::sync::broadcast;

static OUT_OF_ENERGY: AtomicBool = AtomicBool::new(false);

/// The only cross-loop messages in the energy lifecycle. Agents do not ask for the
/// balance before a model call. A failed call raises `Pause`; a later positive balance
/// raises `Resume`, and the held loops continue from their mailboxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnergyEvent {
    Pause,
    Resume,
}

fn events() -> &'static broadcast::Sender<EnergyEvent> {
    static EVENTS: OnceLock<broadcast::Sender<EnergyEvent>> = OnceLock::new();
    EVENTS.get_or_init(|| {
        let (tx, _) = broadcast::channel(32);
        tx
    })
}

fn set(out: bool) {
    let was = OUT_OF_ENERGY.swap(out, Ordering::Relaxed);
    if was != out {
        let event = if out { EnergyEvent::Pause } else { EnergyEvent::Resume };
        // A receiver may have gone away with its session; the state flag remains the
        // source of truth for late starters, so a lagged/empty bus is not fatal.
        let _ = events().send(event);
    }
}

/// Whether the managed account is currently out of energy (turns are held, not
/// dropped, and the paid capabilities will 402).
pub fn is_out() -> bool {
    OUT_OF_ENERGY.load(Ordering::Relaxed)
}

/// Subscribe a live agent loop to the process-wide pause/resume lifecycle.
pub fn subscribe() -> broadcast::Receiver<EnergyEvent> {
    events().subscribe()
}

/// A 402 from any managed capability (LLM / STT / TTS / vision …) → out of energy,
/// but only in xiaoyuanzhu mode; a BYOK 402 is the user's own vendor account. Raises
/// the flag immediately so the hint doesn't wait for the next balance poll. `data_dir`
/// is read to check the mode.
pub fn note_402(data_dir: &Path) -> bool {
    if matches!(Credentials::load(data_dir).mode, Mode::Xiaoyuanzhu) {
        let was = is_out();
        set(true);
        return !was;
    }
    false
}

/// Reconcile a paused account against a freshly fetched managed balance.
///
/// This path is deliberately recovery-only: empty or unknown balances do nothing.
/// Agents do not use account data as a preflight; only an actual managed 402 raises
/// `Pause`. Once a refresh reports positive energy, this emits `Resume`.
pub fn reconcile(remaining: i64, total: i64) -> Option<EnergyEvent> {
    if is_out() && total > 0 && remaining > 0 {
        set(false);
        return Some(EnergyEvent::Resume);
    }
    None
}

/// Detect the upstream's standard HTTP 402 after a provider boundary has flattened it
/// into an opaque `anyhow` message. The shared parser keeps each capability boundary
/// from inventing its own substring rule.
pub fn is_402_error(err: &anyhow::Error) -> bool {
    is_402_text(&format!("{err:#}"))
}

/// The string form used by boundaries that already have an upstream error message.
pub(crate) fn is_402_text(text: &str) -> bool {
    let needle = "402";
    text.match_indices(needle).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + needle.len()..].chars().next();
        before.map_or(true, |c| !c.is_ascii_digit())
            && after.map_or(true, |c| !c.is_ascii_digit())
    })
}

/// Raise the managed pause when a provider boundary reports 402. Returns `true` only
/// on the transition into the paused state, so a UI/view notice can be announced once.
pub fn note_402_error(data_dir: &Path, err: &anyhow::Error) -> bool {
    if is_402_error(err) {
        note_402(data_dir)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_standalone_402_is_an_energy_edge() {
        assert!(is_402_error(&anyhow::anyhow!("API Error: 402 budget exceeded")));
        assert!(is_402_error(&anyhow::anyhow!("gateway returned 402")));
        assert!(!is_402_error(&anyhow::anyhow!("request id 1140228 timed out")));
        assert!(!is_402_error(&anyhow::anyhow!("connection reset")));
    }

    #[test]
    fn balance_is_recovery_only() {
        set(false);
        assert_eq!(reconcile(0, 100), None, "zero balance must not preflight calls");
        assert!(!is_out());

        set(true);
        assert_eq!(reconcile(0, 100), None, "empty balance keeps an observed 402 paused");
        assert!(is_out());
        assert_eq!(reconcile(1, 100), Some(EnergyEvent::Resume));
        assert!(!is_out());
    }
}
