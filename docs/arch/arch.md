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

| | Scope | Time | Job |
|---|---|---|---|
| **Reflex** | one scene | sub-second | taught quick-actions and barge-in — **no model in the loop** |
| **Reaction** | one scene | one generation | speaks, holds the floor, manages the interaction |
| **Deliberation** | one scene | seconds | works out what was actually asked — reads a little, checks a file, looks at the photo |
| **Cognition** | sceneless | minutes+ | the outward brain: owns Tasks, dispatches workers, stays idle and responsive |
| **Reflection** | sceneless | background | the inward brain: same capability, pointed at `data/` — the work nobody asked for |

Reflex is drawn on the ladder because it is a real tempo, but it is not an agent — it lives in
the host, in [`core.md`](core.md#reflex).

Below the ladder sit **workers** — where the actual jobs get done.

## The bands

```
  SURFACES     Appearance · Apps · Devices                        bidirectional
  CHANNELS     in: text · audio · vision · file(by ref)
               out: say · show · act
  ─────────────────────────────────────────────────────────────────────────────
  CORE         wire · scene router · channel mux · arbiter · presence ·
  (Rust)       sessions (+ heartbeat) · reflex · vendor gate
  ─────────────────────────────────────────────────────────────────────────────
  AGENTS       per scene:  Reaction ⟷ Deliberation
               sceneless:  Cognition · Reflection
               workers:    general · view builder · view reviewer · decision maker
               all of the above = one ACP session, differing only by prompt + tools
  ─────────────────────────────────────────────────────────────────────────────
  data/        memory (raw = the log · episodes · facets · tasks ·
                       prompts = generated, one per agent that needs state) ·
               prompts (bundled) · drive · skills · views
  ─────────────────────────────────────────────────────────────────────────────
  FOUNDATION   engine   runtime · ACP/MCP · gateway · config · store I/O · build ·
                        observatory · energy
               tools    bundled · user-added · agent-learnt (devices are just tools)
```

## Invariants

Each is a statement we can test, and each has a real failure behind it.

1. **One mouth per scene.** Only Reaction speaks, and within a scene utterances never
   overlap. The guarantee is *per scene, not process-wide* — a global mouth would make one
   scene queue behind another's turn, which is exactly what invariant 3 forbids. Everything
   else proposes; the arbiter decides when.
2. **Only workers act** — meaning **workers do the actual jobs**, and nothing more. This is a
   division of labour, not a security boundary. Per-role prompts and
   [tool surfaces](foundation.md#default-tool-surfaces) are a *context optimization* — a
   smaller window and a faster turn — not a rail deciding which agent may call which tool.
3. **Never wait on another scene.** Cross-scene reference is an accepted side effect of one
   shared memory — not a goal, and never worth a scene going deaf for.
4. **Open tasks are projected, not retrieved.** Retrieval can miss, and a missed duty is a
   silently broken promise. The general form — what earns a place in any window at all — is
   the [projection test](data.md#what-earns-a-place).
5. **A wake produces a turn, never an utterance.** Whatever a woken rung wants said goes
   through Reaction, and an empty room holds the turn rather than dropping it.
6. **The host opens the agent's eyes; the agent owns its own timers.** Three loops pace
   glancing up — the scene pulse, the reflection backoff, Cognition's glance-up. Scheduling
   past that is the agent's to build with the shell it already has; see
   [the clock we declined](core.md#glancing-up--and-why-there-is-no-clock).
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

## Contents

| Doc | Covers |
|---|---|
| [`surfaces.md`](surfaces.md) | surfaces, channels, carriers — how the world reaches the agent and back |
| [`core.md`](core.md) | the Rust host: scene routing, the arbiter, sessions, reflex, glancing up |
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
  Its continuous-vs-batch rule and the ACP carrier vocabulary still hold and are restated in
  `surfaces.md`.
- [`legacy/faculties.md`](legacy/faculties.md) — the built-vs-grown organization. Its
  placement test survives as the authorship rule above.
- [`legacy/reactor-cognition-split.md`](legacy/reactor-cognition-split.md) — the three-tempo
  split. Superseded by the ladder above, which adds Deliberation, moves Cognition out of the
  scene, and draws reflex as the rung below Reaction.
