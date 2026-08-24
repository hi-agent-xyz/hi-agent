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
//! **Each message carries who sent it**, as the boundary decided it and not as
//! anything here worked out — the owner default on a typed line, a voiceprint
//! cluster on a spoken one, nobody at all on the agent's own. That is what lets a
//! window put a face beside a message; it is also why the `⟨…⟩` evidence markers the
//! carriers write for the mind are stripped out of the text ([`display_text`]). A
//! recognition belongs in the field, where it can be drawn and can be corrected —
//! not spelled into the middle of the sentence somebody said. See
//! `docs/arch/signal-attribution.md`.
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
use crate::types::{Channel, JournalEntry, Origin, Sender, SenderBasis};

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

/// One message **as a surface receives it** — the presentation of a
/// [`crate::types::Message`], not a second copy of one.
///
/// [`crate::types::Message`] is the fact, minted once at the boundary and
/// journaled; this is how it is drawn. The split is the one `docs/arch/message.md` allows: one value, two
/// renderings (this, and the line the prompt builder writes for the mind). It is
/// derived here rather than stored beside the message, so a window and the journal
/// cannot come to disagree.
///
/// The field names are the ones surfaces already read — `role`, `text`,
/// `attachment` — so the shape a window parses did not change when the shape the
/// system stores did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Wire {
    pub id: String,
    pub ts: DateTime<Utc>,
    pub role: Role,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<Attachment>,
    /// Which person sent this, exactly as the boundary decided it. **Absent is a
    /// real answer**: the agent's own messages have no sender, and a voice nobody
    /// recognized has one that names nobody. Neither is a gap to fill in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<Sender>,
}

impl std::convert::From<crate::types::Message> for Wire {
    fn from(m: crate::types::Message) -> Self {
        let crate::types::Message { id, ts, from, content } = m;
        let role = if from.is_agent() { Role::Agent } else { Role::User };
        let sender = from.sender().cloned();
        // A file's name *is* its text here: it is what a person calls the thing in
        // the conversation, and drawing a nameless thumbnail was the gap that made
        // the name a field in the first place.
        let (text, attachment) = match content {
            crate::types::Content::Text(t) => (t, None),
            crate::types::Content::Speech { text, .. } => (text, None),
            crate::types::Content::File(f) => (
                f.name,
                Some(Attachment { reff: f.reff, mime: f.mime }),
            ),
        };
        Wire { id, ts, role, text, attachment, sender }
    }
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
        messages: Vec<Wire>,
        interim: Option<String>,
    },
    Append(Wire),
    /// The rolling recognition partial, or `None` for its expiry. Not a message —
    /// it is a preview of one, shown pending at the tail until the line settles.
    Interim(Option<String>),
}

