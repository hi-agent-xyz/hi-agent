# `data/` — the whole agent

## Goal

Put everything one hi-agent *is* into a single directory: what it heard, what it believes,
what it owes, what it made, and who it is.

The binary is interchangeable. The directory is the agent.

> **A thought, not yet built.** If `data/` is genuinely everything, then `jack.hi` is a
> complete agent-for-Jack that any hi-agent binary can open and continue. One thing keeps
> that possible and is worth protecting now: **no absolute host paths are ever persisted
> into `data/`**. The honest limit is narrower than it once read — **secrets travel**, in
> the clear, in [`drive/`](#drive); what does not travel is an **OS grant**, which is held
> by the machine and re-clicked there. So opening one on a new box costs the grants back,
> not the keys.
>
> The cost is stated rather than mitigated: a copy of `jack.hi` is a copy of every key in
> it. That is the property a person's own drive has always had, it is the one they were
> told about before the first key went in, and encrypting it here would only move the
> question to where the passphrase lives.

## Who holds the pen

The question is not *where* something lives — it all lives here — but **who writes it**.

| Written by **foundation** (mechanical, no judgment) | Written by **agents** (judgment) |
|---|---|
| `memory/raw/` — the log: everything in and out, as it crossed | `memory/` — episodes, facets, tasks; and `prompts/seed/` |
| `prompts/` — the bundled system prompts, all of them | `drive/` — what was decided worth keeping |
| `skills/`, `views/` — the `factory/` layer | `skills/`, `views/` — everything the agent built |

**One pen per subtree — and the subtree, not the top-level directory, is the unit.**
`memory/raw/` is foundation's; the rest of `memory/` is the agents'. They share a roof and
never a file, which is all the rule ever required. Reading it as a top-level boundary is what
would push the log out to the root, where it means less.

**Factory-versus-generated is a different rule, and it still holds.** Where a subtree carries
both layers — `prompts/`, `skills/`, `views/` — they stay physically separate, in a `factory/`
directory beside what the agent made. An upgrade replaces `factory/` and never touches its
sibling, so there is never a merge conflict, only a precedence decision. Collapse them and an
upgrade either clobbers what the agent built or can no longer refresh what we ship.

## Decisions

| Decision | Reasoning |
|---|---|
| Tasks are **global** | They outlive the exchange that created them, and a restart recovers them with no conversation in progress at all |
| Open tasks are **projected, not retrieved** | Retrieval can miss; a missed duty is a silently broken promise |
| **Projected** = what Reaction must know without reading | It is tools-off, so its window is the whole of what it knows; every other rung can go and look |
| `prompts/factory/` is **ours**; `prompts/seed/` is the **agent's** | Both are text handed to a session at init — what differs is who wrote it, and whether losing it costs knowledge or one reflection pass |
| A session is given **four layers** and each is a property of the material | System prompt, seed, what the person said, events. Nine sections once accreted into one per-turn block because this was never written down |
| Meaning and bytes go to different places | A digest cannot be un-digested; the original is the only thing that stays true |
| There is no "import" | Perception, then deliberate retention — not an ETL pipeline |
| Reflection never prunes an open task | Curation must not be able to garbage-collect a promise |
| A handed-over secret is filed in `drive/`, in the clear | It is already in the log verbatim the moment it is pasted; a second, curated copy is the only part we get to choose, and refusing it buys nothing while losing the key |
| The person is asked **once**, and the answer is durable | A per-key prompt is a nag that gets clicked through; the choice is about a kind of thing, not about one key |

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
   shown, not just what was sent to us, and the host's own wakes and worker reports alongside
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

Everything a session is given before the person says anything. Two layers, one directory
each, because **the pen is the distinction and it may never be ambiguous**:

```
prompts/
  factory/   reaction.md  cognition.md  reflection.md  workers/*.md
  seed/      reaction.md  cognition.md  proactivity.md
```

| | `factory/` | `seed/` |
|---|---|---|
| written by | us, shipped in the binary | the agent, about itself |
| when | reinstalled every boot, replaced on upgrade | whenever the agent judges it worth rewriting |
| where it lands | the **system prompt** — `baseInstructions` at thread start | the thread's **first message** |
| losing it | costs nothing; the binary is the original | costs one reflection pass, not knowledge |

**`factory/` — a file per role.** Reaction, Cognition, Reflection, the workers — each gets
its own whole prompt, which is where [character](arch.md#character) is set. Disposable by
construction: the binary is the original, so nothing here is worth backing up.

**`seed/` — what a rung brings to a thread it has just opened.** Generated, and it is the
`memory/prompts/` of earlier drafts, moved. It sits here rather than under `memory/` because
of what it *is*: a digest **over** the record, for one consumer, in the shape that consumer is
fed in. `memory/` holds the record — the log, episodes, facets, tasks. A digest of the record
is not the record.

**It is not precious, and that is a correction.** This was documented as "rebuildable by
nothing else", which was never consistent with the **floor** stated below:
the window's floor is the log tail, *explicitly* for an agent that never got round to writing
its memory. Its absence has always been a supported state. Everything in a seed is a digest of
things that are themselves durable — a preference is a facet, a duty is a task — so deleting one
costs the curation and no knowledge. It is a **cache of judgment**: rebuildable by the agent,
not by a mechanical pass, which is why the agent writes it and reflection rebuilds it when it
is missing.

**Which rungs get one** — two tests, both must hold:

1. **It outlives the work**, so there is continuity to lose.
2. **It cannot re-derive that continuity** — either it cannot read at all, or coming back cold
   it would not know what to read for.

| rung | outlives the work | can re-derive | seed |
|---|---|---|---|
| Reaction | permanent | **no** — tools-off, it cannot go and look at anything | **yes** |
| Cognition | permanent | can read, but out of a compaction would not know it was mid-thought | **yes** |
| Reflection | no — each pass is complete in itself | the stores *are* its memory | no |
| a worker | no — one task, then gone | its brief is its seed | no |
| a worker *type* | the type does, across instances | nothing writes craft knowledge yet | not yet |

The last row is the one slot deliberately named while empty: what `view-builder` learns about
house style is generated, per type, and belongs at `prompts/seed/workers/view-builder.md`
beside the bundled `prompts/factory/workers/view-builder.md`. Naming it now stops it landing
somewhere worse later.

**There is no user layer, and the reason is not that nobody asked.** An instruction from the
person lands the way everything else they say lands: they say it, and it becomes a preference
facet or a task depending on what it is. Nothing bypasses the agent's judgment on the way in
— which is also the cost, stated plainly: **there is no lever that overrides the agent without
going through it.** A correction that does not stick is a memory bug to fix, not a file to
hand-edit.

> **And there is no operator override either, as of this change.** `install_prompts` used to
> append a `*.local.md` sibling under an "Operator overrides" header. Nothing ever wrote one on
> any install, and it quietly contradicted the rule above — a slot with no occupant is not a
> courtesy, it is a second source of character that goes stale unseen. An operator who wants
> different text edits the factory file and gets it until the next boot rewrites it, or changes
> the binary.

## `memory/`

Design detail lives in [`../memory.md`](../memory.md); this is the architectural shape. The
[log](#memoryraw) is part of this tree too — described first, because everything else here is
derived from it.

**Episodes** — what happened, with provenance. The evidence base from which competence is
computed at read time rather than stored as a level, so answers can honestly distinguish
*"I read that"* from *"I tried that"*.

**Facets** — what is believed, by subject: people, projects, user preference. Growable,
revisable, correctable by one sentence from the person.

A person's facet carries one section under a fixed heading, **`## Working with them`**, and
that section alone is projected into Reaction's window, on the cadence below. Everything above it —
who they are, what they are building, how their world is arranged — is recall. Everything
under it is what changes what the agent does next: how they want work delivered, what is
theirs to decide rather than the agent's, what reliably goes fine and should not be
second-guessed.

**A section, not a second store, and not a schema.** A separate file holding "how to work
with them" would be a second copy of one understanding, and the two would disagree within a
week; a schema would turn the one store that is deliberately prose into a form to fill in,
and forms get filled. The heading is a key code slices on, nothing more — rename it and the
window goes without, the same degradation every other source in the projection already has.

**Why it is projected rather than pointed at** is the proactivity argument exactly: Reaction
is tools-off, so a path to a facet is a path nobody can follow. A preference stated plainly,
agreed to inside the minute, and gone by the next session is not a memory bug — the memory
held it. It is a projection bug, and this is the half that fixes it.

**The `people` dimension is written by a reader, not by the settling pass.** Reflection still
names and merges clusters; the prose is a [`person-reader`](agents.md#reflection--background)
worker's, one per person actually present in a stretch. Two pens on one file disagree — and
the reading that earns a section worth projecting (walk every ask, check the wire when one
didn't land, look for the rule before writing another) does not fit inside a pass that is
also segmenting the whole frontier.

**Generated system prompts** — what each agent that needs state carries into every window.
Written by an agent, injected and bounded by code; [below](#what-a-session-is-given-in-four-layers).

**Proactivity** — the standing licence to speak unprompted, *per subject*: which topics the
person welcomes an unasked word on, which they tolerate, which are unproven, which are muted.
It is **learnt, not configured** — every unprompted word is a bet, and how it was met is the
evidence. Reflection folds each outcome back in.

Two properties it cannot work without. It **moves asymmetrically**: one brush-off pulls a
subject well back, warmth earns a small slow step up — because being talked at about the
wrong thing costs far more than a missed heads-up. And it is **short enough to read every
time**, since it is consulted before every proactive word; a licence too long to check is a
licence nobody checks.

### What a session is given, in four layers

Everything that reaches a rung arrives in exactly one of these, and which one is a property of
the material, not a matter of taste. Nine sections once accreted into a single per-turn block
because nobody had written this down.

| | layer | delivered | changes how |
|---|---|---|---|
| **1** | **system prompt** — [`prompts/factory/<rung>.md`](#prompts) | `baseInstructions` at thread start. Never mid-thread | only by shipping a binary |
| **2** | **seed** — [`prompts/seed/<rung>.md`](#prompts) plus projections computed from the record | the thread's **first message**, before any input | re-sent whole when the thread goes cold |
| **3** | **what the person said** | the turn's input | it *is* the turn |
| **4** | **events** — a worker reported, cognition sent mail, a view went up, the clock came round, a barge-in | the turn's input, once, in order | it *is* the turn |

**Layer 2 is the answer to a rung that cannot go and look.** Reaction is tools-off, so its seed
is the whole of what it knows before the conversation starts: the generated file, plus the
things computed from the record at seed time — `## Working with them`, the open ledger, the
roster, and the recent-signals tail that tells a mind what happened before it existed.

**Computed, not materialised.** Only judgment that cannot be recomputed earns a file in
`prompts/seed/`. `## Working with them` is a read over people facets and stays a read: the
record is authoritative and a second copy would go stale against it.

#### State changes are events, derived by diff

A seed is true when it is sent and stale a minute later — a task opens, a view goes up, a
preference is corrected. Those reach the rung as **layer 4**, and how they are *noticed*
matters more than it looks.

**Announced events are not enough.** Cognition writes `memory/facets/tasks/<subject>/facet.md`
and its own seed *as files*, with file access — there is no tool call for it, so the host never
sees it happen. An announcement the writer forgets is a change Reaction never learns, and for
the ledger that is the failure this whole design exists to prevent: *retrieval can miss, and a
missed duty is a silently broken promise*.

**So the host diffs.** It goes on re-reading all of it every turn — cheap, and what the
invariant actually requires — and forwards only what moved. A diff cannot forget. It diffs at
the **item** level: one task line, one person's paragraph, one standing. Sending a changed task
line *is* the event.

**Cold is the one moment the seed is re-sent whole.** A thread is cold on its first turn and on
the turn after a compaction. The second case is the load-bearing one: compaction rewrites the
history and promises nothing about what it kept — on the 2026-08-13 Reaction thread it kept ten
copies of the standing preamble and dropped every tool call in sixty turns, taking every example
of `hi_say` with it, and Reaction then went two and a half hours without speaking while still
calling its other tools. Everything the host believed the model could see stopped being true at
once. So the same signal that re-seeds the window is the signal that the model may no longer
know how it speaks.

**Who writes a seed.** Reaction holds `hi_say` and `hi_show` and nothing else, so it has no file
access and cannot write its own: Reaction's seed is *consumed* by Reaction and *written by*
[Cognition](agents.md#cognition--minutes-and-beyond) — the rung that already reads around and
works out what was asked. That falls out of the tool surfaces rather than being imposed on them.
Reflection owns rebuilding a **missing** seed, on the pass that already regenerates
`proactivity.md` wholesale; without an owner, "rebuildable" is a wish.

**Code owns the bound, the agent owns the content.**

| | |
|---|---|
| Content | the agent's — judgment about what matters, not a mechanical digest. Nobody's working memory is a truncation of their own transcript |
| Size | code's, a **hard cap**. Over it, code truncates and says so — a ceiling that shows up as text is real; one that shows up as latency is not |
| Floor | the **log tail**, assembled from [`memory/raw/`](#memoryraw). An agent that never got round to writing its memory — busy, crashed, mid-restart — leaves a window that is uncurated, never empty |

**A fresh install starts from what is global, not from nothing.** Who this install is, what is
open, what is generally true of the person — the generator includes them, so a first reply is
not generic. Per-install identity is therefore a *section*, not a file, not a slot, and not a
separate always-projected block.

#### What earns a place

> **Projected = what Reaction must know without reading. Everything else is recall.**

Reaction is tools-off by design, so the projected set is the entirety of what it knows; every
other rung can read on demand. This is a test, not a list — it is what
[open tasks](#tasks) pass, and what everything left to recall fails.

#### What earns it *again*

> **Rebuilt every turn, sent when it changed.** A block the thread can still see upthread is
> already known; re-sending it buys nothing and costs a permanent copy of itself in a finite
> window.

Rebuilding has to happen every turn, or a task opened mid-conversation is invisible until the
session rotates. Re-*sending* it every turn was never a separate decision, and for a long time
the code did both.

Measured on one live thread, 108 turns at 10,125 chars each: `## Working with them` changed 10
times, the proactivity read 4, the reachable roster **0**, and all three rode every turn at
5,848 chars. The thread came out **80% its own re-sent preamble against 20% everything the
agent had ever done or said**. Caching is why nobody noticed — 98% of the last turn's input was
cached, so the repetition was nearly free to *send* and occupied the window exactly the same.

| | chars/turn | over those 108 turns |
|---|---|---|
| re-sent whole, as it was | 10,125 | 1,093,500 |
| section-level, sent on change — **built** | 1,514 | 163,551 |
| seed at init + a ledger that ignores its own clock — **built** | ~1,250 | ~135,000 |
| item-level diff events — **the target** | ~676 | ~73,000 |

**A block is compared on what it means, not on its characters.** A ledger line carries how
long something has been the way it is, and that number moves on its own: 65 of the 92 times
the projection "changed" on that thread, the only difference was `last confirmed alive 1h
ago` becoming `2h ago`, and 431 characters were sent to say it. The comparison blanks the
elapsed quantity and keeps the category, so `never checked` still differs from a check an
hour old, and a task crossing the idle boundary still reads as news. What is *sent* is
always the block verbatim.

**And an empty ledger says so out loud**, which sending-on-change made load-bearing: a block
that renders to nothing is skipped rather than sent, so a silent empty meant the last duty
could close with Reaction still believing it was owed. Nothing else would have told it — a
task is closed by a file edit, not by a message.

The floor is the events themselves, 387 chars a turn: the actual content of the conversation.
Everything else compresses toward zero.

**This is not a compression pass.** A conversation does not restate its own history every time
it takes a breath, and repetition did not merely cost tokens — it decided what a compaction
kept. The previous behaviour was a bug wearing a performance costume.

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

**So closing is the ledger writer's, and only its.** Nothing else in the loop can retire a
task — not reflection, which is barred above, and not the person, whose buttons on the review
surface are there to overrule the agent rather than to do its filing. That makes the *open*
list bounded by closure and by nothing else: an instruction that says how to open a duty and
not when to close one produces a list that only grows, and a projection where nothing reads as
urgent because everything is on it. The two closing moments are symmetrical with the opening
one and just as small — what was owed **reached the person**, or the person stopped wanting
it, in whatever words.

**Reached them, not exists.** These read as the same moment and are not. A worker reports
"delivered" meaning it handed the artifact up; the rung above closes on that word meaning
the person has it; the two are the same word in every report anyone writes. And the rung
doing the closing has no way to tell them apart: it is in no conversation, everything it
sends Reaction is a proposal Reaction may decline, and nothing comes back. So a close
written over a delivery that never happened is not carelessness — it is a rung reasoning
correctly from the only information it has.

**So give it the information, and leave the judgment alone.** `## On their screen` projects
the views that actually went up into the ledger writer's window, the same way the open task
list and the reachable list are projected: it is the one fact about its own work the rung
cannot obtain by thinking harder, so it is the one that has to be handed to it. Closing
stays exactly where it was — one writer, its own call, no status the host overrules.

This was the fork worth naming, because the other road was tempting and wrong. A task could
have carried the ref it owes and the host could have refused to let it close until it had
seen that ref go out. That is enforcement resting on the agent remembering to fill in the
very field that enforces it — and when it forgets, the mechanism is not merely absent, it is
*silently* absent, which is indistinguishable from a delivery that went fine. A safety net
whose failure mode is silence is worse than no net, because everything downstream is built
believing it is there. A projection cannot fail that way: it is either in the window or it
is not, and a rung that reads "you have not shown them this" and closes anyway has made a
decision rather than an omission. A task whose last remaining step belongs to the person is not the agent's work
in progress: it owes the ask, once, not the wait.

> **TODO — closed tasks accumulate.** A task is a subject, a closed task is the record that it
> was closed, and nothing deletes it. The cost is not disk but anything that *enumerates* —
> reflection's own prompt is seeded from the subject index. The invariant below says never
> pruned *while open*, which is the shape of the answer: closed, cold tasks age out the way
> ambient identity clusters already do. Deferred on purpose; not designed here.

Four properties, each earned by a real failure:

1. **Global.** There is one place a result can be said, so a task carries no destination.
2. **Always projected** into every agent's window, never fetched on demand.
3. **Liveness is a contract, not an existence check** — a serving task carries how to verify
   it is really alive (a count, not "something is running"), how to restart it, and either
   an owner or an idempotent start so two rungs cannot both relaunch it.

   This is the property that lets an agent pick **any** timing mechanism it likes — cron,
   `launchd`, a parked worker — without the host knowing or caring, which is what lets the
   host's timing surface stop at [glancing up](host.md#glancing-up). It only works
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
| Notes | verbatim pages — endpoints, calling conventions, how a thing is driven |
| Accounts | what it is logged into, the endpoint and calling convention, **and the key itself** |
| Ledgers | append-only, e.g. message-id → done, so a serving task never duplicates or misses |

There is no devices entry, on purpose: a device is [a tool plus a procedure](foundation.md#devices),
so what is worth keeping about one is a note and a skill, not a registry row.

**Two doors in.** Explicit — *"save this"* — goes through a worker now, at conversational
latency. Emergent — *"this turned out to be worth keeping"* — is graduated later by
reflection. Same destination, two latencies, told apart by judgment.

**Reading and writing it is every agent's**, not one session's errand: whoever knows what
it is putting down and where it goes puts it down. The
[drive organizer](agents.md#drive-organizer) is who the rest ask when they *don't* know —
where a new thing belongs, where an existing one is, or to straighten a corner that has
drifted. It holds the layout; it is not a gate in front of the disk.

**Bytes and meaning split.** A handed-over document puts its bytes here verbatim and a
provenance-bearing claim into memory. Quantitative data is kept whole and analysed by a
separate tool; only conclusions become memory. Digesting a dataset into prose destroys the
thing that made it worth having.

**Sensitivity is cross-cutting** — private at ingest, in storage, on any view, and on any
outbound carrier. Not just at one of them.

### Keys, passwords, and the one question

A key, a password, a token — handed over in the conversation rather than as a file — is
filed here like anything else, under `accounts/`, in the clear. It is not a special class of
thing with a vault of its own: the drive is the person's filing cabinet, and this is what
people keep in one.

**One question, asked the first time it comes up, and never again.** Not per key — per
person, once, because a prompt that fires on every key is a prompt that gets waved through.
What is being decided is that the drive holds keys in the clear and carries them wherever it
goes. Three answers:

| | |
|---|---|
| **this one** | file it; ask again next time |
| **all of them** | standing yes — file keys from here on without asking |
| **none** | standing no — never file a key; hold it for this exchange and let it go |

The answer is [a facet](#memory), the same as any other durable preference about the person,
and it reaches Reaction the way the rest do — through the seed.

**Absent an answer, ask.** A lost or never-written preference costs one extra question. It
must never cost a key filed against a *none* that went missing, so *no answer* resolves to
*ask*, never to *file*.

**Filing it is not using it.** A key in `accounts/` names what it opens and how it is
called, so a job months later can pick it up. Nothing about being filed puts it in front of
a thinking rung that did not go and fetch it, and nothing puts it back in a transcript, on a
screen, or into an outbound carrier — that is the sensitivity rule above, and a key is the
sharpest case of it.

**It is in the log already, and that is what makes this cheap.** The moment it was pasted it
landed verbatim in `memory/raw/`, which is append-only and does not fade for text. So the
decision here was never *whether the directory holds this key* — it does either way. It is
only whether there is also a copy someone curated, in the place the agent will actually look.

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
them, and each opening with a one-line `// purpose:` saying what it is for. The name and
that line are what make reuse possible.

**Nothing graduates out of the toolbox, and nothing indexes it.** A view used recently
leaves a trace in the [seed](#prompts) because it mattered there — the same way
anything else that mattered does, written by the same judgment. Everything else is read on
demand: one scan of the toolbox's purpose lines, against the guidelines for building views.
Slow, and it always works.

Those lines live **in the files, never in an index beside them**. An index is bookkeeping,
and bookkeeping kept by judgment drifts silently — a missed entry reads as "never built"
while the file sits right there, so the toolbox would get less trustworthy the more it
accumulated. Derived from the tree instead, a missing line degrades to a bare filename,
which is merely where this started. It never becomes a confident wrong answer.

## Forgetting

Keep-biased. Text is permanent; only raw replay fades, and never before it has been
consolidated. Rather keep low-value media than lose something later worth querying.

## See also

[`../memory.md`](../memory.md) · [`../data-dir-layout.md`](../data-dir-layout.md) ·
[`foundation.md`](foundation.md) for the code that writes parts of this.
