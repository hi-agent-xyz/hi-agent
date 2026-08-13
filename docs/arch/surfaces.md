# Surfaces & channels

## Goal

Let the world reach the agent the way it would reach a person — through whatever is at
hand — without any of that reaching the thinking layers as protocol.

## Decisions

| Decision | Reasoning |
|---|---|
| Transport lives in the adapter, never in the host | The mind should know senses, not HTTP. Swap the wire and nothing above changes |
| A device is both a surface and an effector | Same hardware, two roles, told apart by who moved first |
| File is a signal, but carries a **ref** — never content | A handed-over object, not something perceived |
| Vision emits a ref; the agent decides whether to look | Perception is *pulled*, not pushed |
| Everything outbound is a tool call, the voice included | The arbiter needs somewhere to stand: a call can be held, queued or refused, and the caller finds out. The words stay natural language |

## Surfaces

**Appearance** — the user's own window: views, voice, chat. The surface we control end to
end.

**Apps** — Feishu, WeChat and the like. hi-agent holds *its own accounts* here, the way a
colleague does. Reaching someone means messaging them from its account, not calling an API
on the user's behalf.

**Devices** — phones and machines it owns. The dual role is the thing to keep straight:

- **as a surface** — the world initiates. Someone messages hi-agent's account on a phone
  it holds.
- **as an effector** — hi-agent initiates. It opens Reddit on its own Android, logged into
  its own account, because that is the only way to reach that content.

