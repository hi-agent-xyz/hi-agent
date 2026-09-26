//! Host-wide, best-effort telemetry. No processes, commands, model turns, or history.
//! A read takes a short measurement window, shared by simultaneous readers and cached
//! for two seconds. With no readers there is no sampling task. OS telemetry belongs
//! here rather than the permission-bearing app mechanisms: it needs no desktop grants.
//! Platform telemetry paths still need live macOS/Windows verification; sensor support
//! is best-effort and must not be treated as a guaranteed scheduling signal.
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use sysinfo::{Components, Networks, System};

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub sampled_at: String,
    pub platform: &'static str,
    pub scope: &'static str,
    pub sample_seconds: f64,
    pub cpu_percent: Option<f32>,
    pub logical_cpus: usize,
    pub memory_total_bytes: Option<u64>,
    pub memory_used_bytes: Option<u64>,
    pub swap_used_bytes: Option<u64>,
    pub networks: Vec<Network>,
    pub temperatures: Vec<Temperature>,
    pub batteries: Vec<Battery>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Network {
    pub name: String,
    pub received_bytes_per_second: f64,
    pub transmitted_bytes_per_second: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Temperature {
    pub label: String,
    pub celsius: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Battery {
    pub percent: Option<f32>,
    pub state: String,
    /// Battery energy flow, NOT whole-machine wall power or cumulative energy usage.
    pub energy_rate_watts: Option<f32>,
}

static CACHE: OnceLock<Mutex<Option<(Instant, Snapshot)>>> = OnceLock::new();

/// Blocking OS reads: callers on an async runtime must use its blocking pool.
/// Cache locking also coalesces simultaneous requests into the same sample.
pub fn snapshot() -> Snapshot {
    let mut cache = CACHE.get_or_init(|| Mutex::new(None)).lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((at, value)) = cache.as_ref() {
        if at.elapsed() < Duration::from_secs(2) {
            return value.clone();
        }
    }
    let value = collect();
    *cache = Some((Instant::now(), value.clone()));
    value
}

fn collect() -> Snapshot {
    let mut system = System::new();
    system.refresh_cpu_usage();
    let mut networks = Networks::new_with_refreshed_list();
    let before: std::collections::HashMap<_, _> = networks.iter()
        .map(|(name, n)| (name.clone(), (n.total_received(), n.total_transmitted())))
        .collect();
    let start = Instant::now();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.max(Duration::from_millis(250)));
    system.refresh_cpu_usage();
    networks.refresh(true);
    let seconds = start.elapsed().as_secs_f64();
    system.refresh_memory();
    let mut traffic: Vec<_> = networks.iter().filter_map(|(name, n)| {
        let (rx, tx) = before.get(name)?;
        // Removed/new interfaces and reset counters are not zero-traffic samples.
        Some(Network {
            name: name.clone(),
            received_bytes_per_second: rate(*rx, n.total_received(), seconds)?,
            transmitted_bytes_per_second: rate(*tx, n.total_transmitted(), seconds)?,
        })
    }).collect();
    traffic.sort_by(|a, b| a.name.cmp(&b.name));
    let components = Components::new_with_refreshed_list();
    let temperatures = components.iter().filter_map(|c| {
        let celsius = c.temperature().filter(|v| v.is_finite())?;
        Some(Temperature { label: c.label().to_owned(), celsius })
    }).collect();
    let batteries = battery::Manager::new().ok().and_then(|m| {
        m.batteries().ok().map(|items| items.filter_map(Result::ok).map(|b| Battery {
            percent: finite(b.state_of_charge().value * 100.0),
            state: match b.state() {
                battery::State::Charging => "charging",
                battery::State::Discharging => "discharging",
                battery::State::Full => "full",
                battery::State::Empty => "empty",
                _ => "unknown",
            }.to_owned(),
            energy_rate_watts: finite(b.energy_rate().value),
        }).collect())
    }).unwrap_or_default();
    let supported = sysinfo::IS_SUPPORTED_SYSTEM;
    let total = system.total_memory();
    Snapshot {
        sampled_at: chrono::Utc::now().to_rfc3339(),
        platform: std::env::consts::OS,
        scope: "host",
        sample_seconds: seconds,
        cpu_percent: if supported && !system.cpus().is_empty() {
            finite(system.global_cpu_usage())
        } else { None },
        logical_cpus: system.cpus().len(),
        memory_total_bytes: (supported && total > 0).then_some(total),
        memory_used_bytes: (supported && total > 0).then_some(system.used_memory()),
        swap_used_bytes: (supported && total > 0).then_some(system.used_swap()),
        networks: traffic,
        temperatures,
        batteries,
    }
}

fn finite(value: f32) -> Option<f32> {
    value.is_finite().then_some(value)
}

fn rate(before: u64, after: u64, seconds: f64) -> Option<f64> {
    if !seconds.is_finite() || seconds <= 0.0 { return None; }
    after.checked_sub(before).map(|delta| delta as f64 / seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_rates_use_elapsed_time_and_reject_reset_counters() {
        assert_eq!(rate(100, 600, 0.25), Some(2000.0));
        assert_eq!(rate(600, 100, 0.25), None);
        assert_eq!(rate(100, 100, 1.0), Some(0.0));
        assert_eq!(rate(100, 600, 0.0), None);
    }

    #[test]
    fn samples_are_serializable_and_shared_by_immediate_readers() {
        let first = snapshot();
        let next = snapshot();
        assert_eq!(first.sampled_at, next.sampled_at);
        assert!(first.sample_seconds >= 0.25);
        assert!(first.cpu_percent.is_none_or(|n| (0.0..=100.0).contains(&n)));
        assert!(serde_json::to_value(first).is_ok());
    }
}
