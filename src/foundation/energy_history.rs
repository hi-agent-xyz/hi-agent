//! The energy level over time — what the account row alone cannot answer.
//!
//! [`crate::foundation::credentials::Energy`] holds one number: the balance as of the
//! last broker poll. That is enough to say "you are out", and nothing else. It cannot
//! say whether the day's energy went in one long job an hour ago or steadily since
//! morning, which is the first thing a person wants when the balance surprises them.
//!
//! So every poll that comes back with a balance is also appended here, and the
//! full-screen energy view draws the last day of it the way a laptop draws its battery.
//! The samples are exactly the polls — this records observations, it does not
//! interpolate or invent them. Two consequences worth keeping in mind:
//!
//!   - **Gaps are real, and stay in the data.** While the app is not running nothing is
//!     observed, so the series has holes, and no sample is ever invented to fill one:
//!     an absent bucket is absent. What the view does with that is presentation — it
//!     draws the curve across the hole dashed, because the level did exist and both
//!     ends of the hole are known, and a severed line reads as a broken chart rather
//!     than as "wasn't watching". The distinction is only available to it because the
//!     holes survive to here intact.
//!   - **Resolution is the poll cadence**, currently 60s ([`crate::foundation::broker`]),
//!     bucketed down to [`BUCKET`] for the wire.
//!
//! Retention is [`KEEP`]; pruning happens on every insert, so the table is bounded by
//! the window rather than by uptime.

use std::path::Path;

use chrono::{DateTime, Duration, Utc};

use crate::foundation::credentials::{self, Energy};

/// How far back samples are kept. Twice the served window, so a view opened at the
/// edge of a poll still has a full day behind it.
const KEEP: Duration = Duration::hours(48);
/// The window the view asks for.
pub const WINDOW: Duration = Duration::hours(24);
/// Wire resolution: one point per 10 minutes (144 across the window). Finer than this
/// is noise at a 60s poll cadence, and coarser hides a burst.
const BUCKET: Duration = Duration::minutes(10);

/// One point on the served series: the level as last observed inside that bucket.
#[derive(serde::Serialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Point {
    /// Bucket start, epoch milliseconds UTC.
    pub at: i64,
    pub remaining: i64,
    pub total: i64,
}

/// The served series, and the two numbers a caller needs to draw it without guessing:
/// how wide the window is and how far apart the points are. Both are decided here, so
/// a client that hardcoded them would silently mislabel its axis the day either moves.
#[derive(serde::Serialize, Debug)]
pub struct History {
    pub window_hours: i64,
    pub bucket_minutes: i64,
    pub points: Vec<Point>,
}

/// Append one observed balance. Called after a poll persists, so the series and the
/// account row can never disagree about the latest value. Best-effort: a failed write
/// costs one point on a chart and must never fail a poll.
pub fn record(data_dir: &Path, energy: &Energy) {
    // A balance with no ceiling is not an observation of anything — it is the
    // pre-bootstrap default. Recording it would draw a floor at zero for accounts that
    // simply haven't been told their tier yet.
    if energy.total <= 0 {
        return;
    }
    let now = Utc::now();
    if let Err(err) = credentials::record_energy_sample(
        data_dir,
        &now.to_rfc3339(),
        energy.remaining,
        energy.total,
        &energy.tier,
        &(now - KEEP).to_rfc3339(),
    ) {
        tracing::debug!(error = %format!("{err:#}"), "failed to record an energy sample");
    }
}

/// The last [`WINDOW`] of observations, bucketed for the wire.
pub fn recent(data_dir: &Path) -> History {
    let now = Utc::now();
    let samples = credentials::energy_samples_since(data_dir, &(now - WINDOW).to_rfc3339())
        .unwrap_or_else(|err| {
            tracing::debug!(error = %format!("{err:#}"), "failed to read energy samples");
            Vec::new()
        });
    summarize(&samples, now)
}

