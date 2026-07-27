# Core — the host

## Goal

Be the part of the system that is always awake, always fast, and never thinking: route
signals to the right scene, decide *when* the agent may speak, own every process and every
clock, and write everything down before anyone reacts to it.

Nothing here consults a model. That is the point — the core has to keep working while the
thinking layers are slow, confused, or dead.

## Decisions

| Decision | Reasoning |
|---|---|
| Scene is the isolation unit | One conversation's context must not bleed into another's |
| One mouth, arbitrated centrally | Many sub-minds may think; a person has one voice |
| The reflex path never reaches a model | Stopping when someone starts talking cannot wait a generation |
| The journal is written *before* anything reacts | Durability must not depend on a session surviving |
| The clock holds no durable state | A timer that dies with the process is worse than no timer |
| Sessions are host-owned and disposable | Continuity lives in `data/`, not in a process |

## Components

### Wire adapters

Bind concrete protocols (HTTP today, WebSocket or local audio later) to transport-free
channel signals. Framing, mime, long-poll, body-close, per-turn frame binding — all of it
lives here so that none of it exists above.

### Scene router

A **scene** is the situation a signal belongs to: with a person, with a group, or alone.
It is the context-isolation unit — one Reaction and one Deliberation per scene, one memory
slice, one tag on every session's tool attach.

Participants are *soft*, inferred from content, never a structural key. An external source
maps onto a scene (a group chat is a scene). The topology is decided once by judgment when
work is delegated, then executed mechanically.

### Channel mux

Fans N input channels into one prompt, and fans one output stream back out to N channels
by the carrier rules. This is a consequence of ACP carrying a single conversation, not a
goal in itself.

### Arbiter

The social layer, and the only place where "should this be said, now" is answered.

- **Mouth singleton.** Utterances queue for one mouth and never overlap. This is a global
  invariant — it binds ordinary replies, clock-driven surfacing, and worker reports alike,
  which is exactly why it cannot live inside any one of them.
- **Turn-taking.** The quiet-settle commit that decides a turn is over.
- **Presence gate.** Self-initiated speech is held while nobody is there, and released when
  they come back. Presence fuses several weak signals — channel activity, OS idle, face and
  speech — into one soft state. Nobody sends it; it is derived.
- **Social timing.** When to voice a worker's question, when to let it wait, when to tell
  the worker to proceed with a placeholder instead.

### Reflex

The sub-second path that **short-circuits every agent**. Barge-in stop and the attention
gesture live here. When someone starts speaking, sound stops mid-syllable and the unspoken
tail is discarded — no generation is in that loop, because a generation is far too slow.

The follow-up is the opposite: what to do about the interruption is a judgment, handled by
Reaction on the next turn with an estimate of how far it got. The same event is therefore
handled at two very different speeds, which is why the reflex is drawn separately.

### Session layer

Exposes each ACP session as an independent handle — prompt it, read its updates, drop it to
close. One subprocess per session, so one session's crash cannot touch another. A warm pool
absorbs spawn latency for the sessions that are created per delegation.

Long-lived sessions rot, so a heartbeat summarizes, pre-warms a replacement and swaps it in
between turns. The conversation never sees it.

### Journal

Every signal in and out is written before anything reacts to it. The journal — not session
lifetime — is authoritative for durability, recovery and cold start. It is mechanical, so
it belongs to [`data/`](data.md#journal); it appears here because it sits on the hot path.

### Clock

**Status: to be built as a module, not yet wired.**

One type, one owner, four rules.

```rust
struct Wake {
    at:     WakeAt,        // Every(Duration) | At(Instant) | Backoff { min, max }
    target: WakeTarget,    // Cognition | Reflection
    note:   String,        // plain language; becomes the prompt
}
```

1. **A wake produces a turn, never an utterance.** The clock cannot speak. Whatever the
   woken agent wants said goes through Reaction and is gated on presence.
2. **Coalesce.** Wakes due within one window collapse into a single turn carrying all
   notes, so a pile-up of tasks never becomes a monologue.
3. **Drop, don't queue.** A wake that fires while its target is busy is discarded —
   fix-forward, consistent with everything else. Only `At(_)` deadlines re-arm.
4. **No durable state.** Every timer is derived from open tasks at startup. Durability
   lives in [Tasks](data.md#tasks); the clock is a rebuildable projection of it.

Intended registrations, kept deliberately small: a **pulse** (self-check and "you just came
back"), the **reflection** backoff, and **task timers** derived from open tasks.

Deliberately *not* clock clients: the social timeout, the speech clock that paces on-screen
text against speech, and the session heartbeat. Each paces a loop inside its own subsystem
rather than waking an agent — and **waking agents is the only reason the clock exists.**

## Fix-forward

There is no true cancel. New input — a correction, a barge-in, a change of mind — is
incorporated by the always-free Reaction, which corrects course. This is more human than a
hard cancel and it fits the one-prompt-in-flight constraint: interruptions land on Reaction,
never on a busy worker.

A hard kill exists (drop the handle, the process exits) and stays available for the cases
where fix-forward genuinely does not apply.

## See also

[`agents.md`](agents.md) for what the core drives ·
[`data.md`](data.md#tasks) for what the clock is rebuilt from.
