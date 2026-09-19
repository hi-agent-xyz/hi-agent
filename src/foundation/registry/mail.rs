//! What one session said to another — kept long enough to be read back.
//!
//! [`Registry::send`](super::Registry::send) is the one agent-to-agent verb, and until now
//! it was **write-only from the outside**: the text went into the recipient's mailbox, was
//! taken out by [`take_pending`](super::Registry::take_pending), rendered into that
//! session's next prompt, and after that the only trace of it was buried in two different
//! frame logs — as an `hi_send_message` tool call in the sender's, and as a paragraph of
//! the prompt in the recipient's. So "what have these two been saying to each other" was a
//! question you answered by reading two transcripts side by side and matching text by eye.
//!
//! This is the small ring that answers it directly. Every delivered `send` is appended
//! here, and [`Registry::traffic_between`](super::Registry::traffic_between) reads back one
//! pair's half of it, both directions, oldest first — which is what the arrow between two
//! cards on the sessions page opens.
//!
//! **The ring is capped and in memory; `mail.jsonl` beside it is neither.** The ring is the
//! working set a page poll can afford to read; the file is what survives the process, and it
//! seeds the ring at boot.
//!
//! The ring was memory-only on the argument that a restart emptied the roster it draws arrows
//! between at the same instant, so no arrow was left pointing at a history it could have kept.
//! **Reopening made that false.** An errand the host ended is reopened under its own slug and
//! its own owner ([`crate::body::reaction::reopen_interrupted`]), so both cards are back on the
//! page and the arrow between two sessions that have been talking for an hour would open onto
//! nothing.
//!
//! The file answers a second question the ring never could, and it is the one that costs a
//! message rather than a page: **what was delivered and never read.** A `read` line is appended
//! whenever an inbox is drained, so `sent` minus `read` for a session is exactly what was
//! sitting unread when the process stopped — and that is restored to the session when it is
//! reopened, because the sender was told `Delivered` and a mailbox that quietly discards what
//! it accepted is worse than one that refuses.
//!
//! Nothing refused is recorded. `NotPermitted` and `Unknown` never reached a mailbox, so
//! they are not communication between the two sessions; they are a sender's mistake, and
//! the place that shows them is the sender's own transcript, where the tool call and its
//! error sit together.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{Message, SessionSlug};
use crate::mind::memory::layout;

/// How many delivered messages the ring keeps, process-wide.
///
/// Sized for reading, not for retention: a pair's conversation over a working afternoon is
/// tens of messages, and what a reader wants from the arrow is the recent stretch of it.
pub const KEPT: usize = 300;

/// The longest single message the ring stores, in characters.
///
/// A worker's report can be a page. Storing the whole of every one of them makes the
/// working set unbounded in the one dimension the count above does not cover, so a long
/// message is clipped here and says so. The whole of it is in the recipient's frame log,
/// in the prompt it became.
pub const TEXT_CHARS: usize = 4_000;

/// One delivered message, as the ring keeps it.
#[derive(Debug, Clone)]
pub struct Sent {
    pub at: DateTime<Utc>,
    pub from: SessionSlug,
    pub to: SessionSlug,
    pub text: String,
    /// Whether `text` is the whole of what was sent, or the first [`TEXT_CHARS`] of it.
    pub clipped: bool,
}

impl Sent {
    pub fn new(from: SessionSlug, to: SessionSlug, text: &str) -> Self {
        let kept: String = text.chars().take(TEXT_CHARS).collect();
        let clipped = kept.len() < text.len();
        Self { at: Utc::now(), from, to, text: kept, clipped }
    }

    /// Whether this message is one of the two directions between `a` and `b`.
    ///
    /// Unordered on purpose: an arrow joins two sessions, and a conversation read from one
    /// end only is not a conversation. `a == b` matches nothing — a session cannot
    /// `send` to itself through the switchboard, and answering as though it could would
    /// draw its own mail as a dialogue.
    pub fn between(&self, a: &SessionSlug, b: &SessionSlug) -> bool {
        a != b && ((&self.from == a && &self.to == b) || (&self.from == b && &self.to == a))
    }
}

/// Append `sent`, dropping the oldest once the ring is full.
pub fn push(ring: &mut std::collections::VecDeque<Sent>, sent: Sent) {
    if ring.len() == KEPT {
        ring.pop_front();
    }
    ring.push_back(sent);
}

