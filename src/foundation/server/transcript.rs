//! The conversation — an append-only list of whole messages, owned by the host.
//!
//! Three things are messages and nothing else is: what the person typed or said, a
//! file they handed over, and one `say` call. Views, worker reports, clock wakes,
//! recognition signals and tool calls all move through this process and none of
//! them are conversation; they have the view slot, the journal and the inspector.
//!
//! **Nothing here is ever rewritten or cleared.** That is the whole difference from
//! the current-appearance state this replaces, and it is what removed the presence
//! gate: text used to be one slot that the next thing overwrote, so speaking into an
//! empty room threw the words away and withholding them was the lesser loss. A
//! message that nobody is looking at is a message waiting in the conversation, so
//! there is nothing left to protect and nothing left to detect. See
//! [`docs/arch/text-transcript.md`].
//!
//! **One `say` is one message.** The call already carries its complete text, so a
//! message is appended when the call is accepted rather than assembled from streamed
//! chunks. Sentence splitting still happens downstream to pace TTS and never reaches
//! this list.
//!
//! Ids are the journal's uuidv7, minted at the append site and passed here, so the
//! live window and the scrollback (`GET /api/messages?before=`) share identifiers
//! without a merge step. An id on a message is **not a delivery cursor**: no client
//! sends one back, and this module keeps no per-window position, acknowledgement or
//! read receipt. Whether a person read something is not observable, and the previous
//! attempt to derive it is what this replaces.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::mind::memory::media as media_mod;
use crate::types::{Channel, JournalEntry, Origin};

/// A rolling speech-recognition partial that never settles is presentation noise.
/// Expire it here, in the authoritative state, rather than independently in every
/// window.
const INTERIM_STALE_AFTER: Duration = Duration::from_secs(3);

/// How many messages stay in the live window. Older ones are still in the journal
/// and are reached by scrolling back, never by growing this.
const LIVE_WINDOW: usize = 200;

/// Backlog of frames a slow subscriber may fall behind by before it is dropped and
/// has to reconnect (which re-sends the whole window, so nothing is lost).
const FRAME_BACKLOG: usize = 256;

/// Who sent a message. There are two ends to a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Agent,
}

/// A file that came with a message, as the face needs it: the signal ref to fetch
/// the bytes from (`GET /api/media/<ref>`) and the mime to know how to render it.
///
/// The file's *name* is deliberately not a field. It is already in the message
/// text — the framing the carrier wrote says "The user handed you a file:
/// passport.jpg" — and lifting it into a second field would mean the live path and
/// the journal-seeded path disagree, since only the live path still has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Attachment {
    /// [`crate::mind::memory::media::signal_ref`] — channel-qualified, so it is a
    /// path rather than a path plus a guess.
    #[serde(rename = "ref")]
    pub reff: String,
    pub mime: String,
}

/// One message in the conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Message {
    /// The journal entry's uuidv7 — time-sortable, durable, and the citation key.
    pub id: String,
    pub ts: DateTime<Utc>,
    pub role: Role,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<Attachment>,
}

/// One line on `GET /api/out/text`.
///
/// `Reset` is always first and is sent again only if the window is rebuilt; a
/// reconnecting surface receives one and is current. Everything else appends. There
/// is no frame that edits or removes a message, because nothing does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Frame {
    Reset {
        messages: Vec<Message>,
        interim: Option<String>,
    },
    Append(Message),
    /// The rolling recognition partial, or `None` for its expiry. Not a message —
    /// it is a preview of one, shown pending at the tail until the line settles.
    Interim(Option<String>),
}

struct Inner {
    messages: VecDeque<Message>,
    interim: Option<String>,
}

/// Cloneable handle to the one conversation.
#[derive(Clone)]
pub struct Transcript {
    inner: Arc<Mutex<Inner>>,
    tx: broadcast::Sender<Frame>,
    /// Bumped on every interim update so a stale expiry timer knows it lost the
    /// race and does nothing.
    interim_generation: Arc<AtomicU64>,
}

