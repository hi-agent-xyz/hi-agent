# Host — the Rust host

## Goal

Be the part of the system that is always awake, always fast, and never thinking: carry
signals into the one conversation, decide *when* the agent may speak, own every process and
the cadence that [opens the agent's eyes](#glancing-up), and write everything down before
anyone reacts to it.

Nothing here waits on a model to keep working. That is the point — the host has to keep working
while the thinking layers are slow, confused, or dead. Where it does ask one — the
[room screen](#the-room-screen) and the pre-send check
([`legibility.md`](legibility.md#e-pre-send-check--typed-questions-to-system-one)) — it asks
System One a typed question on a budget, and a timeout or an error is what the host did before
it asked.

## Decisions

| Decision | Reasoning |
|---|---|
| **There is one conversation, and it has no name** | The agent is one mind talking to one person, continuously. A partition key would have to be assigned by someone — and every candidate (the browser, the device, the surface) names a *client*, not a situation. See [One conversation](#one-conversation) |
| A client is a connection, never an identity | Clients attach and detach. Nothing a client sends may decide what the mind knows, because a client cannot know that |
| One mouth, one floor | Many sub-minds may think; the person hears one voice, one utterance at a time |
| A vendor outage is decided process-wide, not per turn | One upstream, decided once — never rediscovered or apologized for twice |
| The reflex path never reaches a model | Stopping when someone starts talking cannot wait a generation |
| A grooved action is a script the agent wrote, not a rung in the core | Recognizing a field and replaying a click is one implementation per windowing system and zero per idea; the idea is the same everywhere, so it belongs in a note and a tool. See [`mechanisms.md`](mechanisms.md#computer-use-does-not-cross-this-seam) |
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
- **Turn-taking** — still host-side, and it happens at **release**, not before Reaction is
  woken. See [The floor](#the-floor); the quiet-settle timer that used to be named here is
  [batching](surfaces.md#batching) and nothing else.
- **Social timing** — when to say a worker's answer, when to let it wait.

*The fourth duty, a **presence gate**, has been retired outright rather than moved — see
[Attachment](#attachment).*

What the host keeps is what has no model in it: the prepared sets and the floor that releases
them; the fact that `hi_prepare` **returns**, so an over-long line or one the check sent back is
answerable in the turn that wrote it;
and the calls that are not judgments at all — **not synthesizing speech for a speaker that
isn't attached**, and **not speaking into a room the person is still using** — because both
are facts about the wire, not reads of the room.

### The floor

**Reaction writes; the floor decides when, and whether, it is said or shown.** Nothing that
reaches the person — a line or a view — goes out at the instant Reaction writes it. Each is an
action inside a [prepared set](agents.md#prepared-actions), with the moment it is for, and the
host releases it when the room reaches that moment and it still fits. A view put up while they
are reading something else is the same interruption as a line said over them, and is held the
same way. (`send_message` to another agent is not on this floor: it reaches no person, and is
called directly.)
Thinking never waits on the floor, and the floor never waits on thinking.

This replaced a gate that could only say *now* or *never*. A line was checked against the room
when it was ready, and refused if they were talking, typing, or had said something the turn had
not seen; a refused line was simply not said and the next turn wrote it again. Measured over a
month (08-24 → 09-24), that gate refused 370 of 1838 lines, 275 of them because a line had
landed mid-turn — and the pauses of someone in the middle of a thought (median 15.5 s between
two of their lines, p75 29 s) are shorter than a generation (18 s to a first line), so a turn
written during a run of questions was refused almost by construction. A backstop let one line
through after three refusals in a row; 39 times it did, and what it let through was whichever
line came fourth. On 09-24 that was an aside about a deploy, sent into the middle of a run of
questions about a paper as the only thing said in reply to them, while the answer that turn
had led with stayed refused. **The backstop is deleted with the gate, not re-tuned**: a line
that is held instead of dropped has no loop to break out of.

#### The three states of the room

| The room | Told by | What the floor does |
|---|---|---|
| **They are talking or typing** — a recognized partial within the last ~1 s, or a keystroke in an unsent draft | the wire; no model is asked | nothing goes out |
| **They stopped, and are done** | the reading below | what is ready for `finished` goes, in the order it was prepared, each set only if it still fits |
| **They stopped, and have more** — a pause for breath, 然后另外…, the middle of a list | the reading below | what is ready for `paused` goes if it fits *this* stop — a short acknowledgment Reaction wrote for this conversation — and the `finished` sets wait |

Only the first split is mechanical. *Stopped* is a fact — the batch settled — but *done* is not:
someone mid-thought and someone finished produce the same silence, so the difference is read
from what they said. It is one question with one cut, not two: whether a pause earns an
acknowledgment or plain listening is not a second band on `finished` but `fits` asked of the
`paused` set itself — 收到，还有要补的吗 fits a stop after a finished question in a run of them,
and not a stop after 然后另外.

**A stop is a state, not only an event.** A set prepared while the room is already stopped — a
turn woken by a worker's report while they sit quietly, or one still thinking after they
finished — is read at once against the latest reading, not held for a stop that will not come.

#### One reading, at every stop

When a batch settles ([surfaces.md § Batching](surfaces.md#batching)) — they have stopped, for
now — the host asks System One once, over the recent lines with their ages and what the agent
said last, every question the moment raises:

- **`finished`** (`noul`) — have they finished what they were saying, or stopped with more to
  come?
- for each prepared `finished` or `paused` action set, **`fits`** (`choice`: `say` · `hold` ·
  `drop`) — against what they have said *since it was written*: still right and wanted now;
  right but not for this moment (an aside while they are in the middle of something else); or
  made wrong by what they said since;
- the [content branches' own questions](agents.md#a-message-picks-at-most-one) — `which`,
  `qualified`, `on` — when the batch carries a message of theirs.

`finished` at or above its cut releases `finished` sets; below it, the stop releases a `paused`
set if one is ready and `fits` says `say`, and otherwise the floor keeps listening. A set judged
`hold` stays where it is for a later stop; `drop` ends it, with the reason. A timeout or an
error is what the floor did before it could ask: a set written after everything they have said
goes out if they are not talking; one written before a line of theirs stays held.

The same reading is what makes holding safe. The argument against held drafts was that what
releases one — their falling silent, having just said something — is exactly what makes it
stale. That was true of a draft released blind. A held set is released only after `fits` has
read it against what they said since, so staleness is judged per set, not assumed of all of
them.

#### The one wait

A stop read as *has more* releases no `finished` set. If they then say nothing, what is ready must
still go — someone who trails off has said a last thing and will say no other. So **one wait is
armed by a stop that did not release `finished`, and cancelled by anything they say or type.**
When it runs out, the `finished` sets go through the `fits` reading as if they had finished.

Its length depends on one fact the floor has: whether an acknowledgment went out. If it did, they
have been asked whether there is more, and a short silence answers no — `pause_release`, 8 s. If
not, they are left to finish their thought — `thought_release`, 50 s, about the p90 of the pauses
above.

This is a timer, and it passes [the test](#the-upkeep-sweep) for one: the wait costs no turn.
It runs only after a stop, only while something is ready, and what it does when it runs out is
release a line already written. Both numbers are starting values to be read off the pauses
that follow, not settled ones.

#### What Reaction learns, and when

What happened to its sets — said, held and why, dropped and why, an action that failed — goes
back to Reaction **without a turn of its own**:

- **into the running turn**, when one is running, by the same `turn/steer` that carries their
  new lines into it — so a turn that is still thinking knows a line has gone out and does not
  write it again;
- **with the next wake**, whatever wakes it — their message, a worker's report, mail — as a
  section of that turn's window;
- **after a minute of quiet, only if something is waiting on its judgment** — a set held
  `hold`, or an action that did not happen as written. That is the one case with nothing else
  coming to carry it, and a minute of quiet after they finished is the moment an aside was
  waiting for. A set that simply went out is carried by the next wake and never costs a turn,
  and so is a line stopped by the [cap on messages since their last](legibility.md): nothing
  Reaction decides can send it until they write, and their message is the wake that carries it.

#### Their lines reach the turn that is thinking

A line of theirs that lands while Reaction is mid-turn is **steered into that turn**, not held for
the next one. The turn writes against what they have actually said, and what it prepares
replaces, matter by matter, what it prepared before. The counter that used to refuse a stale
line is still kept — it is what `fits` is asked against — but it no longer refuses anything.

**The same "are they talking" fact has a second reader, upstream.** The batching window is
held open while a voice is going, capped — see [surfaces.md](surfaces.md#batching). Holding a
set does not stand in for it: a turn started on a third of a question still thinks for
thirty seconds and still hands an errand down, and neither can be held.

### The room screen

**A batch that is only room wakes Reaction only if someone in it is talking with the agent.**
A microphone in a room hears the whole room, and until this existed every line it finalized
drove a turn: side talk, a child, a phone call, the car's navigation, a face crossing the camera.
Perception still hands all of it up and nothing is dropped
([`surfaces.md`](surfaces.md#channels)). What moves is the wake.

| | |
|---|---|
| **What is screened** | a batch made only of `audio` and `vision` — the [ambient](signal-attribution.md) channels. Anything typed, handed, mailed or reported in the batch wakes Reaction as before, and so does a batch the loop is retrying |
| **The question** | one System One `noul`: is someone in the new lines talking *with* the assistant — asking, answering, dictating, reviewing its work, playing a game it is in — even without naming it. It reads the last twenty-five lines before the batch with each one's age in seconds, and when the agent last spoke. The wording is `src/identity/judges/room.md` |
| **When it is asked** | at every arrival that changes a room-only batch, while the batch [settles](surfaces.md#batching). By the time the settle closes, the answer for exactly that batch has usually been in flight for the whole window |
| **What a no does** | the batch does not start a turn. Its lines wait, in order, and ride into the next turn something else drives, under `## Heard around you` ahead of `## New signals` — so a line that *was* for the agent is still in front of it, and *"did you hear me?"* wakes it with the original right there |
| **What anything else does** | wakes. Off (`room_screen`), unconfigured, over `room_screen_budget_ms` (1.5 s from the question), an error, or an answer that is not a probability — the batch goes to Reaction as it always did |

**Why the cut is 0.30** (`room_screen_wake_at`). Measured offline on 2026-09-21 over the 650
room-only batches in the frame log from 08-15 to 09-20, each labelled blind by a model reader —
not by the person. 297 were someone talking with the agent and 322 were not. At 0.30 the
screen set aside 164 of the 322 and 1 of the 297, the opening line of a review (*"I'm looking at
this summary now"*), which the next line would have brought back. At 0.40 it set aside 71% of
the side talk and 11 of the 297. The cut was chosen on the set it was scored on. The first
wording, which asked one question per line with no timing, could not tell a dictation from side
talk at all — a request to lay out a poster scored 0.10–0.15, the same as a family at dinner —
and the difference was the lines' ages and when the agent last spoke.

**What it costs.** One call per arrival in a room-only batch, about 1.6K input tokens. Measured
one call at a time from a dev machine through the managed gateway: p50 580 ms, p90 1.06 s.
Asked during a 700 ms settle, the median call adds nothing a person would hear; the slow tail
adds up to the budget.

**What it does not do: decide whether a room line makes a prepared set stale.** A room line that
lands while a turn is generating is steered into it like any other, and at the next stop the
[`fits` reading](#one-reading-at-every-stop) reads the set against it. That used to be the larger
half of the harm measured in the one crowded scene looked at closely — on 08-30, five of eight
replies were refused because of side talk — when any line at all refused a reply. Now side talk
costs a reply only if `fits` reads it as changing what the reply should be, and the screen's own
answer for the batch rides in the same reading. Whether that is enough is for the next crowded
scene to show. See [Open](#open).

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
one `say` that went out. **One `say` is one message, whole** — the action already carries its
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

**Barge-in and the attention gesture** live here: when someone starts speaking, sound stops
mid-syllable and the unspoken tail is discarded. A generation is far too slow for that, which
is the whole justification for a rung that cannot think.

**Taught quick-actions used to be the rung's other occupant, and are deleted.** The idea was
a small repeated thing the person showed the agent once — recognized against the
accessibility tree and replayed with synthesized input, no model asked. Two things killed it
together. Its machinery needed four OS-backed capabilities in the core, and *those* are gone
because driving a machine is a note over that machine's own tools
(`mechanisms.md` § *Computer use does not cross this seam*). And it never worked: the
authoring tool was advertised to no role, so the store was permanently empty, the recognizer
permanently abstained, and it fired exactly zero times in its life.

**When it comes back it is expected to live mostly outside this repo** — a learned script or
a tool the agent writes for the machine in front of it and leaves beside the note
(`tools.md`, `equipping-a-tool.md`) — rather than a recognizer rebuilt in Rust. The rung
itself is not in question; what is in question is whether a *grooved action* needs core
machinery, and the answer so far is no.

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
hour and codex's own token accounting says its window is at least half full. There is still no ceiling of our own on the window, no character counter, and no swap — the one number the host counts itself is image bytes, below, which the agent does not count at all.

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
the timing request above grow into one: **for how full the window is**, it thresholds on a
number the agent reports about itself, and the moment it starts thresholding on anything we
counted, it is the retired mechanism wearing a new name.

**Bytes are not that number, and nothing reports them.** The agent budgets its thread in
tokens, and in tokens an image is small — codex estimates about 1,800 for one whatever the
file, which is roughly what providers bill — so a thread can sit comfortably inside its
window while every request carries tens of megabytes of base64 it re-sends on each step. On
2026-09-14 a single request was 24.1 MB, 23.5 MB of it eighteen PNGs a worker had looked at,
and the gateway it went to held whole bodies in memory and fell over. Counting those bytes
is not second-guessing the agent's context number; it is measuring a quantity the agent does
not measure at all. So the codex adapter counts **image bytes a thread carries** — from the
image items that already cross it, reset when a compaction takes them out — and when a turn
ends over an **8 MB budget**, the host asks for the same in-place compaction the sweep does.
The window rule above is untouched: context size is still only ever the agent's own number.

What still replaces a session from out here is **failure, not size**: a turn that errors discards
the possibly-wedged session and the next one cold-opens. That is always survivable, because
**the state a rung needs is re-projected into every turn** — what is owed, what it carries
forward, who it can reach — and the [log](#the-log) is the durable backstop. A cold open loses
the thread, never the truth. The session carries the thread; `data/` carries the truth.

### § Decisions

- **The image budget is checked when a turn ends, not during one.** A compaction needs the
  session's turn, so asking mid-turn would mean interrupting the work to tidy it. What the
  end of a turn buys is that images stop riding along from one turn to the next — replayed
  over 2026-09-11..14 that roughly halves the image bytes re-sent, at 22 compactions in four
  days. **What it does not bound is one long turn**: the heaviest on record viewed 36 images
  inside a single turn, and every request in it carried all of them. The budget sits well
  under the smallest request cap we route to (32 MB) to leave that room, and a turn that
  outgrows it still fails at the vendor.
- **The count starts at zero on a resumed thread.** It is built from this process's own
  frames, so a thread reloaded after a restart with images already in it compacts later than
  one that never left. Seeding it from the resumed history is possible and not done.
- **A file codex viewed is counted at its size on disk.** Codex shrinks anything past 2048 px
  before sending, so a large photo counts high. The budget is a trigger, not a meter, and
  high errs toward compacting.

### Vendor gate

One **process-wide** view of whether the upstream model is reachable, read before a turn is
taken. The vendor is a shared resource, so an outage discovered by one rung must steer every
rung — Reaction, Cognition, Reflection and every worker share one upstream.

- **It gates the turn, not the reply.** During an outage no generation starts at all;
  incoming mail is held rather than answered badly or dropped.
- **Every rung reports, and every rung asks.** One upstream means one gate: a rung that
  fails tells it, and a rung about to open a session asks it first. The alternative was
  tried by accident — the classifier grew on the conversation loop alone, so through a
  real 38-minute outage Cognition bought a fresh subprocess every two minutes to
  rediscover the same wall, ten times, while Reaction sat correctly parked beside it.
- **Backoff, absorbed and capped.** A blip is absorbed before anything is declared down; from
  there the retry gap doubles to a ceiling. A rate limit is not an outage worth mentioning; a
  string of failures is — and the absorb count *is* that sentence, so nothing else needs to
  encode it. **The agent runtime retries once, not more.** Its own retries are a second
  absorb layer below this one, per session and invisible to the gate, and each attempt
  re-sends the whole thread: left at codex's defaults a single failure went out six times,
  and one duty worker put 38 turns of that — ~190K tokens a request — into a broker that
  was already down. One retry absorbs a dropped connection; anything longer is this gate's.
- **While down, one rung tries.** When the backoff deadline passes, the first rung to ask
  takes the attempt and every other rung keeps holding. Waking them all on one deadline
  rediscovered a single outage once per rung in the same instant, each with a whole thread
  in the request. Only the probe's failure grows the gap: a turn that was already in flight
  when the gate went down fails after it for reasons the gap has already accounted for, and
  a dozen of those used to carry a thirty-second backoff to the one-hour cap inside one
  outage. A probe that never reports gives the attempt back after a lease, so a session
  that died mid-probe cannot park the rest for good.
- **The upstream answering is what reopens it, not a turn finishing.** Any model response
  on any session is proof, and it arrives with the first request of a turn rather than at
  its end — so a probe that turns out to be a long errand does not hold everyone else
  parked while the upstream answers it.
- **One apology, once — and it is a state, not a sentence.** The transition, not each
  failed turn, is what earns a word to the person. What the gate publishes is therefore
  the **condition** — one of *unreachable* (still retrying), *out of energy*, *refused
  credentials* — which every surface renders and which comes off by itself on recovery.
  N rungs × M retries collapse to one publish because a repeat of the same condition is
  dropped where it is stored, not by each writer remembering.

  **It is not a message.** A host apologizing into the conversation would be a fourth
  thing that becomes one ([`text-transcript.md`](text-transcript.md) allows three), and
  the apology would then scroll away while still being true. It rides beside the
  recognition interim instead: current state, replaced not appended, carried in the
  opening frame so a window that connects mid-outage is told.

- **Every channel that is attached is told, not just the screen.** The screen had this
  for a year and nothing else did, so an outage was invisible to anyone reading the
  conversation — "correctly stopped and waiting" and "dead" are the same picture. The
  out-of-energy account view is still the screen's own, richer rendering of one of the
  three; it is not the mechanism.

When it clears, the held mail drives a catch-up turn. Fix-forward, like everything else here.

**And the mail is held on the way there, in every rung.** A turn that fails leaves what it
was carrying in hand. This is easy to get wrong in the direction that costs the most: the
conversation loop — the one rung somebody is actually waiting on — used to drop its batch
whenever the vendor had not yet been declared down, on the strength of a comment saying the
turn had already apologized. Nothing apologized, so what happened is that the person's
message was discarded, unanswered and unmentioned.

### § Decisions

- **The condition is not spoken.** A voice-only person is not told about an outage.
  Synthesis is a different vendor and would usually still work, so this is a real gap and
  it is left open deliberately: an interruption from the host, unprompted, over whatever
  else is happening, is a bigger decision than a strip above the composer, and no live run
  has yet shown what it should sound like.
- **The condition carries no deadline.** A retry gap that doubles would change the
  published state on every failed attempt, which is exactly the "N × M apologies" this
  section forbids. How long is still in the log, and for energy it is in the account view.

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
starts**, which is restart recovery. It fires when the ledger holds active work, or when the run
before vanished rather than stopped ([agents.md](agents.md#across-a-restart)) — so a clean restart
with nothing owed costs no turn, and the one boot that is not routine always gets one.

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
one that is quiet, has been quiet about an hour, and whose window is at least half full, and
compacts it. Every session is in scope — a worker genuinely idle an hour with a full window
is as worth tidying as a rung. **It does nothing while the [vendor gate](#vendor-gate) is
not open**: a compaction is a full-window model call, one that fails leaves the thread as
full as it was and so selects it again, and maintenance is never worth a place in the line
of rungs holding real mail.

**Why this is not the cadence removed three times over.** Those woke a rung to *judge* —
read the ledger and decide, look at the room and decide whether to speak — and the wake was
the cost: a full window read to reach a conclusion, 46% of them reaching none. This reads
integers off a map. A sweep that finds nothing costs a lock and a comparison per session,
and it produces a model call only when one is genuinely owed, for work with no judgment in
it. The test to apply to anything added here is that one: **does the tick itself cost a
turn, or only the case it finds?**

**It calls the session directly, and that took making two things true.**

*Sessions are actually host-owned.* They were not: a rung's handle was a local inside its
own loop, so anything wanting to touch a session had to be routed back through that loop as
a message — plumbing standing in for ownership this document already claimed. Workers had a
directory; the three standing rungs did not, and nothing needed to reach all of them until
this did. There is one now, holding `Weak` handles by slug, so registering costs an owner
nothing and dropping a handle still closes a session.

*A turn is a permit, not a race.* The single in-flight-turn slot was always single, but
losing it was an **error** — right for a caller prompting twice, wrong for anything else
wanting the session for a moment, and dangerous because a rung whose prompt errors drops its
long-lived session and cold-opens. So maintenance touching a session from outside could
destroy the thread it was tidying. Now `prompt` **waits** for the permit and `compact`
**steps aside**: a turn arriving during a compaction is delayed by it rather than failed,
and a sweep arriving during a turn does nothing and comes round again. Maintenance is never
urgent, so it is never the one that waits.

Neither of those is about compaction. They are what it costs to have anything at all that
acts across sessions, and the next such thing gets them for free.

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

**One wake is armed by something other than an arrival, and it is not this one again.** When a
prepared set is left `hold` at a stop, or one of its actions failed, and a minute passes with
nothing said and nothing arriving, Reaction is woken with that set ([What Reaction learns](#what-reaction-learns-and-when)).
It differs from `back_in` in what it waits for: not work that will report on its own, but a quiet
room — the moment a held aside was being held for — and nothing else is coming to carry it. It
is armed only while such a set exists and fires at most once for it. Whether it earns its turns
is the same count `back_in` was judged on: how often it fires, and how often what it produces is
the held line said rather than nothing.

Everything else an agent needs from time, **the agent arranges itself.** It has a
shell, so it starts the process it needs, parks a worker that sleeps and messages
home, or writes its own loop — and what it starts is a **child of this process
tree**, owned by the worker that owns the duty and dying when the engine does.
Nothing is restricted to keep it that way: the agent keeps every tool it has, and
what follows is a rule about what to do with them.

**"Dying when the engine does" is codex's to carry, and the host's job is to let it.**
Codex runs every command in a session of its own, so nothing done *to* codex reaches them —
but closed on its stdin, codex ends every process it started, a detached one included, and
SIGKILLed it ends none. So a codex is ended by closing its stdin, never by killing it, and
killed only if it has not exited within a short grace. An engine that dies any way at all
closes that pipe too, which is what covers a crash.

**What escapes codex is not chased, by decision.** A daemon that made a session of its own,
or the commands of a codex that had to be killed, keep running. Reaching them means inferring
ownership from the process table — marks in inherited environment, parent links, process
groups — and that was built and removed: macOS hides the environment of its own binaries, so
the inference was half blind, and a wrong one kills somebody else's process. An occasional
leak that the log names beats a mechanism that can take down the wrong thing. Revisit only
with a leak observed in a live run, not with a shape imagined here.

**Still owed a live run:** the closed-stdin behaviour was measured through `command/exec`,
which is not the path a turn's commands take.

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
- **Whether side talk makes a prepared set stale is left to `fits`.** A room line that lands
  during a generation no longer refuses anything; it is read with the rest at the next stop
  ([One reading](#one-reading-at-every-stop)). Whether `fits` tells a remark at the next table
  from a change of mind has not been measured — on 08-30 five of eight replies in the photo scene
  were refused by side talk, and that scene is the one to replay. `Talking or typing` is a
  different matter and stays as it is: talking over anyone is rude, whoever they are talking to.
- **The wait's two lengths.** `pause_release` 8 s and `thought_release` 50 s are starting values
  from the pause distribution on 09-24 ([The one wait](#the-one-wait)); the pauses that follow a
  `paused` release are what set them.
- **The host knowing what it consumes, and pacing itself on it.** Two kinds of consumption: the
  machine — memory, disk, processor, everything the sessions' commands start — and tokens. The
  intent is that hi-agent watches both and adjusts its own pace to the situation, the way a person
  slows down on a machine that is struggling or a month whose budget is nearly spent. Neither half
  exists. Tokens are metered and enforced at the gateway and said plainly when they run out
  ([foundation.md § Energy](foundation.md#energy)), which is running out, not pacing. The machine
  is not watched at all: nothing in `src/` reads a process's footprint, `rusage` or memory
  pressure.

  *The evidence.* On 2026-09-18..20 a worker's TrackNet `predict.py` decoded a whole 1080p clip
  into each of sixteen DataLoader workers, about 100 GB on a 64 GB machine. The machine panicked
  three times, and rebooted a fourth time minutes after the same command ran again. Nothing in the
  host noticed. A person found it by reading kernel panic logs,
  and the host now says it after the fact ([agents.md § Across a restart](agents.md#across-a-restart)).

  *What the design already has to meet:*
  - **Watching must not cost a turn; only what it finds may.** The same test the upkeep sweep
    passes ([§ Glancing up](#glancing-up)).
  - **macOS's own memory-pressure signal is not a trigger.** `memoryPressure` was `false` in all
    three panic logs. The kernel ran out of compressor segments first.
  - **Pacing only helps a host that is still running.** A job that exhausts the machine kills
    the watcher with it. Whether some bound has to be *enforced* rather than judged is part of
    the question. Linux (cgroup v2) and Windows (job objects) enforce one for every resource at
    once. macOS has no equivalent.
  - **Who adjusts.** The host can measure and hold a number. What to slow down is judgment:
    reflection's backoff, work taken ahead of being asked, how many errands run at once, which
    model a turn uses. That points at the host giving the agent its consumption as a situational
    fact and the agent deciding what to slow, rather than at a host-side policy.


## See also

[`agents.md`](agents.md) for what the host drives ·
[`data.md`](data.md#tasks) for the ledger a glance-up reads, and the `verify` contract.
