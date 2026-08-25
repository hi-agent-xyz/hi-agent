# Message

**Status:** accepted August 21, 2026. Narrows [`text-transcript.md`](text-transcript.md),
which says what the conversation *is* and stays authoritative on that; this file says what
a message *is made of*, and replaces the several shapes that used to carry one.

## Decision

**One human utterance is one value, minted once at the boundary and never rebuilt.**

```rust
pub struct Message {
    pub id: String,
    pub ts: DateTime<Utc>,
    pub from: From,
    pub content: Content,
}
```

Four fields. The ingress mints it, hands the same value to the journal, to Reaction, and to
the conversation, and nothing downstream reconstructs it from parts.

## What this replaces

The same typed line used to exist as seven shapes, and the field holding its text was called
three different things:

```text
types::Signal              { channel, body, stream, ts }        no id, no sender, no media
JournalEntry::SignalIn     { id, ts, channel, body, stream, media, origin, sender }
JournalEntry::SignalOut    { id, ts, channel, body, media, origin }        no stream
transcript::Message        { id, ts, role, text, attachment, sender }
transcript::Attachment     { ref, mime }                        projection of Media
OutboundSignal::Text       { id, ts, text }
registry::Message          { from, text }                       rung mail, not conversation
```

**The load-bearing defect was `Signal` having no `id`.** Reaction received an utterance with
no identity, so nothing it concluded could be attached back to the thing it concluded it
about — which is why relevance had nowhere to live. Everything else follows from that: sender
and media had to be copied or reconstructed, the live path and the journal-replay path built
the conversation by two different routes and could disagree, and `body`/`text`/`from`/`sender`
drifted apart because no single type owned the answer.

`OutboundSignal::Text` already carried `{id, ts}` "so nothing downstream mints its own". That
is this decision, applied to half the system and never to the other half.

## Who sent it

```rust
pub enum Author {
    Agent,
    Person(Sender),   // Sender { subject: Option<String>, basis: SenderBasis }
}
```

The type is `Author` and the field is `from`. `From` is in the prelude, and a message
type that shadows it would make every `impl From<…>` in the crate read ambiguously.

**`from` is total — every message answers it.** The previous pair (`role: Role` beside
`sender: Option<Sender>`) admitted `role: Agent, sender: Some(赵力)`, which is nonsense, while
collapsing the one case that *is* meaningful — a person nobody placed — into the same `None`
the agent's own messages used. One field, no junk state.

`Person(Sender { subject: None, basis: Unknown })` is someone we cannot name. Per
[`signal-attribution.md`](signal-attribution.md) that is **a complete answer, not a degraded
one**, and it is never backfilled from content.

`Agent` carries no sender because attribution answers which *person* something came from and
the agent is not one of the people it keeps.

Surfaces still receive a `role` field on the wire; it is derived from this one at
serialization, not stored beside it.

## What was communicated

```rust
pub enum Content {
    Text(String),      // typed — what they wrote
    Speech(String),    // recognized — best guess at what they said
    File(FileRef),     // handed over
}

pub struct FileRef {
    pub reff: String,          // channel-qualified media ref; GET /api/media/<ref>
    pub mime: String,
    pub name: String,          // what a person calls it
    pub bytes: Option<u64>,    // how big, when the boundary counted
    pub peek: Option<String>,  // how a text artifact opens
}
```

**`Text` and `Speech` are different facts, not a formatting distinction.** Typed text is
exactly what somebody wrote. A transcript is a machine's best guess at what somebody said, and
it can be wrong. The mind should be able to tell, and the variant is what tells it — this is
the entire remaining job of the old `channel` field on a message.

**The asymmetry with the agent is deliberate.** For a person, speech is the source and text is
derived from it. For the agent, text is the source and synthesis is a rendering of it — so the
agent's spoken audio is never a message, and `From::Agent` pairs with `Text`, never `Speech`.

