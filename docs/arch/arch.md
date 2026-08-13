# hi-agent — Architecture

> **This directory is the current architecture contract.** `legacy/` holds the two earlier
> docs it replaces, kept for their reasoning.

## Goal

Be a person-shaped agent: always reachable, never in a hurry to say something useless, and
still working on your behalf while nobody is talking to it.

The guiding test for every decision is **fidelity to the human metaphor**, not simplicity at
the implementation level. Where a choice diverges from how a person would do it, the
divergence is named and justified.

## Character

**hi-agent has its own character. It is not configured into obedience.** It is not an agent
serving one person and therefore owing them total compliance — it works like a helpful person:
you talk to it, and it comes to understand you through use.

That is the reasoning behind several decisions elsewhere that would otherwise read as
arbitrary — no user prompt slot, no per-install persona file, no hand-edit lever anywhere:

- **Who hi-agent is** comes from the [bundled prompts](data.md#prompts) — ours, shipped in the
  binary, unwritable by it, and carrying **a file per role**. Editing Reaction's or Cognition's
  prompt is how character gets shaped, and that is a thing we do, not a thing the deployment
  does.
- **Who it works for** is learnt by meeting them, and lives in facets like anything else it
  has come to believe.

So a preference that fails to stick is a **memory bug to fix, not a file to hand-edit**. The
cost is real and worth stating: nothing overrides the agent without going through its
judgment.

## The authorship rule

**`data/` is the whole agent** — everything it heard, believes, owes, made, and is, in one
directory. The binary is interchangeable.

So the question is never *where does this live* but **who holds the pen**:

- **Foundation writes** the log — `memory/raw/`, everything in and out as it crossed — plus
  the bundled system prompts and the seed skills and views. All mechanical, none of it needing
  an agent to be correct.
- **Agents write** episodes, facets, tasks, their own
  [generated system prompts](data.md#memoryprompts), drive, and everything learnt.

The unit is the **subtree, not the top-level directory**. Foundation's log lives *inside*
`memory/`, alongside what the agents write, because what the agent heard belongs with what it
remembers. One pen per subtree; never two on one file.

Separately, where a subtree carries both a factory and a learnt layer, the two stay separate,
so an upgrade replaces the factory one and never touches the learnt one.

## The tempo ladder

Five rungs, ordered by time horizon and scope. Not by intelligence — the four thinking rungs
all run a capable model. What differs is how long each may take and how much of the world it
is responsible for. The bottom rung is the exception: **reflex runs no model at all**, because
a generation is far too slow for what it handles.

There is **one of each**. The agent is one mind having one continuous conversation, so no
rung is partitioned — see [One conversation](host.md#one-conversation).

| | Time | Job |
|---|---|---|
| **Reflex** | sub-second | taught quick-actions and barge-in — **no model in the loop** |
| **Reaction** | one generation | speaks, holds the floor, manages the interaction |
| **Cognition** | seconds to minutes+ | the outward brain: reads what arrived and works out what was actually asked, owns Tasks, dispatches everything heavy, stays idle and responsive |
| **Reflection** | background | the inward brain: same capability, pointed at `data/` — the work nobody asked for |

Reflex is drawn on the ladder because it is a real tempo, but it is not an agent — it lives in
the host, in [`host.md`](host.md#reflex).

Below the ladder sit **workers** — where the actual jobs get done.

## The bands

```
  SURFACES     Appearance · Apps · Devices                        bidirectional
  CHANNELS     in: text · audio · vision · file(by ref)
               out: say · show · act
  ─────────────────────────────────────────────────────────────────────────────
  HOST         wire · channel mux · transcript ·
  (Rust)       sessions (+ heartbeat) · reflex · vendor gate
  ─────────────────────────────────────────────────────────────────────────────
  AGENTS       the voice:  Reaction
               the brain:  Cognition · Reflection
               workers:    general · view builder · view reviewer · decision maker
               all of the above = one agent session, differing only by prompt + tools
  ─────────────────────────────────────────────────────────────────────────────
  data/        memory (raw = the log · episodes · facets · tasks ·
                       prompts = generated, one per agent that needs state) ·
               prompts (bundled) · drive · skills · views
  ─────────────────────────────────────────────────────────────────────────────
  FOUNDATION   engine   runtime · agent wire/MCP · gateway · config · store I/O · build ·
                        observatory · energy
               tools    bundled · user-added · agent-learnt (devices are just tools)
```

## Invariants

Each is a statement we can test, and each has a real failure behind it.

1. **One mouth.** Only Reaction speaks, and utterances never overlap. Everything else
   proposes; Reaction decides when.
2. **Only workers act** — meaning **workers do the actual jobs**, and nothing more. This is a
   division of labour, not a security boundary. Per-role prompts and
   [tool surfaces](foundation.md#default-tool-surfaces) are a *context optimization* — a
   smaller window and a faster turn — not a rail deciding which agent may call which tool.
3. **The voice never waits on a slower rung.** Reaction hands work down and reads the answer
   when it arrives; it does not block on Cognition or a worker. A mouth that waits is a
   person left staring at silence, and no result is worth going deaf for.

   Its counterpart, which is what makes one brain safe to put on the conversation's path:
   **Cognition never grinds.** It reads and answers, and everything with an artifact, a
   side effect or a long tail goes to a worker — so its turns stay short and the
   conversation is never queued behind its own errand.
4. **Open tasks are projected, not retrieved.** Retrieval can miss, and a missed duty is a
   silently broken promise. The general form — what earns a place in any window at all — is
   the [projection test](data.md#what-earns-a-place).
5. **A wake produces a turn, never an utterance.** Whatever a woken rung wants said goes
   through Reaction, which decides whether it is worth saying and says it as a message.
6. **The host opens the agent's eyes; the agent owns its own timers.** Two loops pace
   glancing up — Cognition's glance-up and the reflection backoff. The voice has no
   cadence of its own: it wakes on input, on mail, and on its own check-in. Scheduling
   past that is the agent's to arrange with the shell it already has; see
   [glancing up](host.md#glancing-up).
7. **Recovery is reconstruction, not continuation.** Workers are volatile, so anything
   valuable is written down before the crash.
8. **A liveness probe that returns nothing means the thing is DOWN.** Count, don't check for
   existence.
9. **Irreversible or outward-facing → ask.** No silent outward action, ever. This one is
   about reversibility, and it stands — as guidance the agent follows, not a gate the host
   enforces.
10. **Secrets never enter the brain.** They live in env or keychain and are decrypted at call
    time.
11. **No absolute host path is persisted into `data/`.** The directory has to stay portable.
12. **Who a signal came from is decided at the boundary, never derived from what it says** —
    and *unknown* is an answer the record keeps. A name inferred from content is
    indistinguishable from one that was verified, so the inference is not available; see
    [`signal-attribution.md`](signal-attribution.md).

## Contents

| Doc | Covers |
|---|---|
| [`topology.md`](topology.md) | core, app and community — where the parts run, how a person is addressed, how an app proves it may reach one |
| [`surfaces.md`](surfaces.md) | surfaces, channels, carriers — how the world reaches the agent and back |
| [`signal-attribution.md`](signal-attribution.md) | who a signal came from — the three source classes, the owner, and keeping "unknown" |
| [`host.md`](host.md) | the Rust host: the one conversation, sessions, reflex, glancing up |
| [`text-transcript.md`](text-transcript.md) | the append-only message list: what is a message, ownership, wire, durability |
| [`stage.md`](stage.md) | what may be on screen at once — bundled vs compiled views, the four roles, the conversation's three presentations |
| [`agents.md`](agents.md) | the tempo ladder in detail, workers, the decision maker |
| [`data.md`](data.md) | the directory that *is* the agent — memory (the log and the generated system prompts included), the bundled prompts, drive, skills, views |
| [`foundation.md`](foundation.md) | what the agent stands on — the engine, plus the tools it reaches with (devices included) |

Adjacent, unchanged: [`../memory.md`](../memory.md) (memory subsystem design),
[`../data-dir-layout.md`](../data-dir-layout.md) (the concrete tree),
[`../human.md`](../human.md) (behaviours we model),
[`../user-journeys/`](../user-journeys/) (what any of this is *for*),
[`../risks.md`](../risks.md).

## Legacy

- [`legacy/runtime-dataflow.md`](legacy/runtime-dataflow.md) — the previous runtime contract.
  Its continuous-vs-batch rule and the carrier vocabulary still hold and are restated in
  `surfaces.md`.
- [`legacy/faculties.md`](legacy/faculties.md) — the built-vs-grown organization. Its
  placement test survives as the authorship rule above.
- [`legacy/reaction-cognition-split.md`](legacy/reaction-cognition-split.md) — the three-tempo
  split. Superseded by the ladder above, which draws reflex as the rung below Reaction.