// ── the durable half ──────────────────────────────────────────────────────────

/// How much of the tail to read at boot. Bounded for the same reason the session index's is:
/// an install that has been up for months must not pay a whole file to seed a page.
const SEED_TAIL_BYTES: u64 = 1024 * 1024;

/// `<memory>/raw/sessions/mail.jsonl` — beside the session index it is read with.
pub fn mail_path(data_dir: &Path) -> PathBuf {
    layout::raw_root(data_dir).join(layout::SESSIONS_DIR).join("mail.jsonl")
}

/// One line in the mail log.
///
/// **Two events, not one, and the second is what makes the first answerable.** A `sent` line
/// alone says a message was delivered to a mailbox; it cannot say whether anyone ever took it
/// out. `read` is appended by [`Registry::take_pending`](super::Registry::take_pending), which
/// drains an inbox whole and in order — so a fold that pops `count` from the front of each
/// session's queue is exact, and what is left is genuinely unread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Record {
    Sent {
        run: String,
        at: DateTime<Utc>,
        /// Absent for a host post — see [`Registry::post`](super::Registry::post). Recorded
        /// even so, which the in-memory ring does not do: the ring exists to draw an arrow
        /// between two sessions and a host post has no other end, but an unread host post is
        /// as owed to its recipient as any other.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<SessionSlug>,
        to: SessionSlug,
        text: String,
    },
    Read {
        run: String,
        at: DateTime<Utc>,
        to: SessionSlug,
        count: u32,
    },
}

pub fn sent_record(from: Option<&SessionSlug>, to: &SessionSlug, text: &str) -> Record {
    Record::Sent {
        run: crate::foundation::run::id().to_string(),
        at: Utc::now(),
        from: from.cloned(),
        to: to.clone(),
        // The same clip the ring applies, for the same reason: one page-long report must not
        // make the file unbounded in the dimension nothing else covers.
        text: text.chars().take(TEXT_CHARS).collect(),
    }
}

pub fn read_record(to: &SessionSlug, count: u32) -> Record {
    Record::Read {
        run: crate::foundation::run::id().to_string(),
        at: Utc::now(),
        to: to.clone(),
        count,
    }
}

/// What the sessions that served the task `subject` sent, oldest first — their own account of
/// the work, which is what a task's record is read against when it closes
/// (`docs/arch/legibility.md` § N).
///
/// **Joined by the session index, never by the text.** A report does not name its task; the
/// session that sent it was opened against one, and [`super::index`] recorded which on the way
/// in. Sessions are keyed by `(run, session)` because slugs repeat every boot. Kept to the newest
/// `budget` characters: the reader wants how it ended more than how it began.
pub async fn sent_by_sessions_serving(
    data_dir: &Path,
    subject: &str,
    budget: usize,
) -> Vec<(DateTime<Utc>, String)> {
    let index = tokio::fs::read_to_string(super::index::index_path(data_dir)).await.unwrap_or_default();
    let serving: std::collections::HashSet<(String, SessionSlug)> = index
        .lines()
        .filter(|l| l.contains(subject))
        .filter_map(|l| serde_json::from_str::<super::index::Record>(l).ok())
        .filter_map(|r| match r {
            super::index::Record::Opened { run, session, subject: Some(s), .. } if s == subject => {
                Some((run, session))
            }
            _ => None,
        })
        .collect();
    if serving.is_empty() {
        return Vec::new();
    }
    let log = tokio::fs::read_to_string(mail_path(data_dir)).await.unwrap_or_default();
    let mut sent: Vec<(DateTime<Utc>, String)> = log
        .lines()
        .filter(|l| l.contains("\"sent\""))
        .filter_map(|l| serde_json::from_str::<Record>(l).ok())
        .filter_map(|r| match r {
            Record::Sent { run, at, from: Some(from), text, .. } => {
                serving.contains(&(run, from)).then_some((at, text))
            }
            _ => None,
        })
        .collect();
    sent.sort_by_key(|(at, _)| *at);
    let mut kept = 0usize;
    let mut from = sent.len();
    while from > 0 && kept + sent[from - 1].1.chars().count() <= budget {
        from -= 1;
        kept += sent[from].1.chars().count();
    }
    sent.split_off(from)
}

