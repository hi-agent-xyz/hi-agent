# Surfaces & channels

## Goal

Let the world reach the agent the way it would reach a person — through whatever is at
hand — without any of that reaching the thinking layers as protocol.

## Decisions

| Decision | Reasoning |
|---|---|
| Transport lives in the adapter, never in the core | The mind should know senses, not HTTP. Swap the wire and nothing above changes |
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
| text | in / out | content block · the `say` call |
| audio | in | text after STT today; an audio block once we model paralinguistics |
| audio | out | the same `say` call, rendered by TTS core-side |
| vision | in | a **ref**; the agent calls a tool to actually look |
| file | in | a **ref** to a handed object |
| surface (rich content) | out | the `show` call, by **path ref** |
| action | out | tool call — request/response |

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

The agent reaches these channels over ACP, which has no concept of a channel. One rule
covers every direction:

> **Everything outbound is a tool call.** Looking, acting, showing — and speaking.

**Speech was the last emission, and making it answerable is what gives the
[presence gate](core.md#presence) somewhere to stand.** `say` returns, so the host can
decline to voice an utterance the room cannot hear and **say so** — and Reaction finds
out, in the same breath, where the words did land: aloud, on screen only, or waiting for
them to come back. Fire-and-forget has nowhere to put that answer, which is the whole
problem: an utterance you cannot be told the fate of is one you spend without knowing.

Note what this does *not* mean: the host holds no queue of things to say later. Text and
views keep on their own, so there is nothing to hold; voice does not keep, so there is
nothing worth holding. What waits for a better moment waits in Reaction's judgment,
which is where the decision lives.

**Showing is a call for the same reason and one of its own.** Putting something on a screen
is an act, not a gesture: it can fail, it has an id, and it can be taken down again.

**A worker hands a view over as a path ref.** The worker builds the view and passes its
**ref**; Reaction calls `show` with the ref, and the **host resolves it server-side**. So
view source never enters a thinking layer's context — which is the point. A view is a build
artifact, sometimes thousands of lines, and the window that has to answer fastest is the last
place it should be paid for.

There is no marker vocabulary anywhere, and nothing is parsed back out of the model's text.

What survives of "think, then organize words" is the half that mattered: **the voice is still
natural language**. Reaction writes the sentence it means — it just hands it over instead of
streaming it at the world. The thinking layers express intent; the core articulates it, and
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

[`core.md`](core.md) for what happens to a signal after it lands ·
[`foundation.md`](foundation.md#tools) for the outbound side ·
[`legacy/runtime-dataflow.md`](legacy/runtime-dataflow.md) for the original derivation.
