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
| Deliberation is per scene | Reaction can speak and show but not read, so *someone* must open the file and look at the photo — and no scene may go deaf because another scene is thinking |
| Cognition is sceneless and stays idle | Someone has to be awake when nobody is talking, and it must be free when they are |
| Only workers act | The moment there is an artifact or a side effect, that is a worker. A division of labour, not a security boundary — [tool surfaces](foundation.md#default-tool-surfaces) are sized for context, not to fence anyone out |
| Cognition never speaks | Single-voice coherence: it proposes, Reaction voices |
| The switchboard is the host | No agent↔agent link; all routing and timers are Rust |
| A worker belongs to the session that created it | Not to a scene. Ownership is what makes a *sceneless* agent able to delegate at all — and a worker whose owner is a scene has nowhere to report but into someone's conversation |
| Work travels **up**, never sideways | A report goes to whoever asked for it, who decides what is worth passing further up. Nothing reaches a scene except through Reaction, because a scene is where a person is spoken to |
| An id names a **session**, not a role | A role has many sessions over a run; two scenes' Deliberations are two sessions of one role |
| **One verb between agents** | `SendMessage(to, message)` — one direction, no reply, queued. Every other shape we tried (delegate, ask, surface, handoff, notify) was this verb wearing a name that described one use of it |
| A worker replies; it does not narrate | It may message **only its owner**, and only in answer. Structural on the address, guidance on the timing |
| Cognition is the sole writer of the ledger | Two writers to one ledger means one of them is wrong and no way to tell which |

### Ownership and addressing

Every agent session has a process-wide id. **Process-wide, not per scene** — ownership
crosses scenes, and a sceneless owner holds sessions no per-scene counter could name
without collision.

It is a locally minted id rather than the underlying agent-protocol session id, and that
is forced rather than chosen: the tool surface identifies its caller by a header set when
the session opens, so the identifier must exist before the protocol assigns one.

A worker records the session that created it. Its report is delivered to that session,
which reads it on its next prompt. If the owner has shut down, the report falls back to
the scene rather than being dropped — **surfacing finished work one rung too high beats
losing it.**

Two things follow, and both are load-bearing:

- **An agent that owns live children is not idle.** Idle-reaping an owner out from under
  running work is what creates orphans; the fix is to not call it idle. Shutdown is
  graceful: finish or hand off, then close.
- **A session id addresses a live agent; a task subject addresses work.** Session ids die
  with the process, so nothing durable may reference one. Recovery reconstructs from
  [Tasks](data.md#tasks), never from a session.

## The ladder

Below the bottom of this ladder sits one more tempo that is *not* an agent: **reflex**, the
sub-second path with no model in the loop — barge-in and taught quick-actions. It lives in
the host, in [`core.md`](core.md#reflex), which is why it has no section here.

### Reaction — per scene, one generation

The mouth — **of its scene**. One Reaction and one mouth per scene; the singleton is not
process-wide, for the reason given in [invariant 1](arch.md#invariants). It speaks, holds the
floor, manages the interaction, and decides whether to answer from what it holds or hand the
question onward.

**Tools: `say`, `show`, and `SendMessage`.** Its two expression channels are calls — the
voice included — plus the one verb that reaches another agent. **No reads, no fetches, no
working directory, and no built-ins at all**: it is fast because it *cannot* wait on
anything, not because it is small. Judging the edge of your own knowledge is a hard problem
and needs a capable model.

**It also owns the social layer** — the mouth, the presence gate, and the timing of anything
unprompted. That was once a separate host component; all four of its duties were per-scene,
and this is the per-scene thing that speaks. See [`core.md`](core.md#the-social-layer-lives-in-reaction-not-here).

> **Enforced, not merely instructed.** This is the one place where a tool surface is a hard
> limit rather than a division of labour: the whole argument for the rung — that it *cannot*
> wait — is worth nothing if it can quietly open a file. Restricting our own tool surface is
> not sufficient on its own; the session's underlying toolset has to be restricted too, or
> "cannot" means "was asked not to".

Not blind, because its memory is **prepared**: the bundled prompt for its role, plus its
scene's [generated one](data.md#memoryprompts) — what the conversation carries forward,
open tasks, the recent log tail — all in context before the first word. **Code injects that
every turn and caps it**, so it cannot grow with usage. Two hundred open tasks project as a
summary, not a list.

It does not write that memory; it has no file access to write anything with. Its scene's
Deliberation writes it. And because this is the one rung that cannot go and look, it is also
the measure for everything else: **projected = what Reaction must know without reading** —
[the test](data.md#what-earns-a-place).

### Deliberation — per scene, seconds

**The scene's reading and thinking.** It exists because Reaction can speak and show but not
*read*: someone has to read a little, check a file, look at the photo that just arrived, and
work out what was actually asked — per scene, before anything reaches the shared brain. That
gap is the whole reason for this rung. Reaction follows up with it every turn.

Anything heavy — a real task, a standing duty, a long errand — is handed **up to Cognition**
rather than done here, by message. **Deliberation has no workers of its own**, and that is
deliberate: one dispatcher, so no two rungs can spawn against each other unseen. Its own
surface enforces it — reads and one write, no shell and no editor, so heavy work has nowhere
to go but up.

The hand-up must be **asynchronous**, and that is a requirement rather than a convenience: if
a scene waited on Cognition, one global session would serialize every scene, which is
[invariant 3](arch.md#invariants) broken where it matters most. `SendMessage` does not wait,
so it cannot happen.

Perception needs no tool here. A photo or a file arrives as a **ref**, a ref is a path, and
an agent that can read files can open it. What Deliberation needs is not a grant but knowing
where things land.

**Its second job is the scene's memory.** Reaction consumes a generated system prompt it
cannot write, so someone has to decide what this conversation carries forward — and that is a
judgment made out of having read around, which is exactly this rung. It writes
[`memory/prompts/scenes/<id>.md`](data.md#memoryprompts) the way reflection
writes a facet: no new tool, no new machinery, file access it already has.

Nothing **addresses** Deliberation except its own scene's Reaction — that is the direction of
the call stack, and it is a stack in *addressing* only, not in **lifetime**: it can be woken
independently and surface upward. Handing work up to Cognition is not a contradiction; that
is Deliberation calling out, not something calling in.

> **A naming correction, now carried out.** This rung existed for a while as the per-scene
> follow-up the reactor drives each turn, under the name "cognition" — now the name of the
> sceneless brain below. Per-scene reading is *Deliberation*; the shared brain is
> *Cognition*. The rename landed in the code; the unrelated *cognition tunables* (the
> agentic model config) keep the word in its other sense.

### Cognition — sceneless, minutes and beyond

**The shared brain.** One of it for the whole agent: it belongs to no scene, and every scene
hands work up to it. It does its heavy lifting by **delegating** — owns [Tasks](data.md#tasks),
dispatches workers, reasons across everything in memory, and tries hard to stay idle so it
is free the moment something arrives.

It is the **only** thing that creates workers, and the **only** writer of the task ledger.
Both follow from the same idea: durable work is what it means for something to be real, and
deciding that is judgment, not bookkeeping a scene rung should do in passing.

It has no scene, which is what makes it the right home for everything that has no scene:

> **After a restart, before any user input:** the clock fires → Cognition wakes → reads open
> tasks → checks what already landed so nothing is redone → creates workers for what is still
> wanted → for the user-facing ones, messages the task's `report_to` scene, where its
> Deliberation frames it and Reaction voices it when the room is right.

Answers travel back the way they came: what a Deliberation handed up returns to that
Deliberation. Cognition is sceneless, so its results arrive unframed — the scene rung is what
turns "the build failed" into something that fits the conversation it belongs to.

Cognition never calls `say`. Everything it wants said is a **proposal** the arbiter
schedules. Two gates keep this human-shaped: Cognition asks *"is this worth raising?"*, the
arbiter asks *"is now the moment?"* The thinking part decides it matters; the social part
decides the timing.

### Reflection — sceneless, background

Curates `data/`. Never speaks — that belongs to Reaction, and a second mouth inside the
consolidation loop is a second voice.

It **does** dispatch. That is a correction: dispatching was once listed here alongside
speaking as something Reflection must not do, on the reasoning that it would make a second
dispatcher. The reasoning does not survive ownership — a worker belongs to the session that
created it, so Reflection's workers are Reflection's, report to Reflection, and are visible
to no scene. There is no contention to avoid. What must stay singular is the *mouth* and
the *task ledger*, not the act of asking for help.

The earlier wording also conflated two things: the "workers" listed below are, today, jobs
Reflection performs itself in prose. Being able to hand one to a real worker is what the
correction buys.

Reflection must be **global, not per scene**: it merges a person seen in two scenes,
dedupes skills, and does drive housekeeping. It is the one agent that is legitimately not
scene-partitioned.

Its workers come in two kinds:

- **Per-store organizers** — people, episodes, facets, views, skills, tools.
- **Cross-store graduations**, named by the edge they perform: `episode → skill`,
  `raw → drive bytes + facets meaning`, and `"promised, never delivered" → open task`.

Views are not on that list. Reuse needs no promotion step — a view that mattered is already in
the scene's own memory, and the rest of the toolbox is [read on demand](data.md#views).

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

**Where the actual jobs get done.** Everything above mostly thinks — not because it is
forbidden to act, but because a rung that does the job is a rung that stopped being fast.

A worker's **type** is the `type` in [`CreateWorker(type)`](foundation.md#the-agent-session-registry),
and it selects a prompt and nothing else — same session, same tools. Adding a kind is
adding a `.md`.

| Worker | Job |
|---|---|
| General | whatever the task is |
| View Builder | builds a view |
| View Reviewer | renders it, screenshots it, and **looks at it** before it ships |
| Decision Maker | makes the call that lets work continue without the user — below |
| File Filer | puts something the person handed over into `drive/`, filed where the drive is already going |

Workers are **volatile**: they live in process memory and die with it. Nothing durable may
live only inside one. Recovery is therefore **reconstruction from Tasks, never continuation**
— we do not checkpoint execution state.

They are **capability peers**, not children: a worker reaches the same memory, skills and
tools. The one asymmetry is channels — a worker cannot speak or show. It produces an
intent; Reaction articulates it.

**A worker fans out with sub-agents, not with `CreateWorker`.** It may spin up as many
sub-agents as the job wants, using its harness's own facility for that — and they are
**invisible here**: no session id, no address, no registry entry, no report of their own.
They live and die inside the one worker session, which stays the single thing that is
accountable and the single thing that reports. So `CreateWorker` remains Cognition's and
Reflection's, and "one dispatcher" survives intact: nothing another rung can see was
created without it.

There is **one verb** between agents, in both directions:
`SendMessage(to, message)` — one way, no reply, queued and merged while the target is busy.
An owner steers its worker with it; the worker answers with it. A reply is just a message
going the other way, which is why `from` is stamped by the registry: it is the return
address.

A worker may address **only its owner** — structural, because that is routing. Whether
something is worth saying at all is judgment and lives in its prompt: **reply, don't
narrate.** Progress is not something a worker announces; it is something an owner asks for,
with a status read that costs no context until it wants the content.

Nothing routes automatically. A turn's output goes nowhere unless the agent sends it, so
**silence is legal** — and the host's completion event is what keeps silence visible rather
than indistinguishable from a hang.

### Decision Maker

**Waiting on the user is the worst outcome available.** Time is part of being useful, so an
autonomous, reviewable, slightly-wrong decision beats a correct question nobody is around to
answer. That is the whole reason this exists — it is what the agent reaches for when it needs
to **keep going without user input**.

So it is a **specialized worker**: dispatched like any other, differing only in prompt. Not a
gate, not a checkpoint, not machinery bolted to the side of the ladder — a new role here is a
new prompt, the same as everywhere else.

```
(question, options, facts, user preference, goal) → (choice, confidence, what would change this)
```

**"What would change this" is the load-bearing half of the output.** A decision that carries
the thing which would overturn it is reviewable and revisable; one that does not is a verdict.
Same reason it **writes its rationale down**: a decision nobody can review is a decision
nobody can correct, and correction is how user preference actually grows.

**It decides; it does not execute.** It hands back a choice and the caller acts on it — no
speech, no side effects. That is not a restriction placed on it, it is simply what being a
worker means here: Reaction is the mouth, and whoever asked owns the act.

**Reachable mid-errand, not only before work starts** — because the moments that genuinely
need a decision turn up in the middle of running work, not at its edges.

Reached **through the owner**, though, not directly: a worker holds no `CreateWorker`, so
one that needs a call says so to whoever created it and keeps going on its stated
assumption meanwhile. Since workers are created by Cognition and Reflection, the ask lands
on one of the two rungs that already holds the surrounding context — which is the right
place for it anyway. The cost is a hop; the thing it buys is that nothing stalls waiting,
which is the whole point of this worker existing.

Reversibility is what it weighs most heavily: the harder something is to walk back, the more
it should prefer asking. That is [invariant 9](arch.md#invariants), applied here as judgment
rather than restated as a rule — nothing enforces it, and the rationale it leaves behind is
how we find out when it judged badly.

## Delegation

> **If something takes more than a few trivial thoughts, delegate it.**

Responsiveness comes from delegation, not from keeping any layer weak. Each rung hands down
what does not belong to its tempo, and the rung below absorbs the silence.

## See also

[`core.md`](core.md) for the switchboard and the clock ·
[`data.md`](data.md) for what they read and write ·
[`legacy/reactor-cognition-split.md`](legacy/reactor-cognition-split.md) for the three-tempo
version this supersedes.
