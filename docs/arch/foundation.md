# Foundation

## Goal

Be everything the agent stands on and reaches with: the engine that runs it, and the tools it
acts through — devices among them.

Nothing here decides anything. It has to keep working while the thinking layers are slow,
wrong, or gone — and [`data/`](data.md), not this, is where the agent itself lives.

## The line

> **Producing this requires no judgment.**

A signal appended to the log, a config value resolving, an embedding being computed, a shell
command running — none of these need an agent to be correct. Everything requiring a decision belongs to an agent; foundation only carries it out.

## What it writes into `data/`

Foundation holds the pen for the mechanical subtrees: the log at `memory/raw/`, the whole of
the bundled `prompts/`, and the factory seeds under `skills/` and `views/`. Those are described
where they live, in [`data.md`](data.md#who-holds-the-pen).

It does **not** write the [generated seeds](data.md#prompts) — deciding
what an agent carries forward is judgment. It does install the bundled ones, inject the
generated ones every turn, and enforce their cap.

The pen is held **per subtree, not per top-level directory** — the log sits inside `memory/`,
whose other subtrees the agents write. Two writers under one roof is fine; two writers on one
file is not.

Wherever a subtree has both a factory and a learnt layer, the two stay physically separate,
so an upgrade replaces one and never the other.

## Engine

| | |
|---|---|
| Runtime | process management, the bundled toolchain |
| Agent wire / MCP | the agent wire and the tool surface, routed by session slug |
| Gateway + vendors | model access, credentials, energy accounting |
| Secret filing + broker | files credentials a person typed and substitutes their paths on the way into a session; resolves a drive-file reference inside a bound HTTP effect |
| Config cascade | layered configuration resolution |
| Store I/O | the read and write paths under every part of `data/` |
| Prompt assembly | installs `prompts/factory/` from the binary at boot; hands each rung its [seed](data.md#prompts) and truncates it, audibly, at the cap |
| Build pipeline | esbuild and headless-browser rendering for views |
| Logger / observatory | operational visibility — [not the log](#observatory), and the difference matters |

### Observatory

A **live operational mirror**: turns, sessions, workers — what the machine is
doing *right now*, structured enough to answer "why is it quiet" without trawling output.

**It is deliberately not [the log](data.md#memoryraw), and those two must not be merged** —
this is the one place that distinction is argued, and everywhere else points here. The log is
a durability mechanism: append-only, both directions, authoritative for recovery. The
observatory and the logger are debug surfaces: lossy and disposable by design. Merge them and
you get either a log shaped by what is convenient to display, or a debug view nobody dares
drop. Both are worse than keeping two things that happen to look alike.

Like every debug surface here, it shows [ground truth](#debug-surfaces) — protocol fields
without per-event interpretation, but never raw private values. Redaction is a boundary rule,
not an interpretation of what an event meant.

### Energy

What a turn costs and what is left: quota and spend, tracked per account and enforced at the
gateway, not by the thinking layers.

**Running out is a first-class state, not an error.** It is knowable in advance, it recovers
on its own clock, and the person can do something about it — so it is surfaced plainly and
early rather than as a failed turn. The failure this exists for is an agent that goes quietly
useless and does not say why.

---

# Tools

There is no separate general "effector" layer. From the agent's side everything is a tool
call, and `Bash` is the effector for most of the world. Private values are ordinary local
drive files: Bash may consume them by path, while a foundation broker offers a safer
destination-bound HTTP path. Exact values are projected before external-model transport.
See [`privacy.md`](privacy.md).

## Decisions

| Decision | Reasoning |
|---|---|
| Per-role tool surfaces are a **context optimization**, not a boundary | A smaller surface is a smaller window and a faster turn. What actually divides the roles is who does the job, not who *could* call what |
| Classify by **who had to be present** | That, not what the tool is, predicts whether a job stalls |
| Operate UI where no API exists | A person with an account can do it; so can we |
| A missing tool is part of the task | Degrading to a web search when the answer was "install it" is a failure |
| No silent outward action | Say after, don't ask before — a gate cannot tell what was just asked for from what nobody mentioned, so it stops both and parks the row on a person |
| General vision over per-app scripting | See-and-click works on any app; a script works on one |

## The agent session registry

**Every agent-to-agent edge goes through one code component with no model in it.** Agents do
not hold references to each other; they hold *addresses*, and the registry resolves them.
That is what makes "the switchboard is the host" a mechanism rather than an aspiration.

| Call | Who | Contract |
|---|---|---|
| `SendMessage(to, message)` | every agent | One direction, **no reply**. Returns whether it was *delivered*, never a response. Queues per target and merges while the target is mid-turn, so a burst arrives as one prompt |
| `CreateWorker(title, task, type, subject)` | Cognition, Reflection | → a session slug. `title` is the errand in one line and `task` is the brief — see below. `subject` is the ledger task this errand serves, and is what makes "is anyone on this task" a lookup instead of a reading; it is **required** for every type that serves the ledger and **refused** for the two that serve no single task (`task-manager`, `person-reader`). It must name a row that already exists — the call opens none, and a miss comes back with the open ledger to pick from ([agents.md](agents.md#cognition--minutes-and-beyond)). There is no `resume`: an errand a restart interrupted is reopened by the host on its own thread, under its own slug, before anyone is asked ([agents.md](agents.md#across-a-restart)) |
| `SessionStatus(id)` | owners | alive · busy/idle · what it was given · **what it was last seen doing** · **how its last turn ended** · turns. **Meta only** — free to call |
| `SessionMessages(id)` | owners | its actual output. Costs context, so it is a separate call from status |

**"What it was given" and "what it is doing" are two fields, never merged.** A task is set once
and does not change, so an owner polling status got the same sentence back whether its worker
was mid-command or wedged — and the output tail could not fill the gap, because it is fed only
from what a session *says* and a session can work for minutes in silence. Keeping them apart
also keeps tool noise out of `SessionMessages`, which is what an owner reads to learn what the
work found.

**And `idle` is two situations, so how the last turn ended is a third field.** Busy/idle is
folded from whether a turn is in flight and whether mail is queued, and neither says anything
about the turn that just ended — so a worker that answered its brief and one whose turn died
on a `429` are the same word with the same clock, on the roster, in `SessionStatus`, and on the
task's own ledger line. That is the silence-read-as-health failure one level in: not a session
whose liveness is unknown, but one whose *outcome* is. It showed on 2026-08-18 — three workers
failed inside two minutes, read as `idle`, and were told "Continue now; do not leave this idle".
So the switchboard keeps the ending of the last finished turn (`completed` · `failed` with its
reason · `interrupted`), and every surface that draws a quiet session draws it. It is recorded
by the loop that ran the turn, as an argument to finishing one, because a loop that can drop a
session out of `busy` without saying how is a loop that will.

**The title and the brief are two arguments, and only the title enters the switchboard.** A
brief for real work is a paragraph or five; the switchboard has no reader for a paragraph —
every one of them (the roster on the person's screen, `SessionStatus`, the line a reopened
errand is reported under, a `reachable` line in another rung's window) renders one line. Registering a session under its
brief therefore showed the brief's *opening clause*, which is setup and never the subject. So
the caller writes both: the title is what the session **is called** everywhere it is read, and
the brief goes out as the session's first prompt, whole, read by the worker alone. Neither is
derived from the other — a cut paragraph is exactly the summary nobody would have written.

**`from` is stamped by the registry, never passed by the caller.** The host knows who is
calling; letting an agent name itself is letting it impersonate.

**An address is a session slug. Nothing else.** One form, for every hop: a worker's id comes
back from `CreateWorker`, and the ids of the standing rungs are **projected** into the
window of whoever may reach them, the same way open tasks are.

That last part is the point. The alternative — letting an agent name a destination out of
an open set and having the registry resolve it — is retrieval: the agent produces a name and
hopes something is behind it. Retrieval can miss, and here a miss is indistinguishable from
"nobody is there". Projecting live ids inverts it: an agent is told who is reachable this
turn, so a rung that is cold is visible as cold *before* a message is sent at it, and the
registry goes back to being a map lookup rather than a search.

**A session slug is a slug, not an ordinal.** The three standing rungs are singletons, and
their ids are the names they already have everywhere else in this design: `reaction`,
`cognition`, `reflection`. A worker's is `<type>-<task>` — `view-builder-kyoto-trip`,
`person-reader-alice` — falling back to its title when the errand serves no ledger task.
This is a spelling, not a second address form; there is still exactly one form and it is
still projected.

Two things follow, and the second is why it changed:

- **The address says what it addresses.** A decimal ordinal said nothing on its own. `2`
  meant Reaction only because a roster line beside it said so, and only until the next boot
  handed the number to something else. The frame logs inherit this: a session's stream is
  `raw/sessions/<run>/cognition.jsonl` rather than `3.jsonl`.
- **It cannot be mistaken for an address in a space we do not own.** The agent runtime hands
  every session its own `send_message` for a sub-agent tree it keeps inside one thread,
  addressed by path from `/root`. A bare integer is a valid address in *both* spaces, so a
  call aimed at the wrong tool resolved as `/root/2`, was refused by a router we do not own,
  and raised nothing on our side — the message simply never arrived. A slug does not resolve
  over there by accident.

Naming the rungs does not reopen retrieval, because the set is closed and fixed at compile
time: three names, the same three every boot, each a singleton. A wrong one can only be a
typo of a word the agent was already given, and the answer says which rung is cold rather
than leaving the agent to read a miss as an absence. Worker slugs are the open half, and
they are addressed exactly as ids were — returned by `CreateWorker`, listed in the roster.

**A rung outlives the process; a worker's session does not.** `cognition` is a durable way to
say cognition, because the rung is a singleton that the host reopens. A worker's slug is not:
after a restart the same string would resolve to a *different* session that happens to serve
the same task, which is the failure the numeric rule was written against, wearing a better
name. So the rule survives where it was load-bearing — **durable work is recovered from
[Tasks](data.md#tasks) and re-addressed to whatever worker is live now**, never by writing a
worker's slug down and sending to it later. The projection is rebuilt every turn because that
is the only rate at which it is true.

That qualifier is what [the session directory](#the-session-directory) turns on: a *record*
of a session that ran may name it forever, because a row about something that has ended is
never a destination anyone can send to. A durable name still needs the run alongside it —
two runs reuse the same slugs, which is exactly why the frames path is keyed by run first.

One structural restriction, because it is routing rather than policy: **a worker may only
address its owner.** Whether something is worth saying mid-task is a judgment and lives in
its prompt; who it is allowed to reach is a fact and lives here.

Two consequences worth stating. **Silence is legal** — a turn's output routes nowhere, so an
agent that finishes without sending has said nothing; the completion event is what keeps that
visible rather than indistinguishable from a hang. And **agents can talk in circles**: nothing
structurally prevents two long-lived agents messaging each other indefinitely. That is
expected, guided by prompt, and worth logging rather than blocking.

### Full frames, not modelled events

The host **records the session stream verbatim and interprets none of it.** An earlier design
modelled a handful of protocol update types so the host could work out what an agent meant;
with intent carried explicitly by `SendMessage`, that reading is no longer anyone's job.
Recording is. Partial modelling was also silently lossy — it discarded every tool-call
payload, which is exactly what [verification](#verification) needs and what a replayed
session is made of.

Historical messages come from the protocol's own session load, not from a second copy we keep.

**Recording is not reading, and the reader folds.** The rule above is about what the host
*stores* and what it *infers intent from* — both stay verbatim and neither is negotiable. It
never said the frames are the right shape to show someone, and they are not: one sentence the
agent says crosses the wire as an `item/started`, several hundred `delta`s and an
`item/completed`, so a page rendering row-per-frame is a wall of fragments. Measured across
twelve logs: **11,891 frames, 369 things that actually happened.**

So a session's log has two readings, from one file, over one address:

| | `GET /api/workers/{id}/frames` | `GET /api/workers/{id}/messages` |
|---|---|---|
| answers | what crossed the wire | what the session did |
| shape | every line, verbatim, uninterpreted | items folded whole, in turns |
| is | the record | derived on every read, stored nowhere |

Three properties keep the fold from becoming the modelling this section rejects:

- **It reads, it never writes.** Nothing folded is persisted, so a fold that is wrong is a
  display bug — the record it was folded from is still on disk, still whole. That is what
  makes it safe for the fold to be opinionated (empty items dropped, ANSI stripped, stderr
  runs joined) where recording may not be.
- **Nothing routes on it.** Intent is carried by `SendMessage`; the fold is for a person
  reading a page, and no host decision depends on it.
- **An unknown item still appears**, carrying the wire's own word for itself and its payload
  verbatim. Codex's item vocabulary keeps growing, so "an item we do not know" is a permanent
  condition; understanding is needed to render one *well*, never to show that it happened.

Each message carries the frame span (`seq`..`through`) it was folded from, so the two
readings name the same moments and a reader can cross from either to the other.

### The session directory

The frames above are written per session under `raw/sessions/<run>/<session>.jsonl`. For a
long time **nothing could read them back**, and the reason is structural rather than an
oversight: the path is keyed by `(run, session)`, ids repeat every boot, and the
switchboard is live-by-construction — an entry exists between `register` and `unregister` and
a session that has ended is simply *absent*. So the id needed to name a session's own frame
log died at the moment the log became history. The frames outlived the session; the index did
not exist.

**One append-only file beside them fixes it: `raw/sessions/index.jsonl`.** The registry writes
an `opened` line at `register` and a self-contained `closed` line at `unregister`; a bounded
tail is folded into an in-memory list of recent ends at boot, so a poll costs no disk.

Three properties, and each answers a specific failure:

- **A `closed` line repeats its `opened`.** Recency is the only order anyone asks for, so the
  common read must not have to pair each close with an open from earlier in the file — a
  long-lived rung's `opened` sits at the top and its `closed` at the bottom, so folding would
  mean reading all of it, every time.
- **An `opened` with no `closed`, in a run that is over, is a session the process died
  underneath.** It is reported as *lost*, not omitted. This is `worker report dropped;
  reaction loop gone` given a name: a worker that vanished mid-flight is the single most
  useful row on the page, and it was previously the one row that could not appear.
- **It is a directory, not a second switchboard.** Every row in it has ended. Nothing routes
  through it, which is why it can hold ids at all (see above).

**Mail is durable beside it, in `raw/sessions/mail.jsonl`, and for two readers rather than
one.** Every delivered message appends a `sent` line; every `take_pending` that drains an
inbox appends a `read` line. That is enough to answer both questions the in-memory ring could
not survive a restart to answer:

- **What have these two been saying to each other?** The ring
  ([`registry::mail`](../../src/foundation/registry/mail.rs)) is seeded from the tail at boot,
  so the arrow between two cards on the sessions page opens a real exchange instead of an
  empty one. Before this, a restart emptied the ring — which was harmless only for as long as
  the restart also emptied the roster it drew arrows between. It no longer does: a reopened
  errand and its owner are both back on the page, so an arrow between two sessions that have
  been talking for an hour would open onto nothing.
- **What was delivered and never read?** `sent` minus `read`, per session, from the previous
  run. Those messages are restored to a reopened session's inbox rather than dropped, because
  the sender was told `Delivered` — see [`agents.md#across-a-restart`](agents.md). A mailbox
  that quietly discards what it accepted is worse than one that refuses.

Same shape and same limits as the index it sits beside: append-only, unpruned, folded from a
bounded tail, and evidence first — losing it costs history and a restored inbox, never a
running agent.

The live roster (`GET /api/workers`) and the ended list (`GET /api/workers/ended`) stay
**separate endpoints**. Everything on the first is live by construction; merging the two would
mean a caller could no longer tell *running* from *ran*, which is the confusion the directory
was built to end. They are joined in the reader, where the difference stays visible.

**Not a retention story.** Nothing prunes this file or the frame logs beside it — the
forgetting pass skips `sessions/` entirely, by name. Bounding them is open work, and the
index deliberately does not decide it quietly: a tail-read at boot keeps startup cheap
without pretending the tail is all there is.

## Default tool surfaces

Each role is handed a **default** surface, sized to keep its context small — every tool
definition in the window costs tokens and latency on a turn that is trying to be fast. This
is a default, not a rail: "only workers act" means workers do the actual jobs, not that the
host fences anyone out.

| Role | Default surface | Why that size |
|---|---|---|
| **Reaction** | `hi_say` · `hi_show` · `SendMessage`, and **no built-ins at all** | its expression channels plus the ability to hand work down. It cannot read, fetch, or run anything — that is why it is fast |
| **Cognition** | `SendMessage` · `CreateWorker` · session reads · **opening** a ledger row | it delegates rather than does, and it may create a duty but never retire one — that goes to a [Task Manager](agents.md#task-manager) |
| **Reflection** | as Cognition, plus memory curation | it curates `data/`; duties are not its to record |
| **Task Manager** | a worker's surface, aimed at one dimension | the only role that may **change** a task's `status` — close, reopen, stand down; it files and delivers none of it |
| **Workers** | projected files and memory, shell, devices, web, build, brokered private capabilities | the job is here, so the surface is wide; ordinary drive files, including managed secret files, are locally usable |

Reaction is the one exception to "default, not rail". Its surface is **enforced at session
open**, because the argument for the rung — that it is fast since it *cannot* wait — is worth
nothing if it can quietly open a file. Restricting our own tools is not enough for that; the
underlying agent's built-ins have to be restricted too, or "cannot" means "was asked not to".

Private refs are drive paths. Workers use projected readers for UTF-8
attachments, journal ranges, and session logs, and a host copier for filing attachment
bytes without routing them through a model-controlled shell. Image/video understanding
remains a separate media capability; the text privacy boundary does not claim OCR or media
inspection.

A **pure transform** changes data; an **effecting tool** changes the world. Workers carry the
effecting ones because that is where the work is, not because anyone above is untrusted — a
role reaching outside its default is a sizing mistake to correct, not an intrusion to block.
The real guardrails are [invariant 9](arch.md#invariants) and
[judgment, not gates](#judgment-not-gates) below — and since 2026-08-28 that is literally
all they are: there is no suspension anywhere, only a rung deciding and saying what it did.

The external-model boundary is enforced at provider egress. Model-driven local commands may
consume secret drive files by path, while every later provider request is projected again.
Commands should keep values out of argv and output. This is not a strict sandbox against a
command intentionally transforming or exfiltrating a value; that stronger threat model
would require broker-only access and is not the chosen design.

## Bundled

Ship with the agent runtime: read/write, shell, web search and fetch, plus the vendor-backed
capabilities — TTS, ASR, text→image, text→video, image→text. Assume present. Most work is
done with these.

Vendor-backed capabilities keep two layers independent: a **capability** (the interface and
its adaptation) and a **vendor** (one API implementation of it). No shared-vendor umbrella,
no cross-capability references.

**A capability holds every vendor configured for it, not one.** One task is commonly served
over several wires at once — the same gateway answers `text-to-image` on both an images
endpoint and a Responses one — so the credential layer carries a *list* per capability and
never picks on the capability's behalf. Which HTTP shapes can actually be spoken is
knowledge that exists only inside the capability, and a chooser upstream of it can do no
better than guess. Two consequences follow, and they are the interface rule:

- Where the caller **names a model**, the model chooses the vendor and every wire stays live
  at once — one tool, a wider menu. Adding a wire adds models, never a second tool.
- Where the caller **names nothing** (speech, vision), the capability keeps the first wire it
  can speak and says which it passed over.

A wire nothing implements is skipped with a log line, never fatal: that list is written by
the broker in its own vocabulary and changes without asking us. The LLM is the one
deliberate exception — its wire *is* the agent runtime, so a second wire there is a second
engine, not a second config.

## User-added

Equipped by the person, because only they can: an account logged in, an API key handed over,
a grant clicked on the actual machine.

This is the class that stalls jobs, in a way no amount of agent capability fixes. So: **ask
once, concretely, in the channel they are actually on**, with the exact steps rather than a
description of the problem. One ask at a time. Credentials never echo back into a transcript.

## Agent-learnt

Equipped by hi-agent itself, at runtime, as part of the job: a CLI, a browser driver, ffmpeg,
model weights, an external API.

The reflex that matters is *"the missing tool is part of the task"*. Researching what to
install, asking approval, installing, configuring, and **actually exercising the first call**
is the work — not a prerequisite to it.

What gets equipped must become **discoverable later**, or the next job re-solves it. That is
what [`skills/`](data.md#skills) is for.

## Devices

A device — an Android handset, a Mac, a box on the network — is **a tool plus a written
procedure**, and nothing further. There is no device registry and no device data structure:
that is deliberate, and it is the softest treatment available. Reachability and grants are
things the agent **notes down when it learns them and re-verifies before relying on them**,
because they change without telling us and a stale registry is worse than no registry.

Devices are equipped, never bundled; none arrives for free. A device is also
[a surface as well as an effector](surfaces.md#surfaces), told apart only by who moved first.

Adding one is a job, not a config step, so it ships as a **seeded skill**: it lays out the
options and equips whichever the person picks — SSH to a machine they own, `adb` to an
Android handset, or **https://abacad.ai** for a device reached over the network. Whatever
gets equipped is written down like any other agent-learnt tool, in
[`skills/`](data.md#skills), or the next job re-solves it.

Two things about devices that no amount of code fixes:

- **The same tool works or not depending on how the host was launched.** A process started
  over SSH has no window server and no granted permissions; identical code driven from a
  desktop session does. So check the environment, and check again after a reboot — never
  carry it as a belief.
- **The logged-in session *is* the credential.** Publishing to a platform with no open API
  means driving an app that is already signed in — no key to store, nothing to leak.

Where there *is* a key: its bytes go into one
[secret drive file](privacy.md#secret-files). A job months from now finds what it may use
and how, then hands the path to a brokered tool or generates a command that reads the text
file at execution time without embedding or printing it.

The target retention policy asks once whether handed-over credentials should be kept for
this exchange, always, or never. That preference flow is not implemented yet; ingest
auto-files detected secrets so their references remain usable across turns.

## Honesty about reach

Before promising anything, consult what is believed about the world's policy surface, not
just our own capabilities. Integrations sort roughly into first-class open APIs,
limited-but-sound official channels, and fragile client automation that violates terms and
gets accounts banned.

When the sound path exists, offer it and equip it. When the person insists on the risky one,
state the risk plainly, leave a trace, and act in a limited way. When something is genuinely
blocked — a captcha, a login wall — hand that step back rather than trying to defeat it, and
never silently retry a blocked automation.

## Judgment, not gates

There are none. Sensitive, irreversible and outward-facing work is decided by the rung doing
it, the same as everything else, and what reached the outside world is **said afterwards**
([invariant 9](arch.md#invariants)).

This was two gates firing *inside* running work — the worker suspending, routing a question
up, resuming — and suspending is the part that did not survive contact. A gate has no way to
tell the action the person just asked for from one they never mentioned, so it stopped both;
and nothing anywhere took a row back off a person once it was parked on one, so the
suspensions accumulated and the resume never came.

A [Decision Maker](agents.md#decision-maker) is what remains of them, and it is the opposite
of a checkpoint: a specialized worker dispatched to **make the call** so the work continues
without the person. Reach for it where being wrong is expensive and one-way — money moving,
something deleted, a message going out under their name that nobody asked for. Not to get
permission; to get a decision.

The person is for what only the person can do: a credential, a login wall, a code that went
to their phone. That is a step genuinely shut to the agent, and it is the only thing that
legitimately stops a row.

## Verification

"The command exited zero" is not "the thing worked". An artifact is not shipped until it has
been **looked at**: render, screenshot, read the image. A liveness probe is not passed until
its output has been **read as content** — empty output means the thing is down.

---

## Debug surfaces

Admin and debug views show **ground truth**: raw fields verbatim, no per-event business
logic, no reshaping of the layer being debugged. A debug view that interprets is a debug view
that lies.

## Hot-loading

A real runtime constraint that shapes the architecture, and worth stating plainly:

- **Hot-loadable** — views, skills, knowledge. The agent creates these at runtime and uses
  them immediately.
- **Not hot-loadable** — backend routes. A capability needing a new endpoint must ship as a
  bundled seed.

This is why a handful of primitives are pre-built rather than grown: the upload carrier needs
a route, and no amount of agent capability conjures one at runtime. It is the one place where
the built-versus-grown line is drawn by mechanics rather than design taste.

## Open

- **Replay of a finished session's frames.** The frame log has live readers
  (`GET /api/workers/{id}/frames`, `raw/sessions/index.jsonl`, and `cognition.md` tells the agent
  where its own stream is), and threads open `ephemeral: false` so `thread/resume` is how the
  rungs come back at boot. What replay would still add is reading a *finished* session's frames,
  which nothing does.


## See also

[`arch.md`](arch.md) for the authorship rule ·
[`data.md`](data.md) for everything this engine serves ·
[`agents.md`](agents.md#workers) for where the jobs that use these get done ·
[`../data-dir-layout.md`](../data-dir-layout.md) for the concrete tree.
