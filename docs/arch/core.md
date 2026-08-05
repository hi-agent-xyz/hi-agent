# Core — the host

## Goal

Be the part of the system that is always awake, always fast, and never thinking: route
signals to the right scene, decide *when* the agent may speak, own every process and every
clock, and write everything down before anyone reacts to it.

Nothing here consults a model. That is the point — the core has to keep working while the
thinking layers are slow, confused, or dead.

## Decisions

| Decision | Reasoning |
|---|---|
| Scene is the isolation unit | One conversation's context must not bleed into another's |
| One mouth **per scene**, arbitrated centrally | Many sub-minds may think; a conversation hears one voice — but two conversations must never queue behind each other |
| A vendor outage is decided process-wide, not per turn | Every scene shares one upstream; N scenes must not each rediscover it, or apologize for it |
| The reflex path never reaches a model | Stopping when someone starts talking cannot wait a generation |
| The log is written *before* anything reacts | Durability must not depend on a session surviving |
| The host opens the agent's eyes; the agent owns its own timers | A scheduler we build dies with the process; a crontab the agent writes does not |
| Sessions are host-owned and **replaceable** | No session is a source of truth — continuity lives in `data/`. Replaceable is not the same as short-lived: every thinking rung keeps **one long-lived session**, so it can remember what it was doing — while nothing downstream depends on it surviving. It is replaced when it breaks, not when it grows; growth is the underlying agent's to compact |

## Components

### Wire adapters

Bind concrete protocols (HTTP today, WebSocket or local audio later) to transport-free
channel signals. Framing, mime, long-poll, body-close, per-turn frame binding — all of it
lives here so that none of it exists above.

### Scene router

