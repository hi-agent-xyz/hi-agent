//! How many messages the agent has sent since the person last sent one.
//!
//! **The conversation is read in one pass by someone coming back to it**, and everything the
//! agent sent since their last message is what they come back to. Measured on one install
//! (2026-08-26 to 09-17, typed, spoken and handed-over lines all counted as theirs): 71% of
//! 1,189 agent messages sat in runs of four or more, the longest 58, and one afternoon's 19
//! interleaved six subjects. See `docs/arch/legibility.md` § *F*.
//!
//! **The run is an input to the floor's reading, not a cap.** From 09-17 to 09-25 it was a cap
//! of three, enforced in `say`, and the floor made that cap the wrong shape: a floor holds the
//! answers to a run of questions and lets them out together, so four questions asked in a row
//! earned three answers and the fourth was refused — on 09-24 that lost the answer to a
//! question and a finished piece of work's result, and Reaction prepared the refused line again
//! six times over two hours. Now every release after the first since their last message is read
//! by System One with this count in front of it (`prepared.rs`, `judges/prepared.md` § *Say*).
//! **The count weighs only what nobody asked for.** An answer to something they asked is owed
//! however long the run — four questions in a row are owed four answers, and the first wording
//! of this rule, which let the count bear on everything, had the fallback reader hold the fourth
//! answer on "three already sent" (09-26). A progress note, an aside or a reminder is what the
//! run weighs against: the longer it is, the less such a message is worth reading now, and one
//! that is not waits for them — held, not refused.
//!
//! **What resets it is a message from the person landing in the conversation** — a typed
//! line, a settled spoken one, or a handed file, which are exactly the inputs that become
//! messages. Not an observation, a worker's report or a view: none of those is them.
//!
//! **It counts what the mouth accepted, which is not always what reached the list.** The
//! sequencer still drops two kinds after acceptance: speech from the warm-up prologue before
//! any turn, and the tail of a turn they barged in on. The second is followed by their own
//! line, which resets the run anyway.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::types::JournalEntry;

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

    /// How many messages have gone out since their last one. A message and each thing it hands
    /// over are one each — a picture in the conversation is read like a message
    /// (`docs/arch/showing.md` § *In the conversation*).
    pub(super) fn run(&self) -> u64 {
        self.run.load(Ordering::Acquire)
    }

    /// One message was accepted into the conversation.
    pub(super) fn note_sent(&self) {
        self.run.fetch_add(1, Ordering::AcqRel);
    }

    /// Take the run the conversation already ends in. Called as a loop stands up, before the
    /// mouth is registered, so a restart reads the run the person has not answered, not none.
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
    fn the_run_counts_what_went_out_and_a_person_empties_it() {
        let run = Unanswered::default();
        for _ in 0..5 {
            run.note_sent();
        }
        assert_eq!(run.run(), 5, "a count, not a cap: nothing here refuses the fourth");
        run.note_person();
        assert_eq!(run.run(), 0);
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