impl Transcript {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(FRAME_BACKLOG);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                messages: VecDeque::new(),
                interim: None,
            })),
            tx,
            interim_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Fill the window from the journal at boot, oldest first. Replaces whatever is
    /// there and tells every live subscriber to reset — a conversation appearing
    /// under someone mid-session would be a bug, but it costs nothing to be correct
    /// about it, and the same path serves a future rebuild.
    pub fn seed(&self, messages: Vec<Message>) {
        let frame = {
            let mut inner = self.inner.lock().expect("transcript mutex poisoned");
            inner.messages = messages.into_iter().collect();
            trim(&mut inner.messages);
            Frame::Reset {
                messages: inner.messages.iter().cloned().collect(),
                interim: inner.interim.clone(),
            }
        };
        let _ = self.tx.send(frame);
    }

    /// Append one message and publish it. The settled line also clears any interim,
    /// which is the same event: the preview became the message.
    pub fn append(&self, message: Message) {
        let frames = {
            let mut inner = self.inner.lock().expect("transcript mutex poisoned");
            let cleared = inner.interim.take().is_some();
            inner.messages.push_back(message.clone());
            trim(&mut inner.messages);
            (cleared, message)
        };
        if frames.0 {
            let _ = self.tx.send(Frame::Interim(None));
        }
        let _ = self.tx.send(Frame::Append(frames.1));
    }

    /// Update the rolling recognition partial, and arm its expiry.
    ///
    /// The expiry lives here rather than in each window so every surface stops
    /// showing a dead partial at the same moment, and so a window that connects
    /// mid-partial does not inherit one that will never settle.
    pub fn note_interim(&self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let generation = self.interim_generation.fetch_add(1, Ordering::Relaxed) + 1;
        {
            let mut inner = self.inner.lock().expect("transcript mutex poisoned");
            if inner.interim.as_deref() == Some(text) {
                return;
            }
            inner.interim = Some(text.to_owned());
        }
        let _ = self.tx.send(Frame::Interim(Some(text.to_owned())));

        let transcript = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(INTERIM_STALE_AFTER).await;
            if transcript.interim_generation.load(Ordering::Relaxed) != generation {
                return;
            }
            let cleared = {
                let mut inner = transcript.inner.lock().expect("transcript mutex poisoned");
                inner.interim.take().is_some()
            };
            if cleared {
                let _ = transcript.tx.send(Frame::Interim(None));
            }
        });
    }

    /// The opening frame plus the stream of everything after it, taken together
    /// under one lock so no message can slip between the snapshot and the
    /// subscription.
    pub fn subscribe(&self) -> (Frame, broadcast::Receiver<Frame>) {
        let inner = self.inner.lock().expect("transcript mutex poisoned");
        let rx = self.tx.subscribe();
        let frame = Frame::Reset {
            messages: inner.messages.iter().cloned().collect(),
            interim: inner.interim.clone(),
        };
        (frame, rx)
    }

    /// The oldest id in the live window — where a scrollback request starts.
    pub fn oldest_id(&self) -> Option<String> {
        self.inner
            .lock()
            .expect("transcript mutex poisoned")
            .messages
            .front()
            .map(|m| m.id.clone())
    }
}

impl Default for Transcript {
    fn default() -> Self {
        Self::new()
    }
}

fn trim(messages: &mut VecDeque<Message>) {
    while messages.len() > LIVE_WINDOW {
        messages.pop_front();
    }
}

