//! How many messages the agent has sent since the person last sent one, and the cap on it.
//!
//! **The conversation is read in one pass by someone coming back to it**, and everything the
//! agent sent since their last message is what they come back to. Measured on one install
//! (2026-08-26 to 09-17, typed, spoken and handed-over lines all counted as theirs): 71% of
//! 1,189 agent messages sat in runs of four or more, the longest 58, and one afternoon's 19
//! interleaved six subjects — progress, corrections of corrections, and news already
//! overtaken by the next message. Every one of those subjects had its own task row carrying
//! where it got to. `reaction.md` already said one quiet word beats a string of pings, and
//! the runs are what that rule produced on its own. See `docs/arch/legibility.md` § *F*.
//!
//! So `say` refuses a message past [`MAX_UNANSWERED`], the way it refuses one too long to be a
//! message: a fact about the text's place in the conversation, not a judgment of its words.
//! What goes in the messages that do go out, and where the rest lives, is `reaction.md`'s.
//!
//! **It has no backstop and no exemption.** The floor lets a reply through after repeated
//! refusals because silence there is the failure; here the messages already sent are the
//! opposite of silence, and a cap that a fourth attempt could pass is not a cap. Something
//! that needs the person is not exempt either — a flag the writer sets for itself would be
//! set on everything — and it is not lost: it is a `waiting` line on its row, drawn on the
//! row's card, and the first thing worth saying when they are next in the conversation.
//!
//! **What resets it is a message from the person landing in the conversation** — a typed
//! line, a settled spoken one, or a handed file, which are exactly the inputs that become
//! messages. Not an observation, a worker's report or a view: none of those is them.
//!
//! **It counts what the mouth accepted, which is not always what reached the list.** The
//! sequencer still drops two kinds after acceptance: speech from the warm-up prologue before
//! any turn, and the tail of a turn they barged in on. The second is followed by their own
//! line, which resets the run anyway. The first is a fault on its own, and it costs one slot
//! until they next write.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::types::JournalEntry;

/// How many messages go out before the person sends one. The starting value the person asked
/// for; the replay set and their corrections settle it (`legibility.md` § *Open*).
pub(super) const MAX_UNANSWERED: u64 = 3;

/// The run since the person's last message. One per process, shared by the input side that
/// resets it and the mouth that counts it.
#[derive(Default)]
pub(super) struct Unanswered {
    run: AtomicU64,
}

impl Unanswered {
    /// A message from the person landed. The run starts over.
    pub(super) fn note_person(&self) {
        self.run.store(0, Ordering::Release);
    }

    /// Whether the next message would go past the cap. Asked by the mouth under its serial
    /// lock, so two messages in one turn cannot both read the same count.
    pub(super) fn is_full(&self) -> bool {
        self.run.load(Ordering::Acquire) >= MAX_UNANSWERED
    }

    /// One message was accepted into the conversation.
    pub(super) fn note_sent(&self) {
        self.run.fetch_add(1, Ordering::AcqRel);
    }

    /// Take the run the conversation already ends in. Called as a loop stands up, before the
    /// mouth is registered, so a restart cannot hand out a fresh allowance on top of a run the
    /// person has not answered.
    pub(super) fn seed(&self, run: u64) {
        self.run.store(run, Ordering::Release);
    }
}

/// The agent's messages after the person's last one, read from the journal — the same
/// entries, in the same order, the conversation list is seeded from.
pub(super) fn trailing_run(entries: &[JournalEntry]) -> u64 {
    entries
        .iter()
        .rev()
        .filter_map(|entry| match entry {
            JournalEntry::Message { message, .. } => Some(message.from.is_agent()),
            _ => None,
        })
        .take_while(|agent| *agent)
        .count() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Author, Channel, Content, Message, Sender};

    fn message(from: Author) -> JournalEntry {
        JournalEntry::Message {
            channel: Channel::Text,
            message: Message {
                id: uuid::Uuid::now_v7().to_string(),
                ts: chrono::Utc::now(),
                from,
                content: Content::Text("x".into()),
                task: None,
            },
        }
    }

    fn person() -> JournalEntry {
        message(Author::Person(Sender::unknown()))
    }

    fn agent() -> JournalEntry {
        message(Author::Agent)
    }

    #[test]
    fn the_run_fills_at_the_cap_and_a_person_empties_it() {
        let run = Unanswered::default();
        for _ in 0..MAX_UNANSWERED {
            assert!(!run.is_full());
            run.note_sent();
        }
        assert!(run.is_full());
        run.note_person();
        assert!(!run.is_full());
    }

    #[test]
    fn a_restart_inherits_only_what_came_after_their_last_message() {
        let entries = [agent(), person(), agent(), agent()];
        assert_eq!(trailing_run(&entries), 2);
        assert_eq!(trailing_run(&[person()]), 0);
        assert_eq!(trailing_run(&[]), 0);
        // A conversation the journal window holds none of their lines in is all run.
        assert_eq!(trailing_run(&[agent(), agent(), agent(), agent()]), 4);
    }
}
