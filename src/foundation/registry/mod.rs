//! The agent session registry — the switchboard, in code, with no model in it.
//!
//! Agents do not hold references to each other. They hold **addresses**, and this
//! resolves them. That is what makes "the switchboard is the host"
//! ([`docs/arch/foundation.md`](../../../docs/arch/foundation.md#the-agent-session-registry))
//! a mechanism rather than an aspiration: every agent-to-agent edge passes through here,
//! so routing, queueing and liveness live in one place that cannot be slow, confused, or
//! dead.
//!
//! There is **one verb**: [`Registry::send`]. One direction, no reply, queued. A reply is
//! the same verb going the other way — which is why the sender's identity is stamped here
//! and never passed in by the caller. An agent that names itself can name someone else.
//!
//! Nothing in this module talks to the agent wire or to a model. It owns addresses, mailboxes and
//! metadata; who drains a mailbox and what they do with it belongs to the caller.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

use chrono::{DateTime, Utc};

use crate::identity::Role;
use tokio::sync::{watch, Notify};


/// Handle for one agent session, unique within a run.
///
/// **A slug, not an ordinal**
/// ([`docs/arch/foundation.md`](../../../docs/arch/foundation.md#the-agent-session-registry)).
/// The three rungs are singletons and carry the name they already have everywhere else in
/// this design — `reaction`, `cognition`, `reflection`; a worker is `<type>-<task>`, e.g.
/// `view-builder-kyoto-trip` or `person-reader-alice`. One namespace for every rung and
/// every worker, because ownership crosses rungs — an owner holds sessions no per-rung
/// counter could name without collision.
///
/// It names a *session*, not a role. That the rung slugs read like role names is a
/// property of the rungs rather than a merge of the two ideas: a rung's registration is
/// its address and lives as long as the process, while the agent session *underneath* it
/// is replaced freely (a failed turn drops one, and the next turn opens another) without
/// the address ever changing. Nothing here may assume the reverse — that a given slug
/// implies a given role — because a worker's slug is built from a title an agent wrote.
///
/// **Why not the decimal ordinal it was for most of this codebase's life.** Two reasons,
/// and the second is what forced it. An ordinal says nothing: `2` meant Reaction only
/// because a roster line beside it said so, and only until the next boot. And a bare
/// integer is a valid address in the agent runtime's *own* collaboration namespace, which
/// addresses a sub-agent tree by path from `/root` — so a message aimed at the wrong
/// `send_message` resolved as `/root/2`, was refused by a router we do not own, and raised
/// nothing on our side. Between 2026-08-10 and 08-14 that dropped 33 inter-rung messages.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct SessionId(String);

impl SessionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// An address that is not one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotASessionId;

impl std::fmt::Display for NotASessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not a session id")
    }
}

impl std::error::Error for NotASessionId {}

/// Read an address an agent, or a URL, wrote.
///
/// Forgiving about *shape* — trimmed and lower-cased, so `Cognition` reaches cognition —
/// and strict about *character*: letters, digits and `-`, nothing else, never empty. An id
/// that is well-formed but names nothing is a lookup miss, answered with "nothing live at
/// `x`", which is the honest reading and the one an agent can act on. An id carrying a `/`
/// or a `.` is a different thing entirely and is refused here.
///
/// **That strictness is load-bearing, and it is the reason this is not `Infallible`.** A
/// session id is a path component: `raw/sessions/<run>/<id>.jsonl` is built from one, and
/// `GET /api/workers/{id}/frames` hands the value straight from the URL to that builder.
/// While ids were integers, `parse::<u64>()` *was* the traversal guard, silently and by
/// luck. Widening the type to a string without keeping a guard would have re-opened it —
/// `..%2F..%2Fetc%2Fpasswd` parses fine as a slug-shaped string otherwise.
impl std::str::FromStr for SessionId {
    type Err = NotASessionId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_lowercase();
        let ok = !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '-');
        ok.then_some(Self(s)).ok_or(NotASessionId)
    }
}

/// An ordinal as a session id — **tests only**, and deliberately not available outside them.
///
/// Most of what the switchboard is tested for is plumbing: that mail reaches the right
/// mailbox, that a worker may address only its owner, that a roster comes back in creation
/// order. None of that turns on what a session is *called*, and naming every fixture
/// `view-builder-something` would bury the property each test is actually pinning. So a
/// test says `2.into()` and means "some session, distinct from session 1".
///
/// It is `#[cfg(test)]` because production has exactly one way to make an id — [`mint`] —
/// and that is what keeps ids unique within a run and free of route-shadowing literals. A
/// second constructor on the shipping surface would be a second answer to both.
#[cfg(test)]
impl From<u64> for SessionId {
    fn from(n: u64) -> Self {
        Self(n.to_string())
    }
}

impl serde::Serialize for SessionId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

/// **Old rows carry a number here**, from every run before ids became slugs, and
/// `raw/sessions/index.jsonl` is append-only and never rewritten. A number deserializes to
/// its own decimal spelling — it addresses nothing live, which is correct, and it still
/// names the session in the record it came from, which is all a closed row is for.
impl<'de> serde::Deserialize<'de> for SessionId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = SessionId;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a session id: a slug, or a number from a pre-slug run")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<SessionId, E> {
                Ok(SessionId(v.to_string()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<SessionId, E> {
                Ok(SessionId(v.to_string()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<SessionId, E> {
                Ok(SessionId(v.to_string()))
            }
        }
        d.deserialize_any(V)
    }
}

/// Every id handed out this run. Only ever grows.
///
/// **Uniqueness is per run, not per live session**, and the frame logs are why: a session's
/// stream is `raw/sessions/<run>/<id>.jsonl`, so reusing a slug after its session ended
/// would append a second session's frames onto the first one's file, and the two would be
/// unseparable afterwards. Two workers of one type on one task are otherwise perfectly
/// legal — the second simply gets `-2`.
static USED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Mint the id for a session about to open.
///
/// Called before the underlying session is opened, because the tool surface identifies its
/// caller by this id in a request header — so it cannot be an id the protocol assigns later.
///
/// `hint` is what the worker slug is built from: the ledger subject when the errand serves a
/// task, else its title. Ignored for a rung, which is a singleton and has only one name it
/// could have.
pub fn mint(role: Role, hint: Option<&str>) -> SessionId {
    let base = slug_for(role, hint);

    let mut guard = USED.lock().unwrap();
    let used = guard.get_or_insert_with(HashSet::new);
    if used.insert(base.clone()) {
        return SessionId(base);
    }
    for n in 2u32.. {
        let candidate = format!("{base}-{n}");
        if used.insert(candidate.clone()) {
            return SessionId(candidate);
        }
    }
    unreachable!("u32 worth of one slug")
}

/// What a session of this role and errand is *called*, before uniqueness is applied.
///
/// Split out from [`mint`] because the two answer different questions and only one of them
/// is pure: this is naming, and it is the same answer every time; `mint` adds "and not one
/// already handed out this run", which depends on run-global state. Keeping them apart is
/// what lets the naming be tested as naming — a test binary shares one `USED`, so a test
/// calling `mint` twice for one rung sees a counted id and could not pin the plain spelling.
fn slug_for(role: Role, hint: Option<&str>) -> String {
    match role.worker_type() {
        None => role.as_str().to_string(),
        Some(kind) => match hint.map(slugify).filter(|s| !s.is_empty()) {
            Some(task) => format!("{}-{task}", kind.as_str()),
            // A worker with neither subject nor usable title. Rare, and the `-2`-style
            // disambiguation in `mint` is what keeps it addressable rather than ambiguous.
            None => kind.as_str().to_string(),
        },
    }
}

/// A title or a ledger subject, as the middle of a session slug.
///
/// Kept readable rather than made safe-by-stripping: alphanumerics survive in any script
/// (a Chinese title is most of them here), everything else collapses to a single `-`. The
/// cap is on the slug and not on the words, so a long title is cut rather than refused.
fn slugify(s: &str) -> String {
    let mut out = String::new();
    for ch in s.trim().to_lowercase().chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
        if out.chars().count() >= SLUG_HINT_CHARS {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

/// How much of a title or subject a worker's slug carries.
///
/// It is an address an agent types back, and a filename — long enough to say which errand,
/// short enough to read at a glance in a roster line that already carries the title in full.
const SLUG_HINT_CHARS: usize = 32;

/// How much of a session's recent output the registry keeps for `SessionMessages`.
///
/// A live tail, not an archive: enough for "how's it going?" without turning the
/// switchboard into a second transcript store. The durable copy is the log, and anything
/// older is replayed from the protocol's own session load.
const OUTPUT_TAIL_CHARS: usize = 4_000;

/// How long a "what is it doing" line may be before it is cut.
///
/// It renders as one line on a roster beside the title, in a frame that is the window but
/// may have the ~420px conversation popover over a corner of it (`docs/arch/stage.md`), so
/// a line that wraps three times pushes every other session off the page.
const ACTIVITY_LINE_CHARS: usize = 120;

/// How long the reason a turn failed may be before it is cut.
///
/// Wider than an activity line because the useful half of a provider error is at the end —
/// `exceeded retry limit, last status: 429 Too Many Requests` says nothing until its last
/// four words — and narrower than a paragraph because it renders beside a state word on a
/// roster row. The whole error is on the wire log either way.
const OUTCOME_LINE_CHARS: usize = 200;

/// How long a session's [`title`](Status::title) may be before it is cut.
///
/// A headline, so the cap is roughly one column-width line and not a paragraph. The brief
/// itself is not bounded and does not travel here: it is the session's first prompt, which
/// is on the wire log and in the fold, whole.
const TITLE_CHARS: usize = 72;

/// One line, at most `max` characters, ending in `…` when something was cut.
///
/// Every string that renders as a headline goes through here rather than being trusted:
/// an agent hands over whatever it hands over, and a newline or a paragraph in that slot
/// reflows a roster the person is reading. Whitespace collapses first — a "one-line" title
/// with a newline in it is still a one-line title, it just wasn't written as one.
fn headline(text: &str, max: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match one_line.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", one_line[..cut].trim_end()),
        None => one_line,
    }
}

pub mod index;
pub mod mail;

/// The process's registry. One switchboard, as the design says.
pub fn global() -> &'static Registry {
    static G: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
    G.get_or_init(Registry::new)
}

// Which role a session is running comes from [`crate::identity::Role`] — the one
// namespace for all nine, rungs and worker types alike. This module kept its own
// five-variant copy until now, on the reasoning that prompt and tool surface "are not
// this module's business". They still aren't; the *identity* of the session is, because
// routing turns on it (a worker may address only its owner) and `GET /api/workers`
// reports it. Splitting the type is what left the switchboard unable to say which kind
// of worker a session was.
//
// Only workers are restricted here. That predicate is [`Role::is_worker`], which stays
// correct as worker types are added because they nest inside one variant.

/// What happened to a message — **delivery, never a response.** `send` does not wait for
/// the target to read, act, or agree; it reports whether the message reached a mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// In the target's mailbox. It will be picked up whole on its next prompt.
    Delivered,
    /// No live session at that address. The caller decides what that means — for a report
    /// whose owner has shut down, falling back one rung beats losing finished work.
    Unknown,
    /// A worker addressing something other than its owner. Routing, not policy: whether a
    /// thing is worth saying is judgment and lives in prompts; who may be reached is a
    /// fact and lives here.
    NotPermitted,
}

/// How a turn ended, said by the loop that ran it.
///
/// **The switchboard held every fact about a session except whether its work went well.**
/// [`Status::busy`] and [`Status::queued`] fold into one state word — running · waiting ·
/// idle — and none of the three says anything about the turn that just ended, so a worker
/// that answered its brief and one whose turn died on a 429 were the same row with the same
/// clock. That was not a rendering gap: the fact was nowhere on the wire to render. Measured
/// on 2026-08-18, when three workers failed inside two minutes (`exceeded retry limit, last
/// status: 429`), reported `idle` on the roster and in `hi_session_status`, and were told
/// "Continue now; do not leave this idle" — recovery by nagging what looked like a lazy
/// session, because nothing anywhere said it had fallen over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    /// It ran to the end. Whether it *achieved* anything is the reader's to judge from what
    /// it said; this is only the mechanical fact that the turn was not cut short.
    Completed,
    /// It died, and this is why — one line, already capped ([`OUTCOME_LINE_CHARS`]).
    Failed(String),
    /// Cancelled — by its owner, or by a shutdown. Not a fault, and it must not read as one:
    /// a stopped worker is a decision somebody made.
    Interrupted,
}

