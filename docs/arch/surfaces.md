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
| Everything outbound is a tool call, speech included | The arbiter needs somewhere to stand: a call can be held, queued or refused, and the caller finds out. The words stay natural language |

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

Five in, three out. A channel is one sense or expression stream, in human vocabulary, with
zero knowledge of the wire.

| Channel | Direction | Carried as |
|---|---|---|
| text | in / out | content block · the `hi_say` call |
| audio | in | text after STT, plus what the sound itself was like; an audio block once we model paralinguistics properly |
| audio | out | the same `hi_say` call, rendered by TTS host-side |
| vision | in | a **ref**; the agent calls a tool to actually look |
| file | in | a **ref** to a handed object |
| surface (rich content) | out | the `hi_show` call, by **path ref** |
| view | in | the person went to one of the agent's surfaces — a ref, never a window |
| action | out | tool call — request/response |

**An inbound channel also says who a signal came from.** `text`, `file` and `view` are
*addressed* — someone acted on the agent deliberately — so they default to the owner;
`audio` and `vision` are *ambient*, so their sender is a recognized cluster or nobody at
all. That is a property of
how the signal arrived and never of what it says:
[`signal-attribution.md`](signal-attribution.md).

**An ambient channel also says what the sensing was like.** A microphone in a room hears
the whole room, and the difference between a sentence said into it and a remark from the
next table is physics well before it is meaning — how many voices are around, whether
this one is the near one, whether it arrived buried in the room it crossed, whether it
landed on top of someone else's turn. Those ride the signal as `⟨room: …⟩`, and only in
terms that survive an unknown microphone gain: a difference of two levels from one
capture, or this speaker measured against the others in the same room. They are
**evidence for the mind and never a gate in the channel** — perception stays mechanical
and hands up everything it heard
([`31-hear-the-room`](../user-journeys/31-hear-the-room.md)).

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
lands in the log's media store like any other signal, and the [drive
organizer](agents.md#drive-organizer) **copies** it into [`drive/`](data.md#drive) rather
than moving it. Two reasons, and the first is the one that matters:

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
reasonable will otherwise "fix" the drive organizer to move rather than copy. The cost is a
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

The call carries nothing but the words. It used to take `back_in`, the size Reaction had
just put on a silence, and that timer is gone
([`host.md`](host.md#there-is-no-timer-and-the-last-one-to-go-was-the-agents-own)): it fired
just before the work landed and said so, which is the one thing a check-in must not be.
Still one queue-free surface —
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

What survives of "think, then organize words" is the half that mattered: **what reaches the person is still
natural language**. Reaction writes the sentence it means — it just hands it over instead of
streaming it at the world. The thinking layers express intent; the host articulates it, and
now it can also decline to.

## Batching

Every seam is a producer handing work to a consumer:

> **Batch iff the emit-unit is finer than the accept-unit. Otherwise pass through.**

The verdict is always relative to the consumer. A sentence passes straight into TTS, which
wants words; the same sentence batches into a thinking layer, which wants a whole turn.

**One finished thought is one message.** The unit of what a person said is where they
finished saying it. The recognizer's endpoint (800 ms of trailing silence) is where it
*looks* for that, and how the recognizer punctuated what it closed is how it tells the two
apart: a tail ending in `。？！` is a whole thing to say and leaves; one ending in `，` — or
in no mark at all — is somebody who paused mid-sentence, and is kept until the rest of the
sentence arrives and goes out with it. Only two guards can split one — a size cap for a
speaker who does not pause, an age cap for a recognizer whose endpoint never comes — and a
hold bounds the keeping, so a speaker who genuinely stops mid-sentence still gets those
words out.

*The holding half is a change, made 2026-09-10.* Cutting on the bare endpoint gave a
finished sentence and a hesitation the same weight, and a person thinking mid-sentence
produces 800 ms of silence constantly. Measured over one 5-minute briefing: **11 of 22
messages were ragged** — 5 ending in `，`, 6 in no mark at all, several of them 2–4 chars
(`就是`, `然后这个`) — and every one of those 11 had its own continuation delivered as a
separate message 1.4–2.8 s later. **Every message that ended a turn ended in `。` or `？`**,
which is why holding the others costs nothing at the moment a reply is owed.

That last sentence is measured, not argued. Replaying 221.7 s of that speech through the
recognizer and cutting the 358 frames it returned with both policies
(`tests/speech_segmentation_replay.rs`): **20 messages, median 39 chars, 10 of them ragged →
11 messages, median 45, 1 ragged**. And sweeping the hold from 0.15 s to 8 s, **the turn's
last message landed at the same 0.15 s at every value** — the hold is paid entirely out of
mid-turn text, which is what the preview is for.

**A revisable transcript is preview, and preview is never a message.** A recognizer's
rolling partial has two honest uses — showing the speaker their own words, and proving
somebody is still talking — and neither of them is *being cut into a message*. Letting it
be one is what made this hard: a cut taken from text the recognizer had not finished
writing has to be reconciled with the rewrite that follows, which cost an edit-distance
alignment of every revision against everything already sent. Cutting committed text only
removes the question. The buffer went from four strings to two, the alignment and a
`max_segment` age cap went with it (that guard could only ever act on uncommitted text, so
it could no longer do the job it documented), and the replay got **better**, not worse:
2 ragged messages became 1, and the two places the two policies disagreed about *which
words* the person said became zero.

What bounds a run-on is now the size cap alone — a bound in the unit the consumer cares
about, landing on a phrase boundary — and **the only thing shaping an ordinary conversation
is the one rule that should: did they finish?**

**What changed is not the recognizer but what being early is worth.** The cut used to be
made as early as it could be, because early was a faster reply. It is not any more: a reply
composed off half a thought is refused at the mouth ([host.md](host.md#the-floor)) and
re-composed by the turn the rest of the sentence drives. Measured the same day, **7 of 23
replies were generated and then discarded**, 5 of them inside that one briefing. Early
bought no speech at all — it bought wasted generations, and errands dispatched off half a
request that Cognition then had to cancel. A cut that is late and whole costs strictly less
than one that is early and ragged.

*The unit itself changed a day earlier, on 2026-09-09, and that was the code catching up
to the sentence above it.* Speech used to be cut at punctuation, just after every
sentence-ending mark — every one of them, not only the ones that ended something.
That is a **transcription** boundary, not a conversational one: the recognizer marks a
period wherever written Chinese would take one, so an ordinary spoken turn arrived as a
run of messages nobody would have sent separately. Measured over 164 s of real speech,
30 of 36 messages were cut that way, median 18 chars, lower quartile 6 — `嗯。`, `然后呢？`,
`时间嘛。`, `对吧？`. It also threw away the better text: cutting on punctuation means
emitting from the rolling partial, while the recognizer's second pass and ITN correction
arrive later with the final (`刚才，刚才我边那你这个就放在右边` from the partial;
`刚才放在左边，那你这个就放在右边` from the final). On the same recording the new unit gives
15 messages, median 38 chars.

**Waiting for the endpoint costs nothing where it would matter.** The last message of a
turn lands at the same instant either way — the person stopped, so the endpoint fires and
everything settles — and that is when a reply is owed. What it delays is only intermediate
text appearing on screen mid-turn, which is what the recognition preview is for
([text-transcript.md](text-transcript.md)): the words show live as a pending line and
become one whole message when the speaker stops.

**The load-bearing boundary is text → Reaction.** A short quiet-settle timer after the last
input fragment is what turns a continuous stream into the discrete turn a model needs.
Everything else — sentence splitting for TTS, VAD before STT — is incidental, justified
only by provider granularity, and removable in principle.

**That timer is batching and nothing more.** It decides how many utterances one generation
is spent on; it does *not* decide whether the person has finished talking, and it never
could — its only input is finalized utterances, which someone mid-thought produces
constantly. That question is answered at the mouth, where the answer is still current:
[host.md#the-floor](host.md#the-floor). Read as batching, the window wants to be *small* —
every millisecond in it is latency on a reply into a silence that is already real.

**With one extension, and it is not a patience dial either: the window is held open while
they are still audibly talking**, capped. A finalized utterance lands *after* the words are
over, so quiet on this seam is not quiet in the room; if a voice is still going, the rest of
the sentence is on its way and belongs in the same batch. This exists because the mouth's
gate structurally cannot cover it — a `say` can be refused after the fact, but by then the
turn has thought and has handed work down, and neither can be taken back. Measured: a batch
that closed 0.8s early turned one question into two generations and **two overlapping
errands**. In a quiet room the extension never runs and the window is the plain settle.

## Degradation

Every channel must degrade rather than fail. No screen attached → voice only, and anything
the person must act on (a URL, a command) goes verbatim into the channel they are actually
on. No live camera → a readable error that prompts the agent to ask them to turn it on.

## See also

[`host.md`](host.md) for what happens to a signal after it lands ·
[`foundation.md`](foundation.md#tools) for the outbound side ·
[`legacy/runtime-dataflow.md`](legacy/runtime-dataflow.md) for the original derivation.