A **scene** is the situation a signal belongs to: with a person, with a group, or alone.
It is the context-isolation unit — one Reaction and one [Deliberation](agents.md#deliberation--per-scene-seconds)
per scene, one memory slice, one tag on every session's tool attach. **The router's only
target is Reaction** — it is the scene's front door. Everything deeper is reached by
[message](foundation.md#the-agent-session-registry), agent to agent, through the registry:
Reaction hands down to its Deliberation, Deliberation hands up to
[Cognition](agents.md#cognition--sceneless-minutes-and-beyond), and answers come back the way
they went. Cognition is sceneless and the router never addresses it.

Participants are *soft*, inferred from content, never a structural key. An external source
maps onto a scene (a group chat is a scene). The topology is decided once by judgment when
work is delegated, then executed mechanically.

### Channel mux

Fans N input channels into one prompt, and fans one output stream back out to N channels
by the carrier rules. This is a consequence of ACP carrying a single conversation, not a
goal in itself.

### The social layer lives in Reaction, not here

*This was once a host component called the arbiter.* It has been retired, and its four
duties moved into [Reaction](agents.md#reaction--per-scene-one-generation) — because all four
are per-scene, and Reaction is already the only per-scene thing that speaks. A separate
module arbitrating a mouth that only one agent owns was machinery around a decision that
belonged to the agent making it.

- **Mouth singleton, per scene** — now structural rather than enforced: one Reaction per
  scene, taking one turn at a time. The singleton is scene-wide, never process-wide, because
  a global mouth would make one scene wait on another —
  [invariant 3](arch.md#invariants).
- **Turn-taking** — the quiet-settle commit that decides a turn is over. Still host-side:
  it is [batching](surfaces.md#batching), and it happens before Reaction is woken.
- **Presence gate** — Reaction reads [presence](#presence) and decides whether to speak into
  a room that may be empty.
- **Social timing** — when to voice a worker's answer, when to let it wait.

What the host keeps is what has no model in it: the queue behind `say`; the fact that
`say` **returns**, so a held or failed utterance is answerable; and the one call that is
not a judgment at all — **not synthesizing speech for a speaker that isn't attached**,
because that is a fact about the wire, not a read of the room.

### Presence

Whether anyone is actually there. **Nobody sends it — it is derived**, and derived
*only from what the app observes about its own surface and its own conversation*: which
of its channels are open, and how recently the person engaged with it. Deliberately not
from the system — no idle timer, no screen lock, no other-app probing. Those measure
"away from the keyboard" when the question is "away from **hi-agent**", and someone
heads-down in another app for an hour is, to us, away. A face on camera and a voice in
the room are observed elsewhere and reach the agent as **journaled signals it weighs**,
not as inputs to this model: a face is sometimes a photo, and evidence that soft belongs
in judgment rather than in a gate.

It is not one number and not a mode ladder, but three orthogonal axes that combine
freely — **reach** (which of window, speaker and mic a message can land on right now),
**expectation** (a decaying belief about how much output they're awaiting: eager,
around, away), and **posture** (whether a voice exchange is live).

**An open channel is a claim that someone is reading it, and the client owes us the
truth of that claim.** Reach is derived from which out-channels are subscribed, so a
subscription held behind another window is not a stale reading — it is a false one, and
everything downstream reasons correctly from it to the wrong answer. The face therefore
drops its out-channels the moment nobody is looking: hidden, miniaturized, closed, or
**fully occluded by another window** — the last of which no web API reports, so it
arrives from the shell (`windowDidChangeOcclusionState:`). A channel that stays up
because a long-poll re-opens on its own is exactly the hazard this axis has to exclude.

**Reach is answered, not projected. Expectation is projected.** Only one of the two can
be learned by trying, and it should be: the host tells `say` where the words landed, read
at the instant of emission, so nothing above the host has to ask whether it can be heard
before speaking. A projected copy of the same fact is the staler of the two, and they
disagree precisely when it matters, because a turn can outlive the window that started
it. Expectation cannot be learned this way — it is graded rather than binary and true
even when every channel is open, since it shapes *how much* to say rather than *whether*
— so it, alone, is rendered into the window. One reader either way: Reaction.

**What the gate protects is narrower than it first looks, and that is the point.**
Words and views survive an empty room without help: outbound text is buffered per scene
and delivered to a reader that connects later, and the screen is retained state, folded
and replayed to whoever attaches next. Neither is spent. **Voice is the exception** — it
exists only in the moment it is heard, so a spoken line synthesized with no speaker
attached is gone, and the person comes back and never learns it happened. So the host
withholds exactly one thing, the speech synthesis, and reports what it did: `say`
answers with where the words actually landed — aloud, on screen only, or waiting for
them. Everything above that is Reaction's judgment, not a rail.

**That narrowing is only sound while the buffer is honest, and it is the buffer's reader
that decides.** Text is deferred rather than spent because it waits for a reader that
connects later — but the bus is drain-and-delete, not a cursor, so it defers only until
*a* reader takes it, and an unattended reader takes it and shows it to nobody. The
mechanism that was supposed to protect the words is then the one that spends them, and
the outcome is worse than either honest end: the person gets a fragment that reads as a
whole message rather than a message they can tell they missed. **Half-spent is the
failure to design against here** — voice is spent by physics, text by attention, and the
gate can only see the first. Which is why an out-channel must be held open only while
someone is reading it: with that true, "only voice is spent" is true, and without it the
sentence is aspirational.

**Coming back is an event, and the only one here.** Every other presence change is read
off the axes during a turn that was already happening. A return is not: it happens
precisely when nothing is happening, so without an edge nothing would observe it, and
"hold it for their return" would mean "hold it until they type" — or until the pulse
comes round, which is half an hour. So the scene's Reaction is woken when the person
brings the window forward after an absence. **Only a first-party activation counts**: an
out-channel reconnecting proves a browser exists, never that someone is in front of it,
and a long-poll re-opens on its own while a tab sits forgotten in the background. The
wake carries a fact and no instruction, and it is dropped rather than queued if it fires
while the scene is mid-turn — a scene taking a turn is a scene already talking to them.

### Reflex

The sub-second path that **short-circuits every agent** — the bottom rung of the
[tempo ladder](arch.md#the-tempo-ladder), and the only one with no model in the loop.

Two kinds of thing live here. **Barge-in and the attention gesture**: when someone starts
speaking, sound stops mid-syllable and the unspoken tail is discarded. And **taught
quick-actions**: a small repeated thing the person showed the agent once, recognized and
replayed directly, because asking a model to re-derive it every time costs a generation to
reach an answer that never changes.

A generation is far too slow for either, which is the whole justification for a rung that
cannot think.

The barge-in *follow-up* is the opposite: what to do about the interruption is a judgment, handled by
Reaction on the next turn with an estimate of how far it got. The same event is therefore
handled at two very different speeds, which is why the reflex is drawn separately.

### Session layer

Exposes each ACP session as an independent handle — prompt it, read its updates, drop it to
close. One subprocess per session, so one session's crash cannot touch another. A warm pool
absorbs spawn latency for the sessions that are created per delegation.

**Every thinking rung holds one long-lived session, from the moment it is created.** Reaction,
Deliberation and Cognition each keep a single session across turns rather than opening one per
piece of work: a rung that reopens every time cannot remember what it was in the middle of, and
"what I was in the middle of" is not a fact the ledger holds — the ledger holds what is *owed*,
not what has already been tried, ruled out, or half-arranged. A rung that forgets that re-derives
it every wake, and re-deriving it from a ledger that reads "still owed" is how a duty gets redone
or, worse, undone.

Long-lived sessions rot — every turn appends, until early context is crowded out or the
window overflows. **Bounding that is the underlying agent's job, not ours.** The agent behind a
session compacts its own context in place, automatically, near its real window. We do not
duplicate it, and there is no ceiling, counter, or swap in this codebase.

That is a correction, not an omission. A host-side hot-swap existed: it counted the characters
*we* sent and received and, past a ceiling, asked the session to brief its own replacement. It
was wrong on both halves. **We cannot see the context** — the agent's own system prompt and tool
schemas are the bulk of every request and are invisible from out here, so the counter thresholded
on a small, drifting fraction of the truth. And **we cannot compact in place**: a session is
`new`, `prompt`, `cancel`, `update` and nothing else, so summarize-and-reopen was the only move
available from outside, and it is strictly lossier than what the agent does inside. It also
fought the rungs being long-lived — swapping threw away exactly the working thread a long-lived
rung exists to keep.

If a wire ever genuinely lacks auto-compaction, bound it **in that adapter**, where the real
numbers are visible. Do not re-introduce a character counter at this layer.

What still replaces a session from out here is **failure, not size**: a turn that errors discards
the possibly-wedged session and the next one cold-opens. That is always survivable, because
**the state a rung needs is re-projected into every turn** — what is owed, what it carries
forward, who it can reach — and the [log](#the-log) is the durable backstop. A cold open loses
the thread, never the truth. The session carries the thread; `data/` carries the truth.

### Vendor gate

One **process-wide** view of whether the upstream model is reachable, read by every scene
before it takes a turn. The vendor is a shared resource, so one scene discovering an outage
must steer all of them.

- **It gates the turn, not the reply.** During an outage no scene starts a generation at all;
  incoming mail is held rather than answered badly or dropped.
- **Backoff, absorbed and capped.** A blip is absorbed before anything is declared down; from
  there the retry gap doubles to a ceiling. A rate limit is not an outage worth mentioning; a
  string of failures is.
- **One apology, once.** The transition — not each failed turn — is what earns a word to the
  person. N scenes × M retries must never become N × M apologies, and recovery is likewise
  announced once.

When it clears, the held mail drives a catch-up turn. Fix-forward, like everything else here.

### The log

Every signal in and out is written before anything reacts to it. The log — not session
lifetime — is authoritative for durability, recovery and cold start.

**One tree, not two.** What earlier drafts split into `raw/` and a separate `journal/` is a
single append-only log at [`memory/raw/`](data.md#memoryraw). It is mechanical, so it belongs
to [`data/`](data.md); it appears here because it sits on the hot path.

### Glancing up — and why there is no clock

**There is no clock, and there will not be one.** A scheduler module was designed
here, deferred once, and is now **declined**. What replaces it is not a smaller
scheduler: it is the observation that scheduling was never the host's problem.

**The host owns one thing: opening the agent's eyes on a cadence.** Three loops do
that, each pacing itself from inside its own subsystem — the **pulse** for each
scene, the **reflection backoff** for consolidation, and **Cognition's glance-up**
(one wake shortly after the process starts, then on the pulse cadence whenever
anything is owed). That is the whole of the host's timing surface, and it is
already built.

Everything else an agent needs from time, **the agent builds itself.** It has a
shell and it can install a cron entry, a `launchd` job, a systemd timer, or park a
worker that sleeps and messages home. Those are better at this than we would be:
a crontab survives a reboot, and the module designed here explicitly would not
(*"the clock holds no durable state"*). Declining to build it is not a gap left
open — it is the same call this codebase already makes about capabilities
generally. Nothing gets restricted to enforce this; the agent keeps every tool it
has, and simply stops needing a host scheduler.

#### The two shapes, and what each needs from us

| Shape | How it runs | What the host provides |
|---|---|---|
| **Cadence** — check this every N hours | Cognition's glance-up *is* the executor: it wakes, reads the ledger, and does what is due. Or an agent-installed timer does the work and leaves a durable trace. | Nothing new. The glance-up. |
| **Precise moment** — be somewhere at 07:00 | A parked worker sleeps and `send_message`s its owner; the ledger re-arms it after a restart. | Nothing new. `create_worker` + the one verb. |

The first covers the standing duties this system actually has. The second is rare
and costs an idle subprocess, which is the right price for something rare.

#### The rule that makes any of it safe: **`verify` is a result check**

An agent choosing its own mechanism is only sound if a mechanism that quietly dies
is *caught*. That is entirely a property of what [`Liveness::verify`](data.md#tasks)
names:

- **"a cron job with this id exists"** — an existence check. It passes forever, including
  when the job has never once fired. This failed in the field: a watch was armed,
  reported healthy, and had never fetched a price.
- **"`checked` was stamped within the last 3h by a run that returned real prices"** — a
  result check. A cron that never fires, a `launchd` plist that never loaded, a parked
  worker killed by a restart: **all of them fail this within one cadence**, and the
  glance-up repairs them.

So the mechanism is the agent's to choose, and the *contract* is ours to insist on.
Whether it used our machinery or the harness's stops mattering, because a duty that
is not actually running fails its own check either way. `checked` is the one liveness
field code reads, for exactly this reason.

#### `/api/in/text` is not a wake channel

If an agent-installed timer ever needs to poke a running instance over HTTP, note
that the inbound text route journals `Origin::Human` and calls
`presence.note_activity()`. A timer firing into it would tell the agent **a person
just spoke** — resetting the away timer, starting the owed-reply clock, and
defeating the presence gate. Both shapes above avoid it by not needing a door at
all. If a genuine need appears, the answer is a channel that says what it is
(`Channel::Clock`, already in `NON_ACTIVITY_CHANNELS`), never this one.

#### What this costs, stated plainly

A deadline is met **at the next glance, not at its minute** — `due` is read by the
projection and orders what is shown, and nothing in the host fires on it. At a
30-minute pulse that is fine for a filing deadline and wrong for an alarm clock; an
alarm clock is the agent's to build, per the table above. And **nothing wakes the
voice when a promise is running late** — a return is observed by
[presence](#presence), but lateness is not an event anyone observes.

Two things that were listed here as clock clients and are not: **"you just came
back"** belongs to [presence](#presence), which observes it directly rather than
polling for it; and the social timeout, the speech clock, and the session heartbeat
each pace a loop inside their own subsystem rather than waking an agent.

## Fix-forward

There is no true cancel. New input — a correction, a barge-in, a change of mind — is
incorporated by the always-free Reaction, which corrects course. This is more human than a
hard cancel and it fits the one-prompt-in-flight constraint: interruptions land on Reaction,
never on a busy worker.

A hard kill exists (drop the handle, the process exits) and stays available for the cases
where fix-forward genuinely does not apply.

## See also

[`agents.md`](agents.md) for what the core drives ·
[`data.md`](data.md#tasks) for the ledger a glance-up reads, and the `verify` contract.