Nothing distinguishes the two except who moved first. On the effector side a device is just a
tool plus a written procedure — see [`foundation.md`](foundation.md#devices).

## Channels

Four in, three out. A channel is one sense or expression stream, in human vocabulary, with
zero knowledge of the wire.

| Channel | Direction | Carried as |
|---|---|---|
| text | in / out | content block · the `hi_say` call |
| audio | in | text after STT today; an audio block once we model paralinguistics |
| audio | out | the same `hi_say` call, rendered by TTS host-side |
| vision | in | a **ref**; the agent calls a tool to actually look |
| file | in | a **ref** to a handed object |
| surface (rich content) | out | the `hi_show` call, by **path ref** |
| action | out | tool call — request/response |

**An inbound channel also says who a signal came from.** `text` and `file` are *addressed* —
someone sent them to the agent — so they default to the owner; `audio` and `vision` are
*ambient*, so their sender is a recognized cluster or nobody at all. That is a property of
how the signal arrived and never of what it says:
[`signal-attribution.md`](signal-attribution.md).

### Why a ref and not the bytes

A photo arriving does not mean the agent looked at it. The signal says *"a photo arrived,
here is where it is"*, and the agent decides whether the conversation warrants opening it.
This is both cheaper and more honest: eager captioning throws away the original and
replaces it with one fixed interpretation, which is the wrong answer to *"what's the dosage
on this box"*.

The same holds for files, for a different reason: a file is an **object handed over**, not
something sensed. Pushing it through vision and keeping the caption loses the bytes, which
is exactly what the person wanted kept.

### Bulk

**Streamed** bulk — audio minutes, video, capture grids — must **not flow through the log
and then get copied** into drive. Both trees are synced, so that is duplication with no
durability gain. The log holds the event; the bytes are staged and moved.

**A handed file is the deliberate exception, and it is not a violation of the above.** It
lands in the log's media store like any other signal, and the filing worker **copies** it
into [`drive/`](data.md#drive) rather than moving it. Two reasons, and the first is the
one that matters:

- **The two stores have different retention, which is the whole point of having both.**
  The log's copy [fades](data.md#forgetting) once its day is consolidated and cold; the
  drive's copy is permanent. So the second copy buys exactly the durability the general
  rule says is absent — for the one class of object where the bytes, not the caption, are
  what the person wanted kept.
- **Moving it would dangle the log's own reference.** A journal entry records its media by
  relative path and *the line is never rewritten*; the reader does a best-available lookup
  (original → keepsake → caption alone). Move the bytes and that entry silently degrades
  to a caption — for a passport or a contract, the worst possible thing to hold a caption
  of.

Stated at this length because the rule above reads as forbidding it, and someone
reasonable will otherwise "fix" the filing worker to move rather than copy. The cost is a
few megabytes duplicated; the alternative is a promise quietly broken.

## Carriers

The agent reaches these channels over the agent wire, which has no concept of a channel.
One rule covers every direction:

> **Everything outbound is a tool call.** Looking, acting, showing — and speaking.

**Speech was the last emission to become a call, and it stays one** — for the same
reason as the rest: an emission that cannot be rejected cannot be length-checked, and
`hi_say` rejects a paragraph so Reaction splits it into messages.

What `hi_say` no longer answers is *where the words landed*. It used to report aloud, on
screen only, or waiting-for-their-return, and Reaction was expected to read that answer
and go quiet on an empty room. That is gone with the presence gate — a message is
appended to the [transcript](text-transcript.md), which keeps, so there is no such thing
as an utterance spent on nobody. Speech that no speaker is attached for simply is not
synthesized, which is a fact about the wire and needs no answer.

Note what this does *not* mean: the host holds no queue of things to say later. Messages
append the moment they are said and are there whenever anyone looks. What waits for a
better moment waits in Reaction's judgment, which is where the decision lives.

The same call also carries `back_in` — the size Reaction just put on a silence, which
arms [the check-in](host.md#the-check-in--the-only-thing-that-fires-at-a-named-time)
that brings it back to keep the promise. It rides on `hi_say` rather than on a verb of its
own precisely because a promise is only a promise once it has been *said*: a separate
call could arm a wake for a number nobody was ever told. Still one queue-free surface —
what is held is a deadline, not an utterance.

### Text transcript

The conversation is one backend-owned, append-only message list, rendered by any number
of windows. Three things are messages: what the person typed or said, a file they handed
over, and one `hi_say` call. Nothing else — a view, a worker's report, a clock wake — is
conversation, and none of it appears here.

`GET /api/out/text` returns one long-lived NDJSON response. The first object is the
current window whole; every following object appends one message or updates the single
rolling recognition interim. The wire carries no client identity, no cursor, no
acknowledgement and no read receipt. Connecting gives you the conversation and keeps you
current; reading never changes anything.

Messages carry the journal's uuidv7 as their id, so the list is seeded from the journal
at boot and `GET /api/messages?before=<id>` scrolls further back through the same
identifiers. An id on a message is not a delivery cursor: nothing sends one back to claim
progress. See [`text-transcript.md`](text-transcript.md) for the complete contract.

The conversation is also a **view** — the host's own, always present — and it shares the
screen with the agent's rather than being replaced by it. What may be on screen at once,
and how the four roles are arranged, is [`stage.md`](stage.md).

**Showing is a call for the same reason and one of its own.** Putting something on a screen
is an act, not a gesture: it can fail, it has an id, and it can be taken down again.

**A worker hands a view over as a path ref.** The worker builds the view and passes its
**ref**; Reaction calls `hi_show` with the ref, and the **host resolves it server-side**. So
view source never enters a thinking layer's context — which is the point. A view is a build
artifact, sometimes thousands of lines, and the window that has to answer fastest is the last
place it should be paid for.

There is no marker vocabulary anywhere, and nothing is parsed back out of the model's text.

What survives of "think, then organize words" is the half that mattered: **the voice is still
natural language**. Reaction writes the sentence it means — it just hands it over instead of
streaming it at the world. The thinking layers express intent; the host articulates it, and
now it can also decline to.

## Batching

Every seam is a producer handing work to a consumer:

> **Batch iff the emit-unit is finer than the accept-unit. Otherwise pass through.**

The verdict is always relative to the consumer. A sentence passes straight into TTS, which
wants words; the same sentence batches into a thinking layer, which wants a whole turn.

**The load-bearing boundary is text → Reaction.** A short quiet-settle timer after the last
input fragment is what turns a continuous stream into the discrete turn a model needs.
Everything else — sentence splitting for TTS, VAD before STT — is incidental, justified
only by provider granularity, and removable in principle.

## Degradation

Every channel must degrade rather than fail. No screen attached → voice only, and anything
the person must act on (a URL, a command) goes verbatim into the channel they are actually
on. No live camera → a readable error that prompts the agent to ask them to turn it on.

## See also

[`host.md`](host.md) for what happens to a signal after it lands ·
[`foundation.md`](foundation.md#tools) for the outbound side ·
[`legacy/runtime-dataflow.md`](legacy/runtime-dataflow.md) for the original derivation.
