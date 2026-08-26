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
| The reading is separate from Reaction | Reaction can speak and show but not read, so *someone* must open the file and look at the photo — and Reaction may not go deaf while that happens. That someone is Cognition |
| Cognition never grinds | It is on the conversation's path, so a turn it spends *doing* is a turn the person waits through. It reads and answers; anything with an artifact, a side effect or a long tail is a worker's |
| Cognition stays idle | Someone has to be awake when nobody is talking, and it must be free when they are |
| Only workers act | The moment there is an artifact or a side effect, that is a worker. A division of labour, not a security boundary — [tool surfaces](foundation.md#default-tool-surfaces) are sized for context, not to fence anyone out |
| Cognition never speaks | Single-voice coherence: it proposes, Reaction speaks |
| The switchboard is the host | No agent↔agent link; all routing and timers are Rust |
| A worker belongs to the session that created it | Ownership is what makes delegation addressable at all — a report has exactly one place to go, and it is not "the conversation" by default |
| Work travels **up**, never sideways | A report goes to whoever asked for it, who decides what is worth passing further up. Nothing reaches the person except through Reaction |
| An id names a **session**, not a role | A role has many sessions over a run; a Cognition replaced after a failure is a second session of one role |
| **One verb between agents** | `SendMessage(to, message)` — one direction, no reply, queued. Every other shape we tried (delegate, ask, surface, handoff, notify) was this verb wearing a name that described one use of it |
| A worker replies; it does not narrate | It may message **only its owner**, and only in answer. Structural on the address, guidance on the timing |
| **A worker keeps a current best, always showable** | The layer split only pays off if the thorough layer has something to hand up: a job that builds privately and assembles at the end leaves Reaction one answer to "show me what you have", and it is "not yet". So a deliverable is built *in place* — one artifact advanced as the work lands, with the unconfirmed parts marked in it — rather than assembled at the end. Unconditional: nothing here reads whether anyone is watching, which is the fact [`host.md`](host.md#attachment) retired the presence gate for not being derivable. If nobody asks it costs a file the worker was keeping anyway |
| Cognition may **create** a ledger row; only a [Task Manager](#task-manager) may **change** one | Opening is witnessed in the conversation and must be instant — a promise waiting on a worker is one a restart eats. Closing is a claim about the world, and whoever handed the work out is the worst-placed agent to make it. Disjoint transitions, so the two can never contradict each other |
| **A task's folder is shared, and only `facet.md` is disposable in it** | Facet prose is a projection reflection re-derives, so a racing write on it costs nothing ([`facets.rs`](../../src/mind/memory/facets.rs)). Its siblings — the deliverable, the working notes — have no episodes behind them, and living in the same directory made that reasoning look general. Two sessions collided on one path; the survivor's file was then read by the loser as its own, and what decided the outcome was the write verb — `apply_patch`'s context check versus a heredoc's silence. One worker per task is also the most that can be *arranged*, not one writer, since a worker may fan out sub-agents that write where it writes. Held as guidance, deliberately: a gate on a field an agent must remember to fill is **silently** absent when it forgets, which is why `deliverable: <ref>` was rejected |
| **One task body, four writers, disjoint parts** | The body's `## Timeline` is a dated, append-only record and each writer owns its own kinds. Cognition writes `created` — why the row exists, in the person's words, at open, because it was in the conversation and nobody downstream will be. The worker writes `update` / `delivered` / `waiting` as it goes, because that record is what the person's task panel renders, and a report reaches one session. The Task Manager writes the closing `update` line and the status word, carrying every other line forward. **The store writes `moved` itself**, on the pass that already stamps the clocks — a consequence of a decision already recorded is not a mind's to remember, so the spine of the history exists whether or not any agent cooperates. The acceptance line is a **reading, not a gate**: nothing waits on it, and no task is held open against a standard the agent invented for itself — that is how a delivered job sat open for four days |
| **A gap in the request is work, not a question** | Every rung, not just the Decision Maker. Waiting on the user is the worst outcome available, so an unknown gets the most defensible reading, stated out loud; asking is the fallback for when no reading is defensible *and* the gap gates the work |

### Ownership and addressing

Every agent session has a process-wide **slug** — one namespace for every rung and every
worker, so any session can name any other without ambiguity.

It is minted here rather than taken from the agent protocol, and that is forced rather than
chosen: the tool surface identifies its caller by a header set when the session opens, so the
identifier must exist before the protocol assigns one. **The slug is not the codex thread
id**, and the two are worth keeping apart in every sentence that touches a restart: the slug
is an address, minted per run and reused freely; the thread is the mind, and the only handle
`thread/resume` accepts. A slug that reappears after a restart says nothing about whether the
session behind it remembers anything.

A worker records the session that created it. Its report is delivered to that session,
which reads it on its next prompt. If the owner has shut down, the report falls back to
Reaction rather than being dropped — **surfacing finished work one rung too high beats
losing it.**

Two things follow, and both are load-bearing:

- **An agent that owns live children is not idle.** Idle-reaping an owner out from under
  running work is what creates orphans; the fix is to not call it idle. Shutdown is
  graceful: finish or hand off, then close.
- **A session slug addresses a live agent; a task subject addresses work.** A worker's session
  dies with the process, so nothing durable may reference one — and its slug repeating after a
  restart makes that sharper, not safer, because the same string then resolves to a different
  session serving the same task. Recovery reconstructs from [Tasks](data.md#tasks), never from
  a session. The three rungs are the exception the singleton earns: `cognition` names cognition
  in any boot, because the host reopens exactly one of it.

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

**Tools: `hi_say`, `hi_show`, and `SendMessage`.** Its two expression channels are calls — speech
included — plus the one verb that reaches another agent. **No reads, no fetches, no
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
that brings Reaction back to keep a promise it made. It belongs here for the same
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
> it was a stretch of turns that stopped calling `hi_say` at all, because a rung holding a
> shell behaves like a coding agent and writes its answer as prose. The tools a turn
> actually held are readable on the upstream request; that is the only place this claim can
> be settled.

**`hi_say` is the only way out, and the host has no second one.** Text the model types is
working-out; it reaches nobody, by design. So a turn that writes prose and calls nothing to
say it has not been thwarted on its way to the person — it has produced silence, which is a
move this rung is allowed and often right to make. **This is the ordinary case, not a fault
state, and it has no name of its own** — there is no "silent Reaction", no "mute" condition to
detect. Requiring an explicit `hi_say` is the whole design: it is what lets the rung think
in the open without narrating itself at the person.

So the host **observes and does not intervene**: no voicing of what was typed, which would
make the contract a suggestion, and no second ask either. `hi_say` answers a call that was
made — too long, and send it as a few shorter ones. There is no ack for a call that wasn't,
and a host-side retry standing in for one buys a whole extra generation for nothing. **Nor
is a turn that typed without saying logged as a failure**: the turn-done line carries
`typed_chars` as a size, and that is all it means. Logging it as an error taught readers —
people and agents both — to hunt for a defect in a working design.

**When it is genuinely wrong, it is wrong in bulk and the answer is upstream**: a rung that
was calling `hi_say` and stops for a whole stretch has been put in the wrong register by
something — a tool surface it should not have, a prompt that reads like a coding brief — and
that is what to take away. One quiet turn says nothing at all; only the pattern does. Asking
twice does not move a session that has already decided what kind of agent it is.

Not blind, because its memory is **prepared**: the bundled prompt for its role, plus the
[generated seed](data.md#prompts) — what the conversation carries forward,
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
> is a path) and writing the conversation's brief. One thing was rebuilt twice — the
> must-relay framing of an answer the person is waiting for, which used to be structural on
> the report path, then became a host-held flag, and is now standing guidance Reaction
> applies to the message in front of it. See [the hand-down](#the-hand-down) below.

### Cognition — minutes and beyond

**The brain, and the conversation's reading.** One of it for the whole agent. Reaction
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

It is the **only** thing that creates workers. Durable work is what it means for something to
be real, and deciding to take some on is judgment, not bookkeeping Reaction should do in
passing.

**It opens duties; it no longer closes them.** Writing the ledger was all Cognition's for the
same reason creating workers is, and it turned out to be two acts rather than one. Opening
stays: the ask happened in the conversation this rung was in, and it has to be filed in that
same turn or it is a promise a restart eats. Ruling that a duty *ended* is a claim about the
world, and the rung that handed the work out is the worst-placed one to make it about its own
errand. So it may create a row and may never change one — closing, reopening and standing a
duty down all go to a [Task Manager](#task-manager). Cognition keeps the noticing and hands
the looking down.

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
> messages Reaction, which speaks it when the room is right.

**Who is on a task is projected with the task, and it is computed, never stored.** A worker
records the ledger subject it was created for (`CreateWorker(subject:)`), so each projected line
carries whether a session is working it, whether that session is mid-turn, and how long it has
been in that state. The join is derived from the switchboard on every turn and written down
nowhere: a facet field naming its worker would be a second copy of a fact the registry already
holds, free to disagree with it, and wrong by construction after a restart — still naming a
session that no longer exists.

**A worker names the task it serves, and the call is refused without one.** `subject` was
optional, and an optional field is the one a dispatcher busy with the errand itself skips — the
worker runs, the join is never made, and the task line reads *nobody on it* for as long as the
work takes, which is the line that gets a second worker started on a folder the first is
already writing into. So it is required wherever the kind serves the ledger, the same fence
`title` has and for a stronger reason: a poor title costs a roster row its readability, a
missing subject costs the ledger its only account of who is doing what.

**The subject must name a row that already exists, and nothing mechanical opens one.** One
check refuses two different mistakes. A subject nothing is filed under leaves the roster reading
`on task fix-the-login` against a ledger holding no such row — tracked to look at, untracked in
fact, which is worse than the blank it replaced. And opening the row *for* the dispatcher, which
is the obvious way to make a required field always satisfiable, fills the list with rows nobody
decided to owe: a review filed as a task of its own, a second row for work already tracked under
another name, a refresh nobody would ever have written down. **The ledger is the list of what is
owed, and a list that fills itself is one nobody reads.** So a row is opened by a mind, with the
shell it already has, and the dispatch only ever joins one.

**A miss answers with the ledger.** A refusal that says only *no* is the one that gets answered
by coining a near-duplicate, so the open rows come back with it and the dispatcher picks. That
is where most of the joining actually happens: the reviewer sees the builder's row and names it
instead of opening a sibling beside it.

The cost is a round trip the first time work is genuinely new — write the facet, then create the
worker. It is paid once per task and it buys the property that makes the list worth reading:
every line on it is something somebody decided was owed.

**Two kinds serve no single task and take none.** The [Task Manager](#task-manager) serves every
row, and a `person-reader` organizer's subject is a person. Passing one is refused rather than
quietly dropped: a `people/` name accepted there would open a task named after a human being.

That makes **`doing` with nobody on it** a fact the ledger reports rather than an inference
someone has to draw. It is the shape every unfinished task takes after a restart, a crash, an
idle-out, or a hand-off that never happened, and until it was projected it was indistinguishable
from work in hand. It is a question, not an alarm — put someone on it, or write down that it is
finished, blocked, or dropped — but it is not a state a task may sit in silently.

**The minute after a boot is the one time that sentence needs its cause attached.** The
switchboard is empty by construction at boot, so every `doing` task reads "nobody on it" at
once. True — and the reading it invites, that the work was dropped, is wrong: the process died
holding those errands, and their sessions are being reopened. That window used to be measured
in Cognition's first turn — around eighty seconds, long enough for Reaction to report five
running tasks as merely open — and it is now the length of the resume itself, which needs no
model turn at all. A subject whose worker is still coming back reads "nobody on it — the
restart cut its worker off and it is being reopened", and the line goes back to the bare phrase
the moment that session registers, which by then means what it says.

**An errand that could not be reopened keeps the cause permanently.** A resume that fails is the
one case where the restart really did take the worker, and the task's line says exactly that
rather than decaying into an ordinary-looking "nobody on it" — which is indistinguishable from
work in hand, to the person, to Reaction, and to the rung that wrote it an hour later. Dropping
an errand is a fine outcome; dropping one silently is the bug this exists against.

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

**What goes down is the request, not Reaction's reading of it.** Reaction holds the
conversation and can look nothing up; Cognition holds the records and never heard the person
speak. So a name in the person's words — a ticket, a file, a person, "the second one" —
crosses as *their* words, and what Reaction believes it refers to crosses beside it, marked
as a belief. A resolution written in place of the name is indistinguishable downstream from
a name the person actually gave, so nothing ever verifies it: observed live on 2026-08-24, a
bare "056" went down already bound to a commit, and the rung holding the ledger read only
what that binding pointed at — correct work, for two ledger rows, on the wrong one.

Answers travel back the way they came: what Reaction handed down is answered to Reaction.
Cognition's results arrive **unframed** — Reaction is what turns "the build failed" into
something that fits the room it is in.

**What comes back is substance, not the agent's own housekeeping.** A gate verdict, a retry,
a contrast ratio, a check about to run — none of that crosses either. Reaction cannot tell an
internal step from a finding, and is told that an answer which arrived is owed; so anything
handed up wearing a finding's clothes gets spoken.

**An answer the person is waiting for is a reply owed, not a proposal**, and that is the one
place the previous line inverts. Everything else Cognition sends is a proposal Reaction may
decide not to say; an answer to a question asked thirty seconds ago is not, because a
rung entitled to drop it means the person who asked never hears back.

**Reaction is what knows the difference, and the host does not mark it.** This corrects the
design as first written, which gave the host the job on the grounds that it posted the
hand-down and therefore knew a reply was outstanding. It knows less than that sentence
implies: it knows *a* hand-down went out, never whether *this* message answers it. Reaction
knows both halves — the request is in its own long-lived session, the message is in front of
it — so the rule is standing guidance in `reaction.md` rather than a flag computed per
delivery. Mail arrives as what was written, and reading it is the reader's job.

**One message carries several things**, and the rule is therefore per item, not per
delivery — which is the other reason the flag could not have worked: a boolean is one
answer for a whole batch. Three findings in one message is ordinary, and the item most
easily dropped is the one furthest from what the room is arguing about that minute. That
is [gaps #29](../user-journeys/gaps.md), layer 3, observed live: a message carried three
things and the third — a finished view nobody had been shown — went unsaid for 29 minutes.

This was structural before the merge: Deliberation's answer came back on the report path,
which the host framed. **It is no longer structural, and it was never enforceable.** The
implementation carried a host-held `owed` boolean for one release; it set the flag on every
turn a person spoke into, handed it to whichever message from Cognition arrived first, and
spent it there — so a background finding could be announced as the awaited answer while the
answer behind it arrived bare. What the flag bought at its best was one extra sentence in a
prompt. That is guidance either way, so it belongs where the rest of the guidance lives,
applied to the message rather than to a boolean about an earlier turn. A rule the model
applies by reading is not a weaker version of a rule code enforces — here, code could not
enforce anything, and the flag's only real effect was to make it look as though something did.

Cognition never calls `hi_say`. Everything it wants said is a **proposal** Reaction schedules.
Two gates keep this human-shaped: Cognition asks *"is this worth raising?"*, Reaction asks
*"is now the moment?"* The thinking part decides it matters; the social part decides the
timing.

**"Proposal" names who controls delivery, not whether it is delivered.** Both readings ride
on the one word and only the first is always true: Cognition never picks the moment or the
words, and never learns whether the thing landed — which is why the ledger cannot be closed
on the strength of having sent something ([data.md](data.md#tasks)). Reaction's gate is real
but it is a gate on *timing and phrasing*; on an answer someone asked for it does not extend
to "not at all". So the two questions stay as written, and Reaction's *"is now the moment?"*
has exactly one answer it may not give to an awaited reply: never.

#### Working ahead

**The reversible half of the next step is done before anyone asks for it.** Every wake this
rung has is about something that already happened — mail, a worker's report, a pulse into
idleness. The moment that carries the most information about what comes *next* is the one
where it is handing something over, and none of that moment was ever spent on it. So the
person pays twice: once waiting for the handover, and again waiting for the obvious thing
that follows it.

Two shapes, and the cheaper one is not machinery at all.

**The handover carries the questions it provokes.** If the next thing a reasonable person
says is a question this rung could already have answered, the handover went out too early.
A readiness board that lists six items and not where each of them stands is not a board
that needs a follow-up question; it is a board that was handed over half-written. This
costs nothing — same turn, same information, already in hand.

**The free half of the likely next step is handed out in the same turn.** The report comes
back as mail that wakes this rung, so the preparation can be running while the person is
still deciding whether they want it. One handout per handover is the posture — the *likely*
next step, not a fan of every step imaginable — and prepared work does not itself prepare: a
session started for work nobody asked for may not start another.

**Ordered after the answer, because a spawn is not free.** `CreateWorker` does not wait for
the work, but it does wait for the session, and a session is a whole `codex app-server`
process of its own — bounded at ten seconds here, observed at three minutes under load.
Opening the speculative errand *before* handing the answer back puts a process launch in
front of the thing the person is waiting on, and does it on every handover including the
ones whose next step is never wanted. That is the mechanism this section exists to remove,
reintroduced one layer up. So the order is load-bearing: answer, then prepare.

**And an unwanted preparation is closed, not abandoned.** Nothing reclaims a working session
on a timer, by decision — so a preparation that misses holds a process until it is closed,
and it is the one kind of errand with nobody waiting to notice that it wasn't. The cost
model only works if the misses are swept: getting ahead is affordable because most of it is
discarded, and discarded has to mean *closed*.

**The boundary is the one that already governs acting alone.** Reversible and invisible →
do it now. One-way, or visible to anyone else → carry it to the door and stop there.
Rendering the picture is reversible and invisible; sending it to a colleague is neither.
Working out which chat, which credential, and proving the command runs changes nothing
outside this machine; the message it would send does. The rule that decides what may be
done without being asked is the same rule that decides what may be done *before* being
asked, and being a step ahead is never a reason to relax it. There is no second safety
concept here, and there must not be one — a separate rule for speculative work is a second
place for the answer to drift.

**Prepared work never takes the floor.** It is not announced, it does not ask, and when it
turns out to be unwanted it is dropped without a word. The single thing that may be said is
a clause inside a handover that was being spoken anyway — naming the follow-through, so a
*yes* is the entire trigger and nobody spends a round trip asking for something already
sitting ready. The words are [Reaction's](#reaction--one-generation), as all words are;
what this rung owes the proposal is the *fact* that the next step is ready, so Reaction has
something true to offer.

**It is a cache, never a claim.** Something prepared twenty minutes ago describes the world
of twenty minutes ago. It is re-checked at the moment of use and degrades to doing the work
then — which is exactly the behaviour there was before, so the worst case of working ahead
is the old speed, not a wrong answer. What must never happen is a prepared thing *reported*
as current: that is the same failure as a `checked_at:` stamped after a probe that came back
down, and it is worse than having prepared nothing.

**What it costs is measured, not assumed.** Most of this work is discarded by design — that
is the trade, and it is only a good one while the ratio is known. Two readers, deliberately
different:

- **The count is for the person tuning it.** The host marks the sessions started for work
  nobody had asked for yet, so how much of the machine is being spent one step ahead is a
  number in the [observatory](foundation.md#observatory) rather than an impression.
- **The judgement is for the agent doing it.** Where working ahead surfaces — something
  prepared is *offered*, and the person either takes it or moves elsewhere — that is an
  exchange in the conversation, which is what the [reflection](#reflection--background) pass
  reads. So it is kept in the same standing read that already learns what the agent's words
  earn (`proactivity.md`), whose subject widens a second time: from what its words earned to
  what its unasked *work* earned.

  **The half that pass cannot see is the honest limit here.** Work prepared, never offered
  and never wanted leaves no mark in the conversation at all — no one declined it, because
  no one was told. That is precisely the waste worth knowing about, and nothing in the
  transcript can report it; only the count can. Two readers is not redundancy, then: each
  sees a half the other is blind to.

Nothing here is a scheduler, a queue, or a store of prepared things. It is one rung
spending part of a turn it was already awake for on the step after the one it was asked
for, under the boundary it already had.

### Reflection — background

**The inward brain, and the same kind of thing as Cognition.** Both are as capable as the
agent gets, both dispatch workers, neither speaks. What separates them is not intelligence
and not machinery — it is **who the work is for**:

| | Work arrives from | Answers to | Owns |
|---|---|---|---|
| **Cognition** | a person, through the conversation | the conversation | what work exists — it opens duties and staffs them |
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
loop is a second Reaction. What it wants said it sends to Cognition or to Reaction, and
Reaction chooses the moment.

**It does dispatch**, and that too was once forbidden here on the reasoning that it would
make a second dispatcher. The reasoning does not survive ownership: a worker belongs to
the session that created it, so Reflection's workers are Reflection's, report to
Reflection, and never surface in the conversation. What must stay singular is the *mouth*
and the *task ledger* — not the act of asking for help.

**It works across the whole store**: it merges a person seen on two occasions, dedupes
skills, hands drive housekeeping to a [drive organizer](#drive-organizer).

Its workers come in two kinds:

- **Per-store organizers** — people, episodes, facets, views, skills, tools, the drive.
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
section Reaction is handed on every turn. A guideline that careful must be versioned and
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

**What the host owes in return: an id it hands out must already answer.** Putting the
lifetime in the owner's hands only works if every verb the owner has about a session tells it
the truth, and the three dispatch verbs are all *about a session* — so all three wait for the
loop that owns it, and none of them reports from the send. This is not a latency preference.
`hi_create_worker` used to answer as soon as the request was queued, so between the reply and
the registration the errand had an id and no session, and every question asked with that id —
status, message, cancel, close — was answered confidently in the negative. Observed
2026-08-17: one reflection pass created a worker, was told three times over three minutes that
no such session existed, did the work itself, and closed a session that had not been created
yet; the close reported *"already gone"*, the worker spawned eleven seconds later, and became
permanently unclosable — its owner had been told it was gone, so it would never ask again. An
owner cannot be held responsible for a lifetime it was lied to about. **A "nothing there" must
mean the session has ended, never that it has not begun.**

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
| Workers | every one **the host ended** — reopened on its own thread, under its own slug, with its inbox — because the host ending a session is not the owner finishing with it |

**A session the host ended comes back; a session its owner ended does not.** That is the whole
rule, and it falls out of the lifetime a worker already has: a working session lives until its
owner calls `CloseWorker` — no timer, no idle-out, nothing else ends one. So a worker still on
the switchboard when the process stopped is a worker its owner had not finished with, and the
host closing it on the way down is the host overriding that. Reopening it puts the decision back
where it lives. A worker the owner *had* closed stays closed, because that was a decision, not
an interruption.

**Which means "was it working?" is the wrong question, and it was the first one asked here.** An
earlier draft reopened only the errands caught *mid-turn* — the reasoning being that a worker
which had reported has nothing left to finish, so reopening it spends a subprocess on nobody.
True about the *work* and false about the *session*: what a warm idle worker holds is not an
unfinished task, it is a place its owner can send the next instruction to, with everything it
already learned still in it. Dropping twelve of those (the count from one real stop, out of
sixteen) does not lose work — it loses every follow-up that would have been a sentence, and
makes each one a fresh session with a brief written from memory. Mid-turn still matters, but for
what the session is *handed*, not for whether it exists.

**The host does this and Cognition is not asked, because the judgment needs the work.** "Is this
half-done state still worth finishing" is a question about what already landed: whether the tool
call went out, whether the file was written, whether the deploy took. The session that was doing
it holds every one of those in its own thread. Cognition holds a title, a subject and a timestamp.
Handing the decision to the party with less information — while the party with more sits dead on
disk, one `thread/resume` away — is what the *offer* did, and what this replaces. The offer was a
list of dead errands appended to the boot glance, from which Cognition picked with
`CreateWorker(resume:)`; it is deleted, along with the take-once bookkeeping that kept two callers
from claiming one mind, because nothing claims a mind any more.

**The slug survives, and that is what a slug is for.** A resumed errand keeps the address it had:
same roster row, same ledger subject, same owner, same name in another rung's `reachable` line.
Nothing downstream has to be told a restart happened. This inverts the rule it replaces — a
claimed offer minted a *new* session, reasoning that to everything downstream the errand began
now. That followed from the offer's own shape, not from anything true about the errand: a list of
dead threads had no way to hand the old address back. The host reopening the session in place
does, and the errand is the same errand.

**What a reopened session is handed depends on what it was doing, and one of the two answers is
nothing at all.**

- **Caught mid-turn** — a turn started and never finished. The host has no brief to give it
  (Cognition wrote the first one and there is nobody to write a second), so the opening prompt
  carries the fact and stops: the host restarted, this much time has passed, and its own last
  actions may or may not have taken effect — establish what actually landed before doing anything
  further, and say so if the work is now stale. Judging it stale is a fine outcome. Redoing
  something that already happened is not, and neither is dropping it without looking.
- **Idle, waiting on its owner** — it is handed **no prompt**, and goes straight back to waiting
  for mail. It was not doing anything; a "the host restarted" turn would be a model turn spent
  on nothing, and the one thing worse than that is a session inventing work to justify having
  been woken. Its owner does not have to know it went away.

**Its inbox comes back with it.** Mail delivered and not yet read is *not* lost to the stop — it
is restored to the reopened session ahead of anything else, because the sender was told
`Delivered` and a mailbox that quietly drops what it accepted is worse than one that refuses.
What rides in front of restored mail is a single host line saying how long the gap was, so an
instruction written before the restart is not read as though it arrived just now.

**A task held for energy is held where the restart can see it.** A worker whose turn hits a
402 does not fail — the drive loop keeps that exact task and reruns it when the balance says
`Resume`. Held in a local, that task is invisible to everything: the session reads as idle, so
a stop would reopen it with no prompt and no mail, and it would wait forever for an instruction
it was already holding. So the hold is recorded on the switchboard, and a reopened session is
re-handed it.

**A resume that cannot happen must not arrive looking like one.** `thread/resume` fails for
ordinary reasons — the rollout was pruned, `CODEX_HOME` moved, the thread never took a turn and so
has no rollout at all. A **rung** falls back to a cold open, because a rung's worth does not depend
on its memory. An **errand must not**: a cold session handed "check what landed before continuing"
does not know what it was doing, and every party downstream would believe a context exists that
does not — which is the same failure as a confabulated thread id, arriving by a different door. So
a failed resume means the errand does not come back at all, and its owner is told so, carrying
what the directory still knows: title, subject, when it started. The ledger row is then a task
with nobody on it *and a reason attached*, which is something a rung can act on.

**Only the previous run's errands, so this ages out by itself.** The directory is append-only and
unpruned, so any wider window would reopen a three-week-old errand at every boot. One restart is
the window, and an errand that survives a second stop without being finished is one nobody is
finishing.

**A resumed thread is re-handed its prompt.** `baseInstructions` is passed on resume exactly as
on open, so a thread resumed by a newer binary runs that binary's prompt — the rungs' prompts are
reinstalled from the bundle every boot, and an upgrade is the most common reason to restart.
Without this the oldest threads would be the ones running the most stale instructions.

**A rung whose resume fails opens cold, and so does the turn after it.** Discarding a wedged
session is the existing rule; this extends it to the one new way a session can arrive broken. It
is what keeps "turn it off and on again" working: a thread poisoned badly enough to take the
process down does not get to take the next one down too. An **errand** is the exception, and for
the reason above — a cold open it cannot tell apart from a resume is worse for it than not coming
back.

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

**A type is a role, not a field beside one.** The three rungs and the seven types are one
namespace of ten, because they are one concept: [the opening of this
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
| Drive Organizer | knows how `drive/` is laid out — puts a new thing where the drive is already going, says where an existing one is, straightens a corner that has drifted |
| Person Reader | reads one person out of the record and folds it into their facet, including the `## Working with them` Reaction is projected |
| Task Manager | keeps [the ledger](data.md#tasks) — the only thing that may **change** a task's `status`, so closing, reopening and standing a duty down are all its. It files; it never delivers — below |

Workers are **volatile**: they live in process memory and die with it. Nothing durable may
live only inside one. Recovery is therefore **reconstruction from Tasks, never continuation**
— we do not checkpoint execution state.

They are **capability peers**, not children: a worker reaches the same memory, skills and
tools. The one asymmetry is channels — a worker cannot speak or show. It produces an
intent; Reaction articulates it.

**A worker fans out with sub-agents, not with `CreateWorker`.** It may spin up as many
sub-agents as the job wants, using its harness's own facility for that — and they are
**invisible here**: no session slug, no address, no registry entry, no report of their own.
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

### Drive Organizer

**Ordinary drive content is everyone's to read and write.** [`drive/`](data.md#drive) is a directory on
the same disk as everything else, and an agent that knows what it is putting down and where
it goes puts it down. There is no gatekeeper and no write path that must be asked for —
routing every save through one session would make the filing cabinet slower to reach than
the memory beside it, and buy no tidiness for the cost. Managed secret files under
`drive/accounts/secrets/` are ordinary local files whose paths must remain stable.

What is scarce is not the access, it is **knowing where**. The layout is a judgment that
accreted; it cannot be derived from a listing. So this worker is the one that holds it, and
it is reached for in the three cases where the answer is not already obvious to whoever
asked:

- **Put this somewhere** — a thing has arrived and nobody knows where it belongs: a file
  handed over, or an account note whose
  [secret reference](data.md#keys-passwords-and-the-one-question) needs a useful home. The
  key is already represented by its stable drive-file path.
- **Where is this** — something is in there and the caller cannot find it, or cannot tell
  which of two candidates is the one meant.
- **Straighten this** — a corner has drifted: a file in the wrong folder, two folders that
  mean the same thing, a name nobody could search for months from now.

Reached **through the owner**, like the [decision maker](#decision-maker) and for the same
structural reason: a worker holds no `CreateWorker`, so one that does not know where
something goes says so to whoever created it and keeps moving meanwhile. Reflection's drive
housekeeping is a **dispatch to one of these**, not work it does inside the pass — the same
shape as its [`person-reader`](#workers) organizers, and for the same reason: a pass that
does the tidying inline pays the whole layout's context on every wake.

**It matches the drive; it does not redesign it.** A new folder only when nothing fits,
named the way the existing ones are named. An errand that quietly reorganizes everything is
how a person loses track of their own things.

**The one rule that cannot bend: a drive path can be the address inside a facet.** Move or
rename a file and every claim pointing at the old path is fixed in the same pass, or the
tidy has left memory aimed at nothing — worse than the mess it cleaned.

**A handed-over file is copied, never moved.** [`surfaces.md`](surfaces.md#bulk) carries
the reasoning at length: the log's copy fades and the drive's is permanent, and moving the
bytes degrades the journal's own reference to a caption.

### Task Manager

**Ruling that a duty ended is a job, so it gets a worker.** It was Cognition's, bundled in with
dispatching — and bundling those two puts the ruling on whether work landed in the hands of
whoever handed the work out. Opening stayed behind, because that half *is* the dispatcher's and
has to be instant; what moved is every transition after it. That is the loop [Tasks](data.md#tasks) records failing in the
open. A manager is not the more honest agent; it is the differently placed one. It did not do
the work, so *is this finished* is a question it can only answer by going and looking.

**It files; it neither delivers nor dispatches.** A task it finds needing work is *reported* —
the manager is a worker, so `CreateWorker` is not its to call and **one dispatcher survives
intact**; it answers its owner and Cognition staffs what it names. Its own output is a filed
ledger and nothing else. The moment it starts finishing things itself it is back inside the
loop it exists to break, and it is a worse executor than a worker briefed on the one job.

**Woken by the glance-up, not by a clock of its own.** Cognition keeps the noticing — a
`(pulse)` is still where "the ledger is worth a look" is decided — and hands the looking down.
What moves is the filing, never the trigger.

**Volatile, and safely so.** It holds nothing: the ledger is on disk, and the pass that stamps
what a status change implies is idempotent. A restart mid-file leaves a ledger part-updated and
nobody on it, which is a state the next glance-up reads and continues — the ordinary shape of
[recovery](#across-a-restart), not a special case.

**One promise, one row, and folding a duplicate is its.** Two rows for one job is a fault in
the list itself: it double-counts a promise, splits that work's record across two folders, and
invites two workers onto one job. Nothing else can see it — the fault is visible only from the
whole list at once, which is exactly what this worker reads and no rung does. So the manager
folds them: carry everything the second row says into the survivor, then close it `cancelled`
with a line naming where the promise went. **This is not the pruning it is forbidden**, and the
difference is the whole of it — pruning ends a promise, a fold moves one, and a row that is
merely stale is neither. The test is delivery, not resemblance: two rows are one job when
delivering either delivers the other.

**It names no subject, and is one of only two kinds that may not.** `CreateWorker(subject)`
binds a worker to a single ledger task and is required of every kind that serves the ledger;
a worker without one reads as *not linked to any task* — the line that means **nobody is on
this, staff it**. A manager serves every task, so it can never name one, and left inside that
rule it would trip the alarm on itself at every glance-up. So it is subjectless by construction,
a subject passed to one is refused, and the reachable list says *serves the whole ledger* in
words rather than by an absence that means something else everywhere else it appears. (The other
is the `person-reader` organizer, for the opposite reason: its subject is a person, and a person
is not something the ledger owes.)

## Delegation

> **If something takes more than a few trivial thoughts, delegate it.**

Responsiveness comes from delegation, not from keeping any layer weak. Each rung hands down
what does not belong to its tempo, and the rung below absorbs the silence.

## See also

[`host.md`](host.md) for the switchboard and glancing up ·
[`data.md`](data.md) for what they read and write ·
[`legacy/reaction-cognition-split.md`](legacy/reaction-cognition-split.md) for the three-tempo
version this supersedes.
