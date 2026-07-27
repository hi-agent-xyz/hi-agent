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
| One mouth **per scene**, arbitrated centrally | Many sub-minds may think; a conversation hears one voice — but two conversations must never queue behind each other |
| A vendor outage is decided process-wide, not per turn | Every scene shares one upstream; N scenes must not each rediscover it, or apologize for it |
| The reflex path never reaches a model | Stopping when someone starts talking cannot wait a generation |
| The log is written *before* anything reacts | Durability must not depend on a session surviving |
| The clock holds no durable state | A timer that dies with the process is worse than no timer |
| Sessions are host-owned and disposable | Continuity lives in `data/`, not in a process |

## Components

### Wire adapters

Bind concrete protocols (HTTP today, WebSocket or local audio later) to transport-free
channel signals. Framing, mime, long-poll, body-close, per-turn frame binding — all of it
lives here so that none of it exists above.

### Scene router

A **scene** is the situation a signal belongs to: with a person, with a group, or alone.
It is the context-isolation unit — one Reaction and one [Deliberation](agents.md#deliberation--per-scene-seconds)
per scene, one memory slice, one tag on every session's tool attach. Reaction is followed up
each turn by its scene's Deliberation; [Cognition](agents.md#cognition--sceneless-minutes-and-beyond)
is sceneless and the router never addresses it.

Participants are *soft*, inferred from content, never a structural key. An external source
maps onto a scene (a group chat is a scene). The topology is decided once by judgment when
work is delegated, then executed mechanically.

### Channel mux

Fans N input channels into one prompt, and fans one output stream back out to N channels
by the carrier rules. This is a consequence of ACP carrying a single conversation, not a
goal in itself.

### Arbiter

The social layer, and the only place where "should this be said, now" is answered.

- **Mouth singleton, per scene.** Utterances queue for one mouth *per scene* and never
  overlap within it. The singleton is **scene-wide, not process-wide**: a global mouth would
  make one scene wait on another's turn, which is exactly what
  [invariant 3](arch.md#invariants) forbids. Inside a scene it binds ordinary replies,
  clock-driven surfacing and worker reports alike — which is why it cannot live inside any
  one of them.
- **Turn-taking.** The quiet-settle commit that decides a turn is over.
- **Presence gate.** Self-initiated speech is held while nobody is there, and released when
  they come back. It reads the derived [presence](#presence) model.
- **Social timing.** When to voice a worker's question, when to let it wait, when to tell
  the worker to proceed with a placeholder instead.

### Presence

Whether anyone is actually there. **Nobody sends it — it is derived**, a soft model fused
from several weak signals, none of which is trustworthy alone: channel activity, OS idle and
lock, a face seen, speech heard. Any single one lies (a person reading is idle; a face is a
photo), so presence is a confidence, not a boolean, and it is rendered for exactly two
readers — the arbiter's presence gate and Reaction, which needs to know whether it is
talking to a room or to nobody.

The failure it exists for: talking to an empty room. An utterance addressed to no one is
worse than silence, because it is spent — the person comes back and never learns it happened.

### Reflex

The sub-second path that **short-circuits every agent** — the bottom rung of the
[tempo ladder](arch.md#the-tempo-ladder), and the only one with no model in the loop.

Two kinds of thing live here. **Barge-in and the attention gesture**: when someone starts
speaking, sound stops mid-syllable and the unspoken tail is discarded. And **taught
quick-actions**: a small repeated thing the person showed the agent once, recognized and
replayed directly, because asking a model to re-derive it every time costs a generation to
reach an answer that never changes.

A generation is far too slow for either, which is the whole justification for a rung that
cannot think.

The barge-in *follow-up* is the opposite: what to do about the interruption is a judgment, handled by
Reaction on the next turn with an estimate of how far it got. The same event is therefore
handled at two very different speeds, which is why the reflex is drawn separately.

### Session layer

Exposes each ACP session as an independent handle — prompt it, read its updates, drop it to
close. One subprocess per session, so one session's crash cannot touch another. A warm pool
absorbs spawn latency for the sessions that are created per delegation.

Long-lived sessions rot — every turn appends, until early context is crowded out or the
window overflows. The **heartbeat** bounds that invisibly: it is itself a session kind, whose
job is to ask the rotting session for a compact self-briefing, open a replacement seeded with
that briefing plus the recent log tail, and hand it back to be swapped in **between**
turns. The conversation never sees a cold restart.

Two things make the summarizer safe to get wrong: it runs in the gap between turns, so a slow
one costs nothing; and the [log](#the-log) — not the briefing — is what durability rests
on, so a bad summary loses fluency, never facts.

### Vendor gate

One **process-wide** view of whether the upstream model is reachable, read by every scene
before it takes a turn. The vendor is a shared resource, so one scene discovering an outage
must steer all of them.

- **It gates the turn, not the reply.** During an outage no scene starts a generation at all;
  incoming mail is held rather than answered badly or dropped.
- **Backoff, absorbed and capped.** A blip is absorbed before anything is declared down; from
  there the retry gap doubles to a ceiling. A rate limit is not an outage worth mentioning; a
  string of failures is.
- **One apology, once.** The transition — not each failed turn — is what earns a word to the
  person. N scenes × M retries must never become N × M apologies, and recovery is likewise
  announced once.

When it clears, the held mail drives a catch-up turn. Fix-forward, like everything else here.

### The log

Every signal in and out is written before anything reacts to it. The log — not session
lifetime — is authoritative for durability, recovery and cold start.

**One tree, not two.** What earlier drafts split into `raw/` and a separate `journal/` is a
single append-only log at [`memory/raw/`](data.md#memoryraw). It is mechanical, so it belongs
to [`data/`](data.md); it appears here because it sits on the hot path.

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
