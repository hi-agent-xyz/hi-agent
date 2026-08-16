# Text transcript

**Status:** accepted August 11, 2026. Replaces the current-appearance contract of
August 9, 2026 (`text-appearance.md`), which this file supersedes in full.

## Decision

The conversation is a **message list**: an ordered, append-only sequence of whole
messages, owned by the backend, durable across restart, and rendered by any number of
windows.

Nothing in it is ever rewritten, cleared, or replaced. A message that was sent stays
sent, in the position it was sent in, until it ages out of the window — and even then it
is still in the journal and still reachable by scrolling back.

This file says what the conversation *is*. Where it sits on screen — as the host's own
always-present view, sharing the stage with the agent's rather than collapsing behind it —
is [`stage.md`](stage.md).

## What a message is

Exactly three things become messages, and nothing else does:

| Message | Source |
|---|---|
| Something the person typed or said | `POST /api/in/text` (typed, or settled recognition) |
| A file the person handed over | `POST /api/in/file`, `POST /api/up/{token}` |
| One thing the agent said | one `hi_say` call |

**One `hi_say` call is one message, whole.** `hi_say` already receives its complete text, so a
message is appended when the call is accepted — not assembled from chunks as a
generation streams. Sentence splitting still happens downstream, but only to pace TTS;
it never reaches the list.

Everything else that moves through the system is not conversation and stays out: views
(they have the view slot), worker reports, mail between rungs, glance-up and check-in wakes,
face and voice recognition, tool calls, the activity meter, wire frames. Each already
has a home in the journal or the inspector. A check-in appears here only if it produced
a `hi_say` — which is correct, because then it is a thing that was said.

## Who sent it

**A message carries its sender**: the [`Sender`](signal-attribution.md) the boundary
decided — `{subject, basis}` — handed to the list rather than worked out again here. The
agent's own messages carry none, because attribution answers which *person* something came
from and the agent is not one of the people it keeps.

The list is a chat between people, plural. One install has one owner, but one room has
whoever is in it: a voiceprint places one spoken line with 赵力 and cannot place the next,
and both are `role: "user"`. Without the sender the conversation could only say *someone
who is not the agent* — so a window drew two people's lines as one run, under one face.

**The face beside a message is that field, drawn.** It is the people store's own state,
made visible where the talking happens: a crop for someone the camera has met
(`GET /api/people/{subject}/avatar`), a coloured disc with an initial for a name it has
only heard, the same disc with a silhouette for a cluster it can tell apart but cannot
name, a plain one for a voice nobody placed. Naming that cluster in 认识的人 changes what
is beside their messages, which is the point: identity work has an effect you can see.

**The basis rides along and is shown as one.** An `owner` default reads as assumed rather
than as a recognition — the person looking at it is exactly who can correct it.

### The words are the words

The `⟨…⟩` markers a carrier attaches at the boundary — the `⟨ref: …⟩` locator on a handed
file, `⟨voice: 赵力⟩` from a voiceprint, the standing note a held-attention session rides
in on — are written **for the mind** and are stripped from the message's text. They stay in
the journal, and the mind still reads them there.

This is the same rule the file locator already followed, applied to the rest: a recognition
belongs in a field, where a window can draw it and a later pass can defeat it. Spelled into
the middle of the sentence somebody said, it was neither — just a name in a chat bubble
that the person never typed.

## Why whole messages, and why short ones

The list is a chat between two people, not a transcript of an agent's working. People
send a message when they have finished writing it, and they send three short ones rather
than one long one. That is the shape the agent writes in: `SAY_MAX_CHARS` is not a guard
against an accidental dump, it is the size of a message, and a rejected `hi_say` means
*send this as a few* rather than *something went wrong*.

Short messages lose nothing, because depth was never supposed to live here. A person
does not paste a report into a chat; they send the file and say what is in it. **The
list is the conversation; views and files are the attachments.**

## Ownership

The backend is the only authority on the list. A window never:

- invents an identity or partition;
- tells the backend what it has read;
- advances or consumes messages;
- keeps a private queue;
- reconstructs the list from local storage.

**There are no read receipts, and this is not an oversight.** Whether the person has
actually read a message is not observable — an open window is not a pair of eyes — and
every mechanism that has tried to derive it has derived it wrongly. The unread-since-you-
scrolled marker is a property of one scroll position in one browser; it stays there and
is never sent.