/// Read journal entries back as messages, oldest first — the boot seed, and the
/// same mapping `GET /api/messages` scrolls back through.
///
/// **This is where "what is a message" is decided for history**, and it has to agree
/// with the live append sites or the conversation would change shape when it
/// reloads. Three shapes get in and everything else is dropped: a human line
/// (typed on `Text`, recognized on `Audio`), a handed `File`, and Reaction's own
/// worded output. A check-in on `Clock`, a face seen on `Vision`, a view put up on
/// `View` — all journaled, none of them things anybody said.
pub fn from_journal(entries: Vec<JournalEntry>) -> Vec<Message> {
    entries
        .into_iter()
        .filter_map(|entry| match entry {
            JournalEntry::SignalIn { id, ts, channel, body, media, origin, .. } => {
                // An absent origin is a journal line older than the field; every one
                // of those on an input channel was a person.
                if !matches!(origin, None | Some(Origin::Human)) {
                    return None;
                }
                match channel {
                    Channel::Text | Channel::Audio => Some(Message {
                        id,
                        ts,
                        role: Role::User,
                        text: body,
                        attachment: None,
                    }),
                    Channel::File => {
                        let attachment = media.map(|m| Attachment {
                            reff: media_mod::signal_ref(Channel::File, ts, &m.file),
                            mime: m.mime,
                        });
                        Some(Message {
                            id,
                            ts,
                            role: Role::User,
                            text: strip_ref(&body),
                            attachment,
                        })
                    }
                    _ => None,
                }
            }
            JournalEntry::SignalOut { id, ts, channel, body, .. } => match channel {
                Channel::Text => Some(Message {
                    id,
                    ts,
                    role: Role::Agent,
                    text: body,
                    attachment: None,
                }),
                _ => None,
            },
        })
        .filter(|m| !m.text.trim().is_empty() || m.attachment.is_some())
        .collect()
}

