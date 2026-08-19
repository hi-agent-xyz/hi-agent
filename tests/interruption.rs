//! Interruption semantics — placeholder, requires a live `codex app-server`.
//!
//! Contract (see `src/reaction/mod.rs` § "Fix-forward, no reflexive cancel"):
//! a new POST arriving while the reaction queue is already running a turn does
//! NOT interrupt the in-flight turn. The reaction loop is serial, so the
//! new signal simply queues and is folded into the next turn; the warm session
//! remembers what it has already heard, so a thought spread across bursts
//! reassembles across turns. The mind corrects course rather than being cut off
//! (fix-forward); the client mutes its own speaker on a hot mic, so an
//! interruption still feels instant.
//!
//! The same holds for a voice barge-in — the human talking over the agent's
//! playback. Nothing is cancelled and nothing new crosses the wire: the client
//! ducks its own speaker when the shared text appearance gains an `interim`;
//! the words buffer and fold into the next turn like any other signal, and the
//! backend infers from its own clock that its voice was cut — recording a "what
//! went unheard" note the next prompt carries. The settled human line becomes
//! the current text appearance immediately, and later text from the older turn
//! is excluded even though the turn itself completes. See
//! `src/reaction/interrupts.rs` (unit-tested there).
//!
//! **This is about Reaction's own turn, and `hi_cancel_worker` does not contradict it.**
//! Two different situations. Here, a second signal arrives while the reaction is
//! mid-thought and nothing should be cut: the person is still talking, the turn folds
//! their words in, and cancelling would only make the agent forget half of what it just
//! heard. There, the person has withdrawn work a *worker* is grinding through in the
//! background, where folding in is impossible — nothing re-reads its mail until the turn
//! ends, so the only alternative to interrupting is letting the cancelled work finish and
//! land. Reflexive cancel of a turn someone is speaking into: still no. Deliberate cancel
//! of an errand nobody wants any more: that is what the tool is for, and it is never
//! automatic — a rung has to decide it and name the session.
//!
//! Driving this for real requires either:
//!
//!   (a) A live `codex app-server` subprocess. Tests would
//!       become integration-grade: slow, flaky, machine-dependent.
//!   (b) A mock wire backend swapped in via a trait. That's a v1-grade
//!       refactor of `src/foundation/codex/` — too much surgery for a docs/tests step.
//!
//! We pick neither. The shell-equivalent verification is below; run it by hand
//! after `cargo build --release && ./target/release/hi-agent`:
//!
//! ```sh
//! # in one terminal, watch the text day-log
//! tail -F data/memory/raw/text/*/text.jsonl
//!
//! # in another, fire two POSTs in rapid succession
//! BASE=http://127.0.0.1:12358
//! curl -X POST \
//!     --data-binary 'first thought, take your time' "$BASE/api/in/text" &
//! sleep 0.2
//! curl -X POST \
//!     --data-binary 'actually never mind, what time is it' "$BASE/api/in/text"
//! ```
//!
//! Expected: tracing logs show NO `turn/interrupt`; the first
//! turn runs to completion; the journal shows both SignalIn entries; and the
//! second signal is folded into a later turn (the warm session already carries
//! the first).

#[tokio::test]
#[ignore = "requires codex on PATH; see file header for the shell-equivalent recipe"]
async fn new_post_does_not_cancel_in_flight_turn() {
    // Stub: see file header.
}
