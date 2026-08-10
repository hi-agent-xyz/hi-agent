# Core — the host

## Goal

Be the part of the system that is always awake, always fast, and never thinking: carry
signals into the one conversation, decide *when* the agent may speak, own every process and
the cadence that [opens the agent's eyes](#glancing-up), and write everything down before
anyone reacts to it.

Nothing here consults a model. That is the point — the core has to keep working while the
thinking layers are slow, confused, or dead.

## Decisions

| Decision | Reasoning |
|---|---|
| **There is one conversation, and it has no name** | The agent is one mind talking to one person, continuously. A partition key would have to be assigned by someone — and every candidate (the browser, the device, the surface) names a *client*, not a situation. See [One conversation](#one-conversation) |
| A client is a connection, never an identity | Clients attach and detach. Nothing a client sends may decide what the mind knows, because a client cannot know that |
| One mouth, one floor | Many sub-minds may think; the person hears one voice, one utterance at a time |
| A vendor outage is decided process-wide, not per turn | One upstream, decided once — never rediscovered or apologized for twice |
| The reflex path never reaches a model | Stopping when someone starts talking cannot wait a generation |
| The log is written *before* anything reacts | Durability must not depend on a session surviving |
| The host opens the agent's eyes; the agent owns its own timers | A scheduler we build dies with the process; a crontab the agent writes does not |
| Sessions are host-owned and **replaceable** | No session is a source of truth — continuity lives in `data/`. Replaceable is not the same as short-lived: every thinking rung keeps **one long-lived session**, so it can remember what it was doing — while nothing downstream depends on it surviving. It is replaced when it breaks, not when it grows; growth is the underlying agent's to compact |

## Components

### Wire adapters

Bind concrete protocols (HTTP today, WebSocket or local audio later) to transport-free
channel signals. Framing, mime, long-poll, body-close, per-turn frame binding — all of it
lives here so that none of it exists above.

### One conversation

**There is no context-isolation key.** One Reaction, one [Deliberation](agents.md#deliberation--seconds),
one memory, one continuous thread — the same conversation whether it arrives by voice from a
browser tab, by a screenshot from the ⌘⌘ gesture, or by a file from a phone. Everything
inbound joins it; everything outbound reaches every attached client.

Signals reach Reaction, which is the mind's front door. Everything deeper is reached by
[message](foundation.md#the-agent-session-registry), agent to agent, through the registry:
Reaction hands down to Deliberation, Deliberation hands up to
[Cognition](agents.md#cognition--minutes-and-beyond), and answers come back the way they went.

Participants are *soft*, inferred from content, never a structural key. The person the agent
recognizes by face or voice is content it knows, not a partition it lives in — someone
walking into the room does not start a second conversation.

> **This replaced `Scene`, which was removed.** A scene was "the situation a signal belongs
> to", the isolation unit keying a Reaction, a Deliberation, a memory slice, and a tag on
> every tool attach. It was removed for three reasons, in increasing order of weight:
>
> 1. **It isolated two rungs of four.** Cognition and Reflection are global by design and
>    memory was always shared, so a scene only ever partitioned Reaction and Deliberation —
>    while costing a parameter on almost every function in the tree.
> 2. **It had no derivation rule.** The design said what a scene *meant* and never who
>    decided one, so the browser decided, with a random id in `localStorage`. Every scene
>    that ever existed on a real install was a browser profile wearing the name of a
>    situation — and clearing site data forked a new mind with a blank memory, silently.
> 3. **It is not how a person works.** Nobody keeps a separate memory per device. Moving
>    from a laptop to a phone mid-thought is the same thought.
>
> The isolation it was meant to provide, when a second party genuinely exists, is not
> reintroduced here. When an external channel adapter lands (a group chat, a mail thread),
> that adapter knows its own thread id exactly, and partitioning can be built on a fact
> rather than on a guess. Deferring it costs nothing today: no such adapter exists, and
> every one of the scenes a real install accumulated was the same person.

### Channel mux

Fans N input channels into one prompt, and fans one output stream back out to N channels
by the carrier rules. This is a consequence of a session carrying a single conversation,
not a goal in itself.

### The social layer lives in Reaction, not here

*This was once a host component called the arbiter.* It has been retired, and its four
duties moved into [Reaction](agents.md#reaction--one-generation) — because all four are the
conversation's, and Reaction is the thing that speaks. A separate module arbitrating a mouth
that only one agent owns was machinery around a decision that belonged to the agent making
it.

- **Mouth singleton** — now structural rather than enforced: there is one Reaction, taking
  one turn at a time.
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
arrives from the shell (`windowDidChangeOcclusionState:`). A state subscription that
stays up or reconnects on its own is exactly the hazard this axis has to exclude.

**The window has three states, and they are the one presence fact the client reports
rather than the host deriving.** Active, background, closed. Dropping the channels makes
the last two identical on the wire — which is correct for reach, since nothing lands
either way — but they are not the same situation, and nothing except the client can tell
them apart. **Background is ambient; closed is an act.** A window behind an editor may be
glanced at in seconds; a window the person shut is one they decided they were done with.

So the three feed **expectation**, not a fourth axis: closed reads away at once, because
the decay exists to *infer* absence from silence and closing states it outright, while
background gets no shortcut — someone reading in the window in front of ours has not
left, and treating "not looking right now" as "gone" would make the agent go quiet on a
person sitting right there. Neither is projected and neither appears in `say`'s answer:
nothing above the host learns a new vocabulary, the states just move the expectation the
mind already reads.

**Reach is answered, not projected. Expectation is projected.** Only one of the two can
be learned by trying, and it should be: the host tells `say` where the words landed, read
at the instant of emission, so nothing above the host has to ask whether it can be heard
before speaking. A projected copy of the same fact is the staler of the two, and they
disagree precisely when it matters, because a turn can outlive the window that started
it. Expectation cannot be learned this way — it is graded rather than binary and true
even when every channel is open, since it shapes *how much* to say rather than *whether*
— so it, alone, is rendered into the window. One reader either way: Reaction.

**What the gate protects is narrower than it first looks, and that is the point.**
Text and views are appearance state, not deliveries. The host owns what is on the face
now; attaching a surface gives it that present state and then replacements. **Voice is
the exception** — it exists only in the moment it is heard, so a spoken line synthesized
with no speaker attached is gone. The host therefore withholds exactly one thing, speech
synthesis, and reports what it did: `say` answers with where the words actually landed —
aloud, on screen only, or waiting for them. Everything above that is Reaction's judgment,
not a rail.

**There is one text appearance, however many windows render it.** Its authoritative state
is the latest settled human line, any live recognition interim, and the agent's current
reply as it grows. A surface receives the whole state when it connects and every later
whole-state replacement. It never consumes text and never tells the host what it has
read. There are no message ids, client ids, cursors, acknowledgements or per-window
bookmarks.

This is current-state synchronization, **not catch-up**. A slow surface may skip
intermediate typing states and still converge on the latest one. A surface attaching
after an exchange was replaced does not receive that older exchange. A process restart
starts the text appearance empty. The journal is the historical conversation; the
appearance is only the present. `/out/view` follows the same whole-state principle, with
the separate decision that views persist across restart.

This ownership rule is what makes multiple windows one appearance instead of several
readers of one mouth. It also removes the interrupted-delivery problem: after a transport
break the surface receives the current whole text again, replacing its local rendering
rather than resuming, appending or acknowledging fragments.

A settled human line also wins the present immediately. If it lands after a reaction turn
started, later text from that older turn is excluded from the appearance; the reaction and
journal still finish, and the next turn may answer the new line. The full state machine,
breaking boundary, and accepted consequences are fixed in
[`text-appearance.md`](text-appearance.md).

**Coming back is an event, and the only one here.** Every other presence change is read
off the axes during a turn that was already happening. A return is not: it happens
precisely when nothing is happening, so without an edge nothing would observe it, and
"hold it for their return" would mean "hold it until they type" — or until the pulse
comes round, which is half an hour. So Reaction is woken when the person brings a window
forward after an absence. **Only a first-party activation counts**: an out-channel
reconnecting proves a browser exists, never that someone is in front of it, and a
state subscription reconnects on its own while a tab sits forgotten in the background. The wake
carries a fact and no instruction, and it is dropped rather than queued if it fires
mid-turn — a turn in progress is already talking to them.

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

Exposes each agent session as an independent handle — prompt it, read its updates, drop it
to close. One subprocess per session, so one session's crash cannot touch another. A warm
pool absorbs spawn latency for the sessions that are created per delegation.

**A rung's prompt is the session's system prompt**, set when the session is opened. Not a
first message, not a preamble the agent's own persona frames — the system prompt. Anything
less means the rung's character is advice layered over somebody else's.

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

One **process-wide** view of whether the upstream model is reachable, read before a turn is
taken. The vendor is a shared resource, so an outage discovered by one rung must steer every
rung — Reaction, Deliberation, Cognition, Reflection and every worker share one upstream.

- **It gates the turn, not the reply.** During an outage no generation starts at all;
  incoming mail is held rather than answered badly or dropped.
- **Backoff, absorbed and capped.** A blip is absorbed before anything is declared down; from
  there the retry gap doubles to a ceiling. A rate limit is not an outage worth mentioning; a
  string of failures is.
- **One apology, once.** The transition — not each failed turn — is what earns a word to the
  person. N rungs × M retries must never become N × M apologies, and recovery is likewise
  announced once.

When it clears, the held mail drives a catch-up turn. Fix-forward, like everything else here.

### The log

Every signal in and out is written before anything reacts to it. The log — not session
lifetime — is authoritative for durability, recovery and cold start.

**One tree, not two.** What earlier drafts split into `raw/` and a separate `journal/` is a
single append-only log at [`memory/raw/`](data.md#memoryraw). It is mechanical, so it belongs
to [`data/`](data.md); it appears here because it sits on the hot path.

### Glancing up

**The host's whole timing surface is opening the agent's eyes on a cadence.** Three
loops do that, each pacing itself from inside its own subsystem — the **pulse** for
the conversation, the **reflection backoff** for consolidation, and **Cognition's
glance-up** (one wake shortly after the process starts, then on the pulse cadence
whenever anything is owed).

Beside them sits the one deadline that is *not* a cadence: **the voice's own
check-in**, below. It is not a fourth loop and not a scheduler — one slot, one
deadline, one wake, no target and no payload.

#### The check-in — the only thing that fires at a named time

Reaction sets it by naming a number in `say`'s `back_in` — the same number it just
said out loud ("give me ten minutes") — and the host wakes it when that is up. The
size of a silence is therefore a property of *the utterance that opened it*: a
promise is only a promise once it has been said, so there is no way to arm a wake for
a number nobody was told.

**Why this is not the clock this design removed.** It holds exactly one deadline per
voice, it fires nothing but that voice's own loop, it carries no task and no target,
and a task's `due` still fires nothing. It is a second deadline in a `select!` that
already carries the pulse's. Every property the removal was protecting survives:
scheduling past a cadence remains the agent's own, arranged with the shell it has.

**And a floor beneath it, because a promise can go unmade.** While the conversation's
own thinking is still running and the voice left the silence open-ended, the host arms
a check-in itself — five minutes, doubling to the pulse. **The dial on that gap is
whether the last one was worth it**: a check-in that produced speech keeps the base
cadence, one that passed in silence widens it. The voice is the only thing that knows
whether there was anything to say, so it is the thing that sets the pace.

The note says which of the two it is. A promise the person heard is a fact they hold
too; a floor is only the agent's own rule about not going dark, and a voice told it
"promised" when it named nothing would be inventing one. Both are **permission to
speak, never an instruction to**: what is worth saying is read off `## Still looking
into` and the projected ledger, and staying quiet is a legitimate answer.

A check-in that comes due into an **empty room is dropped**, not held: the words would
be held anyway, and [presence](#presence) already wakes the voice on their return —
with the same work in front of it and a fresher read of where it stands.

Everything else an agent needs from time, **the agent arranges itself.** It has a
shell, so it installs a cron entry, a `launchd` job, a systemd timer, or parks a
worker that sleeps and messages home — and a crontab survives a reboot, which
nothing living in this process does. Nothing is restricted to keep it that way: the
agent keeps every tool it has.

#### The two shapes, and what each needs from us

| Shape | How it runs | What the host provides |
|---|---|---|
| **Cadence** — check this every N hours | Cognition's glance-up *is* the executor: it wakes, reads the ledger, and does what is due. Or an agent-installed timer does the work and leaves a durable trace. | The glance-up, and nothing else. |
| **Precise moment** — be somewhere at 07:00 | A parked worker sleeps and `send_message`s its owner; the ledger re-arms it after a restart. | `create_worker` + the one verb. |

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
30-minute pulse that is fine for a filing deadline and wrong for a wake-me-at-07:00
alarm, which is the agent's to arrange per the table above.

What used to stand here as the second cost — *nothing wakes the voice when a promise
is running late* — is the check-in above, and it was removed for a reason worth
keeping in view. It read as a rough edge and was a broken product: the voice named a
number, nothing read it, and the person closed the gap by asking "progress?". A
promise whose only enforcement is the model remembering to speak is not a promise, and
`reaction.md` said so in its own words long before the host could act on it — *a
check-in they have to ask for is already late*.

What is still true: a promise is kept only while the process lives. Nothing restores
an armed check-in across a restart, and nothing should — a promise made in minutes is
stale by the time a restart is noticed, and what survives a restart is the **duty**,
in the ledger, which the glance-up picks back up.

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
