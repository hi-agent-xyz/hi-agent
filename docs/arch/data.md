# `data/` — the whole agent

## Goal

Put everything one hi-agent *is* into a single directory: what it heard, what it believes,
what it owes, what it made, and who it is.

The binary is interchangeable. The directory is the agent.

> **A thought, not yet built.** If `data/` is genuinely everything, then `jack.hi` is a
> complete agent-for-Jack that any hi-agent binary can open and continue. Two things keep
> that possible and are worth protecting now: **no absolute host paths are ever persisted
> into `data/`**, and the honest limit — a portable directory carries *knowledge and
> history, not authority*. Secrets and OS grants deliberately do not travel, so opening one
> on a new machine means re-granting.

## Who holds the pen

The question is not *where* something lives — it all lives here — but **who writes it**.

| Written by **foundation** (mechanical, no judgment) | Written by **agents** (judgment) |
|---|---|
| `raw/` — every signal as it arrived | `memory/` — episodes, facets, tasks |
| `journal/` — the durable backstop | `drive/` — what was decided worth keeping |
| `prompts/` — the factory bases | `prompts/` — the self layer |
| `skills/`, `views/` — the factory seeds | `skills/`, `views/` — everything learnt |

**Where a subtree has two writers, the layers stay separate.** An upgrade replaces the
factory layer and never touches the learnt one — so there is never a merge conflict, only a
precedence decision. Collapse them and an upgrade either clobbers what the agent learnt or
can no longer refresh its own seeds.

## Decisions

| Decision | Reasoning |
|---|---|
| Tasks are **global** | Created in one scene, delivered in another; a restart has no scene at all |
| Open tasks are **projected, not retrieved** | Retrieval can miss; a missed duty is a silently broken promise |
| Meaning and bytes go to different places | A digest cannot be un-digested; the original is the only thing that stays true |
| There is no "import" | Perception, then deliberate retention — not an ETL pipeline |
| Reflection never prunes an open task | Curation must not be able to garbage-collect a promise |
| Secrets never enter a thinking layer | The one invariant structure alone cannot enforce |

## `raw/`

Every signal, as it arrived: text, audio, vision, files. Append-only, and **precious the
moment it lands** — before anything has understood it.

That preciousness is exactly why foundation writes it: it must be safe without depending on
any interpretation having happened yet. Graduation into `drive/` is *organization*, not
safety.

Bulk payloads differ in mechanism, not principle: raw holds the **event**, and large bytes
are staged and moved rather than copied through.

## `journal/`

Every signal in and out, written **before anything reacts to it**. The journal — not session
lifetime — is authoritative for durability, recovery and cold start, and it is what makes
long-lived sessions safe to keep. Mechanical: append, read back, snapshot.

Distinct from the logger, which is a debug surface rather than a durability mechanism.

## `prompts/`

The text for each role — Reaction, Deliberation, Cognition, Worker. Character and voice
merge in here rather than living apart; a role's manner is part of its prompt.

**Three slots, in precedence order:**

| Slot | Author | On upgrade |
|---|---|---|
| base | factory | replaced |
| user | the person — sovereign | untouched |
| self | the agent | untouched |

The self slot must be **real and writable**. Without it, a correction the agent accepts has
nowhere to land, and the same mistake returns next week.

## `memory/`

Design detail lives in [`../memory.md`](../memory.md); this is the architectural shape.

**Episodes** — what happened, with provenance. The evidence base from which competence is
computed at read time rather than stored as a level, so answers can honestly distinguish
*"I read that"* from *"I tried that"*.

**Facets** — what is believed, by subject: people, projects, user preference. Growable,
revisable, correctable by one sentence from the person.

**Projected working sets** — what Reaction, Deliberation and Cognition carry into every
window. **Written by code**, so their size stays bounded and their contents predictable.

### Tasks

Tasks absorb what used to be standing commitments, unifying five previously separate
mechanisms:

| Kind | Example |
|---|---|
| WIP | a half-finished delivery interrupted by a restart |
| serving | watch a group, file what arrives, reply in thread |
| watch | a value, a baseline, a threshold, a cadence |
| deadline | a date and what to do at it |
| staged | a multi-stage job suspended for approval |

Four properties, each earned by a real failure:

1. **Global**, with an optional `report_to: SceneId`.
2. **Always projected** into every agent's window, never fetched on demand.
3. **Liveness is a contract, not an existence check** — a serving task carries how to verify
   it is really alive (a count, not "something is running"), how to restart it, and either
   an owner or an idempotent start so two scenes cannot both relaunch it.
4. **Never pruned by reflection while open.**

A task is the **durable record**; a [worker](agents.md#workers) is a volatile execution of
it. Conflating the two is what would turn recovery back into checkpointing.

## `drive/`

The agent's own filing cabinet — what it decided was worth keeping, in the shape it decided.
Distinct from the person's own archive.

| | |
|---|---|
| Projects | artifacts and bytes it produced or was given |
| Notes | verbatim pages — endpoints, calling conventions, where a key lives |
| Accounts | what it is logged into, and **where** each secret lives — never the secret |
| Devices | reachability, grants and accounts, per device |
| Ledgers | append-only, e.g. message-id → done, so a serving task never duplicates or misses |

**Two doors in.** Explicit — *"save this"* — goes through a worker now, at conversational
latency. Emergent — *"this turned out to be worth keeping"* — is graduated later by
reflection. Same destination, two latencies, told apart by judgment.

**Bytes and meaning split.** A handed-over document puts its bytes here verbatim and a
provenance-bearing claim into memory. Quantitative data is kept whole and analysed by a
separate tool; only conclusions become memory. Digesting a dataset into prose destroys the
thing that made it worth having.

**Sensitivity is cross-cutting** — private at ingest, in storage, on any view, and on any
outbound carrier. Not just at one of them.

## `skills/`

The workshop. Procedures in the agent's own words, deposited when a job was **hard, will
recur, and succeeded** — not from every task, or the workshop fills with noise.

- **A skill is a starting point, not truth.** Its durable half is used as-is; its perishable
  half is re-verified every time.
- **The perishable half must be marked.** A skill about filing taxes stores *where to look
  up this year's rules*, never this year's numbers.

Facts go to memory, procedures go here.

## `views/`

The toolbox of built surfaces, named by *what they are* rather than by the task that made
them — the name is what makes reuse possible.

Reuse works in three tiers: still in context (instant), graduated by reflection into a
`purpose → ref` handle projected into hot memory (fast), or found by listing the toolbox
(slow, but always works). The source stays in the toolbox; only the **handle** graduates.

## Forgetting

Keep-biased. Text is permanent; only raw replay fades, and never before it has been
consolidated. Rather keep low-value media than lose something later worth querying.

## See also

[`../memory.md`](../memory.md) · [`../data-dir-layout.md`](../data-dir-layout.md) ·
[`foundation.md`](foundation.md) for the code that writes parts of this.