Opening, foregrounding, refreshing, or reconnecting a window means "give me the
conversation and keep me current". It does not mean "deliver what I missed", because
nothing was missed: the messages are still there.

## Wire

`GET /api/out/text` is one long-lived NDJSON response.

| Frame | Meaning |
|---|---|
| `{"reset": {"messages": [...], "interim": null}}` | The current window, whole. Always the first frame; sent again only if the list is rebuilt. |
| `{"append": {"id", "ts", "role", "text", "media"?, "sender"?}}` | One new message at the end. |
| `{"interim": "..."}` or `{"interim": null}` | The rolling recognition partial, or its expiry. |

`sender` is `{"subject"?: "赵力", "basis": "owner"｜"cluster"｜"stated"｜"unknown"}`. It is
absent on the agent's own messages, and present-with-no-subject when somebody spoke and
nobody could say who. A window that does not recognize a basis drops the sender rather
than showing the name: a name whose grounding cannot be read is the ungrounded name
[`signal-attribution.md`](signal-attribution.md) exists to keep out of the record.

The `interim` carries no sender. Recognition has not settled, so there is nobody to name
yet, and it is a preview rather than a message.

`GET /api/messages?before=<id>&limit=<n>` returns older messages for scrollback, read
from the journal.

`id` is the journal entry's uuidv7 — time-sortable, already durable, already the citation
key — so the live window and the scrollback are the same identifiers and stitch without a
merge step. This is an id **on a message**, not a delivery cursor: no client ever sends
one back to claim progress, and the backend keeps no per-window position.

There is one `interim` slot, and it is not a message. It is the live recognition partial,
shown pending at the tail and replaced by the settled message when the line lands. It
expires after 3 seconds without an update.

## Durability

The list is seeded at boot from the journal — `SignalIn` on `Text` and `File`, `SignalOut`
on `Text` — so a restart shows the conversation that was already happening. The live
window is bounded; older messages are reached by scrolling, not by growing the window.

## Interruption

A settled human line appends and does not disturb anything. A reaction turn already
running keeps running, and whatever it says lands after that line, with its own
timestamp — which is what actually happened, and is how a person reads a message that
crossed with theirs.

The previous contract needed an eligibility rule here (a turn could be superseded, and
its later text had to be suppressed so it did not appear to answer a line it had never
seen). A list needs none: nothing is claiming a slot, so nothing can steal one. That
machinery is deleted rather than ported.

Transport interruption is likewise nothing: the response drops, the window reconnects,
receives a `reset`, and is current. No resumption, no acknowledgement, no fragment
reassembly.

Voice keeps its separate barge-in semantics, which this does not touch.

## What this reverses, and what it keeps

It reverses the *shape*: one replaceable current exchange becomes an append-only list,
and a process restart restores the conversation instead of starting empty.

It keeps the *ownership*, which was the right half: one backend-owned conversation
however many windows render it, no message-delivery protocol, no client identity, no
cursor, no acknowledgement, no epoch, no per-window bookmark. `epoch`, `after`,
`X-HI-Text-Epoch`, `X-HI-Utterance` and the browser's old `hi-agent.out-text-cursor`
stay retired.

The reason for the reversal is the reason the previous contract gave for its own
accepted consequences: *"There is no replay of an exchange after a newer one replaces
it"* and *"Text is not restored after process restart"*. Both held in practice, and both
meant that the only complete record of what the agent had said was `server.log`. A
conversation you have to read the logs to follow is not a conversation.

## Accepted consequences

- The live window is bounded, so the newest N messages are in memory and the rest cost a
  scrollback request.
- Ordering is arrival order. A reply that crossed with a new human line appears after it.
- Nothing tells the agent whether a message was read, and nothing ever will.
- A very long `hi_say` is rejected rather than truncated; the agent splits it.
- **Most spoken lines have no sender**, because most voices are not placed. They draw a
  silhouette, and consecutive ones group together even though they may not be one person —
  the record cannot tell, and the drawing claims no more than the record does.
- **Messages logged before attribution have no sender and never will.** They read as
  unattributed; there is no backfill, for the reason
  [`signal-attribution.md`](signal-attribution.md) gives.
- **The owner's face appears beside lines they may not have typed** — someone else at the
  machine, a window left open. That is the `owner` default being visible rather than
  silent, and it reads as assumed.
