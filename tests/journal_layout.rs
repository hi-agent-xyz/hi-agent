//! The raw store writes the sealed channel-first layout and merges channels on
//! read. Exercises the `Journal`/`store_blob` API directly — the audio HTTP path
//! needs STT configured, which these tests deliberately avoid.

use chrono::{DateTime, TimeZone, Utc};
use hi_agent::mind::memory::layout::{self, MediaSlot};
use hi_agent::mind::memory::media::store_blob;
use hi_agent::mind::memory::Memory;
use hi_agent::types::{Channel, JournalEntry, Media, Origin};
use tempfile::tempdir;

fn signal_in(id: &str, channel: Channel, ts: DateTime<Utc>, body: &str, media: Option<Media>) -> JournalEntry {
    hi_agent::mind::memory::journal::legacy_signal_in(id.into(), ts, channel, body.into(), None, media, Some(Origin::Human), None)
}

fn signal_out(id: &str, channel: Channel, ts: DateTime<Utc>, body: &str) -> JournalEntry {
    hi_agent::mind::memory::journal::legacy_signal_out(id.into(), ts, channel, body.into(), None, Some(Origin::Reaction))
}

fn id_of(e: &JournalEntry) -> &str {
    hi_agent::mind::memory::journal::entry_id(e)
}

#[tokio::test]
async fn appends_route_by_channel_and_recent_merges_in_ts_id_order() {
    let dir = tempdir().expect("tempdir");
    let mem = Memory::open(dir.path()).await.expect("memory");

    let t_typed = Utc.with_ymd_and_hms(2026, 6, 13, 10, 0, 0).unwrap();
    let t_pair = Utc.with_ymd_and_hms(2026, 6, 13, 10, 5, 0).unwrap();

    // Append out of order and across channels; one ts is shared so the uuidv7
    // `id` tiebreak is exercised (id "0002" before "0003" at the same instant).
    let audio_media = Media { file: "10/05-00.mp3".into(), mime: "audio/mpeg".into(), duration_ms: None, width: None, height: None };
    mem.journal.append(signal_in("0003", Channel::Audio, t_pair, "spoken", Some(audio_media))).await.unwrap();
    mem.journal.append(signal_out("0002", Channel::Text, t_pair, "reply")).await.unwrap();
    mem.journal.append(signal_in("0001", Channel::Text, t_typed, "typed", None)).await.unwrap();

    // Logs land under per-channel, per-day folders named for the channel.
    let text_log = layout::channel_log_path(mem.data_dir(), Channel::Text, t_typed);
    let audio_log = layout::channel_log_path(mem.data_dir(), Channel::Audio, t_pair);
    assert!(text_log.ends_with("text/2026-06-13/text.jsonl"), "text log at {text_log:?}");
    assert!(audio_log.ends_with("audio/2026-06-13/audio.jsonl"), "audio log at {audio_log:?}");
    assert!(text_log.exists() && audio_log.exists(), "both channel logs written");

    // recent() merges all channels by (ts, id): typed first, then the same-ts
    // pair in id order (text out 0002 before audio in 0003).
    let since = Utc.with_ymd_and_hms(2026, 6, 13, 9, 0, 0).unwrap();
    let got = mem.journal.recent(since, 10).await.unwrap();
    let ids: Vec<&str> = got.iter().map(id_of).collect();
    assert_eq!(ids, ["0001", "0002", "0003"], "merged in (ts,id) order");
}

#[tokio::test]
async fn legacy_reactor_origin_loads_as_reaction() {
    let dir = tempdir().expect("tempdir");
    let mem = Memory::open(dir.path()).await.expect("memory");
    let ts = Utc.with_ymd_and_hms(2026, 6, 13, 10, 5, 0).unwrap();
    let log = layout::channel_log_path(mem.data_dir(), Channel::Text, ts);

    tokio::fs::create_dir_all(log.parent().unwrap()).await.unwrap();
    let legacy = serde_json::json!({
        "kind": "signal_out",
        "id": "legacy",
        "ts": ts,
        "channel": "text",
        "body": "reply",
        "origin": "reactor",
    });
    tokio::fs::write(&log, format!("{legacy}\n")).await.unwrap();

    let got = mem
        .journal
        .recent(ts - chrono::Duration::minutes(1), 10)
        .await
        .unwrap();
    assert_eq!(got.len(), 1, "legacy row is not skipped");
    match &got[0] {
        JournalEntry::Message { message, .. } => {
            assert!(message.from.is_agent(), "the legacy outbound row is the agent's own")
        }
        other => panic!("expected the outbound legacy row, got {other:?}"),
    }

    // Re-serialized under the shape the system stores now: which *mind* produced a
    // line is no longer a field on a message, because a message has two ends and
    // `from` names the one that matters. The alias it was testing still does its job
    // one layer up — the legacy `reactor` line above loaded at all, and loaded as the
    // agent's own rather than as somebody's words.
    let encoded = serde_json::to_string(&got[0]).unwrap();
    assert!(encoded.contains(r#""from":"agent""#), "{encoded}");
    assert!(!encoded.contains("reactor"), "{encoded}");
}

#[tokio::test]
async fn store_blob_writes_relative_grid_path() {
    let dir = tempdir().expect("tempdir");
    let mem = Memory::open(dir.path()).await.expect("memory");
    let ts = Utc.with_ymd_and_hms(2026, 6, 13, 9, 16, 45).unwrap();

    let rel = store_blob(mem.data_dir(), Channel::Audio, ts, MediaSlot::InputOneOff, "mp3", b"xxxx")
        .await
        .unwrap();
    // A one-off input blob is `<HH>/<MM>-<SS>.<ext>`, relative to the channel-day.
    assert_eq!(rel, "09/16-45.mp3");
    let abs = layout::channel_day_dir(mem.data_dir(), Channel::Audio, ts).join(&rel);
    assert!(abs.exists(), "blob written at {abs:?}");
}