/// What a boot recovers from the log: the exchange to draw, and the mail still owed.
pub struct Seeded {
    /// Every agent-to-agent message the tail holds, oldest first, capped at [`KEPT`].
    pub ring: VecDeque<Sent>,
    /// Per session, what was delivered to it and never taken — **from the previous run only.**
    /// An older run's undrained inbox belongs to a session nothing is going to reopen, and
    /// restoring it would put a three-week-old instruction in a fresh mailbox.
    pub undelivered: HashMap<SessionSlug, Vec<Message>>,
}

/// Read the tail of the mail log and fold it.
pub async fn seed(data_dir: &Path, current_run: &str) -> Seeded {
    fold(&super::jsonl::read_tail(&mail_path(data_dir), SEED_TAIL_BYTES).await, current_run)
}

fn fold(text: &str, current_run: &str) -> Seeded {
    let mut ring: VecDeque<Sent> = VecDeque::new();
    // Per (run, session): what has been delivered and not yet drained, oldest first.
    let mut inboxes: HashMap<(String, SessionSlug), Vec<Message>> = HashMap::new();
    // Which run the newest line belongs to — the previous one, since this fold runs at boot
    // before anything of this run's is written. Same rule as the session directory's.
    let mut newest_run: Option<String> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // A tail-read routinely begins mid-line, and one corrupt line must not blank a page.
        let Ok(record) = serde_json::from_str::<Record>(line) else { continue };
        match record {
            Record::Sent { run, from, to, text, .. } => {
                if run != current_run {
                    newest_run = Some(run.clone());
                }
                if let Some(from) = from.clone() {
                    push(&mut ring, Sent::new(from, to.clone(), &text));
                }
                inboxes.entry((run, to)).or_default().push(Message { from, text });
            }
            Record::Read { run, to, count, .. } => {
                if run != current_run {
                    newest_run = Some(run.clone());
                }
                if let Some(queue) = inboxes.get_mut(&(run, to)) {
                    queue.drain(..(count as usize).min(queue.len()));
                }
            }
        }
    }

    let undelivered = match newest_run {
        None => HashMap::new(),
        Some(previous) => inboxes
            .into_iter()
            .filter(|((run, _), queue)| run == &previous && !queue.is_empty())
            .map(|((_, to), queue)| (to, queue))
            .collect(),
    };
    Seeded { ring, undelivered }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(record: &Record) -> String {
        format!("{}\n", serde_json::to_string(record).unwrap())
    }

    fn sent(run: &str, from: Option<u64>, to: u64, text: &str) -> Record {
        Record::Sent {
            run: run.into(),
            at: Utc::now(),
            from: from.map(SessionSlug::from),
            to: SessionSlug::from(to),
            text: text.into(),
        }
    }

    fn read(run: &str, to: u64, count: u32) -> Record {
        Record::Read { run: run.into(), at: Utc::now(), to: SessionSlug::from(to), count }
    }

    /// **What was delivered and never taken is exactly what a reopened session is owed.** The
    /// `read` line is what makes that answerable: a `sent` line alone says a message reached a
    /// mailbox and can never say whether anyone took it out.
    #[test]
    fn unread_is_what_is_sent_minus_what_is_drained() {
        let text: String = [
            sent("run-prev", Some(1), 2, "first"),
            sent("run-prev", Some(1), 2, "second"),
            read("run-prev", 2, 1),
            sent("run-prev", Some(1), 2, "third"),
        ]
        .iter()
        .map(line)
        .collect();

        let owed = fold(&text, "run-now").undelivered;
        let queue = owed.get(&SessionSlug::from(2)).expect("two messages outlived the drain");
        assert_eq!(
            queue.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            vec!["second", "third"],
            "the drain took the oldest, in order, and left the rest"
        );
    }

    /// A drained inbox owes nothing, and must not reappear as a mailbox full of messages its
    /// session has already acted on — which is the one failure that would turn restoring mail
    /// from a fix into a way of doing everything twice.
    #[test]
    fn a_drained_inbox_owes_nothing() {
        let text: String = [
            sent("run-prev", Some(1), 2, "first"),
            sent("run-prev", Some(1), 2, "second"),
            read("run-prev", 2, 2),
        ]
        .iter()
        .map(line)
        .collect();
        assert!(fold(&text, "run-now").undelivered.is_empty());
    }

    /// **Only the previous run's**, the same staleness rule the session directory keeps: an
    /// older run's undrained inbox belongs to a session nothing is going to reopen, and
    /// restoring it would drop a three-week-old instruction into a fresh mailbox.
    #[test]
    fn an_older_runs_unread_mail_is_not_owed() {
        let text: String = [
            sent("run-ancient", Some(1), 2, "from three weeks ago"),
            sent("run-prev", Some(1), 3, "from the run that just stopped"),
        ]
        .iter()
        .map(line)
        .collect();

        let owed = fold(&text, "run-now").undelivered;
        assert!(owed.get(&SessionSlug::from(2)).is_none(), "the ancient one is not owed");
        assert_eq!(owed.get(&SessionSlug::from(3)).map(Vec::len), Some(1));
    }

    /// A host post has one end, so no arrow can draw it and the ring does not keep it — but it
    /// is as owed to its recipient as anything a session sent, which is why the log records it
    /// and the two readers disagree on purpose.
    #[test]
    fn a_host_post_is_owed_but_not_drawn() {
        let text: String = [sent("run-prev", None, 2, "your clip is ready")].iter().map(line).collect();
        let seeded = fold(&text, "run-now");
        assert!(seeded.ring.is_empty(), "nothing to draw an arrow between");
        assert_eq!(seeded.undelivered.get(&SessionSlug::from(2)).map(Vec::len), Some(1));
    }

    /// The exchange survives a restart now, because the roster it draws arrows between does.
    #[test]
    fn the_ring_is_seeded_from_the_log() {
        let text: String = [
            sent("run-prev", Some(1), 2, "go"),
            sent("run-prev", Some(2), 1, "done"),
            read("run-prev", 2, 1),
            read("run-prev", 1, 1),
        ]
        .iter()
        .map(line)
        .collect();

        let ring = fold(&text, "run-now").ring;
        assert_eq!(ring.len(), 2, "reading a message does not remove it from the exchange");
        let pair: Vec<_> = ring
            .iter()
            .filter(|s| s.between(&SessionSlug::from(1), &SessionSlug::from(2)))
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(pair, vec!["go", "done"], "both directions, oldest first");
    }

    /// One corrupt line — routinely the first, since the read starts mid-file — must not blank
    /// the page or lose an inbox.
    #[test]
    fn a_truncated_first_line_is_skipped() {
        let text = format!("{{\"event\":\"se\n{}", line(&sent("run-prev", Some(1), 2, "kept")));
        assert_eq!(fold(&text, "run-now").undelivered.get(&SessionSlug::from(2)).map(Vec::len), Some(1));
    }

    /// **A task's reports are its sessions' by `(run, session)`, never by slug alone.** Slugs
    /// repeat every boot, so the same name in another run served another task.
    #[tokio::test]
    async fn a_tasks_reports_are_joined_by_the_run_and_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = layout::raw_root(dir.path()).join(layout::SESSIONS_DIR);
        tokio::fs::create_dir_all(&sessions).await.unwrap();
        let opened = |run: &str, subject: &str| {
            format!(
                r#"{{"event":"opened","run":"{run}","session":"general-x","subject":"{subject}","at":"2026-09-18T00:00:00Z","role":"worker"}}"#
            )
        };
        tokio::fs::write(sessions.join("index.jsonl"), format!("{}\n{}\n", opened("r1", "resume"), opened("r2", "shoes")))
            .await
            .unwrap();
        let sent = |run: &str, at: &str, text: &str| {
            format!(
                r#"{{"event":"sent","run":"{run}","at":"{at}","from":"general-x","to":"cognition","text":"{text}"}}"#
            )
        };
        tokio::fs::write(
            mail_path(dir.path()),
            [
                sent("r1", "2026-09-18T01:00:00Z", "简历导进来了"),
                sent("r2", "2026-09-18T02:00:00Z", "鞋的对比做好了"),
                sent("r1", "2026-09-18T03:00:00Z", "两页扫描件没法编辑"),
            ]
            .join("\n"),
        )
        .await
        .unwrap();

        let got = sent_by_sessions_serving(dir.path(), "resume", 10_000).await;
        let texts: Vec<&str> = got.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(texts, vec!["简历导进来了", "两页扫描件没法编辑"]);

        let newest = sent_by_sessions_serving(dir.path(), "resume", 9).await;
        assert_eq!(newest.len(), 1, "a budget keeps the newest");
        assert!(sent_by_sessions_serving(dir.path(), "nothing", 10_000).await.is_empty());
    }
}