struct Inner {
    messages: VecDeque<Wire>,
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
    pub fn seed(&self, messages: Vec<Wire>) {
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
    pub fn append(&self, message: Wire) {
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

fn trim(messages: &mut VecDeque<Wire>) {
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
/// worded output. A check-in on `Clock`, a face seen on `Vision`, a view put up or
/// gone to on `View` — all journaled, none of them things anybody said.
pub fn from_journal(entries: Vec<JournalEntry>) -> Vec<Wire> {
    entries
        .into_iter()
        .filter_map(|entry| match entry {
            JournalEntry::Message { message, .. } => Some(Wire::from(message)),
            _ => None,
        })
        .filter(|m| !m.text.trim().is_empty() || m.attachment.is_some())
        .collect()
}

/// Take the sender back out of the carrier's own `⟨voice: …⟩` marker, for the lines
/// that were journaled before the sender was a field.
///
/// **This is a recovery, not a backfill, and the difference is where the name came
/// from.** `signal-attribution.md` forbids deriving a sender from *content* and
/// accepts that old signals are unattributed — because who sent them is not
/// recoverable and inventing it is the failure the whole document exists to stop.
/// Here it *is* recoverable, verbatim: the voiceprint matched at the boundary and
/// the audio carrier wrote its conclusion down. It wrote it into the body, because
/// at the time the body was the only place there was. Reading it back is finding the
/// boundary's own record where the boundary happened to put it.
///
/// The marker grammar is what makes this safe rather than a parse of prose: `⟨…⟩` is
/// written only by carriers, and a person cannot type it. Nothing is read out of
/// what anybody *said*.
///
/// It defers to a sender that is already grounded, so a live line — which carries the
/// field properly — is never second-guessed by its own tag.
///
/// Deliberately partial, and the limit is worth knowing: the live mic writes the tag
/// only when the **speaker changes**, so within one person's run only the first line
/// carries it. The rest stay unattributed. Carrying a name forward across untagged
/// lines would mean assuming the speaker did not change, and an assumption is exactly
/// what may not be written into this field.
fn recover_voice_sender(sender: Option<Sender>, body: &str) -> Option<Sender> {
    if sender.as_ref().is_some_and(Sender::is_grounded) {
        return sender;
    }
    match voice_marker_subject(body) {
        // `cluster`, because a voiceprint match is what actually happened — the tag
        // is only where it was stored.
        Some(subject) => Some(Sender { subject: Some(subject), basis: SenderBasis::Cluster }),
        // No marker: the line stays exactly as the boundary left it. Unattributed is
        // an answer, and it must survive this pass rather than be flattened into
        // "no sender field at all", which is what a machine channel means.
        None => sender,
    }
}

/// The subject named by a `⟨voice: …⟩` marker: `⟨voice: 赵力⟩` from the live mic and
/// `⟨voice: 老王 ~0.82⟩` from a posted clip, whose similarity is evidence for the mind
/// and not part of the name.
///
/// `⟨voice: unfamiliar⟩` names nobody — it is the carrier saying it heard a voice and
/// could not place it — so it must never become a person called "unfamiliar".
fn voice_marker_subject(body: &str) -> Option<String> {
    const OPEN: &str = "⟨voice:";
    let start = body.find(OPEN)? + OPEN.len();
    let rest = &body[start..];
    let end = rest.find('⟩')?;
    let inner = rest[..end].trim();
    // Drop the trailing ` ~0.82` confidence, keeping names that contain spaces.
    let name = match inner.rsplit_once(" ~") {
        Some((name, _)) => name.trim(),
        None => inner,
    };
    if name.is_empty() || name == "unfamiliar" {
        return None;
    }
    Some(name.to_owned())
}

/// A signal's body as the person should read it: everything the carriers wrote into
/// it *for the agent* removed.
///
/// `⟨…⟩` is this system's one convention for evidence a carrier attached at the
/// boundary — the `⟨ref: …⟩` locator on a handed file, the `⟨voice: 赵力⟩` a
/// voiceprint recognized, the whole standing instruction a held-attention session
/// rides in on. Every one of them is addressed to the mind, and every one of them
/// used to render inside the person's own chat bubble, which is how a name that is
/// now a face beside the message was also a tag in the middle of the sentence.
///
/// The person loses nothing: they know what they sent, they know who was talking,
/// and what the recognition concluded is [`Wire::sender`] now — a field, where a
/// window can draw it as a face and a later pass can still defeat it.
pub fn display_text(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(open) = rest.find('⟨') {
        out.push_str(&rest[..open]);
        rest = &rest[open + '⟨'.len_utf8()..];
        match rest.find('⟩') {
            Some(close) => rest = &rest[close + '⟩'.len_utf8()..],
            // An unterminated marker swallows the tail: it is a carrier's half-written
            // note either way, and half of one is not something to show anybody.
            None => rest = "",
        }
    }
    out.push_str(rest);
    // A marker lifted out of the middle leaves two spaces where it stood.
    let mut collapsed = String::with_capacity(out.len());
    let mut last_was_space = false;
    for ch in out.chars() {
        let is_space = ch == ' ' || ch == '\t';
        if is_space && last_was_space {
            continue;
        }
        last_was_space = is_space;
        collapsed.push(ch);
    }
    collapsed.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, role: Role, text: &str) -> Wire {
        Wire {
            id: id.to_owned(),
            ts: Utc::now(),
            role,
            text: text.to_owned(),
            attachment: None,
            sender: None,
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
        sig_in_from(id, channel, body, origin, None)
    }

    fn sig_in_from(
        id: &str,
        channel: Channel,
        body: &str,
        origin: Option<Origin>,
        sender: Option<Sender>,
    ) -> JournalEntry {
        crate::mind::memory::journal::legacy_signal_in((id.to_owned()).to_string(), Utc::now(), channel, body.to_owned(), None, None, origin, sender)
    }

    fn sig_out(id: &str, channel: Channel, body: &str) -> JournalEntry {
        crate::mind::memory::journal::legacy_signal_out((id.to_owned()).to_string(), Utc::now(), channel, body.to_owned(), None, Some(Origin::Reaction))
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
    fn a_handed_file_is_named_by_its_name_not_its_framing() {
        let entry = crate::mind::memory::journal::legacy_signal_in("1".into(), Utc::now(), Channel::File, "The user handed you a file: passport.jpg ⟨ref: file/2026-08-11/09/31-04.jpg⟩".to_string(), None, Some(crate::types::Media {
                file: "09/31-04.jpg".into(),
                mime: "image/jpeg".into(),
                duration_ms: None,
                width: None,
                height: None,
            }), Some(Origin::Human), None);
        let msgs = from_journal(vec![entry]);
        assert_eq!(msgs.len(), 1);
        // The framing prose is gone: the name is a field now, and what a surface
        // draws under the thumbnail is that name — not the sentence a carrier wrote
        // for the mind, and not the locator that sat behind it.
        assert_eq!(msgs[0].text, "passport.jpg");
        let att = msgs[0].attachment.as_ref().expect("the file rides along");
        assert_eq!(att.mime, "image/jpeg");
        assert!(att.reff.ends_with("09/31-04.jpg"), "ref: {}", att.reff);
        assert!(att.reff.starts_with("file/"), "the ref names its channel: {}", att.reff);
    }

    #[test]
    fn stripping_a_locator_leaves_the_sentence_readable() {
        assert_eq!(display_text("handed you a file: a.png ⟨ref: file/x/y/z.png⟩"), "handed you a file: a.png");
        assert_eq!(display_text("⟨ref: file/x/y/z.png⟩"), "");
        assert_eq!(display_text("no locator here"), "no locator here");
        assert_eq!(display_text("before ⟨ref: broken"), "before", "an unterminated locator still goes");
    }

    /// Everything a carrier wrote for the mind goes, not just the file locator —
    /// the voiceprint's conclusion and the held-attention standing note both used to
    /// render inside the person's own bubble.
    #[test]
    fn the_evidence_a_carrier_wrote_for_the_mind_is_not_in_the_persons_chat() {
        assert_eq!(display_text("其实在我预想中 ⟨voice: 赵力⟩"), "其实在我预想中");
        assert_eq!(
            display_text("好了吗 ⟨voice: 赵力⟩ ⟨live attention: the user is holding the right ⌘⟩"),
            "好了吗"
        );
        assert_eq!(
            display_text("说 ⟨voice: 赵力⟩ 完了"),
            "说 完了",
            "a marker lifted from the middle leaves one space, not two"
        );
    }

    /// The face beside a message is the boundary's decision, read back — not this
    /// module's guess, and not a name lifted out of the text.
    #[test]
    fn a_message_carries_the_sender_the_boundary_decided() {
        let recognized = Sender { subject: Some("赵力".into()), basis: SenderBasis::Cluster };
        let msgs = from_journal(vec![
            sig_in_from("1", Channel::Audio, "其实在我预想中 ⟨voice: 赵力⟩", Some(Origin::Human), Some(recognized.clone())),
            sig_in_from("2", Channel::Audio, "someone else in the room", Some(Origin::Human), Some(Sender::unknown())),
            sig_out("3", Channel::Text, "answered"),
        ]);
        assert_eq!(msgs[0].sender.as_ref(), Some(&recognized));
        assert_eq!(msgs[0].text, "其实在我预想中", "and the tag it replaces is gone");
        assert_eq!(
            msgs[1].sender.as_ref().map(|s| s.is_grounded()),
            Some(false),
            "an unrecognized voice says so rather than borrowing the last name seen"
        );
        assert_eq!(msgs[2].sender, None, "the agent is not a person in the people store");
    }

    /// Lines journaled before the sender was a field kept the voiceprint's answer in
    /// their body, because that was the only place there was. Stripping the marker
    /// without reading it first is how a name that had been visible for months became
    /// a silhouette.
    #[test]
    fn a_line_older_than_the_field_gets_its_speaker_back_from_the_marker() {
        let msgs = from_journal(vec![
            // Exactly the shape on disk: the voiceprint matched, and the sender the
            // old code wrote was the hardcoded unknown.
            sig_in_from(
                "1",
                Channel::Audio,
                "其实在我预想中 ⟨voice: 赵力⟩",
                Some(Origin::Human),
                Some(Sender::unknown()),
            ),
            // A posted clip carries the similarity too; it is evidence, not a name.
            sig_in_from("2", Channel::Audio, "在的 ⟨voice: 老王 ~0.82⟩", Some(Origin::Human), None),
        ]);

        assert_eq!(
            msgs[0].sender,
            Some(Sender { subject: Some("赵力".into()), basis: SenderBasis::Cluster })
        );
        assert_eq!(msgs[0].text, "其实在我预想中");
        assert_eq!(
            msgs[1].sender,
            Some(Sender { subject: Some("老王".into()), basis: SenderBasis::Cluster })
        );
    }

    /// The two ways this could invent somebody, both closed.
    #[test]
    fn recovery_never_names_a_voice_the_carrier_could_not_place() {
        // "unfamiliar" is the carrier saying it heard someone and could not say who.
        assert_eq!(recover_voice_sender(None, "喂 ⟨voice: unfamiliar⟩"), None);
        // Nothing is read out of what anybody said — only out of the marker grammar,
        // which a person cannot type.
        assert_eq!(recover_voice_sender(None, "赵力说他晚点到"), None);
        assert_eq!(
            recover_voice_sender(Some(Sender::unknown()), "voice: 赵力"),
            Some(Sender::unknown()),
            "and an unattributed line stays unattributed rather than losing its field"
        );
    }

    /// A live line carries the field properly, so its own tag never gets a vote.
    #[test]
    fn a_grounded_sender_is_never_second_guessed_by_a_marker() {
        let owner = Sender::owner_or_unknown(Some("赵力"));
        assert_eq!(
            recover_voice_sender(Some(owner.clone()), "读一下 ⟨voice: 老王⟩"),
            Some(owner),
            "a sender the boundary grounded wins over anything in the body"
        );
    }

    /// A file with no text left after stripping is still a message — the file *is*
    /// the message. Only a line with neither text nor file is dropped.
    #[test]
    fn a_bare_file_falls_back_to_the_blobs_own_name() {
        let entry = crate::mind::memory::journal::legacy_signal_in("1".into(), Utc::now(), Channel::File, "⟨ref: file/2026-08-11/09/31-04.jpg⟩".to_string(), None, Some(crate::types::Media {
                file: "09/31-04.jpg".into(),
                mime: "image/jpeg".into(),
                duration_ms: None,
                width: None,
                height: None,
            }), Some(Origin::Human), None);
        let msgs = from_journal(vec![entry]);
        assert_eq!(msgs.len(), 1);
        // A file handed over with nothing said still names itself — falling back to
        // the stored blob when the carrier wrote no name is a real answer, and it is
        // what keeps the message from rendering as a nameless square.
        assert_eq!(msgs[0].text, "31-04.jpg");
        assert!(msgs[0].attachment.is_some());
    }
}
