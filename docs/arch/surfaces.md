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
| Emission is natural language; anything answerable is a tool call | People talk without invoking an API, but do take deliberate, answerable actions |

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

Nothing distinguishes the two except who moved first. The device itself — its reachability,
grants and accounts — is described in [`foundation.md`](foundation.md#devices).

## Channels

Four in, three out. A channel is one sense or expression stream, in human vocabulary, with
zero knowledge of the wire.

| Channel | Direction | Carried as |
|---|---|---|
| text | in / out | content block · output stream |
| audio | in | text after STT today; an audio block once we model paralinguistics |
| audio | out | output stream → TTS, core-side |
| vision | in | a **ref**; the agent calls a tool to actually look |
| file | in | a **ref** to a handed object |
| surface (rich content) | out | inline markers — emission |
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

Large payloads must **not flow through raw and then get copied** into drive — both trees
are synced, so that is duplication with no durability gain. Raw holds the event; the bytes
are staged and moved.

## Carriers

The agent reaches these channels over ACP, which has no concept of a channel. Three
carriers, and the line between them is both technically real and human:

- **Emission — fire and forget.** Speaking and showing. Natural language plus inline
  markers. No return value. A person talks and gestures without invoking anything.
- **Perception and action — needs an answer.** "Look at that," "set a timer," "click this."
  Tool calls: structured arguments, request/response. A person deliberately turns to look,
  picks up the cup.

Keeping the voice in natural language while routing *answerable* needs through tools is
what preserves "think, then organize words". The thinking layers express intent; the core
articulates it.

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
