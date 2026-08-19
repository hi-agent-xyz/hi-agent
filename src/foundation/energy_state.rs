//! Process-wide "is the managed account out of energy right now?" flag.
//!
//! In xiaoyuanzhu (managed) mode the whole account draws on one shared budget, so
//! **any** 402 means the same thing — the LLM (songguo), STT/TTS, or vision all
//! signal "out of energy" — and a later positive balance is the recovery signal.
//! This flag collects those signals so the vendor gate can raise the out-of-energy
//! view the instant we notice, from whichever source noticed first. It also broadcasts
//! the two lifecycle edges to that gate:
//!   - [`EnergyEvent::Pause`] — an observed managed 402.
//!   - [`EnergyEvent::Resume`] — a fetched balance with energy again.
//! A zero balance never preflights or suppresses a call: providers run normally until
//! one actually returns 402. This matters when the serving layer has an override or the
//! balance cache lags reality.
//! In BYOK a 402 is the user's own vendor account, not our energy — so [`note_402`]
//! is a no-op there. The observed managed pause is persisted so a restart cannot forget
//! the condition while restoring its retained view. One process, one account, one event
//! bus — a global keeps the live wiring trivial.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use crate::foundation::credentials::{Credentials, Mode};
use tokio::sync::broadcast;

static OUT_OF_ENERGY: AtomicBool = AtomicBool::new(false);
const KEY_OBSERVED_PAUSE: &str = "managed_energy_observed_pause";

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

fn persist(data_dir: &Path, out: bool) {
    if let Err(err) = crate::foundation::credentials::set_setting(
        data_dir,
        KEY_OBSERVED_PAUSE,
        if out { "true" } else { "" },
    ) {
        tracing::warn!(
            error = %format!("{err:#}"),
            out_of_energy = out,
            "failed to persist managed energy state"
        );
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

/// Restore the last observed managed 402 before the startup broker refresh. A
/// successful positive refresh immediately clears it through [`reconcile`]; if the
/// broker is unreachable, the process keeps holding work and showing the retained
/// view instead of guessing that a restart fixed the account.
pub fn restore(data_dir: &Path) {
    let managed = matches!(Credentials::load(data_dir).mode, Mode::Xiaoyuanzhu);
    let observed = crate::foundation::credentials::get_setting(data_dir, KEY_OBSERVED_PAUSE)
        .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "on"));
    if !managed && observed {
        persist(data_dir, false);
    }
    set(managed && observed);
}

/// A 402 from any managed capability (LLM / STT / TTS / vision …) → out of energy,
/// but only in xiaoyuanzhu mode; a BYOK 402 is the user's own vendor account. Raises
/// the flag immediately so the gate doesn't wait for the next balance poll. `data_dir`
/// is read to check the mode and persist the observed condition across restarts.
pub fn note_402(data_dir: &Path) -> bool {
    if matches!(Credentials::load(data_dir).mode, Mode::Xiaoyuanzhu) {
        let was = is_out();
        persist(data_dir, true);
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
pub fn reconcile(data_dir: &Path, remaining: i64, total: i64) -> Option<EnergyEvent> {
    if is_out() && total > 0 && remaining > 0 {
        persist(data_dir, false);
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
/// on the transition into the paused state.
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
    use std::sync::Mutex;

    static TEST_STATE: Mutex<()> = Mutex::new(());

    #[test]
    fn only_a_standalone_402_is_an_energy_edge() {
        assert!(is_402_error(&anyhow::anyhow!("API Error: 402 budget exceeded")));
        assert!(is_402_error(&anyhow::anyhow!("gateway returned 402")));
        assert!(!is_402_error(&anyhow::anyhow!("request id 1140228 timed out")));
        assert!(!is_402_error(&anyhow::anyhow!("connection reset")));
    }

    #[test]
    fn balance_is_recovery_only() {
        let _guard = TEST_STATE.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        set(false);
        assert_eq!(
            reconcile(dir.path(), 0, 100),
            None,
            "zero balance must not preflight calls"
        );
        assert!(!is_out());

        set(true);
        assert_eq!(
            reconcile(dir.path(), 0, 100),
            None,
            "empty balance keeps an observed 402 paused"
        );
        assert!(is_out());
        assert_eq!(
            reconcile(dir.path(), 1, 100),
            Some(EnergyEvent::Resume)
        );
        assert!(!is_out());
    }

    #[test]
    fn observed_pause_survives_a_process_restart() {
        let _guard = TEST_STATE.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();

        set(false);
        assert!(note_402(dir.path()));
        assert!(is_out());

        // Simulate a fresh process: the atomic is gone, the config DB remains.
        set(false);
        assert!(!is_out());
        restore(dir.path());
        assert!(is_out());

        set(false);
        persist(dir.path(), false);
    }

    #[test]
    fn positive_balance_clears_the_durable_pause() {
        let _guard = TEST_STATE.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();

        set(false);
        assert!(note_402(dir.path()));
        assert_eq!(
            reconcile(dir.path(), 5, 100),
            Some(EnergyEvent::Resume)
        );
        assert!(!is_out());

        restore(dir.path());
        assert!(!is_out(), "the cleared pause must not return after restart");
    }
}
