//! What a restarted mind can reconstruct from the log alone.
//!
//! The log's job is not bookkeeping — it is so a mind that has just come back can
//! tell what already happened without asking. That only works if the outbound half
//! is there too: a reconstruction holding one side of the conversation will happily
//! say the same thing again, and show the same view again. So this writes a mixed
//! run of everything that can cross the conversation — the person typing, the agent's worded
//! reply, that reply going out as speech, a view put on screen, a check-in coming due,
//! cognition reporting back — and reads it out through the snapshot path a fresh session is
//! actually opened with.

use chrono::{DateTime, Duration, Utc};
use hi_agent::mind::memory::{Memory, build};
use hi_agent::types::{Channel, JournalEntry, Origin};
use tempfile::tempdir;

fn signal_in(
    id: &str,
    channel: Channel,
    origin: Origin,
    ts: DateTime<Utc>,
    body: &str,
) -> JournalEntry {
    hi_agent::mind::memory::journal::legacy_signal_in(id.into(), ts, channel, body.into(), None, None, Some(origin), None)
}

fn signal_out(id: &str, channel: Channel, ts: DateTime<Utc>, body: &str) -> JournalEntry {
    hi_agent::mind::memory::journal::legacy_signal_out(id.into(), ts, channel, body.into(), None, Some(Origin::Reaction))
}

/// The position of `needle` in `haystack`, failing the test with the whole
/// reconstruction in hand when it isn't there — a missing line is the interesting
/// case, so print what we did get.
fn find(haystack: &str, needle: &str) -> usize {
    match haystack.find(needle) {
        Some(at) => at,
        None => panic!("reconstruction is missing {needle:?}\n---\n{haystack}\n---"),
    }
}

#[tokio::test]
async fn a_fresh_session_reads_back_what_was_said_and_shown() {
    let dir = tempdir().expect("tempdir");
    let mem = Memory::open(dir.path()).await.expect("memory");

    // Inside the snapshot's recent window, in the order they happened. Ids are
    // lexically ordered so the cross-channel merge is deterministic.
    let t0 = Utc::now() - Duration::minutes(10);
    let at = |m: i64| t0 + Duration::minutes(m);

    for entry in [
        signal_in("0001", Channel::Text, Origin::Human, at(0), "how did Q3 land?"),
        signal_out("0002", Channel::Text, at(1), "pulling the numbers now."),
        signal_out("0003", Channel::Audio, at(1), "spoke the reply aloud (audio/mpeg, 18432 bytes)"),
        signal_in(
            "0004",
            Channel::Clock,
            Origin::Host,
            at(3),
            "(check-in) You've been quiet 3m while your own thinking runs",
        ),
        signal_in(
            "0005",
            Channel::Worker,
            Origin::Worker,
            at(5),
            "cognition finished — task \"how did Q3 land?\": revenue up 12%",
        ),
        signal_out("0006", Channel::View, at(6), "showed \"q3-numbers\" (/views/_compiled/abcd1234.mjs)"),
    ] {
        mem.journal.append(entry).await.expect("append");
    }

    // The reconstruction a fresh reaction session is handed.
    let snap = build(&mem).await.expect("snapshot");
    assert_eq!(snap.recent_entries.len(), 6, "every channel is read back, none dropped");
    let recon = snap.render_for_prompt();

    // What was said: the person's line, the agent's worded reply, and the fact
    // that the reply was actually voiced (a turn with no TTS writes no such row).
    let asked = find(&recon, ">how did Q3 land?");
    let replied = find(&recon, "<pulling the numbers now.");
    let spoke = find(&recon, "</audio spoke the reply aloud");

    // Why a turn happened at all, when nobody said anything: the host's own clock,
    // and its own thinking coming back.
    let woken = find(&recon, ">/clock (check-in) You've been quiet 3m");
    let thought = find(&recon, ">/worker cognition finished");

    // What was shown — the half that used to leave no trace at all, so a restart
    // would put the same view up a second time.
    let shown = find(&recon, "</view showed \"q3-numbers\"");
    assert!(
        recon.contains("/views/_compiled/abcd1234.mjs"),
        "the module hash identifies which view was up:\n{recon}"
    );

    // Order is the only clock a reconstruction has (the transcript carries no
    // timestamps), so it has to survive the cross-channel merge.
    assert!(
        asked < replied && replied < spoke && spoke < woken && woken < thought && thought < shown,
        "reconstruction is out of order:\n{recon}"
    );
}

/// Nothing about the new rows changes how the old ones are stored or found: they
/// land in the sealed per-channel day-log layout like any other signal.
#[tokio::test]
async fn the_new_channels_use_the_same_on_disk_layout() {
    use hi_agent::mind::memory::layout;

    let dir = tempdir().expect("tempdir");
    let mem = Memory::open(dir.path()).await.expect("memory");
    let ts = Utc::now();

    mem.journal
        .append(signal_out("0001", Channel::View, ts, "showed \"card\""))
        .await
        .expect("append");
    mem.journal
        .append(signal_in("0002", Channel::Clock, Origin::Host, ts, "(check-in) quiet"))
        .await
        .expect("append");

    for channel in [Channel::View, Channel::Clock] {
        let log = layout::channel_log_path(mem.data_dir(), channel, ts);
        assert!(log.exists(), "{} log written at {log:?}", channel.as_str());
        assert!(
            log.ends_with(format!("{0}/{1}/{0}.jsonl", channel.as_str(), layout::day_key(ts))),
            "self-describing channel-day path, got {log:?}"
        );
    }
}