**`FileRef` carries the name.** The name used to be deliberately excluded, on the ground that
it was already inside the message text and a second copy would let the live path and the
journal-seeded path disagree. Both halves of that reason are gone: a `File` message has no
prose to hide the name in, and one canonical `Message` is what removes the disagreement. Both
consumers need it — the renderer draws it under the thumbnail, the prompt builder writes
"they handed you passport.jpg".

**`bytes` and `peek` are what make a file *decidable* by a rung that cannot read one.**
Reaction never opens an artifact; on the rendered line alone it decides whether the turn is
worth handing down, and "they handed you notes.txt" does not distinguish a sticky note from a
database dump. The size answers that, and for text the opening answers what the thing is. Both
are optional because absent is a real answer: a carrier that never counted must say so rather
than write a zero, and a photo has no opening a reader could use.

A peek is **not** the content. The artifact holds what was handed over; the peek is a
fixed-size look at its head that the prompt can afford to carry every turn. Whoever wants the
rest joins `{raw_dir}` to the ref and reads it.

## What somebody says, and what they hand over

**Size is what separates the two, and the boundary decides it — not the carrier, and not the
person.** A typed body under 64 KiB is words: `Content::Text`, journalled on the text channel,
into the prompt verbatim. The same channel above 64 KiB is a handed artifact: the bytes are
written through to a blob as they arrive, kept under the file channel (which forgetting
exempts), and delivered as `Content::File` carrying a ref, a size and a peek.

**No input channel has a size limit, and none should.** A person hands over what they hand
over; bounding *that* is the system refusing to receive. What is bounded is how much of it
rides in the prompt every turn, which is a different question with a different answer. The
inbound text route therefore streams and holds at most the seam in memory — an implementation
consequence of the same rule, since a route that buffers whole bodies cannot honestly claim to
be unbounded.

This sits **upstream** of the underlying agent's own compaction, and does not duplicate it.
Compaction shrinks history that accumulated; nothing in it can shrink the item that just
arrived. An oversized single message is exactly the shape that makes compaction fail — the
summary request is assembled from a thread already over the limit — and the seam is what keeps
arrivals in a range where compaction can still work.

## One arrival, one message each

**Each message carries exactly one content.** A caption and a photo are two messages, ordered.
Text plus three files is four.

This is safe because of the batching window, and only because of it: the turn queue settles
for `RESPONSE_SETTLE` (700ms), held open to `BATCH_WHILE_COMPOSING` (5s) while somebody is
still talking or typing. Parts of one arrival are enqueued microseconds apart, so they cannot
be split by a window that only closes after 700ms of quiet, and Reaction sees one turn.

**The invariant that makes that true: a single arrival's parts are enqueued together, with
nothing awaited between them.** This is not decoration. The window has been measured missing —
one question arrived as three utterances, the batch closed 0.8s before the second landed, and
it cost two generations, two spoken replies, a promise armed off a third of the request and
two overlapping errands into Cognition. Human-paced gaps do that. Machine-paced parts of one
arrival must not, and the only thing standing between them is that nothing yields in the
middle of minting them.

Parts of one arrival share a channel, so they land in one day-log in append order and read
back in the order they were made. Nothing depends on uuidv7 tie-breaking within a millisecond.

**No `Link` or `Location` variants.** With one content per message, every content kind is a
message kind, so the list stays at what actually exists. A link is text. A location is a fact,
and facts are not messages.

**An empty message is refused at ingress, not filtered at read.** `Content::Text("")` is never
minted. Filtering it on the way out is what let the journal hold messages the conversation did
not, which is the replay divergence this file exists to remove.

## What is not a message

The journal carries five kinds, and four of them are not conversation:

| Kind | What it is |
|---|---|
| `Message` | Human ↔ Hi Agent communication |
| `Presentation` | A view put up, replaced, dismissed — replayed to restore the screen |
| `Observation` | Ambient perception: a face seen, a room gone quiet |
| `InternalEvent` | Worker reports, check-ins coming due, mail between rungs |
| `Appraisal` | Reaction's own judgement *about* a message |