impl TurnOutcome {
    /// The word this outcome renders as, everywhere. One spelling for the roster row, the
    /// tool answer, and the ledger line, for the same reason the state word has one:
    /// a journey test greps what the page shows.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed(_) => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    /// The failure line, on a failure only.
    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Failed(err) => Some(err.as_str()),
            _ => None,
        }
    }

    /// Whether this is an ending a reader should chase. `completed` is the answer nobody
    /// needs told, and a row that announces it teaches the eye to skip the field.
    pub fn is_trouble(&self) -> bool {
        !matches!(self, Self::Completed)
    }
}

/// How the last turn ended, and when it ended.
///
/// The clock is its own rather than [`Status::state_since`], which moves again the moment
/// mail lands on the quiet session — so a worker that failed four minutes ago and has had a
/// message waiting since would otherwise report its failure as ten seconds old.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnEnd {
    pub outcome: TurnOutcome,
    pub at: DateTime<Utc>,
}

/// What a session is and how it is doing — **metadata only, no content.**
///
/// Separate from reading its messages on purpose. "Is it still going?" and "what did it
/// find?" are asked at completely different rates, and only the second should cost
/// context.
#[derive(Debug, Clone)]
pub struct Status {
    pub id: SessionId,
    pub role: Role,
    /// The session that created this one and to which its work answers.
    pub owner: Option<SessionId>,
    /// What it is working on, as **one line a person can read**: a headline, written by
    /// whoever asked for the work.
    ///
    /// **This is not the brief, and that is the whole point.** It used to be — a worker was
    /// registered under the instruction it was sent, which for real work is a paragraph or
    /// five, so every reader of this field showed a clause and an ellipsis: the roster card,
    /// `session_status`, the boot glance's resume offer. A first-clause-of-a-paragraph is the
    /// one summary nobody would have written on purpose, because the sentence a brief opens
    /// with is setup, never the subject.
    ///
    /// So the caller writes both: `create_worker` takes a `title` (this) and a `task` (the
    /// brief). The brief goes where a brief belongs — the session's first prompt, whole, on
    /// the wire log — and never through the switchboard, which has no reader for a paragraph.
    /// Capped and flattened to one line by [`headline`] on the way in, because what an agent
    /// hands over is not something a roster can be reflowed by.
    pub title: String,
    /// The ledger subject this session was created to serve, for a worker created against a
    /// task. `None` for the rungs, and for an errand nobody wrote down.
    ///
    /// **This is the whole of the task↔worker join, and it is deliberately live-only.** The
    /// alternative was a field on the task facet naming its worker, and that would be a second
    /// copy of a fact the switchboard already holds — free to disagree with it, and unable to
    /// be right after a restart, when the session it names no longer exists. Here the join is
    /// computed from whatever is actually registered, every turn, and stored nowhere: a task
    /// with no live worker reads as *nobody on it* because there genuinely is nobody, not
    /// because a field went stale.
    ///
    /// It is the ledger's **key** — the directory name under `memory/facets/tasks/` — so it
    /// survives a retitling that would break a match on prose. Which is what
    /// [`title`](Self::title) is: a line written for a reader, never a key.
    pub subject: Option<String>,
    /// Mid-turn right now, versus idle and waiting.
    pub busy: bool,
    /// Whether anything is queued for its next turn.
    pub queued: bool,
    pub turns: u64,
    pub started: DateTime<Utc>,
    /// When this session last **changed state** — opened a turn, finished one, or had mail
    /// land on a quiet inbox.
    ///
    /// `started` cannot answer the question a reader is actually asking. A session that has
    /// been quiet for five minutes and one that finished a turn two seconds ago have the
    /// same `started`, so the only clock on the roster measured the wrong thing: uptime,
    /// when what says whether anything is wrong is *how long it has been like this*. A turn
    /// running for 12 seconds is working; one running for 40 minutes is stuck, and until
    /// this field there was nothing on the wire that could tell them apart.
    pub state_since: DateTime<Utc>,
    /// The last thing this session was seen **doing** — a tool call, a shell command, a
    /// thought — as one short line, or `None` before it has done anything.
    ///
    /// Distinct from its output tail ([`Registry::messages`]), and the distinction is the
    /// point. `output` is what a session has *said*, which is what its owner reads to learn
    /// what it found; this is what it is *doing*, which is what a person reads to learn
    /// whether it is alive. Folding the two would put tool noise into `SessionMessages` and
    /// make a report unreadable — and leaving `doing` out is why a worker four minutes into
    /// a shell command showed a blank line on the roster, which is the same "silence read as
    /// health" this whole surface exists to end.
    pub doing: Option<String>,
    /// When [`doing`](Self::doing) was last replaced, or `None` alongside a `None` `doing`.
    ///
    /// A line with no age says a session is alive and nothing more. `$ cargo test` four
    /// minutes in is working; the same line forty minutes in is hung, and those are the two
    /// answers a reader wants from a busy row. Without this the roster could not distinguish
    /// them — which is the same shape as the `tail`/`doing` split one level down: it is not
    /// enough to know a thing happened, you have to know when.
    pub doing_at: Option<DateTime<Utc>>,
    /// How the last **finished** turn ended, and when — `None` until this session has
    /// finished one. See [`TurnOutcome`] for what it exists to answer.
    ///
    /// It describes the previous turn while a new one runs, which is why every reader draws
    /// it on a quiet row only: mid-turn, what the session is doing now is the answer, and
    /// last turn's ending is the kind of stale fact that reads as current.
    pub last_turn: Option<TurnEnd>,
}

/// One message in flight, with the return address the registry stamped on it.
///
/// **`from` travels with the text and is not optional.** A reply is just a message going
/// the other way, so a message that arrives anonymously is one that cannot be answered —
/// and "answer whoever asked" is the whole of a worker's addressing rule.
#[derive(Debug, Clone)]
pub struct Message {
    /// Who to answer — or `None` when the **host** put this here rather than another
    /// agent. A follow-up the reaction loop hands down is not a message from a colleague,
    /// and rendering it with a return address would put a second voice in a room that
    /// has only one.
    pub from: Option<SessionId>,
    pub text: String,
}

