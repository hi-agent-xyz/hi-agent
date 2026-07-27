# Agents

## Goal

Be responsive and thorough at the same time, by refusing to make one layer do both.

Every agent here is the same thing — a general agent on an ACP session. They differ only in
**system prompt** and **tool surface**. That uniformity is deliberate: a new role is a new
prompt, not new machinery.

## Decisions

| Decision | Reasoning |
|---|---|
| Fast means *no fetch*, not *no knowledge* | Reaction runs a capable model; it is fast because it cannot wait on anything |
| Deliberation is per scene | No scene may ever go deaf because another scene is thinking |
| Cognition is sceneless and stays idle | Someone has to be awake when nobody is talking, and it must be free when they are |
| Only workers act | The moment there is an artifact or a side effect, that is a worker — enforced by the [grant table](foundation.md#who-may-act) |
| Cognition never speaks | Single-voice coherence: it proposes, Reaction voices |
| The switchboard is the host | No agent↔agent link; all routing and timers are Rust |

## The ladder

### Reaction — per scene, one generation

The mouth. Speaks, holds the floor, manages the interaction, decides whether to answer from
what it holds or hand the question onward.

**Tools: none.** An empty working directory, no reads, no fetches. It is fast because it
*cannot* wait on anything, not because it is small — judging the edge of your own knowledge
is a hard problem and needs a capable model.

Not blind, because its memory is **prepared**: system prompt, open tasks, hot memory and
scene recall are all in context before the first word. That set is **bounded and curated by
code** — it is in every window, so it cannot grow with usage. Two hundred open tasks project
as a summary, not a list.

### Deliberation — per scene, seconds

The scene's thinking. Interprets what was actually asked, reads around a little, calls a few
tools, and decides what would need to happen.

Anything heavy — a real task, a standing duty, a long errand — is handed **up to Cognition**
rather than done here. Deliberation stays light on purpose: it exists so that a scene can
think without leaving the scene, and so that no scene ever waits on another.

Its only correspondent is Reaction. That is a call stack in **addressing** — not in
**lifetime**: it can be woken independently and surface upward.

### Cognition — sceneless, minutes and beyond

The heavy lifter, which does its heavy lifting by **delegating**. Owns [Tasks](data.md#tasks),
dispatches workers, reasons across everything in memory, and tries hard to stay idle so it
is free the moment something arrives.

It has no scene, which is what makes it the right home for everything that has no scene:

> **After a restart, before any user input:** the clock fires → Cognition wakes → reads open
> tasks → checks what already landed so nothing is redone → dispatches workers for what is
> still wanted → for the user-facing ones, `surface` into the task's own scene, where
> Reaction voices it when the room is right.

Cognition never calls `say`. Everything it wants said is a **proposal** the arbiter
schedules. Two gates keep this human-shaped: Cognition asks *"is this worth raising?"*, the
arbiter asks *"is now the moment?"* The thinking part decides it matters; the social part
decides the timing.

### Reflection — sceneless, background

Curates `data/`. Never speaks, never dispatches — those belong to Reaction and Cognition
respectively, and putting either inside the consolidation loop would create a second mouth
or a second dispatcher.

Reflection must be **global, not per scene**: it merges a person seen in two scenes,
dedupes skills, and does drive housekeeping. It is the one agent that is legitimately not
scene-partitioned.

Its workers come in two kinds:

- **Per-store organizers** — hot, people, episodes, facets, views, skills, tools.
- **Cross-store graduations**, named by the edge they perform: `episode → skill`,
  `view → handle → hot`, `raw → drive bytes + facets meaning`, and
  `"promised, never delivered" → open task`.

Both kinds are **prose in `reflection.md`**. Adding one is not a code change.

One red line: reflection may **archive verbatim and write pointers, never paraphrase stored
bytes**.

## Cross-scene knowledge

Cross-scene reference is an **accepted side effect of one shared memory — not a goal.**

It happens the way it happens for a person: by *recall*. Any agent reading memory reaches
everything, because memory is global. What we specifically do not do is share a session
window between scenes — that is not one mind, it is one transcript, and it costs both
privacy and the ability of one scene to proceed while another thinks.

Privacy across scenes is handled the way everything else is: soft guidance and judgment.

## Workers

The **only layer that touches the world**. Everything above only thinks.

| Worker | Job |
|---|---|
| General | whatever the task is |
| View Builder | builds a view |
| View Reviewer | renders it, screenshots it, and **looks at it** before it ships |
| Decision Maker | below |

Workers are **volatile**: they live in process memory and die with it. Nothing durable may
live only inside one. Recovery is therefore **reconstruction from Tasks, never continuation**
— we do not checkpoint execution state.

They are **capability peers**, not children: a worker reaches the same memory, skills and
tools, and may spawn further workers. The one asymmetry is channels — a worker cannot speak
or show. It produces an intent; Reaction articulates it.

The bus is **bidirectional and non-blocking**. A worker posts progress, a question, or a
need for input; Cognition injects guidance or "proceed with a placeholder". Asks are intents,
never blocking calls — the worker keeps going and reconciles later.

### Decision Maker

Isolated because deciding under ambiguity is hard, critical, and the one thing worth a real
test suite.

```
(question, options, facts, user preference, goal) → (choice, confidence, what would change this)
```

Four boundaries:

1. **A judgment function, not an actor and not a mouth.** No speech, no side effects. It
   returns a decision; someone else executes it.
2. **A role, not a node.** Callable by Cognition *and* by a worker mid-errand — the gates
   that matter fire in the middle of running work, not before it starts.
3. **Its output includes `decide` or `must-ask`, and the test is reversibility.** Outward-facing
   or irreversible never auto-decides. This is the highest-value thing to unit-test, because
   being wrong here is the only kind of wrong we cannot walk back.
4. **It writes its rationale down.** A decision nobody can review is a decision nobody can
   correct, and correction is how user preference actually grows.

Why it exists at all: waiting on the user is the worst outcome available. Time is part of
being useful, so an autonomous, reviewable, slightly-wrong decision beats a correct question
nobody is around to answer.

## Delegation

> **If something takes more than a few trivial thoughts, delegate it.**

Responsiveness comes from delegation, not from keeping any layer weak. Each rung hands down
what does not belong to its tempo, and the rung below absorbs the silence.

## See also

[`core.md`](core.md) for the switchboard and the clock ·
[`data.md`](data.md) for what they read and write ·
[`legacy/reactor-cognition-split.md`](legacy/reactor-cognition-split.md) for the three-tempo
version this supersedes.