`Presentation` is the one that has to be settled in the same change rather than after. Views
are journaled today as `SignalOut` on `Channel::View`, and `snapshot.rs` reads them back to
restore the screen — so they are neither conversation nor machinery, and the only thing
currently keeping them out of the conversation is a `Channel::Text` filter on the outbound
arm. Take channel off the message and that filter has nothing left to match on.

**`Appraisal` is separate because `Message` is immutable.** Relevance is computed by Reaction
*after* the message is journaled. Storing it on the message would mean a field that is
authoritative in memory and always null on disk, reconstructed at read time by folding update
records — which is the live-versus-replay divergence coming back through the one mutable
field. It is a judgement about a message, in the same category as an observation, and it lives
alongside:

```rust
pub struct Appraisal { message_id: String, relevance: f32, ts: DateTime<Utc> }
```

Presentation for the UI joins the two at read time.

## Where channel went

`Channel` is off the message and **is not gone from the system**. It is the journal's routing
key: `raw/<channel>/<YYYY-MM-DD>/<channel>.jsonl` is chosen by channel, media bytes sit inside
the channel-day folder, `signal_ref` is channel-qualified, and `decay.rs` fades a whole
`(channel, date)` subtree as one unit. Per-sense storage and per-sense forgetting both depend
on it.

So it moves from the message to the envelope around it:

```rust
pub enum JournalEntry {
    Message { channel: Channel, message: Message },
    // …
}
```

The journal line has a channel; the `Message` inside it does not. Routing, per-sense fading
and every reader that scans by channel keep working unchanged, and the value that flows to
Reaction and to the conversation carries no field it does not use. Nothing migrates, and
journals written before this change stay where they are and keep loading.

`stream` (`audio#headset`) goes the same way, and earns less: it distinguished concurrent
sources of one channel, which was a proxy for *who is talking* — and `from` now answers that
directly. It stays on the ingress signal where capture sources still need telling apart.

## Rendering

**A caption and its photo group; they do not merge.** Two adjacent messages from the same
`from` render under one avatar as two bubbles, which is what every chat surface already does
with a photo and its caption. Grouping is by `from` plus adjacency and needs no field.

**The face beside a message is `from`, drawn** — unchanged from
[`text-transcript.md`](text-transcript.md). `Agent` draws none.

**The rolling recognition partial stays outside `Message`.** It is a preview of a
`Content::Speech`, not a message, and `Frame::Interim` already models it correctly.

**`Text` stays a plain string.** Whether a surface renders markdown is that surface's
contract, not a field here.

## What the mind reads is rendered, not stored

Carriers used to write their conclusions *into the message text* — `⟨voice: 赵力⟩` for a
voiceprint match, `⟨ref: …⟩` for a file's locator — because the body was the only place there
was. Three functions then maintained that channel: one to write it, `display_text` to strip it
back out for the UI, `recover_voice_sender` to parse it back into a `Sender`.

**Under this file, carriers set the field and nothing writes a marker into text.** The prompt
builder renders `⟨voice: …⟩` from `from` when composing the model's view; the UI draws a face
from the same field. One fact, two renderings, no stripping and no parsing of prose.

`recover_voice_sender` survives as a **legacy read path only**, for lines journaled before
`sender` was a field. It is a recovery, not a backfill: it reads the boundary's own written
conclusion out of where the boundary happened to put it, and the `⟨…⟩` grammar is what makes
that safe rather than a parse of what somebody said.

## Both ends can hand over a file

`From::Agent` with `Content::File` is a message like any other: the agent putting a photo,
a document or a generated artifact into the conversation, rendered where the words are rather
than in the view slot. A view is something the agent *shows* and takes back; a file is
something it *hands over* and the person keeps. `Content` does not branch on who sent it.

## Relevance has somewhere to attach

Reaction receives a message with an id, so what it concludes can name what it concluded about.
That was the point.