/// Render a batch of mail as the text that goes into the recipient's next prompt.
///
/// **One renderer, here, because there is one mailbox.** There were three — one per
/// driver — and they had already drifted into three different strings, three different
/// separators, and one that forgot to trim. Turning an inbox into a prompt is the
/// switchboard's job, not something each rung reinvents; a rung decides what to *do* with
/// its mail, never what it looks like.
///
/// A `from` is a return address, so it is shown: whoever reads this can answer. Host-posted
/// mail (`from: None`) renders bare — the host is not a colleague, and giving it a return
/// address would put a second voice in a room that has one.
pub fn render(batch: &[Message]) -> String {
    batch
        .iter()
        .map(|m| match &m.from {
            Some(from) => format!("(from session {from}) {}", m.text.trim()),
            None => m.text.trim().to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Render `reachable` as the block that goes into an agent's window.
///
/// Empty when there is nobody — an empty section is worse than none, because a heading
/// with nothing under it reads as a load that failed rather than as an honest "no one".
/// What a worker's line says about the task it serves, if anything.
///
/// **The unlinked case is the one that has to speak.** A worker with a subject is already
/// reported on its task's own projected line, so naming the task here just ties the two
/// together by id. A worker *without* one appears on no task line at all — so from the ledger's
/// side its task reads `nobody on it` while the work is running, and the natural response to
/// that line is to start a second worker on it. Saying "not linked to any task" is what puts
/// the two halves of that mistake in one window: a task claiming nobody, and a session claiming
/// no task.
///
/// Said only of a worker whose kind serves the ledger at all
/// ([`crate::identity::WorkerType::expects_a_subject`]). An organizer has no task to be
/// missing, and marking one is how the phrase stops being read.
fn link_note(e: &Entry) -> String {
    let Some(kind) = e.role.worker_type() else {
        return String::new();
    };
    match e.subject.as_deref() {
        Some(subject) => format!(" — on task `{subject}`"),
        None if kind.expects_a_subject() => " — not linked to any task".to_string(),
        None => String::new(),
    }
}

pub fn render_reachable(who: &[(String, SessionId)]) -> String {
    if who.is_empty() {
        return String::new();
    }
    // **The tool is named in full, and that is load-bearing.** The agent runtime hands
    // every session its own `send_message` for a sub-agent tree it keeps inside one
    // thread; a rung told to "send with `send_message`" reached *that* one, which answered
    // `live agent path not found` on its own stderr and delivered nothing. This block is
    // rebuilt into every rung's window on every turn, so an unqualified verb here outvotes
    // whatever the prompt says.
    let mut s = String::from(
        "## Who you can reach right now\nSend with `hi_send_message`, using the id.\n",
    );
    for (label, id) in who {
        s.push_str(&format!("- `{id}` — {label}\n"));
    }
    s
}

/// A session's inbox: messages merged rather than queued.
///
/// Several landing while a session is mid-turn are picked up **together**, so it reads
/// all of them in one prompt rather than running each as its own round-trip. No
/// LLM-smart merge — they are handed over in arrival order and the receiving model
/// reads them as one batch.
#[derive(Default)]
struct Inbox {
    pending: Vec<Message>,
    closed: bool,
}

struct Entry {
    role: Role,
    owner: Option<SessionId>,
    /// One line for a reader — see [`Status::title`]. Already flattened and capped.
    title: String,
    /// The ledger subject this session serves — see [`Status::subject`].
    subject: Option<String>,
    busy: bool,
    turns: u64,
    started: DateTime<Utc>,
    /// When `busy`, or the emptiness of `inbox.pending`, last changed — see
    /// [`Status::state_since`].
    state_since: DateTime<Utc>,
    inbox: Inbox,
    /// Bounded tail of what this session has said, for `SessionMessages`.
    output: String,
    /// The last thing it was seen doing — see [`Status::doing`].
    doing: Option<String>,
    /// When `doing` was last replaced — see [`Status::doing_at`].
    doing_at: Option<DateTime<Utc>>,
    /// How the last finished turn ended — see [`Status::last_turn`].
    last_turn: Option<TurnEnd>,
    /// The codex thread hosting this session, once `thread/start` has answered. `None`
    /// between registration and that moment — the session exists first, deliberately.
    thread: Option<String>,
    /// Woken when something lands, so an idle session picks it up without polling.
    notify: std::sync::Arc<Notify>,
}

impl Entry {
    /// This entry as the metadata a reader gets. One place, because there are three
    /// callers ([`Registry::status`], [`Registry::session_of_role`],
    /// [`Registry::statuses`]) and they were three copies of the same nine-field literal —
    /// so a field added to `Status` had three chances to be forgotten in one.
    fn status(&self, id: SessionId) -> Status {
        Status {
            id,
            role: self.role,
            owner: self.owner.clone(),
            title: self.title.clone(),
            subject: self.subject.clone(),
            busy: self.busy,
            queued: !self.inbox.pending.is_empty(),
            turns: self.turns,
            started: self.started,
            state_since: self.state_since,
            doing: self.doing.clone(),
            doing_at: self.doing_at,
            last_turn: self.last_turn.clone(),
        }
    }

    /// Stamp a state change, if this is one.
    ///
    /// Every transition goes through here rather than each call site writing `Utc::now()`,
    /// because the field only means anything if *all* of them stamp it: one path that
    /// changes `busy` without moving the clock reports a turn as older than it is, and the
    /// reading it exists to give — how long has it been like this — is silently wrong
    /// exactly on the path that skipped it.
    fn note_state_change(&mut self, changed: bool) {
        if changed {
            self.state_since = Utc::now();
        }
    }

    /// Whether this entry is quiet: no turn in flight and nothing waiting. Mail landing on
    /// a quiet session is a state change (idle → waiting); mail landing on a busy or
    /// already-queued one is not.
    fn is_quiet(&self) -> bool {
        !self.busy && self.inbox.pending.is_empty()
    }
}

/// A registration that ends when it goes out of scope.
///
/// The reaction loop leaves by several paths — inbound closed, closed mid-settle, shutdown
/// — and a registration released at only some of them is how the agent ends up with more
/// than one Reaction, `reachable` then offering an arbitrary dead one. Rather than
/// remember every exit, hold this: the exits are then not something anyone has to get
/// right again, including whoever adds the next one.
pub struct Registration {
    id: SessionId,
    /// Woken when mail lands. Cloneable and outlives nothing — dropping the handle is
    /// what closes the registration, not dropping this.
    pub mail: std::sync::Arc<Notify>,
}

impl Registration {
    pub fn id(&self) -> SessionId {
        self.id.clone()
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        global().unregister(&self.id);
    }
}

/// Register a session in the process switchboard, releasing it when the returned handle
/// is dropped. The scope-bound form of [`Registry::register`]; prefer it for anything
/// whose lifetime is a scope rather than a task.
pub fn register_scoped(
    id: SessionId,
    role: Role,
    owner: Option<SessionId>,
    title: String,
) -> Registration {
    // Rungs only — the scope-bound form is for sessions whose lifetime is a scope, and a
    // worker's is a task. So there is no ledger subject to record here.
    //
    // A rung needs no title/brief split either: what it is doing is standing and already a
    // phrase ("the shared brain"), which is why this form takes the one line and stops.
    let mail = global().register(id.clone(), role, owner, title, None);
    Registration { id, mail }
}

/// The switchboard. One per process.
pub struct Registry {
    sessions: Mutex<HashMap<SessionId, Entry>>,
    activity: watch::Sender<u64>,
    /// The durable session directory, once [`Registry::attach_index`] has been called at
    /// boot. Absent in tests and anywhere without a data dir, in which case the switchboard
    /// behaves exactly as it did before — live-only, nothing kept.
    index: std::sync::OnceLock<index::Writer>,
    /// Sessions that are no longer live, newest first — seeded from the directory at boot
    /// and appended to as sessions close.
    ///
    /// **In memory because the read is per-poll and the file is unpruned.** A roster
    /// refreshing every few seconds must not re-read a months-old append-only log to answer
    /// a question about this afternoon; the file is the durable copy and this is the working
    /// set, which is the same split [`Registry::messages`] already makes against the frame
    /// log.
    recent: Mutex<Vec<index::Ended>>,
    /// The thread each resident rung resumes at boot, keyed by role, seeded once by
    /// [`Registry::attach_index`] and **taken** rather than read.
    ///
    /// Take-once is the discard rule, expressed as a data structure. The first session a
    /// rung opens in a run is the resume; every later one — and every reopen after a turn
    /// fails — finds the slot empty and opens cold. So a thread wedged badly enough to
    /// break a turn cannot be handed back to the session replacing it, and a thread that
    /// crashed the host is resumed exactly once before the next boot starts fresh.
    resumable: Mutex<HashMap<String, String>>,
    /// The errands the last restart killed, for the boot glance to offer Cognition.
    ///
    /// Seeded at [`Registry::attach_index`] rather than derived from `recent` on demand,
    /// because `recent` grows this run's own ends as sessions close and
    /// [`index::lost_workers`] reads the head of the list to decide which run "the previous
    /// one" was. A read taken after the first session closes would answer about this run and
    /// find nothing.
    ///
    /// **Outstanding rather than a snapshot: an entry leaves when it stops being owed.** It
    /// was read and never taken while the only reader was the one-shot boot note, and a
    /// second reader is what made that wrong. The ledger now consults it to say *why* a
    /// `doing` task has nobody on it, and a list frozen at boot answers that with the state
    /// of the boot for the rest of the run: a task restaffed forty minutes ago still reads as
    /// cut off. The direction that actually cost something is the other one — a thread
    /// already resumed stays on the offer, so the same dead errand can be picked up twice and
    /// two sessions each believe they own it.
    ///
    /// It drains at the two points an errand stops being owed: its thread is taken
    /// ([`Registry::take_lost_thread`]), or a live worker registers under its subject
    /// ([`Registry::register`]). A task *closed* without either needs no drain — every reader
    /// here is asked only about tasks that are active and `doing`, so an entry for a task
    /// that left `doing` is already unreachable.
    lost: Mutex<Vec<index::Ended>>,
    /// What one session said to another, newest last, capped at [`mail::KEPT`].
    ///
    /// [`Registry::send`] used to leave no trace a reader could reach: the text went into a
    /// mailbox, out again into a prompt, and after that existed only as a tool call in one
    /// frame log and a paragraph in another. See [`mail`] for why this is in memory and
    /// what it is not.
    traffic: Mutex<VecDeque<mail::Sent>>,
}

impl Default for Registry {
    fn default() -> Self {
        let (activity, _) = watch::channel(0);
        Self {
            sessions: Mutex::new(HashMap::new()),
            activity,
            index: std::sync::OnceLock::new(),
            recent: Mutex::new(Vec::new()),
            resumable: Mutex::new(HashMap::new()),
            lost: Mutex::new(Vec::new()),
            traffic: Mutex::new(VecDeque::new()),
        }
    }
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Announce a live session. `id` comes from [`mint`], claimed before the session was
    /// opened.
    ///
    /// `title` is the one line this session shows up as — see [`Status::title`]. It is
    /// flattened and capped here rather than at the caller, because there are several
    /// callers and only one roster.
    pub fn register(
        &self,
        id: SessionId,
        role: Role,
        owner: Option<SessionId>,
        title: String,
        subject: Option<String>,
    ) -> std::sync::Arc<Notify> {
        let notify = std::sync::Arc::new(Notify::new());
        let started = Utc::now();
        let title = headline(&title, TITLE_CHARS);
        let owner_for_record = owner.clone();
        {
            let mut map = self.sessions.lock().unwrap();
            map.insert(
                id.clone(),
                Entry {
                    role,
                    owner,
                    title: title.clone(),
                    subject: subject.clone(),
                    busy: false,
                    turns: 0,
                    started,
                    // A fresh session is idle, and has been since it existed.
                    state_since: started,
                    inbox: Inbox::default(),
                    output: String::new(),
                    doing: None,
                    doing_at: None,
                    last_turn: None,
                    thread: None,
                    notify: notify.clone(),
                },
            );
        }
        // **A live worker under this subject ends that errand's claim on the offer.** The
        // task has somebody on it now, whether this session resumed the dead thread or
        // started cold, and an entry left behind would go on explaining an absence that is
        // over — see [`Registry::lost`](#structfield.lost).
        if let Some(subject) = subject.as_deref() {
            self.lost.lock().unwrap().retain(|end| end.subject.as_deref() != Some(subject));
        }
        // Recorded on the way in, not only on the way out, because the way out is exactly
        // what a crash skips. An `opened` with no `closed` is how a restart-killed session
        // becomes visible at all — see [`index::EndedHow::Restart`].
        if let Some(writer) = self.index.get() {
            writer.write(index::opened_record(
                crate::foundation::run::id(),
                &id,
                role,
                owner_for_record,
                &title,
                subject.as_deref(),
                started,
            ));
        }
        self.note_activity();
        notify
    }

    /// Start recording sessions to the durable directory under `data_dir`, and seed the
    /// recent-ends list from what previous runs left there.
    ///
    /// Called once at boot. Idempotent: a second call is ignored, because the writer owns a
    /// spawned task and two of them would interleave lines into one file.
    pub async fn attach_index(&self, data_dir: std::path::PathBuf) {
        let run = crate::foundation::run::id();
        let seeded = index::seed(&data_dir, run).await;
        if self.index.set(index::Writer::start(data_dir)).is_err() {
            tracing::warn!("the session index is already attached; ignoring");
            return;
        }
        let found = seeded.len();
        let lost = seeded.iter().filter(|e| e.how == index::EndedHow::Restart).count();
        let resumable = index::resumable(&seeded);
        let resuming = resumable.len();
        // Before `recent` takes the list: `lost_workers` reads its head to decide which run
        // was the previous one, and this run's first close would move that head.
        let offered = index::lost_workers(&seeded);
        let offering = offered.len();
        *self.lost.lock().unwrap() = offered;
        *self.resumable.lock().unwrap() = resumable;
        *self.recent.lock().unwrap() = seeded;
        // Worth a line at boot: `lost` is the count of sessions a previous run died
        // underneath, which nothing used to be able to say out loud. `resuming` is how many
        // resident rungs are picking their previous thread back up — zero on a fresh
        // install, and zero for any rung whose last run predates thread recording. `offering`
        // is the subset of `lost` that is a worker with a thread, which the boot glance hands
        // to Cognition to pick from — never resumed by the host, only offered.
        tracing::info!(
            run,
            recent = found,
            lost,
            resuming,
            offering,
            "session directory attached"
        );
    }

    /// Record the codex thread a session opened, in memory and in the directory.
    ///
    /// Called once per session, right after `thread/start` answers. A session that never
    /// gets one (the spawn failed, the process died first) simply has no thread on its row,
    /// which reads as "there is no mind to go back to" rather than as missing data.
    pub fn note_thread(&self, id: &SessionId, thread_id: &str) {
        if let Some(e) = self.sessions.lock().unwrap().get_mut(id) {
            e.thread = Some(thread_id.to_string());
        }
        if let Some(writer) = self.index.get() {
            writer.write(index::thread_record(
                crate::foundation::run::id(),
                id,
                thread_id,
                Utc::now(),
            ));
        }
    }

    /// Take the thread `role` should resume, leaving nothing behind — see
    /// [`Registry::resumable`](#structfield.resumable) for why taking rather than reading is
    /// the discard rule.
    pub fn take_resumable(&self, role: Role) -> Option<String> {
        self.resumable.lock().unwrap().remove(role.as_str())
    }

    /// The errands the last restart killed, newest first — see [`index::lost_workers`].
    ///
    /// Offered, not resumed: the caller is the boot glance, and what it does with these is
    /// put them in front of Cognition.
    pub fn lost_workers(&self) -> Vec<index::Ended> {
        self.lost.lock().unwrap().clone()
    }

    /// Take the offer for `thread`, leaving nothing behind — `true` if it was on the offer.
    ///
    /// **The check and the discard are one act.** Validating with a read and removing later
    /// leaves a window where two callers both pass, and what comes out the other side is two
    /// sessions resumed from one dead errand's mind, each registered as owning it. Taking is
    /// also why a resume that fails downstream gets no second go at the thread: the rule
    /// [`Registry::resumable`](#structfield.resumable) already states, for the reason it
    /// states it — a retry is a cold open, never a second claim on the same mind.
    pub fn take_lost_thread(&self, thread: &str) -> bool {
        let mut lost = self.lost.lock().unwrap();
        let before = lost.len();
        lost.retain(|end| end.thread.as_deref() != Some(thread));
        before != lost.len()
    }

    /// The ledger subjects whose worker the last restart killed and nothing has picked back
    /// up, for the projection to say so — see [`crate::mind::memory::tasks::OnIt`].
    pub fn lost_subjects(&self) -> std::collections::HashSet<String> {
        self.lost.lock().unwrap().iter().filter_map(|end| end.subject.clone()).collect()
    }

    /// Unregister every live session, in id order.
    ///
    /// For the shutdown path: a graceful stop genuinely ends these, and the host is still
    /// alive to say so. That is the whole difference between the directory reporting
    /// [`index::EndedHow::Closed`] and [`index::EndedHow::Restart`] — the latter means
    /// nothing got to record an end, and it only carries a warning if a clean exit does not
    /// produce it.
    ///
    /// Ids are collected before unregistering because [`Registry::unregister`] takes the
    /// same lock a snapshot would still be holding.
    pub fn close_all(&self) {
        let ids: Vec<SessionId> = {
            let map = self.sessions.lock().unwrap();
            let mut rows: Vec<(DateTime<Utc>, SessionId)> =
                map.iter().map(|(id, e)| (e.started, id.clone())).collect();
            // Oldest first, which is what sorting on the ordinal used to mean. A slug
            // sorts alphabetically and would close a worker before the rung that owns it.
            rows.sort_unstable();
            rows.into_iter().map(|(_, id)| id).collect()
        };
        if ids.is_empty() {
            return;
        }
        tracing::info!(sessions = ids.len(), "closing the switchboard");
        for id in ids {
            self.unregister(&id);
        }
    }

    /// Wait until every record queued for the session directory has reached the disk.
    ///
    /// Pairs with [`close_all`](Self::close_all) on the shutdown path — see
    /// [`index::Writer::flush`] for why closing without flushing would write the crash
    /// signature on a clean exit. A no-op when no index is attached.
    pub async fn flush_index(&self) {
        if let Some(writer) = self.index.get() {
            writer.flush().await;
        }
    }

    /// Sessions that are no longer live, newest first, at most `limit`.
    pub fn recent_ended(&self, limit: usize) -> Vec<index::Ended> {
        let recent = self.recent.lock().unwrap();
        recent.iter().take(limit).cloned().collect()
    }

    /// Drop a session. Anything still in its inbox goes with it — undelivered is the
    /// honest outcome, and the sender was told `Delivered` about a mailbox, never about
    /// an outcome.
    pub fn unregister(&self, id: &SessionId) {
        let removed = if let Some(mut e) = self.sessions.lock().unwrap().remove(id) {
            e.inbox.closed = true;
            Some(index::ended_now(
                crate::foundation::run::id(),
                id,
                e.role,
                e.owner,
                &e.title,
                e.subject.as_deref(),
                e.turns,
                e.started,
                e.thread.clone(),
            ))
        } else {
            None
        };
        if let Some(ended) = removed {
            // The file and the in-memory list get the same row. The list is what a poll
            // reads; the file is what survives this process.
            if let Some(writer) = self.index.get() {
                writer.write(index::closed_record(&ended));
            }
            index::push_recent(&mut self.recent.lock().unwrap(), ended);
            self.note_activity();
        }
    }

    /// Send `message` to `to`, from `from`.
    ///
    /// **`from` is supplied by the host, not by the calling agent.** The host knows who is
    /// calling; letting an agent name itself is letting it impersonate another.
    ///
    /// One direction, no reply. The return value says whether it reached a mailbox — a
    /// reply, if there is one, arrives later as its own `send` in the other direction.
    /// **`to` is a session id, and that is the only address there is.**
    ///
    /// A worker's id comes back from `CreateWorker`; a standing rung's is projected into
    /// the window of whoever may reach it ([`Registry::reachable`]). What this replaced —
    /// letting an agent name a destination by some other string and searching for the
    /// session behind it — was
    /// retrieval, and a retrieval that misses is indistinguishable from nobody being
    /// there. Being told who is live, every turn, is strictly more information than being
    /// allowed to guess, and it turns this from a scan into a map lookup.
    pub fn send(&self, from: &SessionId, to: &SessionId, message: String) -> Delivery {
        let delivery = {
            let mut map = self.sessions.lock().unwrap();

            // A worker answers to whoever asked, and to nobody else.
            if let Some(sender) = map.get(from)
                && sender.role.is_worker()
                && sender.owner.as_ref() != Some(to)
            {
                return Delivery::NotPermitted;
            }

            let Some(entry) = map.get_mut(to) else {
                return Delivery::Unknown;
            };
            if entry.inbox.closed {
                return Delivery::Unknown;
            }
            entry.note_state_change(entry.is_quiet());
            // Recorded before the text is moved into the mailbox, and only on this path:
            // the two `return`s above never reached one, and a refusal is the sender's
            // mistake rather than something these two said to each other.
            mail::push(
                &mut self.traffic.lock().unwrap(),
                mail::Sent::new(from.clone(), to.clone(), &message),
            );
            entry.inbox.pending.push(Message { from: Some(from.clone()), text: message });
            entry.notify.notify_one();
            Delivery::Delivered
        };
        self.note_activity();
        delivery
    }

    /// Everything `a` and `b` have said to each other that is still in the ring, oldest
    /// first — both directions, because an arrow joins two sessions and half a conversation
    /// is not one.
    ///
    /// Returns the **tail** when there is more than `limit`, together with the total, for
    /// the same reason every other reader here does: the end of an exchange is what someone
    /// opening it came for.
    pub fn traffic_between(&self, a: &SessionId, b: &SessionId, limit: usize) -> (Vec<mail::Sent>, usize) {
        let ring = self.traffic.lock().unwrap();
        let all: Vec<mail::Sent> = ring.iter().filter(|m| m.between(a, b)).cloned().collect();
        let total = all.len();
        let tail = all[total.saturating_sub(limit)..].to_vec();
        (tail, total)
    }

    /// Who `asker` may reach right now, as `(label, id)` — the projection that replaced
    /// name-a-destination addressing.
    ///
    /// Deliberately narrow, and narrow **per asker**, because this is the whole of what an
    /// agent knows about the rest of the agent: what it is offered here is what it can
    /// do. A worker gets its owner and nothing else, which is also the only thing the
    /// routing rule would let it send to; Reaction's rungs get the shared brain; Cognition
    /// gets Reaction, because that is the one way anything reaches the person.
    ///
    /// Rebuilt every turn by the caller. There is no cache and should not be: the answer
    /// is only true for as long as those sessions are up, and a stale id is worse than no
    /// id — it sends somewhere real.
    pub fn reachable(&self, asker: &SessionId) -> Vec<(String, SessionId)> {
        let map = self.sessions.lock().unwrap();
        let Some(me) = map.get(asker) else { return Vec::new() };

        let mut out: Vec<(String, SessionId)> = Vec::new();
        match me.role {
            // Its owner, which the routing rule already limits it to.
            Role::Worker(_) => {
                if let Some(owner) = &me.owner {
                    out.push(("the session that asked for this work".to_string(), owner.clone()));
                }
            }
            // Reaction hands work up, and that is all it addresses.
            Role::Reaction => {
                if let Some((id, _)) = map.iter().find(|(_, e)| e.role == Role::Cognition) {
                    out.push(("cognition — the shared brain".to_string(), id.clone()));
                }
            }
            // Reaction, so anything worth saying has somewhere to land, plus whatever
            // this rung has running. A Reaction that is cold simply is not here, which is
            // the fact Cognition needs before it decides to hold a result rather than
            // send at it.
            Role::Cognition | Role::Reflection => {
                for (id, e) in map.iter() {
                    if e.role == Role::Reaction {
                        out.push(("what reaches the person".to_string(), id.clone()));
                    }
                }
                for (id, e) in map.iter() {
                    if e.owner.as_ref() == Some(asker) {
                        out.push((
                            format!("your worker: {}{}", e.title.trim(), link_note(e)),
                            id.clone(),
                        ));
                    }
                }
            }
        }
        // Oldest first. The ordinal used to carry this for free — the rung a worker
        // reports to was minted before the worker — and a slug does not sort that way.
        out.sort_by_key(|(_, id)| map.get(id).map(|e| e.started));
        out
    }

    /// Whether any live worker owned by `asker` is running without a task subject.
    ///
    /// The one question the ledger cannot answer about itself. A worker created without a
    /// `subject` is invisible to the task↔worker join, so its task reads *nobody on it* while
    /// the work is genuinely in flight — and the obvious reading of that line is to put
    /// someone on it, which starts a second worker on work already running. That duplicate is
    /// the real cost of a missed label, and it is worse than the silence the join was built to
    /// end, so the omission has to be visible from the same window that invites the mistake.
    ///
    /// Asked only of the kinds that serve the ledger — same rule as [`link_note`], because a
    /// disagreement between the two would mean a rung told there is an unlabelled worker and
    /// shown a roster where none is marked.
    pub fn has_unlinked_worker(&self, asker: &SessionId) -> bool {
        self.sessions.lock().unwrap().values().any(|e| {
            e.owner.as_ref() == Some(asker)
                && e.subject.is_none()
                && e.role.worker_type().is_some_and(|k| k.expects_a_subject())
        })
    }

    /// Put `text` in `id`'s inbox **on the host's own behalf** — no sender, and none of
    /// the addressing rules that govern one agent reaching another.
    ///
    /// The rules exist to stop an agent talking somewhere it has no business; the host
    /// is not an agent and is the thing that enforces them. This is how a follow-up
    /// reaches a warm session, and it answers the one question the caller actually has:
    /// is that session still able to take work, or has it closed and does this need a
    /// fresh one?
    pub fn post(&self, id: &SessionId, text: String) -> Delivery {
        let delivery = {
            let mut map = self.sessions.lock().unwrap();
            let Some(entry) = map.get_mut(id) else {
                return Delivery::Unknown;
            };
            if entry.inbox.closed {
                return Delivery::Unknown;
            }
            entry.note_state_change(entry.is_quiet());
            entry.inbox.pending.push(Message { from: None, text });
            entry.notify.notify_one();
            Delivery::Delivered
        };
        self.note_activity();
        delivery
    }

    /// Mark a session's turn as running.
    ///
    /// `take_pending` already performs this transition for mailbox-driven turns.
    /// Directly-driven turns (Reaction's queue and a worker's initial task) use this
    /// method so every status reader observes the same lifecycle.
    pub fn start_turn(&self, id: &SessionId) {
        let changed = {
            let mut map = self.sessions.lock().unwrap();
            let Some(entry) = map.get_mut(id) else {
                return;
            };
            if entry.busy {
                false
            } else {
                entry.busy = true;
                entry.turns += 1;
                entry.note_state_change(true);
                true
            }
        };
        if changed {
            self.note_activity();
        }
    }

    /// Take everything queued for `id`, if anything is. Marks the session busy — it is
    /// about to take a turn, and an agent with a turn in flight is not idle.
    pub fn take_pending(&self, id: &SessionId) -> Option<Vec<Message>> {
        let batch = {
            let mut map = self.sessions.lock().unwrap();
            let entry = map.get_mut(id)?;
            if entry.inbox.pending.is_empty() {
                return None;
            }
            // Only when it was not already mid-turn: emptying the inbox of a *busy* session
            // leaves it running, which is the state it was already in.
            entry.note_state_change(!entry.busy);
            if !entry.busy {
                entry.busy = true;
                entry.turns += 1;
            }
            std::mem::take(&mut entry.inbox.pending)
        };
        self.note_activity();
        Some(batch)
    }

    /// Drain queued mail without opening a turn.
    ///
    /// Reaction folds this mailbox into its separate input queue, then starts one
    /// combined turn after the settle window. Marking a turn here would create a
    /// false busy/idle edge before that real turn begins.
    pub fn drain_pending(&self, id: &SessionId) -> Option<Vec<Message>> {
        let batch = {
            let mut map = self.sessions.lock().unwrap();
            let entry = map.get_mut(id)?;
            if entry.inbox.pending.is_empty() {
                return None;
            }
            // Waiting → idle. A busy session was, and stays, running.
            entry.note_state_change(!entry.busy);
            std::mem::take(&mut entry.inbox.pending)
        };
        self.note_activity();
        Some(batch)
    }

    /// Close `id`'s inbox because its owner said so, and wake whoever is waiting on it.
    ///
    /// This replaced an atomic `take_pending_or_close`, which closed the mailbox only on
    /// finding it empty. That existed to settle a *race* — a message landing at the instant
    /// a session idled out — and the idle-out is gone, so the race is too. Ending a session
    /// is now a decision with an author, and it closes unconditionally: the owner has
    /// finished with the errand, and mail it queued a moment before changing its mind is
    /// not a reason to keep a subprocess alive. Undelivered is the honest outcome, and
    /// [`unregister`](Self::unregister) already says so for the same case.
    ///
    /// Notifying after closing is what makes the waiter's loop terminate: `Notify` holds a
    /// permit if the wake races ahead of the wait, so a worker parked in
    /// [`wait_for_mail`](crate::body::reaction::workers) sees `closed` on its next pass
    /// rather than sleeping through it.
    ///
    /// Returns whether there was a live session to close.
    pub fn close_inbox(&self, id: &SessionId) -> bool {
        let notify = {
            let mut map = self.sessions.lock().unwrap();
            let Some(entry) = map.get_mut(id) else {
                return false;
            };
            entry.inbox.closed = true;
            entry.notify.clone()
        };
        notify.notify_one();
        self.note_activity();
        true
    }

    /// Whether `id`'s inbox will take no more work — closed, or gone entirely.
    ///
    /// A session that has left the switchboard answers `true` for the same reason a closed
    /// one does: there is nothing more coming, which is the only question the caller has.
    pub fn inbox_closed(&self, id: &SessionId) -> bool {
        self.sessions
            .lock()
            .unwrap()
            .get(id)
            .map(|e| e.inbox.closed)
            .unwrap_or(true)
    }

    /// The handle woken when mail lands for `id`, for a loop that wants to wait on its
    /// own inbox without polling. Same `Notify` [`register`](Self::register) returned.
    pub fn notifier(&self, id: &SessionId) -> Option<std::sync::Arc<Notify>> {
        self.sessions.lock().unwrap().get(id).map(|e| e.notify.clone())
    }

    /// Mark a turn finished, saying how it ended.
    ///
    /// **The outcome is a parameter and not a second call**, so that no loop can drop a
    /// session out of `busy` without answering the question a quiet row raises. Every
    /// caller here already holds the answer — it is the `Result` it just matched on — and
    /// the one that genuinely has none is undoing a `start_turn` for a turn that never
    /// ran, which is [`abandon_turn`](Self::abandon_turn) and says so.
    pub fn finish_turn(&self, id: &SessionId, outcome: TurnOutcome) {
        let outcome = match outcome {
            TurnOutcome::Failed(err) => TurnOutcome::Failed(headline(&err, OUTCOME_LINE_CHARS)),
            other => other,
        };
        let changed = {
            let mut map = self.sessions.lock().unwrap();
            if let Some(e) = map.get_mut(id) {
                let changed = e.busy;
                e.busy = false;
                // Recorded whether or not `busy` moved. A loop calling this twice for one
                // turn is reporting the same ending twice, not two turns — but a loop whose
                // `start_turn` was folded into a `take_pending` edge never set `busy` from
                // here at all, and dropping its outcome on that ground would lose the
                // ending of exactly the turns that ran.
                e.last_turn = Some(TurnEnd { outcome, at: Utc::now() });
                e.note_state_change(changed);
                changed
            } else {
                false
            }
        };
        if changed {
            self.note_activity();
        }
    }

    /// Undo a `start_turn` for a turn that never ran — the send that failed, the loop that
    /// exited between accepting a reason to speak and speaking.
    ///
    /// Distinct from [`finish_turn`](Self::finish_turn) because there is no ending to
    /// record: writing `completed` here would report a turn that never happened as a
    /// success, and `failed` would raise an alarm about a turn nothing attempted.
    pub fn abandon_turn(&self, id: &SessionId) {
        let changed = {
            let mut map = self.sessions.lock().unwrap();
            if let Some(e) = map.get_mut(id) {
                let changed = e.busy;
                e.busy = false;
                e.note_state_change(changed);
                changed
            } else {
                false
            }
        };
        if changed {
            self.note_activity();
        }
    }

    /// Replace the one-line title attached to a live session.
    ///
    /// A session is registered before it receives its first real task, so the
    /// switchboard entry must be able to move from a startup placeholder to the work it
    /// was actually handed. Capped like the registration path — a caller replacing a title
    /// is the same kind of caller that wrote the first one.
    pub fn set_title(&self, id: &SessionId, title: String) {
        if let Some(e) = self.sessions.lock().unwrap().get_mut(id) {
            e.title = headline(&title, TITLE_CHARS);
        }
    }

    /// Append to a session's visible output, keeping only the recent tail.
    pub fn record_output(&self, id: &SessionId, chunk: &str) {
        if let Some(e) = self.sessions.lock().unwrap().get_mut(id) {
            e.output.push_str(chunk);
            let n = e.output.chars().count();
            if n > OUTPUT_TAIL_CHARS {
                e.output = e.output.chars().skip(n - OUTPUT_TAIL_CHARS).collect();
            }
        }
    }

    /// Note the last thing a session was seen **doing** — see [`Status::doing`].
    ///
    /// One line, replaced rather than appended: this answers "is it alive and on what",
    /// which only the newest answer serves. Long lines are cut, because the caller is
    /// summarizing a tool call and a shell command can be a screenful.
    pub fn record_activity(&self, id: &SessionId, what: &str) {
        let what = what.trim();
        if what.is_empty() {
            return;
        }
        let line: String = match what.char_indices().nth(ACTIVITY_LINE_CHARS) {
            Some((cut, _)) => format!("{}…", &what[..cut]),
            None => what.to_string(),
        };
        if let Some(e) = self.sessions.lock().unwrap().get_mut(id) {
            e.doing = Some(line);
            e.doing_at = Some(Utc::now());
        }
    }

    /// What a session has recently said. Costs context — which is exactly why it is a
    /// different call from [`status`](Self::status).
    pub fn messages(&self, id: &SessionId) -> Option<String> {
        let map = self.sessions.lock().unwrap();
        map.get(id).map(|e| e.output.clone())
    }

    /// Metadata for one session. Cheap by construction — no content crosses.
    pub fn status(&self, id: &SessionId) -> Option<Status> {
        let map = self.sessions.lock().unwrap();
        Some(map.get(id)?.status(id.clone()))
    }

    /// The live session holding `role`, if there is one — for the **singleton** rungs,
    /// where "the Cognition" names a thing rather than a category.
    ///
    /// Lowest id wins if two are somehow up, which is a tie that should not happen and is
    /// resolved deterministically rather than arbitrarily: a `HashMap` iteration order
    /// would make the answer differ between two calls in one turn, and a caller asking
    /// "is it busy" twice and getting two sessions is worse than a caller consistently
    /// reading the older one.
    ///
    /// **Only meaningful for a rung.** Asking for `Role::Worker(_)` gets an arbitrary
    /// worker, which is never a useful question — a worker is addressed by the id its
    /// creator holds.
    pub fn session_of_role(&self, role: Role) -> Option<Status> {
        let map = self.sessions.lock().unwrap();
        let (id, e) = map.iter().filter(|(_, e)| e.role == role).min_by_key(|(_, e)| e.started)?;
        Some(e.status(id.clone()))
    }

    /// Metadata for every live session, oldest first.
    pub fn statuses(&self) -> Vec<Status> {
        let map = self.sessions.lock().unwrap();
        let mut rows: Vec<Status> = map.iter().map(|(id, e)| e.status(id.clone())).collect();
        rows.sort_by(|a, b| a.started.cmp(&b.started).then_with(|| a.id.cmp(&b.id)));
        rows
    }

    /// Subscribe to changes that can affect live activity projection.
    pub fn subscribe_activity(&self) -> watch::Receiver<u64> {
        self.activity.subscribe()
    }

    /// Every session `owner` created, oldest first.
    pub fn children(&self, owner: &SessionId) -> Vec<SessionId> {
        let map = self.sessions.lock().unwrap();
        let mut rows: Vec<(DateTime<Utc>, SessionId)> = map
            .iter()
            .filter(|(_, e)| e.owner.as_ref() == Some(owner))
            .map(|(id, e)| (e.started, id.clone()))
            .collect();
        rows.sort_unstable();
        rows.into_iter().map(|(_, id)| id).collect()
    }

    /// Whether `id` owns anything still live.
    ///
    /// **An agent with live children is not idle.** Reaping an owner out from under
    /// running work is what creates orphans; the fix is to not call it idle in the first
    /// place, so whatever decides to close a session asks this first.
    pub fn has_live_children(&self, id: &SessionId) -> bool {
        let map = self.sessions.lock().unwrap();
        map.values().any(|e| e.owner.as_ref() == Some(id))
    }

    fn note_activity(&self) {
        self.activity.send_modify(|version| *version = version.wrapping_add(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::WorkerType;

    /// A distinct session id, shadowing [`super::mint`] for the tests below.
    ///
    /// These tests are about the switchboard's *plumbing* — mail reaching one mailbox and
    /// not another, a worker refused when it addresses anyone but its owner, a roster
    /// coming back in creation order. Not one of them turns on what a session is called,
    /// and spelling every fixture `person-reader-alice` would hide the property each is
    /// pinning behind scenery. What they need is "another session, distinct from the last".
    ///
    /// Naming it `mint` on purpose: the call sites read the same as before the ids became
    /// slugs, so nothing about *those* tests appears to have changed — because nothing did.
    /// Slug-shaped ids get their own tests in [`slug_tests`], where the shape is the point.
    fn mint() -> SessionId {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        SessionId::from(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }

    fn reg() -> Registry {
        Registry::new()
    }

    /// The arrow between two cards reads this, so it has to carry both directions and
    /// nothing from anybody else.
    #[test]
    fn traffic_between_two_sessions_is_both_directions_and_only_theirs() {
        let r = reg();
        let cognition = mint();
        let reaction = mint();
        let bystander = mint();
        for (id, role) in [
            (&cognition, Role::Cognition),
            (&reaction, Role::Reaction),
            (&bystander, Role::Reflection),
        ] {
            r.register(id.clone(), role, None, "standing".into(), None);
        }

        r.send(&reaction, &cognition, "someone is asking about the deploy".into());
        r.send(&cognition, &reaction, "tell them it is out".into());
        r.send(&bystander, &reaction, "the ledger is filed".into());

        let (between, total) = r.traffic_between(&reaction, &cognition, 50);
        assert_eq!(total, 2, "the bystander's message is not theirs");
        assert_eq!(between.len(), 2);
        // Oldest first: this is read as a conversation, not as a feed.
        assert_eq!(between[0].from, reaction);
        assert_eq!(between[0].text, "someone is asking about the deploy");
        assert_eq!(between[1].from, cognition);

        // Unordered — the same exchange, asked the other way round.
        let (flipped, _) = r.traffic_between(&cognition, &reaction, 50);
        assert_eq!(flipped.len(), 2);
    }

    /// A refused send never reached a mailbox, so it is not something these two said to
    /// each other — it is a mistake, and it belongs in the sender's own transcript.
    #[test]
    fn only_delivered_mail_is_recorded() {
        let r = reg();
        let owner = mint();
        let worker = mint();
        let stranger = mint();
        r.register(owner.clone(), Role::Cognition, None, "the shared brain".into(), None);
        r.register(
            worker.clone(),
            Role::Worker(WorkerType::General),
            Some(owner.clone()),
            "audit the runtimes".into(),
            None,
        );
        r.register(stranger.clone(), Role::Reaction, None, "what reaches the person".into(), None);

        assert_eq!(r.send(&worker, &stranger, "hello".into()), Delivery::NotPermitted);
        assert_eq!(r.send(&owner, &mint(), "anyone there".into()), Delivery::Unknown);
        assert_eq!(r.send(&worker, &owner, "done".into()), Delivery::Delivered);

        assert_eq!(r.traffic_between(&worker, &stranger, 50).1, 0);
        assert_eq!(r.traffic_between(&worker, &owner, 50).1, 1);
    }

    /// The ring is a working set and not the record: it drops the oldest rather than
    /// growing, and a reader asking for less than it holds gets the *end* of the exchange.
    #[test]
    fn the_ring_keeps_the_tail() {
        let r = reg();
        let a = mint();
        let b = mint();
        r.register(a.clone(), Role::Cognition, None, "the shared brain".into(), None);
        r.register(b.clone(), Role::Reaction, None, "what reaches the person".into(), None);

        for n in 0..(mail::KEPT + 10) {
            r.send(&a, &b, format!("message {n}"));
        }

        let (tail, total) = r.traffic_between(&a, &b, 3);
        assert_eq!(total, mail::KEPT, "the oldest are dropped, not accumulated");
        assert_eq!(tail.len(), 3);
        assert_eq!(tail[2].text, format!("message {}", mail::KEPT + 9));
    }

    /// The offer survives the trip through `attach_index`, and survives this run's own
    /// sessions closing on top of it.
    ///
    /// The ordering inside `attach_index` is the part worth pinning: [`index::lost_workers`]
    /// reads the head of the ends list to decide which run was the previous one, and `recent`
    /// grows this run's ends as sessions close. Snapshot it after `recent` takes the list — or
    /// derive it on demand — and the first session to close moves that head to *this* run,
    /// where there are no lost errands, and the offer silently empties. Which is exactly the
    /// shape of bug that only shows up on a boot where something closed early.
    #[tokio::test]
    async fn the_offer_is_snapshotted_at_boot_and_outlives_this_runs_own_ends() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_path_buf();
        let path = index::index_path(&data_dir);
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();

        let at = Utc::now();
        let mut text = String::new();
        for record in [
            index::opened_record(
                "run-prev",
                &4.into(),
                Role::Worker(WorkerType::General),
                Some(3.into()),
                "chase the deploy",
                None,
                at,
            ),
            index::thread_record("run-prev", &4.into(), "th-errand", at),
        ] {
            text.push_str(&format!("{}\n", serde_json::to_string(&record).unwrap()));
        }
        tokio::fs::write(&path, text).await.unwrap();

        let r = reg();
        r.attach_index(data_dir).await;

        let offered = r.lost_workers();
        assert_eq!(offered.len(), 1, "the previous run's unfinished errand");
        assert_eq!(offered[0].thread.as_deref(), Some("th-errand"));
        assert_eq!(offered[0].title.as_deref(), Some("chase the deploy"));

        // Now let this run close a session of its own, which pushes onto `recent`.
        let id = mint();
        r.register(id.clone(), Role::Cognition, None, "the shared brain".into(), None);
        r.unregister(&id);

        assert_eq!(
            r.lost_workers().len(),
            1,
            "a close in this run must not empty the offer"
        );
    }

    /// Seed a registry whose previous run died holding one errand on `th-errand`.
    async fn boot_with_a_lost_errand(subject: Option<&str>) -> (tempfile::TempDir, Registry) {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_path_buf();
        let path = index::index_path(&data_dir);
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        let at = Utc::now();
        let mut text = String::new();
        for record in [
            index::opened_record(
                "run-prev",
                &4.into(),
                Role::Worker(WorkerType::General),
                Some(3.into()),
                "finish the KT8-046 build",
                subject,
                at,
            ),
            index::thread_record("run-prev", &4.into(), "th-errand", at),
        ] {
            text.push_str(&format!("{}\n", serde_json::to_string(&record).unwrap()));
        }
        tokio::fs::write(&path, text).await.unwrap();
        let r = reg();
        r.attach_index(data_dir).await;
        (dir, r)
    }

    /// **The offer is taken, and taking it is the check.** Before this it was read with an
    /// `any()` and left in place, so nothing stopped the same dead errand being resumed
    /// twice — two sessions opening from one mind, each registered as owning the task, and
    /// the ledger naming whichever the join happened to iterate last.
    #[tokio::test]
    async fn a_lost_thread_is_offered_exactly_once() {
        let (_dir, r) = boot_with_a_lost_errand(None).await;
        assert!(r.take_lost_thread("th-errand"), "the offer this boot made");
        assert!(!r.take_lost_thread("th-errand"), "the second claim on the same mind");
        assert!(r.lost_workers().is_empty(), "taken means gone from the offer");
    }

    /// A thread nobody offered is refused, which is the older half of the same rule: a resume
    /// argument is the one value a caller cannot derive from the work in front of it.
    #[tokio::test]
    async fn a_thread_that_was_never_offered_is_not_taken() {
        let (_dir, r) = boot_with_a_lost_errand(None).await;
        assert!(!r.take_lost_thread("th-confabulated"));
        assert_eq!(r.lost_workers().len(), 1, "and a refused take leaves the offer alone");
    }

    /// **Putting somebody on the task is the other way an errand stops being owed.** The
    /// ledger asks this list why a `doing` task has nobody on it; once a live worker is
    /// registered under the subject, the honest answer is that somebody *is* on it, and a
    /// frozen list would go on explaining an absence that ended forty minutes ago.
    #[tokio::test]
    async fn a_live_worker_under_the_subject_drains_the_entry() {
        let (_dir, r) = boot_with_a_lost_errand(Some("kt8-046")).await;
        assert!(r.lost_subjects().contains("kt8-046"), "cut off, and nothing on it yet");

        let id = mint();
        r.register(
            id,
            Role::Worker(WorkerType::General),
            Some(3.into()),
            "finish the KT8-046 build".into(),
            Some("kt8-046".into()),
        );

        assert!(r.lost_subjects().is_empty(), "restaffed, whether it resumed or started cold");
        assert!(r.lost_workers().is_empty(), "and its thread is no longer on the offer");
    }

    /// A rung registers with no subject and must not touch the offer — the drain is keyed by
    /// the ledger subject, and everything without one is nothing to do with it.
    #[tokio::test]
    async fn a_session_with_no_subject_drains_nothing() {
        let (_dir, r) = boot_with_a_lost_errand(Some("kt8-046")).await;
        r.register(mint(), Role::Cognition, None, "the shared brain".into(), None);
        assert!(r.lost_subjects().contains("kt8-046"));
    }

    #[test]
    fn a_message_reaches_the_target_inbox() {
        let r = reg();
        let (a, b) = (mint(), mint());
        r.register(a.clone(), Role::Cognition, None, "thinking".into(), None);
        r.register(b.clone(), Role::Worker(WorkerType::General), Some(a.clone()), "the errand".into(), None);

        assert_eq!(r.send(&a, &b, "go".into()), Delivery::Delivered);
        let mail = r.take_pending(&b).expect("delivered");
        assert_eq!(mail.len(), 1);
        assert_eq!(mail[0].text, "go");
        assert_eq!(mail[0].from, Some(a), "the return address rides with the message");
        assert!(r.take_pending(&b).is_none(), "taking drains the inbox");
    }

    /// Several messages arriving while a session is mid-turn must cost one turn, not
    /// several: the point of merging is that a burst reads as one prompt.
    #[test]
    fn messages_landing_together_merge_into_one_prompt() {
        let r = reg();
        let (a, b) = (mint(), mint());
        r.register(a.clone(), Role::Cognition, None, String::new(), None);
        r.register(b.clone(), Role::Worker(WorkerType::General), Some(a.clone()), String::new(), None);

        r.send(&a, &b, "first".into());
        r.send(&a, &b, "second".into());
        let mail = r.take_pending(&b).expect("both delivered");
        assert_eq!(
            mail.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            ["first", "second"],
            "a burst is taken together, in arrival order"
        );
        assert_eq!(r.status(&b).unwrap().turns, 1, "a burst costs one turn, not several");
    }

    /// What a close means to everyone *else*: the address stops accepting work, and says
    /// so, so a sender opens something fresh instead of posting into a session that is on
    /// its way out. Ending a worker without this would swallow the next brief silently.
    #[test]
    fn a_closed_inbox_turns_later_senders_away() {
        let r = reg();
        let (a, b) = (mint(), mint());
        r.register(a.clone(), Role::Cognition, None, String::new(), None);
        r.register(b.clone(), Role::Worker(WorkerType::General), Some(a.clone()), String::new(), None);

        assert_eq!(r.send(&a, &b, "one more thing".into()), Delivery::Delivered);
        assert!(r.close_inbox(&b), "there was a live session to close");
        assert_eq!(
            r.send(&a, &b, "too late".into()),
            Delivery::Unknown,
            "a closed inbox reports Unknown so the sender starts something fresh"
        );
        assert!(r.inbox_closed(&b));
    }

    /// Closing what is not there is a normal thing for an owner to do — it tidies up after
    /// a restart — so it answers rather than panicking, and answers honestly.
    #[test]
    fn closing_an_unknown_session_reports_that_nothing_was_there() {
        let r = reg();
        assert!(!r.close_inbox(&mint()));
        assert!(r.inbox_closed(&mint()), "a session that never existed takes no more work");
    }

    /// The host is not an agent: it may hand work to any live session, and what it
    /// hands over carries no return address because there is nobody to answer.
    #[test]
    fn the_host_can_post_without_being_a_sender() {
        let r = reg();
        let (owner, w) = (mint(), mint());
        r.register(owner.clone(), Role::Cognition, None, String::new(), None);
        r.register(w.clone(), Role::Worker(WorkerType::General), Some(owner), String::new(), None);

        // A worker may not address itself as an agent — that is not its owner.
        assert_eq!(r.send(&w, &w, "self".into()), Delivery::NotPermitted);
        // The host posting the same follow-up is fine, and arrives anonymous.
        assert_eq!(r.post(&w, "keep going".into()), Delivery::Delivered);
        let mail = r.take_pending(&w).expect("posted");
        assert_eq!(mail[0].from, None);
        assert_eq!(mail[0].text, "keep going");

        r.unregister(&w);
        assert_eq!(r.post(&w, "too late".into()), Delivery::Unknown);
    }

    /// The bug this exists to make impossible: the reaction loop leaves by several paths, and
    /// a registration released at only some of them leaves a second Reaction behind for the
    /// one role — which `reachable` would then offer, and a sender would send at.
    #[test]
    fn a_scoped_registration_ends_with_its_scope() {
        let sender = mint();
        global().register(sender.clone(), Role::Cognition, None, String::new(), None);

        let id = {
            let reaction =
                register_scoped(mint(), Role::Reaction, None, String::new());
            let id = reaction.id();
            assert_eq!(
                global().send(&sender, &id, "hi".into()),
                Delivery::Delivered
            );
            id
        };

        assert!(global().status(&id).is_none(), "leaving the scope closed the registration");
        assert_eq!(
            global().send(&sender, &id, "hi again".into()),
            Delivery::Unknown,
            "no stale reaction is left registered"
        );
        global().unregister(&sender);
    }

    #[test]
    fn a_notifier_is_reachable_after_registration() {
        let r = reg();
        let a = mint();
        r.register(a.clone(), Role::Reaction, None, String::new(), None);
        assert!(r.notifier(&a).is_some());
        assert!(r.notifier(&9_999.into()).is_none());
    }

    /// The sender must be able to tell the difference between "it arrived" and "there was
    /// nobody there" — a report whose owner has gone needs to fall back rather than be
    /// silently dropped.
    #[test]
    fn an_absent_target_is_reported_not_swallowed() {
        let r = reg();
        let a = mint();
        r.register(a.clone(), Role::Cognition, None, String::new(), None);
        assert_eq!(r.send(&a, &9_999.into(), "hello".into()), Delivery::Unknown);

        let gone = mint();
        r.register(gone.clone(), Role::Worker(WorkerType::General), Some(a.clone()), String::new(), None);
        r.unregister(&gone);
        assert_eq!(r.send(&a, &gone, "hello".into()), Delivery::Unknown);
    }

    /// Routing, not policy: a worker answers whoever asked and cannot reach past them —
    /// not a sibling, and not the conversation.
    #[test]
    fn a_worker_may_address_only_its_owner() {
        let r = reg();
        let (owner, other, worker) = (mint(), mint(), mint());
        r.register(owner.clone(), Role::Cognition, None, String::new(), None);
        r.register(other.clone(), Role::Reaction, None, String::new(), None);
        r.register(worker.clone(), Role::Worker(WorkerType::General), Some(owner.clone()), String::new(), None);

        assert_eq!(r.send(&worker, &owner, "done".into()), Delivery::Delivered);
        assert_eq!(
            r.send(&worker, &other, "psst".into()),
            Delivery::NotPermitted
        );
    }

    /// The projection that replaced name-a-destination addressing. Reaction is offered
    /// the shared brain and nothing else, because handing work up is the only edge it has.
    #[test]
    fn the_voice_is_offered_the_shared_brain() {
        let r = reg();
        let (rx, cog) = (mint(), mint());
        r.register(rx.clone(), Role::Reaction, None, String::new(), None);
        r.register(cog.clone(), Role::Cognition, None, "thinking".into(), None);

        let who = r.reachable(&rx);
        assert_eq!(who.len(), 1, "{who:?}");
        assert_eq!(who[0].1, cog);
        assert!(who[0].0.contains("shared brain"), "{who:?}");

        // And the id it was handed is one it can actually send to.
        assert_eq!(r.send(&rx, &who[0].1, "a real errand".into()), Delivery::Delivered);
        assert_eq!(r.take_pending(&cog).expect("delivered")[0].text, "a real errand");
    }

    /// A cold rung is simply absent from the list, which is the point: the asker learns
    /// there is nobody there *before* sending, instead of guessing a name and being told
    /// `Unknown` after the fact.
    #[test]
    fn a_rung_that_is_not_up_is_not_offered() {
        let r = reg();
        let rx = mint();
        r.register(rx.clone(), Role::Reaction, None, String::new(), None);
        assert!(r.reachable(&rx).is_empty());
    }

    /// Cognition is offered the live Reaction, because that is the one way anything it
    /// works out reaches the person.
    #[test]
    fn cognition_is_offered_the_voice_and_its_own_workers() {
        let r = reg();
        let (cog, rx, other, w) = (mint(), mint(), mint(), mint());
        r.register(cog.clone(), Role::Cognition, None, "thinking".into(), None);
        r.register(rx.clone(), Role::Reaction, None, String::new(), None);
        r.register(other.clone(), Role::Worker(WorkerType::General), Some(rx.clone()), String::new(), None);
        r.register(w.clone(), Role::Worker(WorkerType::General), Some(cog.clone()), "file the receipts".into(), None);

        let who = r.reachable(&cog);
        let ids: Vec<SessionId> = who.iter().map(|(_, id)| id.clone()).collect();
        assert!(ids.contains(&rx), "Reaction: {who:?}");
        assert!(ids.contains(&w), "its own worker: {who:?}");
        assert!(!ids.contains(&other), "someone else's worker is not offered: {who:?}");
    }

    /// **A worker linked to no task says so, on the same line that offers it.**
    ///
    /// This is the half of the task↔worker join the ledger cannot report about itself. A
    /// worker created without a `subject` never appears on any task's projected line, so its
    /// task reads *nobody on it* while the work is running — and the natural response to that
    /// line is to start a second worker on it. The duplicate is the real cost of a missed
    /// label, so the omission has to be visible from the same window that invites the mistake.
    #[test]
    fn a_worker_on_no_task_says_so_and_one_on_a_task_names_it() {
        let r = reg();
        let (cog, linked, unlinked) = (mint(), mint(), mint());
        r.register(cog.clone(), Role::Cognition, None, "thinking".into(), None);
        r.register(
            linked.clone(),
            Role::Worker(WorkerType::General),
            Some(cog.clone()),
            "chase the deploy".into(),
            Some("ktv-doubao-ref-only".into()),
        );
        r.register(
            unlinked.clone(),
            Role::Worker(WorkerType::General),
            Some(cog.clone()),
            "chase the deploy".into(),
            None,
        );

        let who = r.reachable(&cog);
        let line = |id: SessionId| {
            who.iter().find(|(_, i)| *i == id).map(|(l, _)| l.clone()).expect("offered")
        };
        assert!(line(linked.clone()).contains("ktv-doubao-ref-only"), "{:?}", line(linked));
        assert!(!line(linked.clone()).contains("not linked"), "{:?}", line(linked));
        assert!(line(unlinked.clone()).contains("not linked to any task"), "{:?}", line(unlinked));
    }

    /// The rungs are standing and belong to no task. Marking them unlinked would put the
    /// warning on every window, on rows where it is not a fault — which is how a warning stops
    /// being read before it is ever needed.
    #[test]
    fn a_rung_is_never_marked_as_linked_to_nothing() {
        let r = reg();
        let (cog, rx) = (mint(), mint());
        r.register(cog.clone(), Role::Cognition, None, "thinking".into(), None);
        r.register(rx, Role::Reaction, None, "what reaches the person".into(), None);

        let who = r.reachable(&cog);
        assert!(
            who.iter().all(|(label, _)| !label.contains("not linked")),
            "Reaction is not an unlabelled worker: {who:?}"
        );
        assert!(!r.has_unlinked_worker(&cog), "a rung is not an unlinked worker");
    }

    /// **An organizer has no task to be missing, so it is not marked as missing one.**
    ///
    /// A `person-reader` is Reflection's housekeeping — one per person present in a stretch,
    /// keyed to a `people/<name>` facet, never work the ledger owes anyone. It is also the
    /// worker type dispatched in the largest fan-out there is, so marking it would print the
    /// warning once per person on a page where none of them is a fault. Both halves are pinned
    /// here: the roster line and the cheap predicate, which must not disagree.
    #[test]
    fn an_organizer_is_not_marked_as_linked_to_nothing() {
        let r = reg();
        let (refl, reader) = (mint(), mint());
        r.register(refl.clone(), Role::Reflection, None, "housekeeping".into(), None);
        r.register(
            reader.clone(),
            Role::Worker(WorkerType::PersonReader),
            Some(refl.clone()),
            "read 赵力".into(),
            None,
        );

        let who = r.reachable(&refl);
        let line = who
            .iter()
            .find(|(_, i)| *i == reader)
            .map(|(l, _)| l.clone())
            .expect("offered");
        assert!(!line.contains("not linked"), "{line:?}");
        assert!(!r.has_unlinked_worker(&refl), "an organizer is not an unlinked worker");
    }

    /// The cheap half of the same question, for the check that runs before staffing a task.
    #[test]
    fn an_unlinked_worker_is_visible_to_its_owner_alone() {
        let r = reg();
        let (cog, refl, w) = (mint(), mint(), mint());
        r.register(cog.clone(), Role::Cognition, None, "thinking".into(), None);
        r.register(refl.clone(), Role::Reflection, None, "housekeeping".into(), None);
        r.register(w, Role::Worker(WorkerType::General), Some(cog.clone()), "an errand".into(), None);

        assert!(r.has_unlinked_worker(&cog));
        assert!(!r.has_unlinked_worker(&refl), "not someone else's to answer for");
    }

    /// The lookup the hand-down rides on. Reaction has no id for Cognition — nothing
    /// hands it one, and `reachable` rebuilds its list per turn precisely because a
    /// stored id goes stale — so the host asks the switchboard by role at the moment it
    /// posts.
    #[test]
    fn a_singleton_rung_can_be_found_by_its_role() {
        let r = reg();
        assert!(r.session_of_role(Role::Cognition).is_none(), "nothing is up yet");

        let (rx, cog) = (mint(), mint());
        r.register(rx.clone(), Role::Reaction, None, String::new(), None);
        r.register(cog.clone(), Role::Cognition, None, "thinking".into(), None);

        let found = r.session_of_role(Role::Cognition).expect("cognition is up");
        assert_eq!(found.id, cog);
        assert_eq!(found.title, "thinking");
        assert_eq!(r.session_of_role(Role::Reaction).map(|s| s.id), Some(rx));
    }

    /// A rung that has gone is *absent*, not stale — the caller must be able to tell
    /// "nobody to hand to" from "handed and waiting", because only one of those means
    /// the person is owed an answer that is coming.
    #[test]
    fn a_rung_that_unregistered_is_no_longer_found_by_role() {
        let r = reg();
        let cog = mint();
        r.register(cog.clone(), Role::Cognition, None, String::new(), None);
        r.unregister(&cog);
        assert!(r.session_of_role(Role::Cognition).is_none());
    }

    /// Two of one rung should not happen; if it does, the answer must not depend on hash
    /// order. A caller that asks twice in one turn and gets two different sessions would
    /// post the request to one and then read the other's status.
    ///
    /// **The winner is the one that has been registered longest, not the one whose id
    /// sorts first.** While ids were ordinals those were the same sentence, and they are
    /// not any more — a slug sorts alphabetically, which is nothing to do with which
    /// session the rest of the process has been talking to. Registration order is: the
    /// incumbent keeps the role, and a stray second registration cannot take it over by
    /// being named earlier in the alphabet.
    #[test]
    fn two_of_one_rung_resolve_to_the_same_session_every_time() {
        let r = reg();
        let (late, incumbent) = (mint(), mint());
        r.register(incumbent.clone(), Role::Cognition, None, "incumbent".into(), None);
        r.register(late.clone(), Role::Cognition, None, "late".into(), None);

        let id = r.session_of_role(Role::Cognition).map(|s| s.id);
        assert_eq!(id.as_ref(), Some(&incumbent), "the one that was already there");
        for _ in 0..8 {
            assert_eq!(r.session_of_role(Role::Cognition).map(|s| s.id), id);
        }
    }

    /// A worker is offered its owner and nothing else — which is also the only thing the
    /// routing rule would let it send to, so the list and the rule agree.
    #[test]
    fn a_worker_is_offered_only_its_owner() {
        let r = reg();
        let (owner, worker, other) = (mint(), mint(), mint());
        r.register(owner.clone(), Role::Cognition, None, String::new(), None);
        r.register(other, Role::Reaction, None, String::new(), None);
        r.register(worker.clone(), Role::Worker(WorkerType::General), Some(owner.clone()), String::new(), None);

        let who = r.reachable(&worker);
        assert_eq!(who.len(), 1, "{who:?}");
        assert_eq!(who[0].1, owner);
    }

    /// **Take-once is the discard rule.** The first session a rung opens in a run gets the
    /// previous thread; every later one — which is what a reopen after a failed turn is —
    /// gets nothing and opens cold. Without this a thread wedged badly enough to break a
    /// turn would be handed straight back to the session replacing it, and "turn it off and
    /// on again" would stop working.
    #[test]
    fn a_resumable_thread_is_handed_out_exactly_once() {
        let r = Registry::new();
        r.resumable
            .lock()
            .unwrap()
            .insert(Role::Cognition.as_str().to_string(), "th-1".to_string());

        assert_eq!(r.take_resumable(Role::Cognition).as_deref(), Some("th-1"));
        assert_eq!(r.take_resumable(Role::Cognition), None, "the second open is cold");
    }

    /// A rung with nothing seeded — a fresh install, or one whose last run predates thread
    /// recording — simply opens cold rather than erroring.
    #[test]
    fn an_unseeded_rung_has_nothing_to_resume() {
        assert_eq!(Registry::new().take_resumable(Role::Reaction), None);
    }

    /// The thread lands on the live entry, so the `closed` row written when the session ends
    /// carries it too — a rung that quit cleanly must be as resumable as one that crashed.
    #[test]
    fn noting_a_thread_puts_it_on_the_live_session() {
        let r = Registry::new();
        let id = mint();
        r.register(id.clone(), Role::Reaction, None, "Reaction".into(), None);
        r.note_thread(&id, "th-reaction");

        assert_eq!(
            r.sessions.lock().unwrap().get(&id).and_then(|e| e.thread.clone()).as_deref(),
            Some("th-reaction"),
        );
    }

    #[test]
    fn an_empty_reach_renders_as_nothing_at_all() {
        assert_eq!(render_reachable(&[]), "", "a heading with nothing under it reads as a failure");
    }

    /// One renderer, because there is one mailbox. The three that preceded it had already
    /// drifted — different strings, different separators, one missing a trim.
    #[test]
    fn mail_renders_with_a_return_address_and_host_posts_without_one() {
        let batch = vec![
            Message { from: Some(7.into()), text: "  did you see this?  ".into() },
            Message { from: None, text: "  a follow-up  ".into() },
        ];
        assert_eq!(render(&batch), "(from session 7) did you see this?\n\na follow-up");
    }

    /// The rule that keeps owners from being reaped out from under running work.
    #[test]
    fn an_owner_with_live_children_is_not_idle() {
        let r = reg();
        let (owner, child) = (mint(), mint());
        r.register(owner.clone(), Role::Cognition, None, String::new(), None);
        assert!(!r.has_live_children(&owner));

        r.register(child.clone(), Role::Worker(WorkerType::General), Some(owner.clone()), String::new(), None);
        assert!(r.has_live_children(&owner));
        assert_eq!(r.children(&owner), vec![child.clone()]);

        r.unregister(&child);
        assert!(!r.has_live_children(&owner), "a closed child stops holding its owner open");
    }

    #[test]
    fn status_carries_meta_and_never_content() {
        let r = reg();
        let (a, b) = (mint(), mint());
        r.register(a.clone(), Role::Cognition, None, String::new(), None);
        r.register(b.clone(), Role::Worker(WorkerType::General), Some(a.clone()), "file the receipts".into(), None);

        let s = r.status(&b).expect("registered");
        assert_eq!(s.role, Role::Worker(WorkerType::General));
        assert_eq!(s.owner, Some(a.clone()));
        assert_eq!(s.title, "file the receipts");
        assert!(!s.busy && !s.queued && s.turns == 0);

        r.send(&a, &b, "go".into());
        assert!(r.status(&b).unwrap().queued);

        r.take_pending(&b);
        let s = r.status(&b).unwrap();
        assert!(s.busy && !s.queued && s.turns == 1);

        r.finish_turn(&b, TurnOutcome::Completed);
        assert!(!r.status(&b).unwrap().busy);
    }

    /// Every transition moves `state_since`, and the ones that are *not* transitions leave
    /// it alone. The field is only worth having if all of them stamp: one path that flips
    /// `busy` without moving the clock reports a turn as older than it is, on exactly the
    /// path that skipped it.
    #[test]
    fn every_state_change_moves_its_clock_and_nothing_else_does() {
        let r = reg();
        let id = mint();
        r.register(id.clone(), Role::Worker(WorkerType::General), None, String::new(), None);
        let registered = r.status(&id).unwrap();
        assert_eq!(registered.state_since, registered.started, "idle since it existed");

        // idle → waiting
        r.post(&id, "go".into());
        let waiting = r.status(&id).unwrap();
        assert!(waiting.queued && !waiting.busy);
        assert!(waiting.state_since > registered.state_since);

        // A second letter onto an already-queued inbox is not a state change.
        r.post(&id, "and also".into());
        assert_eq!(r.status(&id).unwrap().state_since, waiting.state_since, "still waiting");

        // waiting → running
        r.take_pending(&id);
        let running = r.status(&id).unwrap();
        assert!(running.busy);
        assert!(running.state_since > waiting.state_since);

        // Mail landing mid-turn leaves it running, so the clock holds.
        r.post(&id, "one more".into());
        assert_eq!(r.status(&id).unwrap().state_since, running.state_since, "still running");

        // running → idle
        r.finish_turn(&id, TurnOutcome::Completed);
        let done = r.status(&id).unwrap();
        assert!(!done.busy);
        assert!(done.state_since > running.state_since);

        // A second finish is not a transition.
        r.finish_turn(&id, TurnOutcome::Completed);
        assert_eq!(r.status(&id).unwrap().state_since, done.state_since);
    }

    /// The ending is kept, because `busy: false` is the same word for a session that
    /// answered and one whose turn died — and the roster, the ledger line and
    /// `hi_session_status` all read `busy` to decide what to say.
    #[test]
    fn a_finished_turn_leaves_behind_how_it_ended() {
        let r = reg();
        let id = mint();
        r.register(id.clone(), Role::Worker(WorkerType::General), None, String::new(), None);
        assert!(r.status(&id).unwrap().last_turn.is_none(), "nothing has ended yet");

        r.post(&id, "go".into());
        r.take_pending(&id);
        r.finish_turn(&id, TurnOutcome::Failed("429 Too Many Requests".into()));
        let end = r.status(&id).unwrap().last_turn.expect("an ending was recorded");
        assert_eq!(end.outcome.as_str(), "failed");
        assert_eq!(end.outcome.error(), Some("429 Too Many Requests"));
        assert!(end.outcome.is_trouble());

        // The next turn's ending replaces it: what a reader wants is the last one, and a
        // failure that outlived its recovery is worse than no field at all.
        r.take_pending(&id);
        r.finish_turn(&id, TurnOutcome::Completed);
        let end = r.status(&id).unwrap().last_turn.expect("still recorded");
        assert_eq!(end.outcome, TurnOutcome::Completed);
        assert!(!end.outcome.is_trouble(), "a clean ending is not news");
        assert!(end.outcome.error().is_none());
    }

    /// A turn that never ran leaves no ending. The two call sites are a send that failed
    /// and a loop exiting between accepting a reason to speak and speaking — `completed`
    /// there would report a success nothing attempted, and `failed` would raise an alarm
    /// about a turn nobody ran.
    #[test]
    fn an_abandoned_turn_records_no_ending() {
        let r = reg();
        let id = mint();
        r.register(id.clone(), Role::Reaction, None, String::new(), None);
        r.start_turn(&id);
        r.abandon_turn(&id);
        let st = r.status(&id).unwrap();
        assert!(!st.busy);
        assert!(st.last_turn.is_none(), "nothing ended, so nothing to say about how");
    }

    /// The reason renders on a roster row, so it arrives already one line and already cut —
    /// the same rule the title and the activity line follow, for the same reason.
    #[test]
    fn a_failure_reason_is_capped_to_one_line() {
        let r = reg();
        let id = mint();
        r.register(id.clone(), Role::Worker(WorkerType::General), None, String::new(), None);
        r.start_turn(&id);
        r.finish_turn(&id, TurnOutcome::Failed(format!("stack
trace {}", "x".repeat(400))));
        let end = r.status(&id).unwrap().last_turn.unwrap();
        let err = end.outcome.error().unwrap();
        assert!(!err.contains('\n'), "flattened: {err}");
        assert!(err.chars().count() <= OUTCOME_LINE_CHARS + 1, "capped: {}", err.chars().count());
        assert!(err.ends_with('…'));
    }

    /// `doing` without an age says a session is alive and nothing more — the line reads the
    /// same four minutes in and forty minutes in.
    #[test]
    fn doing_carries_when_it_was_last_seen() {
        let r = reg();
        let id = mint();
        r.register(id.clone(), Role::Worker(WorkerType::General), None, String::new(), None);
        assert!(r.status(&id).unwrap().doing_at.is_none(), "nothing done, no clock");

        r.record_activity(&id, "$ cargo test");
        let first = r.status(&id).unwrap().doing_at.expect("stamped");

        // The real shape, `{server}/{tool}` from `SessionUpdate::activity` — so `hi-agent`
        // and the tool's declared name, prefix included. The fixture said `send_message`
        // until now, which is a label the wire has not produced since the rename.
        r.record_activity(&id, "hi-agent/hi_send_message");
        let second = r.status(&id).unwrap();
        assert_eq!(second.doing.as_deref(), Some("hi-agent/hi_send_message"));
        assert!(second.doing_at.unwrap() >= first, "replaced, so re-stamped");

        // A blank line is not activity and must not refresh the clock — a session that has
        // gone quiet would otherwise look busy forever.
        r.record_activity(&id, "   ");
        assert_eq!(r.status(&id).unwrap().doing_at, second.doing_at);
    }

    /// **A title is one line whatever the caller hands over.** Every reader of this field
    /// renders it as a single line beside a state word, so a newline or a paragraph in it
    /// reflows a roster someone is reading. The caller is told to write one line; this is
    /// what makes it true.
    #[test]
    fn a_title_is_flattened_and_capped_on_the_way_in() {
        let r = reg();

        let wrapped = mint();
        let messy = "recover the\n  stalled\tdeploy\n".to_string();
        r.register(wrapped.clone(), Role::Worker(WorkerType::General), None, messy, None);
        assert_eq!(r.status(&wrapped).unwrap().title, "recover the stalled deploy");

        let long = mint();
        let brief = "Deploy only hi-agent.xyz end to end. The user explicitly authorized \
                     this deployment. First read the deployment ledger."
            .to_string();
        r.register(long.clone(), Role::Worker(WorkerType::General), None, brief, None);
        let title = r.status(&long).unwrap().title;
        assert!(title.ends_with('…'), "a cut title says it was cut: {title:?}");
        assert_eq!(title.chars().count(), TITLE_CHARS + 1, "the cap, plus the ellipsis");

        // Whatever replaces it is held to the same line.
        r.set_title(&long, format!("{}\nand more", "x".repeat(TITLE_CHARS + 10)));
        let replaced = r.status(&long).unwrap().title;
        assert!(replaced.ends_with('…') && !replaced.contains('\n'), "{replaced:?}");
    }

    /// A headline shorter than the cap is passed through untouched — no ellipsis on a line
    /// that was already whole, which is what makes the ellipsis mean anything.
    #[test]
    fn a_short_title_is_left_exactly_as_written() {
        assert_eq!(headline("chase the deploy", TITLE_CHARS), "chase the deploy");
        assert_eq!(headline("", TITLE_CHARS), "");
        assert_eq!(headline("  padded  ", TITLE_CHARS), "padded");
        // The cut lands on a character boundary, not a byte one.
        let cjk = "把这个部署恢复过来".repeat(20);
        assert_eq!(headline(&cjk, 8).chars().count(), 9);
    }

    #[test]
    fn a_prewarmed_session_can_replace_its_placeholder_title() {
        let r = reg();
        let id = mint();
        r.register(id.clone(), Role::Cognition, None, "waiting for the first question".into(), None);

        r.set_title(&id, "review the restart behavior".into());

        assert_eq!(
            r.status(&id).expect("registered").title,
            "review the restart behavior"
        );
    }

    #[test]
    fn output_is_a_bounded_tail_not_an_archive() {
        let r = reg();
        let a = mint();
        r.register(a.clone(), Role::Worker(WorkerType::General), None, String::new(), None);
        r.record_output(&a, "hello ");
        r.record_output(&a, "world");
        assert_eq!(r.messages(&a).as_deref(), Some("hello world"));

        r.record_output(&a, &"x".repeat(OUTPUT_TAIL_CHARS + 500));
        let kept = r.messages(&a).unwrap();
        assert_eq!(kept.chars().count(), OUTPUT_TAIL_CHARS, "the tail is capped");
        assert!(kept.ends_with('x'), "it is the *recent* tail that survives");
    }

    #[test]
    fn ids_are_unique_process_wide() {
        let ids: Vec<SessionId> = (0..50).map(|_| mint()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
    }
}

/// What a session id is *called*, which is the whole of what changed here.
///
/// These use the real [`super::mint`] rather than the tests' ordinal shorthand, because the
/// shape it produces is the property under test.
#[cfg(test)]
mod slug_tests {
    use super::*;
    use crate::identity::WorkerType;

    /// The pure naming half — see [`super::slug_for`]. Naming tests use this so they read
    /// the same answer every run; the uniqueness counter gets its own test below.
    fn named(hint: &str) -> String {
        slug_for(Role::Worker(WorkerType::ViewBuilder), Some(hint))
    }

    /// **A rung's id is the name the rest of the design already uses for it.** Not a
    /// synonym, not a role word invented for addressing: `docs/arch/agents.md` calls these
    /// three reaction, cognition and reflection, so an agent told to reach cognition types
    /// `cognition`. Pinned because a rename here silently breaks every prompt at once —
    /// the address an agent is given comes from `Role::as_str`, not from prose.
    #[test]
    fn a_rungs_id_is_its_role_name() {
        for role in [Role::Reaction, Role::Cognition, Role::Reflection] {
            assert_eq!(slug_for(role, None), role.as_str());
        }
    }

    /// A hint is ignored for a rung. There is one Cognition, so there is one thing it can
    /// be called; letting a caller's stray argument reach the slug would make the one
    /// address in the design that must be predictable depend on the call site.
    ///
    #[test]
    fn a_rung_ignores_a_hint_because_it_is_a_singleton() {
        assert_eq!(slug_for(Role::Cognition, Some("some-stray-errand")), "cognition");
    }

    /// **A worker's id says which errand it is.** Type first so the specialism reads at a
    /// glance in a roster, then the task — this is the half an ordinal could never carry.
    #[test]
    fn a_workers_id_is_its_type_and_its_task() {
        assert_eq!(named("kyoto-trip"), "view-builder-kyoto-trip");
    }

    /// The hint is prose an agent wrote, not an identifier: it arrives with spaces,
    /// punctuation, capitals and trailing junk. Collapsed to single dashes and trimmed,
    /// because this string is both an address someone retypes and a filename.
    #[test]
    fn a_title_becomes_a_readable_slug() {
        assert_eq!(named("Chase the deploy!"), "view-builder-chase-the-deploy");
        assert_eq!(named("  spaced  out  "), "view-builder-spaced-out");
        assert_eq!(named("...leading and trailing..."), "view-builder-leading-and-trailing");
    }

    /// **Non-ASCII survives.** Most titles in this deployment are Chinese, and stripping to
    /// ASCII would turn every one of them into the same empty hint — every worker sharing
    /// one base name, told apart only by a counter. That is the ordinal again, wearing a
    /// prefix.
    #[test]
    fn a_chinese_title_keeps_its_characters() {
        assert_eq!(named("追一下部署"), "view-builder-追一下部署");
    }

    /// A slug is capped, because it is a filename and an address, not a description. The
    /// full title is on the roster line beside it and is never truncated.
    #[test]
    fn a_long_title_is_cut_rather_than_refused() {
        let id = named(&"a-very-long-errand-title ".repeat(20));
        assert!(id.len() < 60, "{id}");
        assert!(id.starts_with("view-builder-a-very-long-errand-title"), "{id}");
    }

    /// **Uniqueness is per run, not per live session.** Two workers of one type on one task
    /// is ordinary; sharing an id is not, because a session's frame log is named for it and
    /// two sessions writing one file cannot be told apart afterwards. The disambiguator is
    /// a suffix, so the errand is still legible.
    #[test]
    fn a_repeated_errand_gets_a_distinct_id() {
        let hint = "twice-over-the-same-thing";
        let mint_one = || mint(Role::Worker(WorkerType::ViewBuilder), Some(hint));
        let (a, b, c) = (mint_one(), mint_one(), mint_one());
        assert_eq!(a.as_str(), "view-builder-twice-over-the-same-thing", "the first takes the plain name");
        assert_eq!(b.as_str(), "view-builder-twice-over-the-same-thing-2");
        assert_eq!(c.as_str(), "view-builder-twice-over-the-same-thing-3");
    }

    /// A worker whose hint yields nothing usable still gets an address — the type alone,
    /// then counted. It reads worse, and that is honest: `create_worker` requires a title,
    /// so an unnameable errand is a caller that sent punctuation.
    #[test]
    fn an_unusable_hint_falls_back_to_the_type() {
        assert_eq!(slug_for(Role::Worker(WorkerType::DriveOrganizer), Some("!!!")), "drive-organizer");
        assert_eq!(slug_for(Role::Worker(WorkerType::DriveOrganizer), None), "drive-organizer");
    }

    /// **No minted id can collide with a literal route segment.** `/api/workers/ended` is
    /// registered beside `/api/workers/{id}`, and a session slugged `ended` would be
    /// unreadable through the API. Nothing guards this at mint time and nothing needs to:
    /// a worker always carries a `<type>-` prefix and a rung is always a role name, so the
    /// property falls out of the shape. This test is what makes it stay true if a new
    /// literal segment is ever added under `/api/workers/`.
    #[test]
    fn no_id_can_shadow_a_literal_route_segment() {
        const LITERAL_SEGMENTS: &[&str] = &["ended"];
        let mut names: Vec<String> =
            [Role::Reaction, Role::Cognition, Role::Reflection].map(|r| slug_for(r, None)).into();
        for kind in WorkerType::ALL {
            // Including the adversarial case: an errand actually titled "ended".
            names.push(slug_for(Role::Worker(*kind), Some("ended")));
            names.push(slug_for(Role::Worker(*kind), None));
        }
        for name in names {
            assert!(!LITERAL_SEGMENTS.contains(&name.as_str()), "`{name}` shadows a route");
        }
    }

    /// **The parse is the path guard, and that is why it can fail.** A session id is a path
    /// component — `raw/sessions/<run>/<id>.jsonl` — and `GET /api/workers/{id}/frames`
    /// hands the value straight from the URL to that builder. While ids were integers
    /// `parse::<u64>()` was the traversal guard by luck; widening to a string without
    /// keeping one would have reopened it.
    #[test]
    fn an_address_that_is_a_path_is_refused() {
        for bad in ["../../etc/passwd", "..", "a/b", "a.b", "", "   ", "has space", "semi;colon"] {
            assert!(bad.parse::<SessionId>().is_err(), "`{bad}` must not parse");
        }
    }

    /// Forgiving about case and whitespace, because this is typed by a model reading a
    /// roster: `Cognition ` is unambiguous and refusing it teaches nothing.
    #[test]
    fn an_address_is_read_case_and_space_insensitively() {
        assert_eq!("  Cognition\n".parse::<SessionId>().unwrap().as_str(), "cognition");
    }

    /// A well-formed id that names nothing is **not** a parse error — it is a lookup miss,
    /// which is a different fact and gets a different answer ("nothing live at `x`"). Losing
    /// that distinction would turn a cold rung into a spelling complaint.
    #[test]
    fn a_wellformed_id_parses_even_when_nothing_answers_to_it() {
        assert!("nobody-is-called-this".parse::<SessionId>().is_ok());
    }

    /// **Old rows carry a number**, from every run before ids became slugs, and
    /// `raw/sessions/index.jsonl` is append-only. A number reads back as its own decimal
    /// spelling: it addresses nothing live, which is correct for a session that ended, and
    /// it still names that session in the record it came from — which is all a closed row
    /// is for.
    #[test]
    fn a_pre_slug_numeric_row_still_reads() {
        let from_number: SessionId = serde_json::from_str("7").unwrap();
        assert_eq!(from_number.as_str(), "7");
        let from_slug: SessionId = serde_json::from_str("\"cognition\"").unwrap();
        assert_eq!(from_slug.as_str(), "cognition");
        assert_eq!(serde_json::to_string(&from_slug).unwrap(), "\"cognition\"");
    }
}
