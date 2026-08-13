# Host — the Rust host

## Goal

Be the part of the system that is always awake, always fast, and never thinking: carry
signals into the one conversation, decide *when* the agent may speak, own every process and
the cadence that [opens the agent's eyes](#glancing-up), and write everything down before
anyone reacts to it.

Nothing here consults a model. That is the point — the host has to keep working while the
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

**There is no context-isolation key.** One Reaction, one
[Cognition](agents.md#cognition--minutes-and-beyond), one memory, one continuous thread —
the same conversation whether it arrives by voice from a
browser tab, by a screenshot from the ⌘⌘ gesture, or by a file from a phone. Everything
inbound joins it; everything outbound reaches every attached client.

Signals reach Reaction, which is the mind's front door. Everything deeper is reached by
[message](foundation.md#the-agent-session-registry), agent to agent, through the registry:
Reaction hands the turn's request down to
[Cognition](agents.md#cognition--minutes-and-beyond), and answers come back the way they
went. That used to be two hops through a Deliberation between them; it is one since that
rung was [retired](agents.md#deliberation-was-retired-into-cognition).

Participants are *soft*, inferred from content, never a structural key. The person the agent
recognizes by face or voice is content it knows, not a partition it lives in — someone
walking into the room does not start a second conversation.

> **This replaced `Scene`, which was removed.** A scene was "the situation a signal belongs
> to", the isolation unit keying a Reaction, a Deliberation (the rung then between the
> voice and the brain), a memory slice, and a tag on
> every tool attach. It was removed for three reasons, in increasing order of weight:
>
> 1. **It isolated two rungs of four.** Cognition and Reflection are global by design and
>    memory was always shared, so a scene only ever partitioned Reaction and Deliberation —
>    while costing a parameter on almost every function in the tree. (One of those two has
>    since been retired for a related reason: once there was one conversation, a
>    per-conversation reading rung was a singleton in front of a singleton.)
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
- **Social timing** — when to voice a worker's answer, when to let it wait.

*The fourth duty, a **presence gate**, has been retired outright rather than moved — see
[Attachment](#attachment).*

What the host keeps is what has no model in it: the queue behind `say`; the fact that
`say` **returns**, so an over-long utterance is answerable; and the one call that is not
a judgment at all — **not synthesizing speech for a speaker that isn't attached**,
because that is a fact about the wire, not a read of the room.

### Attachment

*This section was once called **Presence**, and it derived — from open channels, window
activations, and a decaying belief — whether the person was actually there. It has been
removed. What is left is one fact with one consumer.*

**Whether a speaker is attached.** Counted from live out-channel subscriptions, read at
the instant a turn opens its TTS span, and used for exactly one thing: not synthesizing
speech nobody can hear. That is a fact about the wire — the frames go out as they are
made and a span with no listener is spent — and it is the only thing in this host that
has ever needed to know who is connected.

**Why the rest went.** An open channel answers *is a window subscribed*, which was never
the same question as *are you reading*. The gap is not a tuning problem: a window behind
an editor, a tab left open on another desk, and a person leaning in are the same
subscription, and no amount of decay separates them. Everything derived from that
reading inherited the error — the agent went quiet on someone sitting right there, and
spoke to an empty desk, from the same signal. It was checked against the one ground
truth available, which was the person saying it was wrong.

**What replaced it is not a better estimate — it is not needing one.** The gate existed
because words did not keep: text was current state, so speaking into an empty room threw
the words away, and withholding them was the lesser loss. Messages keep. A message said
to nobody is a message waiting in the conversation, exactly like a message sent to a
phone that is face-down. There is nothing left to protect, so there is nothing left to
detect.

Removed with it: the eager/around/away expectation and its projection into every turn's
prompt; the three window states and the first-party attention lane that reported them;
the return edge that woke Reaction when someone came back, and the held telling it
existed to deliver; and `say`'s answer about where the words landed. A due check-in now
fires into an empty room like any other message, because that is what a message is for.

**A face on camera and a voice in the room are still observed**, and they still reach the
agent as journaled signals it weighs. They were never inputs to this model, and that
distinction survives it: a face is sometimes a photo, and soft evidence belongs in
judgment rather than in a gate.

### The conversation is a message list

**There is one conversation, however many windows render it.** It is an ordered,
append-only list of whole messages, owned by the host, seeded from the journal at boot.
A window receives the current window of it on connect and every later message as it is
appended. It never consumes a message, never tells the host what it has read, and holds
no queue, cursor or bookmark of its own.

Three things are messages: what the person typed or said, a file they handed over, and
one `say` call. **One `say` is one message, whole** — the call already carries its
complete text, so nothing is assembled from streamed chunks. Sentence splitting still
happens, but only to pace TTS, and it never reaches the list. Views, worker reports,
mail between rungs, clock wakes, recognition signals and tool calls are not conversation
and are not in it; they have the view slot, the journal and the inspector.

**Nothing is ever rewritten or cleared**, which is what makes the ownership rule simple
enough to keep. The previous contract had to decide what happened when a human line
landed mid-turn, because both wanted the same slot; a list has no slot to contest, so
the reply appends after the line it crossed with, carrying the timestamp that says so.
`/out/view` keeps its own whole-state principle and its persistence across restart.

**There are no read receipts and there will not be.** That is the same underivable fact
the presence gate was built on, and putting it back on the wire in a lighter costume
would rebuild the same error. A window's unread marker is a scroll position in one
browser and stays there.

The message ids are the journal's, which is what lets scrollback and the live window
share identifiers without a merge. An id on a message is not a delivery cursor: nothing
sends one back to claim progress. The full contract — what is a message, the frames,
durability, accepted consequences — is fixed in
[`text-transcript.md`](text-transcript.md).

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

**Every thinking rung holds one long-lived session, from the moment it is created.** Reaction
and Cognition each keep a single session across turns rather than opening one per
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
rung — Reaction, Cognition, Reflection and every worker share one upstream.

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

**The host's whole timing surface is opening the agent's eyes on a cadence.** Two loops
do that, each pacing itself from inside its own subsystem — **Cognition's glance-up** (one
wake shortly after the process starts, then on the `pulse` cadence whenever anything is
owed) and the **reflection backoff** for consolidation.

**The voice has no cadence, and that absence is deliberate.** A pulse used to wake the
conversation loop on the same knob and run a turn into an empty room. Reaction is
tools-off, so the wake handed it nothing it could not already see in the window it gets on
*every* turn — the least-informed rung was the one deciding whether to speak. The journeys
measured what that produced: two post-restart pulses, both concluding without a `say`,
while a standing duty sat unread in the ledger ([gaps #1](../user-journeys/gaps.md)). It
was also the most expensive wake in the system, because the projected window rides every
turn and accumulates in the session. Unprompted speech comes instead from the rung that
can actually check — Cognition glances up, reads the ledger, and messages Reaction, which
is invariant 5 doing its job. **Reaction wakes on three things and no clock: input from
the person, mail from another rung, and its own check-in.**

That check-in is the one deadline that is *not* a cadence. It is not a third loop and not
a scheduler — one slot, one deadline, one wake, no target and no payload.

#### The check-in — the only thing that fires at a named time

Reaction sets it by naming a number in `say`'s `back_in` — the same number it just
said out loud ("give me ten minutes") — and the host wakes it when that is up. The
size of a silence is therefore a property of *the utterance that opened it*: a
promise is only a promise once it has been said, so there is no way to arm a wake for
a number nobody was told.

**Why this is not the clock this design removed.** It holds exactly one deadline per
voice, it fires nothing but that voice's own loop, it carries no task and no target,
and a task's `due` still fires nothing. With the pulse gone it is the only deadline in
Reaction's `select!` that is not vendor recovery — which is an argument for keeping it
exactly this small, never for letting it grow into the removed clock's replacement. Every
property the removal was protecting survives: scheduling past a cadence remains the
agent's own, arranged with the shell it has.

**And a floor beneath it, because a promise can go unmade.** While the conversation's
own thinking is still running and the voice left the silence open-ended, the host arms
a check-in itself — five minutes, doubling to the glance-up cadence. **The dial on that gap is
whether the last one was worth it**: a check-in that produced speech keeps the base
cadence, one that passed in silence widens it. The voice is the only thing that knows
whether there was anything to say, so it is the thing that sets the pace.

The note says which of the two it is. A promise the person heard is a fact they hold
too; a floor is only the agent's own rule about not going dark, and a voice told it
"promised" when it named nothing would be inventing one. Both are **permission to
speak, never an instruction to**: what is worth saying is read off `## Still looking
into` and the projected ledger, and staying quiet is a legitimate answer.

A check-in **fires whether or not anyone is looking**. It used to be dropped into an
empty room, on the reasoning that the words would be withheld anyway and a return would
wake the voice with a fresher read. Both halves of that went with the
[gate](#attachment): a check-in produces a message, a message waits in the conversation,
and there is no return edge left to defer it to.

Everything else an agent needs from time, **the agent arranges itself.** It has a
shell, so it installs a cron entry, a `launchd` job, a systemd timer, or parks a
worker that sleeps and messages home — and a crontab survives a reboot, which
nothing living in this process does. Nothing is restricted to keep it that way: the
agent keeps every tool it has.

#### The three shapes, and what each needs from us

| Shape | How it runs | What the host provides |
|---|---|---|
| **Cadence** — check this every N hours | Cognition's glance-up *is* the executor: it wakes, reads the ledger, and does what is due. Or an agent-installed timer does the work and leaves a durable trace. | The glance-up, and nothing else. |
| **Precise moment** — be somewhere at 07:00 | A parked worker sleeps and `send_message`s its owner; the ledger re-arms it after a restart. | `create_worker` + the one verb. |
| **Arrival** — something reached the group | The agent's own listener holds the connection and posts what arrived to `/api/in/duty/<start_key>`; a working session handles it in seconds. | The duty inbox: coalesce, resolve the key against the ledger, open a handler from the facet if none is live. |

The first covers the standing duties this system actually has. The second is rare
and costs an idle subprocess, which is the right price for something rare.

##### Arrival, and why it does not weaken any of the above

A duty was reactive at its edge and cadence-paced at ours: the listener received a
message the instant it was sent, wrote a row, and nothing read that row until the next
glance — up to a pulse later. The third row closes that, under three constraints that
keep it from becoming a second, weaker way of keeping a promise.

**The nudge is not the truth.** A delivery carries what arrived, and the listener's own
append-only ledger remains the record — `verify` still reads it on the cadence,
unchanged. A nudge lost to a restart, a saturated inbox, a closed handler or an energy
pause degrades to exactly row one. So arrival is an **optimisation over cadence, never a
replacement for it**, and nothing on this path has to be reliable for a duty to be kept.
That is the property that lets the door drop traffic rather than hold a listener open.

**The ledger authorises.** A delivery names a `start_key`, and a key no `serving` task
claims is dropped. This is not a door for making working sessions; it is a door for
reaching one the ledger already says should exist.

**Cognition is not in the path.** The handler takes its traffic straight from the inbox
and its per-message report is dropped, so routine traffic wakes no rung. Its owner is
Cognition so that what it *chooses* to raise — a decision that is not its to make,
something needing the person — has somewhere to land, over `send_message` like any
worker. Reaching the person is still two hops and both are load-bearing: Cognition
decides whether it is worth saying, Reaction decides when the room is right.

The handler is a **cache, not the carrier.** What persists is the ledger entry and the
listener's rows; the session is re-derivable from both, and **the facet is the brief** on
a cold open. So it may be closed once its errand is done or die with the process, and the binding
from key to session is held in memory and never written down — a delivery to a session
that has gone is not an error but the signal to open a fresh one. A burst therefore
continues in one warm session with full context, and a message the next morning starts
from the ledger.

Two clocks, and they are not the same kind of thing. The **settle** is the reaction
loop's own commit-after-quiet, shared value and all: six lines pasted into a group are
one thing to react to. The **floor** is a cost ceiling — an LLM turn per arrival is
affordable for a person typing and is not affordable for a busy group, and the 402 gate
is a cliff rather than a brake. A cap bounds the settle, because a trickle just faster
than it would push the deadline forward forever and a watch that is permanently about to
handle something is worse than a slow one; the floor outranks the cap, because spending
is the failure waiting cannot undo.

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
that the inbound text route journals `Origin::Human` and appends a message to the
conversation. A timer firing into it would put **a line the person never wrote** in
their chat, above the agent's reply to it. Both shapes above avoid it by not needing a
door at all. If a genuine need appears, the answer is a channel that says what it is
(`Channel::Clock`, already in `NON_ACTIVITY_CHANNELS`), never this one.

#### What this costs, stated plainly

A deadline is met **at the next glance, not at its minute** — `due` is read by the
projection and orders what is shown, and nothing in the host fires on it. At a
30-minute glance-up that is fine for a filing deadline and wrong for a wake-me-at-07:00
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

[`agents.md`](agents.md) for what the host drives ·
[`data.md`](data.md#tasks) for the ledger a glance-up reads, and the `verify` contract.
