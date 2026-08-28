//! Episode frontmatter is agent-written prose, and the forgetting pass steers on it.
//!
//! `from_ts`/`to_ts` are typed by a model into a file it owns, then read by code it
//! does not run, to decide which cold days may have their full-fidelity bytes dropped
//! (`docs/arch/data.md#reading-back-across-the-pen-line`). These tests pin the reading
//! half of that rule: a value that does not parse is **absent**, and absent never
//! widens what the forgetting pass is willing to touch.

use std::fs;
use std::path::Path;

use hi_agent::mind::memory::episodes::{episode_from_dates, names_overlapping_day};
use hi_agent::mind::memory::layout;
use tempfile::tempdir;

/// Write one episode bundle by hand — the way a worker with a shell does, which is
/// how the malformed one that froze consolidation for thirty-five hours got there.
fn write_episode(data_dir: &Path, name: &str, from_ts: &str, to_ts: &str) {
    let dir = layout::episodes_dir(data_dir).join(name);
    fs::create_dir_all(&dir).expect("episode dir");
    fs::write(
        dir.join("episode.md"),
        format!(
            "---\ntitle: \"{name}\"\nfrom_ts: \"{from_ts}\"\nto_ts: \"{to_ts}\"\nkind: reflection\n---\n\nGist.\n"
        ),
    )
    .expect("episode.md");
}

#[tokio::test]
async fn unparseable_span_never_claims_to_cover_a_day() {
    let dir = tempdir().expect("tempdir");
    let data_dir = dir.path();

    write_episode(data_dir, "2026-08-26-real", "2026-08-26T03:00:00+00:00", "2026-08-26T03:16:02+00:00");
    // Prose where a timestamp belonged. The old reader sliced ten bytes instead of
    // parsing: `"yesterday"` is nine, so the slice was `None` and `unwrap_or("")` made
    // the lower bound the empty string — which is `<=` every date. The upper bound
    // then compared `"2026-08-26" <= "sometime l"`, true because `'2' < 's'`. Both
    // bounds passed and the episode was reported as covering a day it says nothing about.
    write_episode(data_dir, "2026-08-26-handwritten", "yesterday", "sometime later");

    let names = names_overlapping_day(data_dir, "2026-08-26").await.expect("scan");
    assert_eq!(names, vec!["2026-08-26-real".to_string()], "only the episode with a readable span covers the day");
}

#[tokio::test]
async fn a_readable_span_still_covers_its_own_days_and_no_others() {
    let dir = tempdir().expect("tempdir");
    let data_dir = dir.path();
    write_episode(data_dir, "2026-08-24-spanning", "2026-08-24T22:00:00+00:00", "2026-08-26T01:00:00+00:00");

    for day in ["2026-08-24", "2026-08-25", "2026-08-26"] {
        let got = names_overlapping_day(data_dir, day).await.expect("scan");
        assert_eq!(got.len(), 1, "{day} falls inside the span");
    }
    for day in ["2026-08-23", "2026-08-27"] {
        let got = names_overlapping_day(data_dir, day).await.expect("scan");
        assert!(got.is_empty(), "{day} falls outside the span, got {got:?}");
    }
}

#[tokio::test]
async fn burial_depth_counts_only_dates_it_could_read() {
    let dir = tempdir().expect("tempdir");
    let data_dir = dir.path();

    write_episode(data_dir, "2026-08-20-a", "2026-08-20T09:00:00+00:00", "2026-08-20T09:30:00+00:00");
    write_episode(data_dir, "2026-08-22-b", "2026-08-22T09:00:00+00:00", "2026-08-22T09:30:00+00:00");
    write_episode(data_dir, "2026-08-26-handwritten", "yesterday", "sometime later");

    // The forgetting pass weighs a cold day by how many episodes began after it. An
    // unreadable date contributes nothing rather than a ten-byte prefix of prose:
    // fewer dates means less burial depth, means less pressure to fade — the
    // recoverable direction for a pass that drops bytes.
    let dates = episode_from_dates(data_dir).await.expect("scan");
    assert_eq!(dates, vec!["2026-08-20".to_string(), "2026-08-22".to_string()]);
}

#[tokio::test]
async fn the_day_is_the_utc_day_the_raw_store_files_by() {
    let dir = tempdir().expect("tempdir");
    let data_dir = dir.path();
    // 01:00 on the 26th in +08:00 is still the 25th in UTC, and the raw day-folders
    // these dates are compared against are UTC. Parsing is what makes the offset
    // visible at all; a prefix slice would have read the local date and matched the
    // wrong folder.
    write_episode(data_dir, "2026-08-26-shanghai", "2026-08-26T01:00:00+08:00", "2026-08-26T02:00:00+08:00");

    assert_eq!(episode_from_dates(data_dir).await.expect("scan"), vec!["2026-08-25".to_string()]);
    assert_eq!(names_overlapping_day(data_dir, "2026-08-25").await.expect("scan").len(), 1);
    assert!(names_overlapping_day(data_dir, "2026-08-26").await.expect("scan").is_empty());
}
