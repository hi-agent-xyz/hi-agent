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
| The host opens the agent's eyes; the agent owns its own timers — inside this process tree | A duty that outlives the engine is a duty nobody supervises. When hi-agent is down its machinery is down, and that is the intended behaviour, not a gap |
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
browser tab, by a screenshot from the ⌘⌘ gesture or the phone's Action Button, or by a
file from a phone. Everything
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
> to", the isolation unit keying a Reaction, a Deliberation (the rung then between
> Reaction and the brain), a memory slice, and a tag on
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
- **Turn-taking** — still host-side, and it happens at the **mouth**, not before Reaction is
  woken. See [The floor](#the-floor); the quiet-settle timer that used to be named here is
  [batching](surfaces.md#batching) and nothing else.
- **Social timing** — when to say a worker's answer, when to let it wait.

*The fourth duty, a **presence gate**, has been retired outright rather than moved — see
[Attachment](#attachment).*

What the host keeps is what has no model in it: the queue behind `hi_say`; the fact that
`hi_say` **returns**, so an over-long utterance — or one the floor refused — is answerable;
and the calls that are not judgments at all — **not synthesizing speech for a speaker that
isn't attached**, and **not speaking into a room the person is still using** — because both
are facts about the wire, not reads of the room.

### The floor

**Whether Reaction may speak is decided when the words are ready, not when the turn that
wrote them began.** A generation takes seconds and the room moves inside them. So `hi_say`
is gated at the mouth, on two facts the host can check and the model cannot:

| Refusal | The fact | What it catches |
|---|---|---|
| **their voice is sounding** | a recognized partial within the last ~1s | starting on top of a sentence that has not finalized yet |
| **a line went unheard** | lines accepted since this turn's batch was frozen | a reply released into a real two-second gap, but written without the sentence carrying their actual point |

Both mean *not said*, and `say` answers with which. **A refusal is not a hold.** Nothing is
queued, released later or superseded: the words are simply not said, and the reply is written
afresh by the next turn — which costs nothing, because whatever they said is already in the
queue and drives a turn by itself. That is also why the host needs no "they have stopped"
signal and the client is never asked for one: **to stop talking they must have said a last
thing, and that utterance is the wake.**

What to do about a refusal is `reaction.md`'s. The host says which of the two it was and
stops there.

**The same "are they talking" fact has a second reader, upstream.** The batching window is
held open while a voice is going, capped — see [surfaces.md](surfaces.md#batching). The gate
here cannot stand in for it: refusing a `say` unsays nothing the turn already *did*, and a
turn spent on a third of a question still thinks for thirty seconds and still hands an errand
down. One fact, two stakes: upstream it saves a generation and a dispatch, at the mouth it
saves the person from being talked over.

**Why the words are refused rather than held.** A held draft would go out about a second
after the room clears instead of a generation later, which sounds like the better trade until
you ask what releases it: they fall silent *having just said something*, and that something
is what makes the draft stale. The payoff case needs them to stop without having spoken since
the draft was written — the rare one. So holding buys latency in the case that barely happens
and costs a supersede rule, an expiry rule and a release timer in every case that does.

**One bound, because a refusal has no ceiling of its own.** Someone who speaks during every
generation refuses every reply, and total silence is a worse failure than a slightly late
line — so after a few refusals in a row, one goes through. That is the mechanical form of
what a person does as the wait grows: stop holding out for a clean opening and take a small
one.

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
existed to deliver; and `hi_say`'s answer about where the words landed. A due check-in now
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
one `hi_say` call. **One `hi_say` is one message, whole** — the call already carries its
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
window overflows. **Bounding that is the underlying agent's job. Choosing the moment is
ours.** The agent behind a session compacts its own context in place, automatically, near its
real window; that automatic trigger stays, and stays as the last resort. What was added is a
request for the same operation at a moment somebody picked — `thread/compact/start`, at the
far side of the [upkeep sweep](#the-upkeep-sweep), once a session has been quiet about an
hour and codex's own token accounting says its window is at least half full. There is still no ceiling of our own, no character counter, and no swap.

**Why that is not the mechanism this section retired**, and the difference is two facts about
the wire rather than a change of mind. Both halves of "we cannot" below were true when they
were written and are not now. We *can* see the context: `thread/tokenUsage/updated` reports
`last.inputTokens` against `modelContextWindow` on every request — codex's own count of the
whole thing, system prompt and tool schemas included, not the drifting fraction a byte
counter out here could reach. And we *can* compact in place: `thread/compact/start` runs the
same in-thread compaction codex runs itself, keeping the thread id, its rollout and
`thread/resume` (verified against the 0.147 pin). Nothing is summarized out here, nothing is
reopened, and the working thread a long-lived rung exists to keep is the thing that survives.

What that buys is only timing, and timing was the whole complaint: codex compacts when it
notices, which on 2026-09-02 was 29 times across eight sessions, six of them inside a single
worker's turns. Asking early costs a model call at a moment nothing is waiting; being asked
late costs one in the middle of the work.

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
numbers are visible. Do not re-introduce a character counter at this layer, and do not let
the timing request above grow into one: it thresholds on a number the agent reports about
itself, and the moment it starts thresholding on anything we counted, it is the retired
mechanism wearing a new name.

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

**The host has almost no timing surface, and what is left is not a cadence.** One loop
paces itself — the **reflection backoff** for consolidation, which is memory settling rather
than attention. Beside it sits a single wake: **Cognition, once, shortly after the process
starts**, which is restart recovery.

**There is no recurring glance-up, and it was removed rather than tuned.** A fixed period is
what a design reaches for when it has no event for something, and the events exist: input,
mail, a worker's report, a restart. What the period actually bought was measured across the
frame log — 1819 turns driven by that timer alone, of which 46% made no tool call and ran no
command at all, against 28% for turns something had asked for. A wake that reads a full
window to conclude nothing is the most expensive nothing in the system, and it stayed in the
thread afterwards. The failure usually cited for keeping it — a standing duty lost across a
restart ([gaps #1](../user-journeys/gaps.md)) — was fixed by the boot wake, which survives.

The ledger is therefore read when something finishes rather than every half hour, which is
the shape a person's own attention has: you look at your list because you finished a thing,
not because a bell rang. Anything that genuinely must happen on a period runs its own loop —
a worker that owns the process. **Nothing at all fires at a named time, and only one thing
fires on a period: the upkeep sweep, which wakes no agent to think.**

#### The upkeep sweep

Every ten minutes, host code walks the sessions the switchboard already holds and looks for
one that is quiet, has been quiet about an hour, and whose window is at least half full. It
rings that session's bell; the loop that owns the session compacts at its own next pass.

**Why this is not the cadence removed three times over.** Those woke a rung to *judge* —
read the ledger and decide, look at the room and decide whether to speak — and the wake was
the cost: a full window read to reach a conclusion, 46% of them reaching none. This reads
integers off a map. A sweep that finds nothing costs a lock and a comparison per session,
and it produces a model call only when one is genuinely owed, for work with no judgment in
it. The test to apply to anything added here is that one: **does the tick itself cost a
turn, or only the case it finds?**

**It decides *whether*, never *when*.** A compaction takes a session's single
in-flight-turn slot, so a sweep holding the handle would collide with that loop's own
`prompt` — and a rung whose prompt fails drops its long-lived session and cold-opens,
losing the thread the design keeps it for. So the sweep rings and the loop acts, which
makes the race structurally impossible rather than unlikely.

Both numbers are deliberately loose. Ten minutes against an hour is slack nobody can
observe: the question is "has this been quiet for about an hour", and no reader downstream
can tell sixty minutes of silence from seventy.

**Reaction lost its cadence first, and the reasoning generalised.** A pulse used to wake the
conversation loop on the same knob and run a turn into an empty room. Every argument below
was later found to apply to Cognition's glance too, which is why neither has one now. Reaction is
tools-off, so the wake handed it nothing it could not already see in the window it gets on
*every* turn — the least-informed rung was the one deciding whether to speak. The journeys
measured what that produced: two post-restart pulses, both concluding without a `hi_say`,
while a standing duty sat unread in the ledger ([gaps #1](../user-journeys/gaps.md)). It
was also the most expensive wake in the system, because the projected window rides every
turn and accumulates in the session. Unprompted speech comes instead from the rung that
can actually check — Cognition glances up, reads the ledger, and messages Reaction, which
is invariant 5 doing its job. **Reaction wakes on two things and no clock: input from
the person, and mail from another rung.**

#### There is no timer, and the last one to go was the agent's own

Reaction used to set one by naming a number in `hi_say`'s `back_in` — the same number it
said out loud ("give me ten minutes") — and the host woke it when that was up. It was the
best-shaped timer this design ever had: one slot, one deadline, no target, no payload, armed
only by an utterance, so a wake could never be set for a number nobody was told. It fired
zero times if Reaction never promised anything. Of the three wakes measured here it was also
by far the most productive — 87% of its firings produced speech, against 11% for the
host-armed check-in beneath it and 24% for the glance-up.

**It went anyway, and on a measurement the 87% was hiding.** "Produced speech" answers
whether Reaction opened its mouth, not whether the words were worth it. Across the frame log
it fired 53 times, and the work it was waiting on reported a median **1.2 minutes** later —
42% within one minute, 90% within five. So what it bought was a line saying *"still going,
give me another five minutes"* a minute or so before the real answer arrived and drove a
turn on its own. That line is exactly the empty check-in `reaction.md` spends a paragraph
forbidding, and each one armed the next, which is why it kept firing.

It could not be fixed by asking Reaction to promise better. Naming a number that survives
contact requires estimating how long an unfinished thing will take; when the estimate runs
short — which is most of the time — the timer necessarily fires just before the answer.

So a number Reaction names is a **forecast it will be judged on**, not a hook. What ends a
silence is the work coming back, which drives a turn regardless, and if that never happens
the person asks. `reaction.md` states that consequence rather than the host absorbing it,
which is where a judgment belongs.

Everything else an agent needs from time, **the agent arranges itself.** It has a
shell, so it starts the process it needs, parks a worker that sleeps and messages
home, or writes its own loop — and what it starts is a **child of this process
tree**, owned by the worker that owns the duty and dying when the engine does.
Nothing is restricted to keep it that way: the agent keeps every tool it has, and
what follows is a rule about what to do with them.

##### One background item, and it is this process

**Do not register OS-level keepalive** — no `launchd` agent, no crontab, no systemd
timer — unless the person asks for one. Not for want of the shell to do it, but for
what it produces: a fixture that outlives the app, keeps firing after the row that
wanted it is closed, and reaches the person as a background item they never
installed and a notification about it. One of them woke every sixty seconds for
sixteen days after its task closed `done`; closing the row did not touch it, and
nothing in the engine knew it was there.

And the thing it buys is not wanted. A duty is **managed**: if the engine is down,
its machinery is down, and what it would have done does not happen. That is the
behaviour to keep — a deploy loop still running while the mind that authorises it is
gone is the failure, not the coverage gap.

Two things follow, and a `serving` row is not sound without them:

- **A duty catches up on start.** It was not running while the engine was down, so
  its machinery keeps a cursor in its own ledger and fetches what it missed rather
  than assuming it saw everything. A duty that can only work by never stopping cannot
  be kept by a desktop app at all, and belongs in a server-side deployment.
- **`restart:` is the way back up.** The glance-up reads it on the cadence and brings
  a duty back the way it repairs anything else. Nothing at the OS level is holding it
  up, so nothing at the OS level has to be found and removed when the duty ends.

When the person does ask for a system trigger, it is theirs — and it gets named in
the row that wanted it, with its exact label or crontab line, so whoever closes that
row can take it off their machine.

#### The three shapes, and what each needs from us

| Shape | How it runs | What the host provides |
|---|---|---|
| **Cadence** — check this every N hours | Cognition's glance-up *is* the executor: it wakes, reads the ledger, and does what is due. Or a process a worker started does the work and leaves a durable trace. | The glance-up, and nothing else. |
| **Precise moment** — be somewhere at 07:00 | A parked worker sleeps and `hi_send_message`s its owner; the ledger re-arms it after a restart. | `hi_create_worker` + the one verb. |
| **Arrival** — something reached the group | The agent's own listener holds the connection and posts what arrived to `/api/in/duty/<start_key>`; a working session handles it in seconds. | The duty inbox: coalesce, resolve the key against the ledger, open a handler from the facet if none is live. |

The first covers the standing duties this system actually has. The second is rare
and costs an idle subprocess, which is the right price for something rare.

##### Arrival, and why it does not weaken any of the above

A duty was reactive at its edge and unread at ours: the listener received a message the
instant it was sent, wrote a row, and nothing read that row until something else happened to
wake the rung that could. The third row closes that, under three constraints that
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
something needing the person — has somewhere to land, over `hi_send_message` like any
worker. Reaching the person is still two hops and both are load-bearing: Cognition
decides whether it is worth saying, Reaction decides when the room is right.

The handler is a **cache, not the carrier.** What persists is the ledger entry and the
listener's rows; the session is re-derivable from both, and **the facet is the brief** on
a cold open — capped, like the arrival it is pasted above. A duty's record has no end: one
live facet reached 375 KB, all of it carried into every cold open beside an arrival clipped
at 8,000 characters. The cost that matters is not the tokens. **A handler's example of how
to write a line is the last few hundred it was shown**, so an uncapped record teaches each
new session to write the record it already is. What rides is the head of the account (whose
newest reading is on top), the tail of the running record, the `created` line however far
back it sits, and a count of what was left out — the rest is a file the session can open. So it may be closed once its errand is done or die with the process, and the binding
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

#### The same rule, turned inward

**We hold the agent's duties to a result check and have never held our own loops to
one.** Reflection's backoff, the glance-up, the check-in deadline — each is standing
machinery that can stop doing its work while continuing to run, and each is exactly the
shape the rule above exists to catch. None of them carries a `checked`, nothing reads one,
and a rung that has consolidated nothing since Tuesday looks from every angle like a rung
with nothing to consolidate.

That is not a hypothetical: reflection did precisely this for thirty-five hours
([`data.md`](data.md#reading-back-across-the-pen-line)), and every observable stayed
normal — the loop woke on schedule, the process was healthy, the log line was a `debug`.
It was found because a person said the agent felt forgetful.

The fix is not a watchdog subsystem. A second mechanism watching the first is one more
thing that can be built, typecheck, and never run, and this repo has shipped that shape
before. The rule is the cheaper one:

- **A periodic loop stamps that it did the work, never that it woke.** "Slept and found
  nothing" and "swept and found nothing" are the same line today and must not be.
- **The loop that skips is the one that can tell.** Skipping is ordinary — a caught-up
  store *should* skip. What is not ordinary is skipping while new signals keep arriving,
  and that is a contradiction the loop can already see: [`reflection`](../../src/body/reaction/reflection.rs)
  reads `last_signal_at` on every iteration for its own backoff, and computes
  `last_activity > anchor` — fresh input since the last pass — one line above the sweep
  that comes back empty. It has both facts and has never compared them.
- **What code can repair, code repairs; what it cannot, it says loudly.** Re-deriving a
  cursor over only the ids that parse is repair. Anything past that is a `warn`, in the
  log, where [diagnostics belong](../../src/foundation/server) — never a card in front of
  the person. **And loud is once, not every pass.** A defect in a file the loop merely
  reads does not change between reads, so a `warn` re-emitted on the loop's own schedule
  adds nothing after the first and costs the thing it was for: a line that reprints
  forever is one a reader learns to scroll past, and being scrolled past is how the
  `debug` above hid a stalled sweep for thirty-five hours. Report a standing defect once
  per boot, per thing.

**A self-healing system whose only healer is the agent has no floor.** Reflection is the
rung whose whole job is the agent's own house, and reflection is what broke;
[`data.md`](data.md#prompts) hands it "rebuilding a missing seed" for the same reason, and
that owner was gone too. So the floor here is code's, exactly as the window's floor is the
log tail: the agent owns the judgment, and code owns the guarantee that there is a next
pass to judge with.

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

What used to stand here as the second cost — *nothing wakes Reaction when a promise
is running late* — is the check-in above, and it was removed for a reason worth
keeping in view. It read as a rough edge and was a broken product: Reaction named a
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

## Open

- **The barge-in stop path** — core-owned or forever client-side? Today it is in the browser and
  the backend hook is dead: `Floor::mark_flush` ([`floor.rs`](../../src/body/reaction/floor.rs))
  has no caller outside its own tests. Wire it or delete it; leaving it is the third option that
  keeps being taken.
- **`record_reflex` is declared to no role.** The recognizer and `POST /api/reflex/invoke` are
  live, so a reflex can be *fired* but never *written* — the authoring end is reachable by name
  only. Give it a live role or delete the module.


## See also

[`agents.md`](agents.md) for what the host drives ·
[`data.md`](data.md#tasks) for the ledger a glance-up reads, and the `verify` contract.
