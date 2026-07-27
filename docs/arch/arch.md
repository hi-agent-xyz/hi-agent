# hi-agent — Architecture

> **This directory is the current architecture contract.** `legacy/` holds the two earlier
> docs it replaces, kept for their reasoning.

## Goal

Be a person-shaped agent: always reachable, never in a hurry to say something useless, and
still working on your behalf while nobody is talking to it.

The guiding test for every decision is **fidelity to the human metaphor**, not simplicity at
the implementation level. Where a choice diverges from how a person would do it, the
divergence is named and justified.

## The authorship rule

**`data/` is the whole agent** — everything it heard, believes, owes, made, and is, in one
directory. The binary is interchangeable.

So the question is never *where does this live* but **who holds the pen**:

- **Foundation writes** raw, the journal, the factory prompts, the seed skills and views —
  all mechanical, none of it needing an agent to be correct.
- **Agents write** episodes, facets, tasks, drive, and everything learnt.

Where a subtree has both, the layers stay separate, so an upgrade replaces the factory one
and never touches the learnt one.

## The tempo ladder

Four thinking layers, ordered by time horizon and scope. Not by intelligence — every one of
them is a capable model. What differs is how long it may take and how much of the world it is
responsible for.

| | Scope | Time | Job |
|---|---|---|---|
| **Reaction** | one scene | one generation | speaks, holds the floor, manages the interaction |
| **Deliberation** | one scene | seconds | the scene's thinking — light reads, a few tools |
| **Cognition** | sceneless | minutes+ | owns Tasks, dispatches workers, stays idle and responsive |
| **Reflection** | sceneless | background | curates `data/` |

Below them sit **workers** — the only layer granted tools that touch the world.

## The bands

```
  SURFACES     Appearance · Apps · Devices                        bidirectional
  CHANNELS     in: text · audio · vision · file(by ref)
               out: say · show · act
  ─────────────────────────────────────────────────────────────────────────────
  CORE         wire · scene router · channel mux · arbiter ·
  (Rust)       sessions · reflex · clock
  ─────────────────────────────────────────────────────────────────────────────
  AGENTS       per scene:  Reaction ⟷ Deliberation
               sceneless:  Cognition · Reflection
               workers:    general · view builder · view reviewer · decision maker
               all of the above = one ACP session, differing only by prompt + tools
  ─────────────────────────────────────────────────────────────────────────────
  data/        raw · journal · prompts · memory · drive · skills · views
  ─────────────────────────────────────────────────────────────────────────────
  FOUNDATION   engine   runtime · ACP/MCP · gateway · config · store I/O · build
               tools    bundled · user-added · agent-learnt
               devices  android · macOS · …
```

## Invariants

Each is a statement we can test, and each has a real failure behind it.

1. **One mouth.** Only Reaction speaks. Everything else proposes; the arbiter decides when.
2. **Only workers act** — enforced by the [grant table](foundation.md#who-may-act),
   not by hope.
3. **Never wait on another scene.** Cross-scene reference is an accepted side effect of one
   shared memory — not a goal, and never worth a scene going deaf for.
4. **Open tasks are projected, not retrieved.** Retrieval can miss, and a missed duty is a
   silently broken promise.
5. **Clocks wake agents; they never speak.** An empty room holds the turn rather than
   dropping it.
6. **The clock holds no durable state.** Every timer is rebuilt from open tasks at startup.
7. **Recovery is reconstruction, not continuation.** Workers are volatile, so anything
   valuable is written down before the crash.
8. **A liveness probe that returns nothing means the thing is DOWN.** Count, don't check for
   existence.
9. **Irreversible or outward-facing → ask.** No silent outward action, ever.
10. **Secrets never enter the brain.** They live in env or keychain and are decrypted at call
    time.
11. **No absolute host path is persisted into `data/`.** The directory has to stay portable.

## Contents

| Doc | Covers |
|---|---|
| [`surfaces.md`](surfaces.md) | surfaces, channels, carriers — how the world reaches the agent and back |
| [`core.md`](core.md) | the Rust host: scene routing, the arbiter, sessions, reflex, clock |
| [`agents.md`](agents.md) | the tempo ladder in detail, workers, the decision maker |
| [`data.md`](data.md) | the directory that *is* the agent — raw, journal, prompts, memory, drive, skills, views |
| [`foundation.md`](foundation.md) | what the agent stands on — the engine, plus the tools and devices it reaches with |

Adjacent, unchanged: [`../memory.md`](../memory.md) (memory subsystem design),
[`../data-dir-layout.md`](../data-dir-layout.md) (the concrete tree),
[`../human.md`](../human.md) (behaviours we model),
[`../user-journeys/`](../user-journeys/) (what any of this is *for*),
[`../risks.md`](../risks.md).

## Legacy

- [`legacy/runtime-dataflow.md`](legacy/runtime-dataflow.md) — the previous runtime contract.
  Its continuous-vs-batch rule and the ACP carrier vocabulary still hold and are restated in
  `surfaces.md`.
- [`legacy/faculties.md`](legacy/faculties.md) — the built-vs-grown organization. Its
  placement test survives as the authorship rule above.
- [`legacy/reactor-cognition-split.md`](legacy/reactor-cognition-split.md) — the three-tempo
  split. Superseded by the four-tempo ladder, which adds Deliberation and moves Cognition out
  of the scene.