/// Bucket the raw samples for the wire. Pure, so the shape of the series is testable
/// without a store.
fn summarize(samples: &[(String, i64, i64)], now: DateTime<Utc>) -> History {
    let bucket_ms = BUCKET.num_milliseconds();
    let mut points: Vec<Point> = Vec::new();

    for (at, remaining, total) in samples {
        let Ok(at) = DateTime::parse_from_rfc3339(at.trim()) else {
            continue;
        };
        let at = at.with_timezone(&Utc).timestamp_millis();
        let bucket = at - at.rem_euclid(bucket_ms);
        let point = Point { at: bucket, remaining: *remaining, total: *total };
        // Last observation in the bucket wins: it is the level the bucket ended at.
        match points.last_mut() {
            Some(last) if last.at == bucket => *last = point,
            _ => points.push(point),
        }
    }

    History {
        window_hours: WINDOW.num_hours(),
        bucket_minutes: BUCKET.num_minutes(),
        points,
    }
    .ending_at(now)
}

impl History {
    /// Drop anything the caller's clock says is in the future (a sample written before
    /// a system-clock correction), so the view never has to draw past its own edge.
    fn ending_at(mut self, now: DateTime<Utc>) -> Self {
        let edge = now.timestamp_millis();
        self.points.retain(|p| p.at <= edge);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(now: DateTime<Utc>, minutes_ago: i64) -> String {
        (now - Duration::minutes(minutes_ago)).to_rfc3339()
    }

    /// A fixed instant sitting exactly on a [`BUCKET`] boundary.
    ///
    /// [`summarize`] buckets on the **absolute epoch** (`at.rem_euclid(bucket_ms)`), not
    /// relative to `now`. So a `now` read off the wall clock lands every relative offset
    /// below at an arbitrary position inside its bucket, and whether two samples share one
    /// is then a fact about what time the suite happened to run: the 65/62/61-minute
    /// samples span four minutes, so they share a bucket for six wall-clock minutes in ten
    /// and straddle a boundary for the other four. That is a test that fails 40% of the
    /// time for a reason having nothing to do with the code under it — measured, on an
    /// unmodified checkout: green at :38, :39 and :40, red at :41.
    ///
    /// Pinning it here leaves the bucket arithmetic as the only variable in the assertion,
    /// which is the thing these tests are actually about. Derived rather than written as a
    /// literal so the alignment is visible instead of being a constant to go verify.
    fn aligned_now() -> DateTime<Utc> {
        let secs = 1_700_000_000 - 1_700_000_000 % BUCKET.num_seconds();
        DateTime::from_timestamp(secs, 0).expect("a fixed epoch inside chrono's range")
    }

    #[test]
    fn samples_in_one_bucket_collapse_to_the_last_level() {
        let now = aligned_now();
        let samples = vec![
            (at(now, 65), 900_i64, 1000_i64),
            (at(now, 62), 800, 1000),
            (at(now, 61), 700, 1000),
        ];
        let history = summarize(&samples, now);
        assert_eq!(history.points.len(), 1, "one 10-minute bucket, one point");
        assert_eq!(history.points[0].remaining, 700, "the bucket ends where it ended");
    }

    #[test]
    fn a_reset_stays_in_the_series_as_a_rise() {
        let now = aligned_now();
        let samples = vec![
            (at(now, 200), 500_i64, 1000_i64),
            (at(now, 150), 100, 1000),
            (at(now, 100), 1000, 1000), // window reset
            (at(now, 50), 600, 1000),
        ];
        let levels: Vec<i64> = summarize(&samples, now).points.iter().map(|p| p.remaining).collect();
        assert_eq!(levels, vec![500, 100, 1000, 600], "the refill is drawn, not smoothed away");
    }

    #[test]
    fn a_gap_stays_a_gap() {
        let now = aligned_now();
        let samples = vec![
            (at(now, 600), 900_i64, 1000_i64),
            (at(now, 30), 400, 1000), // nothing observed for nine and a half hours
        ];
        let history = summarize(&samples, now);
        assert_eq!(history.points.len(), 2, "the missing hours are absent, not interpolated");
        let span = history.points[1].at - history.points[0].at;
        assert!(span > Duration::hours(9).num_milliseconds(), "the hole is left in the series");
    }

    #[test]
    fn nothing_observed_yet_is_an_empty_series_not_a_zero_line() {
        let history = summarize(&[], aligned_now());
        assert!(history.points.is_empty());
        assert_eq!(history.window_hours, 24, "the window is stated even with nothing in it");
    }
}
