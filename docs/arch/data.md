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
| `memory/raw/` — the log: everything in and out, as it crossed | `memory/` — episodes, facets, tasks, and `memory/prompts/` |
| `prompts/` — the bundled system prompts, all of them | `drive/` — what was decided worth keeping |
| `skills/`, `views/` — the factory seeds | `skills/`, `views/` — everything learnt |

**One pen per subtree — and the subtree, not the top-level directory, is the unit.**
`memory/raw/` is foundation's; the rest of `memory/` is the agents'. They share a roof and
never a file, which is all the rule ever required. Reading it as a top-level boundary is what
would push the log out to the root, where it means less.

**Factory-versus-learnt is a different rule, and it still holds.** Where a subtree carries
both layers — `skills/` and `views/` — they stay physically separate. An upgrade replaces the
factory layer and never touches the learnt one, so there is never a merge conflict, only a
precedence decision. Collapse them and an upgrade either clobbers what the agent learnt or can
no longer refresh its own seeds. `prompts/` no longer carries both: it is bundled through and
through, and what the agent writes for itself lives in
[`memory/prompts/`](#memoryprompts) instead.

## Decisions

| Decision | Reasoning |
|---|---|
| Tasks are **global** | Created in one scene, delivered in another; a restart has no scene at all |
| Open tasks are **projected, not retrieved** | Retrieval can miss; a missed duty is a silently broken promise |
| **Projected** = what Reaction must know without reading | It is tools-off, so its window is the whole of what it knows; every other rung can go and look |
| `prompts/` is **bundled**; carried-forward state is **generated** | Both are text handed to an agent at init — what differs is who wrote it and whether losing it matters |
| Meaning and bytes go to different places | A digest cannot be un-digested; the original is the only thing that stays true |
| There is no "import" | Perception, then deliberate retention — not an ETL pipeline |
| Reflection never prunes an open task | Curation must not be able to garbage-collect a promise |
| Secrets never enter a thinking layer | The one invariant structure alone cannot enforce |

## `memory/raw/`

**The log — one tree, not two.** Everything crossing the agent's boundary lands here, in
order, as it arrived. What earlier drafts called the *journal* was never a second place; it
was this same log named after its other half, and carrying both names is the one genuine
confusion in this directory.

It sits **inside `memory/`** on purpose: what the agent heard belongs with what it remembers.

Four properties, each earned:

1. **Written before anything reacts to it.** Durability must not depend on a session
   surviving. This is the claim the rest rests on — recovery reads the log, never a live
   process.
2. **Both directions, everything that crossed.** In *and* out: what was said and what was
   shown, not just what was sent to us, and pulse-driven wakes and worker reports alongside
   them. A restart that cannot reconstruct what the agent already said will say it again.
   **The honest gap today is outbound**, which is barely recorded — the concept is decided,
   the coverage is not there yet.
3. **Append-only, and precious the moment it lands** — before anything has understood it.
   That preciousness is exactly why foundation holds the pen: it must be safe without
   depending on any interpretation having happened. Graduation into `drive/` is
   *organization*, not safety.
4. **Authoritative for recovery and cold start** — not session lifetime, which is what makes
   long-lived sessions safe to keep. Mechanical throughout: append, read back, snapshot.

Bulk differs in mechanism, not principle: the log holds the **event**, and large bytes are
staged and moved rather than copied through.

Distinct from the logger and the [observatory](foundation.md#observatory), which are debug
surfaces — lossy and disposable by design. That difference is argued once, there.

## `prompts/`

The text for each role — Reaction, Deliberation, Cognition, Worker. Character and voice
merge in here rather than living apart; a role's manner is part of its prompt.

**One slot: what the app installs from the binary.** Factory-authored, reinstalled every boot,
replaced on upgrade, disposable — the binary is the original, so nothing here is worth backing
up.

**A file per role.** Reaction, Deliberation, Cognition, Reflection, the workers — each gets its
own bundled prompt, which is where [character](arch.md#character) is set. So `prompts/cognition.md`
(bundled, ours) sits alongside `memory/prompts/cognition.md` (generated, its own): same leaf
name, different parent, which is the whole pattern.

There is no user slot and no self slot. **An instruction from the person now lands the way
everything else they say lands**: they say it, and it becomes a preference facet or a task
depending on what it is. Nothing bypasses the agent's judgment on the way in — which is also
the cost, stated plainly: **there is no longer a lever that overrides the agent without going
through it.** A correction that does not stick is now a memory bug to fix, not a file to
hand-edit.

State the agent carries forward is not here at all. It is *generated*, and lives one level
down in [`memory/prompts/`](#memoryprompts) — the same leaf name under a different parent,
and the parent is the point.

## `memory/`

Design detail lives in [`../memory.md`](../memory.md); this is the architectural shape. The
[log](#memoryraw) is part of this tree too — described first, because everything else here is
derived from it.

**Episodes** — what happened, with provenance. The evidence base from which competence is
computed at read time rather than stored as a level, so answers can honestly distinguish
*"I read that"* from *"I tried that"*.

**Facets** — what is believed, by subject: people, projects, user preference. Growable,
revisable, correctable by one sentence from the person.

**Generated system prompts** — what each agent that needs state carries into every window.
Written by an agent, injected and bounded by code; [below](#memoryprompts).

**Proactivity** — the standing licence to speak unprompted, *per subject*: which topics the
person welcomes an unasked word on, which they tolerate, which are unproven, which are muted.
It is **learnt, not configured** — every unprompted word is a bet, and how it was met is the
evidence. Reflection folds each outcome back in.

Two properties it cannot work without. It **moves asymmetrically**: one brush-off pulls a
subject well back, warmth earns a small slow step up — because being talked at about the
wrong thing costs far more than a missed heads-up. And it is **short enough to read every
time**, since it is consulted before every proactive word; a licence too long to check is a
licence nobody checks.

### `memory/prompts/`

One file per agent that needs state carried forward.

| File | Whose state |
|---|---|
| `scenes/<id>.md` | what one scene carries forward |
| `cognition.md` | the sceneless brain's |

That is not a full set, on purpose — an agent gets one when it turns out to need one.
Reflection plausibly never will: its state is a frontier cursor plus the stores themselves,
and neither belongs in a window.

**Bundled versus generated is the whole distinction, and the parent directory is what carries
it.** Both leaves are named `prompts/` on purpose — they hold the same kind of thing, text
handed to an agent at init. Everything that differs is one level up:
[`data/prompts/`](#prompts) is **bundled** — shipped in the binary, reinstalled every boot,
disposable. `data/memory/prompts/` is **generated** — written by the agent, precious, and
rebuildable by nothing else. It sits under `memory/` because that is what it is: what this
agent remembers to bring.

**The agent writes the content. Code owns injection and the bound.**

| | |
|---|---|
| Content | the agent's — this is judgment about what matters, not a mechanical digest. Nobody's working memory is a truncation of their own transcript |
| Injection | code's, **every turn** — not only on a fresh session. A window that is only correct at session open is stale for the rest of the conversation |
| Size | code's, a **hard cap**. Over it, code truncates and says so — a ceiling that shows up as text is real; one that shows up as latency is not |
| Floor | the **log tail**, which code already assembles from [`memory/raw/`](#memoryraw). An agent that never got round to writing its memory — busy, crashed, mid-restart — leaves a window that is uncurated, never empty |

**Who writes a scene's.** Reaction holds `say` and `show` and nothing else, so it has no file
access and cannot write its own. A scene's memory is therefore *consumed* by Reaction and
*written by* [Deliberation](agents.md#deliberation--per-scene-seconds) — the rung that already
reads around and works out what was asked. That falls out of the tool surfaces rather than
being imposed on them, and it hands Deliberation its second job: deciding what this
conversation carries forward. No new tool is needed — it has file access, and writes the
scene's memory the way reflection writes a facet.

**A fresh scene starts from what is global, not from nothing.** Who this install is, what is
open, what is generally true of the person — the generator includes them, so a first reply is
not generic. Per-install identity is therefore a *section*, not a file, not a slot, and not a
separate always-projected block.

#### What earns a place

> **Projected = what Reaction must know without reading. Everything else is recall.**

Reaction is tools-off by design, so the projected set is the entirety of what it knows; every
other rung can read on demand. This is a test, not a list — it is what
[open tasks](#tasks) pass, and what everything left to recall fails.

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

**A task is a facet.** It lives as one more open-ended dimension — `memory/facets/tasks/` —
because the dimension list was always meant to be open, so this uses the design rather than
bending it: no new store, no new file format, and no new tools, since the agent already reads
and writes facets. What is special is the guidance attached, not the machinery.

**One ledger.** Nothing else records a duty: there is no second, friendlier list of what is
owed, because two ledgers means one of them is wrong and no way to tell which.

Reflection owns `facets/` and rewrites facets whole — so the one rule that keeps this safe is
guidance, not a rail: it may read the `tasks` dimension freely, and may notice that something
long-promised was never delivered, but while a task is open it does not prune, close, tidy or
merge it.

> **TODO — closed tasks accumulate.** A task is a subject, a closed task is the record that it
> was closed, and nothing deletes it. The cost is not disk but anything that *enumerates* —
> reflection's own prompt is seeded from the subject index. The invariant below says never
> pruned *while open*, which is the shape of the answer: closed, cold tasks age out the way
> ambient identity clusters already do. Deferred on purpose; not designed here.

Four properties, each earned by a real failure:

1. **Global**, with an optional `report_to: SceneId`.
2. **Always projected** into every agent's window, never fetched on demand.
3. **Liveness is a contract, not an existence check** — a serving task carries how to verify
   it is really alive (a count, not "something is running"), how to restart it, and either
   an owner or an idempotent start so two scenes cannot both relaunch it.

   This is the property that lets an agent pick **any** timing mechanism it likes — cron,
   `launchd`, a parked worker — without the host knowing or caring, so it carries the weight
   of [the clock we declined](core.md#glancing-up--and-why-there-is-no-clock). It only works
   if `verify` names a **result**. *"a cron job with this id exists"* passes forever, including
   when the job has never once fired — a watch shipped exactly that way, reported healthy, and
   had never fetched a price. *"`checked` was stamped in the last 3h by a run that returned
   real prices"* fails within one cadence for a job that never fires, a plist that never
   loaded, or a worker a restart killed. **`checked` is the one liveness field code reads**,
   and it is stamped only when the check came back *alive* — a probe that came back down must
   never stamp it, or the field records attention rather than health.
4. **Never pruned by reflection while open.**

A task's `due` is **read, never fired**. It orders the projection and marks what is overdue;
nothing in the host wakes on it, so a deadline is met at the next glance rather than at its
minute. An alarm that must land on the minute is the agent's to build.

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
| Ledgers | append-only, e.g. message-id → done, so a serving task never duplicates or misses |

There is no devices entry, on purpose: a device is [a tool plus a procedure](foundation.md#devices),
so what is worth keeping about one is a note and a skill, not a registry row.

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

**Nothing graduates out of the toolbox.** A view used recently leaves a trace in its scene's
[memory](#memoryprompts) because it mattered there — the same way anything else that
mattered does, written by the same judgment. Everything else is read on demand: list the
toolbox, against the guidelines for building views. Slow, and it always works.

## Forgetting

Keep-biased. Text is permanent; only raw replay fades, and never before it has been
consolidated. Rather keep low-value media than lose something later worth querying.

## See also

[`../memory.md`](../memory.md) · [`../data-dir-layout.md`](../data-dir-layout.md) ·
[`foundation.md`](foundation.md) for the code that writes parts of this.