/// Drop the `⟨ref: …⟩` locator from a file signal's body. The locator exists so the
/// *agent* can open the bytes; the person already knows what they sent, and a path
/// in their chat is noise.
pub fn strip_ref(body: &str) -> String {
    let Some(open) = body.find("⟨ref:") else {
        return body.trim().to_owned();
    };
    let rest = &body[open..];
    let end = rest.find('⟩').map(|i| open + i + '⟩'.len_utf8());
    let mut out = body[..open].to_owned();
    if let Some(end) = end {
        out.push_str(&body[end..]);
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, role: Role, text: &str) -> Message {
        Message {
            id: id.to_owned(),
            ts: Utc::now(),
            role,
            text: text.to_owned(),
            attachment: None,
        }
    }

    fn window(t: &Transcript) -> Vec<String> {
        let (Frame::Reset { messages, .. }, _) = t.subscribe() else {
            panic!("subscribe must open with a reset");
        };
        messages.into_iter().map(|m| m.text).collect()
    }

    #[tokio::test]
    async fn a_new_subscriber_gets_the_conversation_not_just_the_present() {
        // The whole point of the change: an exchange that has been answered and
        // followed by another is still there when a window opens later.
        let t = Transcript::new();
        t.append(msg("1", Role::User, "first"));
        t.append(msg("2", Role::Agent, "old answer"));
        t.append(msg("3", Role::User, "second"));
        t.append(msg("4", Role::Agent, "current answer"));

        assert_eq!(window(&t), ["first", "old answer", "second", "current answer"]);
    }

    #[tokio::test]
    async fn a_reply_that_crossed_with_a_new_line_lands_after_it() {
        // No eligibility rule, no suppression: arrival order is what happened, and
        // it is how a person reads a message that crossed with theirs.
        let t = Transcript::new();
        t.append(msg("1", Role::User, "what day is it?"));
        t.append(msg("2", Role::User, "actually never mind"));
        t.append(msg("3", Role::Agent, "Sunday."));

        assert_eq!(window(&t), ["what day is it?", "actually never mind", "Sunday."]);
    }

    #[tokio::test]
    async fn subscribers_see_appends_after_their_opening_frame() {
        let t = Transcript::new();
        t.append(msg("1", Role::User, "hello"));

        let (opening, mut rx) = t.subscribe();
        let Frame::Reset { messages, interim } = opening else {
            panic!("subscribe must open with a reset");
        };
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "hello");
        assert_eq!(interim, None);

        t.append(msg("2", Role::Agent, "hi"));
        let next = rx.recv().await.expect("append reaches the subscriber");
        let Frame::Append(m) = next else {
            panic!("expected an append, got {next:?}");
        };
        assert_eq!(m.text, "hi");
        assert_eq!(m.role, Role::Agent);
    }

    /// The opening frame and the subscription are taken under one lock, so a
    /// message appended concurrently is in exactly one of them — never dropped
    /// between the two, never delivered twice.
    #[tokio::test]
    async fn the_opening_frame_and_the_stream_do_not_overlap_or_gap() {
        let t = Transcript::new();
        t.append(msg("1", Role::User, "a"));
        let (Frame::Reset { messages, .. }, mut rx) = t.subscribe() else {
            panic!("subscribe must open with a reset");
        };
        t.append(msg("2", Role::User, "b"));

        assert_eq!(messages.len(), 1, "the opening frame is a point in time");
        let Frame::Append(m) = rx.recv().await.unwrap() else {
            panic!("expected an append");
        };
        assert_eq!(m.text, "b", "and everything after it arrives exactly once");
    }

    #[tokio::test]
    async fn the_window_is_bounded_and_drops_from_the_front() {
        let t = Transcript::new();
        for i in 0..(LIVE_WINDOW + 10) {
            t.append(msg(&i.to_string(), Role::User, &i.to_string()));
        }
        let w = window(&t);
        assert_eq!(w.len(), LIVE_WINDOW);
        assert_eq!(w.first().unwrap(), "10", "the oldest fall out of the window");
        assert_eq!(w.last().unwrap(), &(LIVE_WINDOW + 9).to_string());
    }

    #[tokio::test]
    async fn a_settled_message_clears_the_interim_it_was_previewing() {
        let t = Transcript::new();
        t.note_interim("what day is");
        let (opening, mut rx) = t.subscribe();
        assert!(matches!(opening, Frame::Reset { interim: Some(_), .. }));

        t.append(msg("1", Role::User, "what day is it?"));
        assert_eq!(rx.recv().await.unwrap(), Frame::Interim(None));
        let Frame::Append(m) = rx.recv().await.unwrap() else {
            panic!("expected the settled message");
        };
        assert_eq!(m.text, "what day is it?");
    }

    #[tokio::test]
    async fn a_repeated_interim_is_not_republished() {
        let t = Transcript::new();
        t.note_interim("hello");
        let (_, mut rx) = t.subscribe();
        t.note_interim("hello");
        t.note_interim("hello there");
        assert_eq!(
            rx.recv().await.unwrap(),
            Frame::Interim(Some("hello there".into())),
            "the identical partial produced no frame of its own"
        );
    }

    #[tokio::test]
    async fn seeding_replaces_the_window_and_resets_subscribers() {
        let t = Transcript::new();
        let (_, mut rx) = t.subscribe();
        t.seed(vec![msg("1", Role::User, "from the journal")]);
        let Frame::Reset { messages, .. } = rx.recv().await.unwrap() else {
            panic!("seed must reset");
        };
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "from the journal");
    }

    // ---- Reading the journal back as a conversation ----

    fn sig_in(id: &str, channel: Channel, body: &str, origin: Option<Origin>) -> JournalEntry {
        JournalEntry::SignalIn {
            id: id.to_owned(),
            ts: Utc::now(),
            channel,
            body: body.to_owned(),
            stream: None,
            media: None,
            origin,
            sender: None,
        }
    }

    fn sig_out(id: &str, channel: Channel, body: &str) -> JournalEntry {
        JournalEntry::SignalOut {
            id: id.to_owned(),
            ts: Utc::now(),
            channel,
            body: body.to_owned(),
            media: None,
            origin: Some(Origin::Reaction),
        }
    }

    #[test]
    fn a_typed_line_a_spoken_line_and_a_reply_are_the_conversation() {
        let msgs = from_journal(vec![
            sig_in("1", Channel::Text, "typed", Some(Origin::Human)),
            sig_in("2", Channel::Audio, "spoken", Some(Origin::Human)),
            sig_out("3", Channel::Text, "answered"),
        ]);
        let roles: Vec<_> = msgs.iter().map(|m| (m.role, m.text.as_str())).collect();
        assert_eq!(
            roles,
            [
                (Role::User, "typed"),
                (Role::User, "spoken"),
                (Role::Agent, "answered"),
            ]
        );
    }

    /// The list is a conversation, not a log. Everything the journal keeps that
    /// nobody *said* has to stay out of it, or reloading the page would fill the
    /// chat with machinery.
    #[test]
    fn clock_wakes_faces_and_views_are_journaled_but_are_not_messages() {
        let msgs = from_journal(vec![
            sig_in("1", Channel::Clock, "(check-in) quiet 5m", Some(Origin::Host)),
            sig_in("2", Channel::Vision, "赵力 appeared on camera.", Some(Origin::Human)),
            sig_in("3", Channel::Text, "a worker reported in", Some(Origin::Worker)),
            sig_out("4", Channel::View, "view show · tasks"),
            sig_out("5", Channel::Text, "the only message here"),
        ]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "the only message here");
    }

    #[test]
    fn a_journal_line_older_than_the_origin_field_reads_as_a_person() {
        let msgs = from_journal(vec![sig_in("1", Channel::Text, "from before", None)]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, Role::User);
    }

    #[test]
    fn a_handed_file_keeps_its_framing_and_loses_its_locator() {
        let entry = JournalEntry::SignalIn {
            id: "1".into(),
            ts: Utc::now(),
            channel: Channel::File,
            body: "The user handed you a file: passport.jpg ⟨ref: file/2026-08-11/09/31-04.jpg⟩"
                .into(),
            stream: None,
            media: Some(crate::types::Media {
                file: "09/31-04.jpg".into(),
                mime: "image/jpeg".into(),
                duration_ms: None,
                width: None,
                height: None,
            }),
            origin: Some(Origin::Human),
            sender: None,
        };
        let msgs = from_journal(vec![entry]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "The user handed you a file: passport.jpg");
        let att = msgs[0].attachment.as_ref().expect("the file rides along");
        assert_eq!(att.mime, "image/jpeg");
        assert!(att.reff.ends_with("09/31-04.jpg"), "ref: {}", att.reff);
        assert!(att.reff.starts_with("file/"), "the ref names its channel: {}", att.reff);
    }

    #[test]
    fn stripping_a_locator_leaves_the_sentence_readable() {
        assert_eq!(strip_ref("handed you a file: a.png ⟨ref: file/x/y/z.png⟩"), "handed you a file: a.png");
        assert_eq!(strip_ref("⟨ref: file/x/y/z.png⟩"), "");
        assert_eq!(strip_ref("no locator here"), "no locator here");
        assert_eq!(strip_ref("before ⟨ref: broken"), "before", "an unterminated locator still goes");
    }

    /// A file with no text left after stripping is still a message — the file *is*
    /// the message. Only a line with neither text nor file is dropped.
    #[test]
    fn a_bare_file_survives_an_empty_text() {
        let entry = JournalEntry::SignalIn {
            id: "1".into(),
            ts: Utc::now(),
            channel: Channel::File,
            body: "⟨ref: file/2026-08-11/09/31-04.jpg⟩".into(),
            stream: None,
            media: Some(crate::types::Media {
                file: "09/31-04.jpg".into(),
                mime: "image/jpeg".into(),
                duration_ms: None,
                width: None,
                height: None,
            }),
            origin: Some(Origin::Human),
            sender: None,
        };
        let msgs = from_journal(vec![entry]);
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].text.is_empty());
        assert!(msgs[0].attachment.is_some());
    }
}
