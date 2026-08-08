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
| Deliberation is separate from Reaction | Reaction can speak and show but not read, so *someone* must open the file and look at the photo — and the voice may not go deaf while that happens |
| Cognition stays idle | Someone has to be awake when nobody is talking, and it must be free when they are |
| Only workers act | The moment there is an artifact or a side effect, that is a worker. A division of labour, not a security boundary — [tool surfaces](foundation.md#default-tool-surfaces) are sized for context, not to fence anyone out |
| Cognition never speaks | Single-voice coherence: it proposes, Reaction voices |
| The switchboard is the host | No agent↔agent link; all routing and timers are Rust |
| A worker belongs to the session that created it | Ownership is what makes delegation addressable at all — a report has exactly one place to go, and it is not "the conversation" by default |
| Work travels **up**, never sideways | A report goes to whoever asked for it, who decides what is worth passing further up. Nothing reaches the person except through Reaction |
| An id names a **session**, not a role | A role has many sessions over a run; a Deliberation replaced after a failure is a second session of one role |
| **One verb between agents** | `SendMessage(to, message)` — one direction, no reply, queued. Every other shape we tried (delegate, ask, surface, handoff, notify) was this verb wearing a name that described one use of it |
| A worker replies; it does not narrate | It may message **only its owner**, and only in answer. Structural on the address, guidance on the timing |
| Cognition is the sole writer of the ledger | Two writers to one ledger means one of them is wrong and no way to tell which |
| **A gap in the request is work, not a question** | Every rung, not just the Decision Maker. Waiting on the user is the worst outcome available, so an unknown gets the most defensible reading, stated out loud; asking is the fallback for when no reading is defensible *and* the gap gates the work |

### Ownership and addressing

Every agent session has a process-wide id — one namespace for every rung and every worker,
so any session can name any other without ambiguity.

It is a locally minted id rather than the underlying agent-protocol session id, and that
is forced rather than chosen: the tool surface identifies its caller by a header set when
the session opens, so the identifier must exist before the protocol assigns one.

A worker records the session that created it. Its report is delivered to that session,
which reads it on its next prompt. If the owner has shut down, the report falls back to
Reaction rather than being dropped — **surfacing finished work one rung too high beats
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

**One rule runs the length of the ladder: a gap in the request is work, not a question.**
[Invariant 9](arch.md#invariants) says irreversible or outward-facing → ask, and that is the
*whole* list. Not knowing something is not on it. An undefined term, a figure nobody gave, a
section with nothing behind it — every rung takes the most defensible reading, says which
reading it took, and keeps going; a stated assumption costs a word to correct, where a
question costs however long the person is away. Asking is the fallback for when no reading is
defensible *and* the gap gates the work. Nothing the agent produces arrives empty because an
answer never came. The [Decision Maker](#decision-maker) is the escalation of this rule, not
the only place it applies.

### Reaction — one generation

The mouth. One Reaction, one mouth, one turn at a time —
[invariant 1](arch.md#invariants). It speaks, holds the floor, manages the interaction, and
decides whether to answer from what it holds or hand the question onward.

**Tools: `say`, `show`, and `SendMessage`.** Its two expression channels are calls — the
voice included — plus the one verb that reaches another agent. **No reads, no fetches, no
working directory, and no built-ins at all**: it is fast because it *cannot* wait on
anything, not because it is small. Judging the edge of your own knowledge is a hard problem
and needs a capable model.

**It also owns the social layer** — the mouth, the presence gate, and the timing of anything
unprompted. That was once a separate host component; all four of its duties belong to
whatever speaks, and this is what speaks. See [`core.md`](core.md#the-social-layer-lives-in-reaction-not-here).

> **Enforced, not merely instructed.** This is the one place where a tool surface is a hard
> limit rather than a division of labour: the whole argument for the rung — that it *cannot*
> wait — is worth nothing if it can quietly open a file. Restricting our own tool surface is
> not sufficient on its own; the session's underlying toolset has to be restricted too, or
> "cannot" means "was asked not to".

Not blind, because its memory is **prepared**: the bundled prompt for its role, plus the
[generated one](data.md#memoryprompts) — what the conversation carries forward,
open tasks, the recent log tail — all in context before the first word. **Code injects that
every turn and caps it**, so it cannot grow with usage. Two hundred open tasks project as a
summary, not a list.

It does not write that memory; it has no file access to write anything with. Deliberation
writes it. And because this is the one rung that cannot go and look, it is also
the measure for everything else: **projected = what Reaction must know without reading** —
[the test](data.md#what-earns-a-place).

### Deliberation — seconds

**The conversation's reading and thinking.** It exists because Reaction can speak and show
but not *read*: someone has to read a little, check a file, look at the photo that just
arrived, and work out what was actually asked, before anything reaches the shared brain. That
gap is the whole reason for this rung. Reaction follows up with it every turn.

Anything heavy — a real task, a standing duty, a long errand — is handed **up to Cognition**
rather than done here, by message. **Deliberation has no workers of its own**, and that is
deliberate: one dispatcher, so no two rungs can spawn against each other unseen. Its own
surface enforces it — reads and one write, no shell and no editor, so heavy work has nowhere
to go but up.

The hand-up must be **asynchronous**, and that is a requirement rather than a convenience: a
voice that waited on Cognition would go deaf for as long as the brain took to think, which is
[invariant 3](arch.md#invariants) broken where it matters most. `SendMessage` does not wait,
so it cannot happen.

Perception needs no tool here. A photo or a file arrives as a **ref**, a ref is a path, and
an agent that can read files can open it. What Deliberation needs is not a grant but knowing
where things land.

**Its second job is the conversation's memory.** Reaction consumes a generated system prompt
it cannot write, so someone has to decide what the conversation carries forward — and that is
a judgment made out of having read around, which is exactly this rung. It writes
[`memory/prompts/conversation.md`](data.md#memoryprompts) the way reflection
writes a facet: no new tool, no new machinery, file access it already has.

Nothing **addresses** Deliberation except Reaction — that is the direction of
the call stack, and it is a stack in *addressing* only, not in **lifetime**: it can be woken
independently and surface upward. Handing work up to Cognition is not a contradiction; that
is Deliberation calling out, not something calling in.

> **A naming correction, now carried out.** This rung existed for a while as the follow-up
> the reaction drives each turn, under the name "cognition" — now the name of the brain
> below. The conversation's reading is *Deliberation*; the brain that outlives any one
> exchange is *Cognition*. The rename landed in the code; the unrelated *cognition tunables*
> (the agentic model config) keep the word in its other sense.

### Cognition — minutes and beyond

**The brain.** One of it for the whole agent, and Deliberation hands work up to it. It does its heavy lifting by **delegating** — owns [Tasks](data.md#tasks),
dispatches workers, reasons across everything in memory, and tries hard to stay idle so it
is free the moment something arrives.

It is the **only** thing that creates workers, and the **only** writer of the task ledger.
Both follow from the same idea: durable work is what it means for something to be real, and
deciding that is judgment, not bookkeeping the voice should do in passing.

It outlives any one exchange, which is what makes it the right home for everything that
outlives one:

> **After a restart, before any user input:** the glance-up fires → Cognition wakes → reads
> open tasks → runs each one's `verify` and believes the answer → checks what already landed
> so nothing is redone → does or re-arms what is still wanted → for the user-facing ones,
> messages Deliberation, which frames it, and Reaction voices it when the room is right.

This is the sequence, not a plan for one: the glance-up is a timer arm on Cognition's own
loop ([`core.md`](core.md#glancing-up--and-why-there-is-no-clock)) — one wake shortly after
the process starts, then on the pulse cadence while anything is owed.

Answers travel back the way they came: what Deliberation handed up returns to Deliberation.
Cognition's results arrive unframed — Deliberation is what turns "the build failed" into
something that fits the conversation.

Cognition never calls `say`. Everything it wants said is a **proposal** Reaction schedules.
Two gates keep this human-shaped: Cognition asks *"is this worth raising?"*, Reaction asks
*"is now the moment?"* The thinking part decides it matters; the social part decides the
timing.

### Reflection — background

**The inward brain, and the same kind of thing as Cognition.** Both are as capable as the
agent gets, both dispatch workers, neither speaks. What separates them is not intelligence
and not machinery — it is **who the work is for**:

| | Work arrives from | Answers to | Owns |
|---|---|---|---|
| **Cognition** | a person, through the conversation | the conversation | the task ledger |
| **Reflection** | nobody — it notices | itself | `data/` |

That asymmetry is the reason Reflection needs a rung of its own rather than being a job
Cognition does when it is idle: **work nobody is waiting on never happens if it has to
queue behind work someone is.** An agent that only ever did what was asked would never
tidy its own memory, and would degrade in a way nothing in the conversation reveals.

*This corrects an earlier framing.* Reflection was cast as a **curator** beside a
**brain**, and the implementation followed the words: a one-shot pass that opened a
session, prompted it once and dropped it. It could dispatch a worker and then not read
the report, because the session that asked was already gone. The rung is Cognition's
shape now — a process-lifetime address, a loop that drains its inbox, its own worker
host — and it wakes two ways: its own backoff clock for a settling pass, and mail for
everything else. It keeps the **session per pass** that Cognition has since given up, and
that divergence is deliberate: see [Session lifetime](#session-lifetime-per-rung).

**Never speaks.** That belongs to Reaction, and a second mouth inside the consolidation
loop is a second voice. What it wants said it sends to Cognition or to Reaction, and
Reaction chooses the moment.

**It does dispatch**, and that too was once forbidden here on the reasoning that it would
make a second dispatcher. The reasoning does not survive ownership: a worker belongs to
the session that created it, so Reflection's workers are Reflection's, report to
Reflection, and never surface in the conversation. What must stay singular is the *mouth*
and the *task ledger* — not the act of asking for help.

**It works across the whole store**: it merges a person seen on two occasions, dedupes
skills, does drive housekeeping.

Its workers come in two kinds:

- **Per-store organizers** — people, episodes, facets, views, skills, tools.
- **Cross-store graduations**, named by the edge they perform: `episode → skill`,
  `raw → drive bytes + facets meaning`, and `"promised, never delivered" → open task`.

Views are not on that list. Reuse needs no promotion step — a view that mattered is already in
the conversation's own memory, and the rest of the toolbox is [read on demand](data.md#views).

Both kinds are **prose in `reflection.md`**. Adding one is not a code change.

One red line: reflection may **archive verbatim and write pointers, never paraphrase stored
bytes**.

### Session lifetime, per rung

Specified here because it was previously specified nowhere, and two documents drifted apart in
the gap: `core.md` described how a long-lived session is kept bounded, while Cognition
was built to reopen per wake. Both were defensible readings. This is the decision.

| Rung | Session | Replaced when |
|---|---|---|
| **Reaction** | one, process-wide, long-lived | a turn fails |
| **Deliberation** | one, process-wide, long-lived | a turn fails, or it sits idle past a TTL |
| **Cognition** | one, process-wide, long-lived | a turn fails |
| **Reflection** | **one per pass** | never — the pass ends |
| Workers | one per errand | the errand ends, or an idle TTL |

**Nothing in this column is about size.** Context growth is bounded by the underlying agent,
which compacts in place; see [`core.md`](core.md#session-layer) for why that is not ours to
do. A session is replaced here only because it **broke**.

Deliberation keeps the idle TTL it inherited from the worker machinery it shares: quiet for
long enough drops it, and the next turn opens a fresh one. That is a **resource** bound — it
exists so an install nobody has spoken to since Tuesday does not hold a subprocess forever,
which is the one cost a resident rung carries.

**The three thinking rungs are long-lived from creation.** A rung that reopens each time cannot
remember what it was in the middle of — and that is not something the ledger can hand back,
because the ledger records what is **owed**, not what has already been tried, ruled out, or
half-arranged. The failure this prevents is specific and was observed: a rung that arranged a
mechanism, forgot it had, woke to a ledger entry warning that the mechanism was fragile, and
deleted it as redundant. The ledger was correct at every step; the rung had no memory of its own
authorship.

**Reflection is the deliberate exception.** Its pass is self-contained — it sweeps, writes, and
is done — and its backoff can reach hours, so a resident session would only rot between passes.
Per-pass is not a lesser version of long-lived here; it is the right shape for work that has no
thread to keep.

**Losing a long-lived session is always survivable**, and that is what makes this safe rather
than a new dependency: every rung's state is *re-projected into every turn* — what is owed, what
it carries forward, who it can reach. A session that wedges is discarded and reopened cold; it
loses the thread, never the truth. This is why a session may break loudly and the system is fine.

## Recall

**Memory is one store, and every agent reading it reaches everything.** There is no
partition to cross — recall works the way it does for a person, by going and looking.

Discretion about what to repeat, and to whom, is handled the way everything else is: soft
guidance and judgment. It was never anything else. Even when conversations were partitioned,
memory underneath them was shared, so the partition bought a narrower thing than it appeared
to — a separate *session window*, not a separate mind. What it did buy is worth naming now
that it is gone: material from one exchange sits in the same window as the next, so the
agent's judgment about what to bring up is doing work that structure used to do partway.
That is the trade [`core.md`](core.md#one-conversation) took deliberately, and the place to
revisit it is when a second party genuinely exists.

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

[`core.md`](core.md) for the switchboard and glancing up ·
[`data.md`](data.md) for what they read and write ·
[`legacy/reaction-cognition-split.md`](legacy/reaction-cognition-split.md) for the three-tempo
version this supersedes.
