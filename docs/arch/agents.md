# Agents

## Goal

Be responsive and thorough at the same time, by refusing to make one layer do both.

Every agent here is the same thing — a general agent on one session. They differ only in
**system prompt** and **tool surface**. That uniformity is deliberate: a new role is a new
prompt, not new machinery.

## Decisions

| Decision | Reasoning |
|---|---|
| Fast means *no fetch*, not *no knowledge* | Reaction runs a capable model; it is fast because it cannot wait on anything |
| The reading is separate from the voice | Reaction can speak and show but not read, so *someone* must open the file and look at the photo — and the voice may not go deaf while that happens. That someone is Cognition |
| Cognition never grinds | It is on the conversation's path, so a turn it spends *doing* is a turn the person waits through. It reads and answers; anything with an artifact, a side effect or a long tail is a worker's |
| Cognition stays idle | Someone has to be awake when nobody is talking, and it must be free when they are |
| Only workers act | The moment there is an artifact or a side effect, that is a worker. A division of labour, not a security boundary — [tool surfaces](foundation.md#default-tool-surfaces) are sized for context, not to fence anyone out |
| Cognition never speaks | Single-voice coherence: it proposes, Reaction voices |
| The switchboard is the host | No agent↔agent link; all routing and timers are Rust |
| A worker belongs to the session that created it | Ownership is what makes delegation addressable at all — a report has exactly one place to go, and it is not "the conversation" by default |
| Work travels **up**, never sideways | A report goes to whoever asked for it, who decides what is worth passing further up. Nothing reaches the person except through Reaction |
| An id names a **session**, not a role | A role has many sessions over a run; a Cognition replaced after a failure is a second session of one role |
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
the host, in [`host.md`](host.md#reflex), which is why it has no section here.

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

**Tools: `hi_say`, `hi_show`, and `SendMessage`.** Its two expression channels are calls — the
voice included — plus the one verb that reaches another agent. **No reads, no fetches, no
working directory, and no built-ins at all**: it is fast because it *cannot* wait on
anything, not because it is small. Judging the edge of your own knowledge is a hard problem
and needs a capable model.

**It also owns the social layer** — the mouth, and the timing of anything unprompted.
That was once a separate host component; its duties belong to whatever speaks, and this
is what speaks. (A fourth duty, the presence gate, was retired rather than inherited —
see [Attachment](host.md#attachment).) See
[`host.md`](host.md#the-social-layer-lives-in-reaction-not-here).

Owning that timing includes the one deadline in this host that fires at a named minute:
`hi_say`'s `back_in` arms the [check-in](host.md#the-check-in--the-only-thing-that-fires-at-a-named-time)
that brings the voice back to keep a promise it made. It belongs here for the same
reason the rest of the social layer does — the rung that named the number is the rung
that owes the word.

> **Enforced, not merely instructed.** This is the one place where a tool surface is a hard
> limit rather than a division of labour: the whole argument for the rung — that it *cannot*
> wait — is worth nothing if it can quietly open a file. Restricting our own tool surface is
> not sufficient on its own; the session's underlying toolset has to be restricted too, or
> "cannot" means "was asked not to".
>
> **And it has to be checked against the wire, not against the config we sent.** Twice the
> restriction was written, believed, and never in force — a setting the agent accepts,
> ignores, and reports nowhere. Both times the symptom was not a shell command in the log;
> it was a *silent voice*, because a rung holding a shell behaves like a coding agent and
> writes its answer as prose. The tools a turn actually held are readable on the upstream
> request; that is the only place this claim can be settled.

**`hi_say` is the only way out, and the host has no second one.** Text the model types is
working-out; it reaches nobody, by design. So a turn that writes a reply and calls nothing
to say it has not been thwarted on its way to the person — it has produced silence, which is
a move this rung is allowed and often right to make. The host **notices and does not
intervene**: no voicing of what was typed, which would make the contract a suggestion, and
no second ask either. `hi_say` answers a call that was made — too long, and send it as a few
shorter ones. There is no ack for a call that wasn't, and a host-side retry standing in for
one buys a whole extra generation on the turn already going wrong. A silent turn is logged
as the fault it may be, server-side only, never a card in the UI.

**The answer to a voice that writes instead of speaks is upstream**: find what put the turn
in that register — a tool surface it should not have, a prompt that reads like a coding
brief — and take it away. Asking twice does not move a session that has already decided
what kind of agent it is.

Not blind, because its memory is **prepared**: the bundled prompt for its role, plus the
[generated one](data.md#memoryprompts) — what the conversation carries forward,
open tasks, the recent log tail — all in context before the first word. **Code injects that
every turn and caps it**, so it cannot grow with usage. Two hundred open tasks project as a
summary, not a list.

It does not write that memory; it has no file access to write anything with. Cognition
writes it. And because this is the one rung that cannot go and look, it is also
the measure for everything else: **projected = what Reaction must know without reading** —
[the test](data.md#what-earns-a-place).

#### Deliberation was retired into Cognition

> A fourth rung sat here: the conversation's
> own reading, one per conversation, handing work up to the brain. Its *reason* — Reaction
> can speak but not read — was real and is now Cognition's job. Its *scoping* was not: it
> was per-conversation because [scene](host.md#one-conversation) was, and when scene went
> there was one of it and one of Cognition, two singletons in a row on the same path. What
> it bought over the merge was a session guaranteed free for the conversation; what it cost
> was a hop, and every hop between rungs is a place substance gets lost or restated.
>
> The merge is safe only because of the rule it came with: **Cognition never grinds**
> (above). A brain that dispatches everything heavy stays as free as a rung reserved for
> the purpose, and answers in one hop instead of two.
>
> Two things moved rather than died, and both are load-bearing: opening what arrived (a ref
> is a path) and writing the conversation's brief. One thing had to be rebuilt — the
> must-relay framing of an answer the person is waiting for, which used to be structural on
> the report path and is now the host marking a hand-down as owed. See
> [the hand-down](#the-hand-down) below.

### Cognition — minutes and beyond

**The brain, and the conversation's reading.** One of it for the whole agent. The voice
hands the turn's request down to it, and it does its heavy lifting by **delegating** — owns
[Tasks](data.md#tasks), dispatches workers, reasons across everything in memory, and stays
idle so it is free the moment something arrives.

**Reading is done here; doing is handed out.** Opening the photo that just arrived, reading
a file, checking what a page says, working out what was actually meant — that is seconds of
work with a person waiting on it, and handing it to a worker would cost a whole round-trip
to learn something this rung could have read itself. Past that the line is hard: the moment
there is an artifact to produce, a side effect to cause, a shell to run, or a stretch long
enough that it would stop answering, it goes to a worker.

That rule is not tidiness, it is what makes one brain safe on the conversation's path. This
rung used to have [Deliberation](#deliberation-was-retired-into-cognition) in front of it
absorbing the fast reads, and the argument for keeping that rung was that a brain busy with
an errand leaves the person waiting. Staying free is now a duty rather than a rung.

It is the **only** thing that creates workers, and the **only** writer of the task ledger.
Both follow from the same idea: durable work is what it means for something to be real, and
deciding that is judgment, not bookkeeping the voice should do in passing.

**Dispatch is two verbs, not one: hand out and take back.** A worker can be *interrupted*
mid-turn (`hi_cancel_worker` → `turn/interrupt`), and only by the rung that created it. This is
not a convenience — without it there is no way to stop work at all, because everything else
that reaches a session is mail, and mail is read between turns. A stop delivered that way
arrives after the thing it was meant to stop, so a retraction could be acknowledged in words
and never take effect: the person meets the result of work they cancelled, which is worse
than never having been able to cancel, because they were told it stopped.

Interrupting is **not** killing. The turn unwinds as `interrupted`, the worker reports what
it had reached, and the session stays warm with its full context — so "no, do this instead"
is a cancel plus a message to the same id. The rung that owns the cancel is the rung that
owns the dispatch, for the same reason "one dispatcher" holds: a second party able to stop
another's work is a second dispatcher wearing different clothes. Reaction, which hears the
retraction first, passes it up and says only that it is being called off — it has no such
tool, and must not claim the stop it cannot perform.

It outlives any one exchange, which is what makes it the right home for everything that
outlives one:

> **After a restart, before any user input:** the glance-up fires → Cognition wakes → reads
> open tasks → runs each one's `verify` and believes the answer → checks what already landed
> so nothing is redone → does or re-arms what is still wanted → for the user-facing ones,
> messages Reaction, which voices it when the room is right.

**Who is on a task is projected with the task, and it is computed, never stored.** A worker
records the ledger subject it was created for (`CreateWorker(subject:)`), so each projected line
carries whether a session is working it, whether that session is mid-turn, and how long it has
been in that state. The join is derived from the switchboard on every turn and written down
nowhere: a facet field naming its worker would be a second copy of a fact the registry already
holds, free to disagree with it, and wrong by construction after a restart — still naming a
session that no longer exists.

That makes **`doing` with nobody on it** a fact the ledger reports rather than an inference
someone has to draw. It is the shape every unfinished task takes after a restart, a crash, an
idle-out, or a hand-off that never happened, and until it was projected it was indistinguishable
from work in hand. It is a question, not an alarm — put someone on it, or write down that it is
finished, blocked, or dropped — but it is not a state a task may sit in silently.

"Nobody on it" is said only where nobody is a problem, which is `doing`. An unattended `todo` is
what a `todo` is, and a `serving` duty is between handler bursts for most of its life, so
flagging those would put the phrase on most of the list and train the reader straight past it —
including on the one line where it means something. A live worker is reported wherever there is
one, since that is positive information and cannot be a false alarm.

**Judging a worker stuck stays Cognition's**, not a host watchdog's. The host supplies the facts
that make the judgment possible — busy or idle, what it was last seen doing, how long it has
been that way — and stops there. A timer that killed sessions on a threshold would be code
making the call `agents.md` gives to the rung, and it would be wrong about exactly the work worth
protecting: a build that legitimately runs for an hour looks identical to a wedge until someone
reads what it is doing.

This is the sequence, not a plan for one: the glance-up is a timer arm on Cognition's own
loop ([`host.md`](host.md#glancing-up)) — one wake shortly after
the process starts, then on the pulse cadence while anything is owed.

#### The hand-down

Answers travel back the way they came: what the voice handed down is answered to the voice.
Cognition's results arrive **unframed** — Reaction is what turns "the build failed" into
something that fits the room it is in.

**An answer the person is waiting for is a reply owed, not a proposal**, and that is the one
place the previous line inverts. Everything else Cognition sends is a proposal Reaction may
decide not to voice; an answer to a question asked thirty seconds ago is not, because a
voice entitled to drop it means the person who asked never hears back. The host is what
knows the difference — it posted the hand-down, so it marks the answer as owed when it
returns, and Reaction relays it in its own words rather than weighing whether to.

This was structural before the merge: Deliberation's answer came back on the report path,
which the host framed. It is still structural; only the path changed.

Cognition never calls `hi_say`. Everything it wants said is a **proposal** Reaction schedules.
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

**One organizer is the exception, and the exception is the point.** The `people` one is a
real [worker type](#workers) — `person-reader`, with a bundled prompt — because prose in
`reflection.md` is prose the settling pass re-authors into a task description every time it
dispatches, and this is the one job whose entire value is in the exact wording. It walks
every ask in the stretch; it reads the worker reports and timestamps rather than the agent's
own account when something didn't land, because that account is what the agent *believed*
and comes apart from what it did precisely when it matters; it searches for an existing rule
before writing a new one, since a second copy of an instruction that already failed grows
the store and changes nothing; and it keeps the [`## Working with them`](data.md#memory)
section the voice is handed on every turn. A guideline that careful must be versioned and
ours to tune, not improvised per pass.

So `people` is also the one dimension the settling pass does **not** write. It still names
and merges clusters — that is cluster work, not prose — and hands each named person present
in the stretch to a reader.

One red line: reflection may **archive verbatim and write pointers, never paraphrase stored
bytes**.

### Session lifetime, per rung

Specified here because it was previously specified nowhere, and two documents drifted apart in
the gap: `host.md` described how a long-lived session is kept bounded, while Cognition
was built to reopen per wake. Both were defensible readings. This is the decision.

| Rung | Session | Replaced when |
|---|---|---|
| **Reaction** | one, process-wide, long-lived | a turn fails |
| **Cognition** | one, process-wide, long-lived | a turn fails |
| **Reflection** | **one per pass** | never — the pass ends |
| Workers | one per errand | **its owner closes it** |

**Nothing in this column is about size.** Context growth is bounded by the underlying agent,
which compacts in place; see [`host.md`](host.md#session-layer) for why that is not ours to
do. A session is replaced here only because it **broke**.

**A worker's lifetime belongs to the rung that created it, and nothing reclaims one on a
clock.** This row used to read "the errand ends, or an idle TTL", and the TTL is now gone
outright rather than tuned. It could not answer the question it was being asked. A worker
that has reported and is waiting for its next instruction is indistinguishable, from the
outside, from a worker whose owner has forgotten it — the difference is *the owner's
intent*, which is knowable only to the owner. Fifteen minutes of quiet was never evidence
either way.

What made that concrete: on 2026-08-13 Cognition wedged in a sixteen-minute turn that died
on a vendor 502, and while it was stuck, five of its workers — three of them mid-deployment
— hit the timer and were reclaimed. Nothing had gone wrong with the work. The owner had
merely not spoken, at exactly the moment it *could* not speak.

So dispatch is **three verbs, not two**: start an errand, take back a turn, and finish with
the session. The third (`hi_close_worker`) is what the timer was standing in for, and it has a
caller who knows the answer. Cancelling and closing are deliberately different acts —
cancelling stops a turn and keeps the context for "no, do this instead"; closing ends the
session and lets the context go.

**The cost is stated, not hidden:** a worker its owner never closes lives until the process
does. That is a real leak and the honest place for it — an owner that loses track of its
errands has a problem no timer was fixing, only concealing.

**The two thinking rungs are long-lived from creation.** A rung that reopens each time cannot
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

### Across a restart

The column above reasons only *within* a run. A restart used to end every thread — threads were
opened `ephemeral`, so nothing was written and nothing could be resumed. That made the paragraph
above true only until the process died, and the failure it describes — a rung that arranged a
mechanism, forgot it had, and deleted it as redundant — recurred on every boot.

**Every thread is opened durable. That part has no per-rung policy.** A rollout costs nothing
until something resumes from it, and a rule is smaller than a table.

**Who is resumed at boot is a separate decision, and it is not everyone:**

| Rung | At boot |
|---|---|
| **Reaction**, **Cognition** | resumed from the previous run's thread |
| **Reflection** | never — a dead pass is re-driven by the frontier cursor, which already points where it stopped |
| Workers | **not** automatically; the thread is kept and offered, and Cognition decides per errand |

**Workers are kept, not resumed.** An errand's thread is full of tool calls whose effects already
landed, and forty minutes later most errands are stale — which is a judgment, not a rule code can
apply. So the session directory carries the dead worker's thread id, and the boot glance offers
it: Cognition reads "this errand died mid-flight, here is its mind" and picks the one worth
finishing. Resuming one is then the same act as resuming a rung. What this replaces is the
worker losing its context entirely and being recovered only if a task facet happened to exist.

**The offer is taken with `CreateWorker(resume:)`, and only ever from the offer.** The handle is
the thread id, and the host refuses one it did not just offer — a resume is the one argument a
caller cannot derive from the work in front of it, and an unchecked thread id would fall back to
a cold open that the caller believes carries context. Resuming yields a *new* session: new id,
new registration, same prompt. Only where its mind starts differs, because to everything
downstream — ownership, addressing, reporting — this is a session that began now.

**Only the previous run's errands are offered, so the offer ages out by itself.** The directory
is append-only and unpruned and nothing marks an offer as consumed, so any wider filter would
re-offer a three-week-old errand at every boot until it read as furniture. One restart is the
window.

**What the offer is against is not lost context — it is the silent drop.** An errand resumed and
an errand judged stale are equally fine outcomes; a task left `doing` with nobody on it is not,
because it is indistinguishable from a task being worked on — to the person, to the voice, and
to the rung that wrote it an hour later. So the offer names the alternative rather than leaving
it implied, and asks for the disposition in the ledger either way.

**A resumed thread is re-handed its prompt.** `baseInstructions` is passed on resume exactly as
on open, so a thread resumed by a newer binary runs that binary's prompt — the rungs' prompts are
reinstalled from the bundle every boot, and an upgrade is the most common reason to restart.
Without this the oldest threads would be the ones running the most stale instructions.

**A resume that fails is a cold open, and so is the turn after it.** Discarding a wedged session
is the existing rule; this extends it to the one new way a session can arrive broken. It is what
keeps "turn it off and on again" working: a thread poisoned badly enough to take the process down
does not get to take the next one down too.

Three facts about the wire this rests on, verified against the 0.147 pin rather than its docs:
`thread/resume` accepts fresh `baseInstructions`; a resumed thread appends to its original
rollout, so a thread id is stable for the life of the thread however many processes have hosted
it; and a thread that never took a turn has no rollout at all, so resume answering *no rollout
found* is an ordinary boot outcome and not an error worth surfacing.

**The thread id is recorded, never derived.** `thread/start` takes no path, so where a rollout
lands is codex's to choose — `(run, session)` cannot address it. The id comes back from
`thread/start` and is written to the session directory, which is also what makes a dead worker's
thread addressable at all, for the same reason its frames already are.

## Recall

**Memory is one store, and every agent reading it reaches everything.** There is no
partition to cross — recall works the way it does for a person, by going and looking.

Discretion about what to repeat, and to whom, is handled the way everything else is: soft
guidance and judgment. It was never anything else. Even when conversations were partitioned,
memory underneath them was shared, so the partition bought a narrower thing than it appeared
to — a separate *session window*, not a separate mind. What it did buy is worth naming now
that it is gone: material from one exchange sits in the same window as the next, so the
agent's judgment about what to bring up is doing work that structure used to do partway.
That is the trade [`host.md`](host.md#one-conversation) took deliberately, and the place to
revisit it is when a second party genuinely exists.

## Workers

**Where the actual jobs get done.** Everything above mostly thinks — not because it is
forbidden to act, but because a rung that does the job is a rung that stopped being fast.

A worker's **type** is the `type` in [`CreateWorker(type)`](foundation.md#the-agent-session-registry),
and it selects a prompt and nothing else — same session, same tools. Adding a kind is
adding a `.md`.

**A type is a role, not a field beside one.** The four rungs and the five types are one
namespace of nine, because they are one concept: [the opening of this
document](#goal) says every agent differs only in prompt and tool surface, and a type
differs in exactly the first of those. So the type travels with the session wherever its
role does — which is what lets the switchboard say a live session is a *view reviewer*
rather than an anonymous worker, and `GET /api/workers` report it.

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

[`host.md`](host.md) for the switchboard and glancing up ·
[`data.md`](data.md) for what they read and write ·
[`legacy/reaction-cognition-split.md`](legacy/reaction-cognition-split.md) for the three-tempo
version this supersedes.
