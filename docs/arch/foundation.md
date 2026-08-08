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

It does **not** write the [generated system prompts](data.md#memoryprompts) — deciding
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
| Agent wire / MCP | the agent wire and the tool surface, routed by session id |
| Gateway + vendors | model access, credentials, energy accounting |
| Config cascade | layered configuration resolution |
| Store I/O | the read and write paths under every part of `data/` |
| Prompt assembly | installs the bundled `prompts/` from the binary at boot; injects each agent's [generated one](data.md#memoryprompts) every turn and truncates it, audibly, at the cap |
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

Like every debug surface here, it shows [ground truth](#debug-surfaces) — raw fields, no
per-event interpretation.

### Energy

What a turn costs and what is left: quota and spend, tracked per account and enforced at the
gateway, not by the thinking layers.

**Running out is a first-class state, not an error.** It is knowable in advance, it recovers
on its own clock, and the person can do something about it — so it is surfaced plainly and
early rather than as a failed turn. The failure this exists for is an agent that goes quietly
useless and does not say why.

---

# Tools

There is no separate "effector" layer. From the agent's side everything is a tool call, and
`Bash` is the effector for most of the world.

## Decisions

| Decision | Reasoning |
|---|---|
| Per-role tool surfaces are a **context optimization**, not a boundary | A smaller surface is a smaller window and a faster turn. What actually divides the roles is who does the job, not who *could* call what |
| Classify by **who had to be present** | That, not what the tool is, predicts whether a job stalls |
| Operate UI where no API exists | A person with an account can do it; so can we |
| A missing tool is part of the task | Degrading to a web search when the answer was "install it" is a failure |
| Irreversible or outward-facing → ask | Publishing cannot be undone; caches and indexes outlive deletion |
| General vision over per-app scripting | See-and-click works on any app; a script works on one |

## The agent session registry

**Every agent-to-agent edge goes through one code component with no model in it.** Agents do
not hold references to each other; they hold *addresses*, and the registry resolves them.
That is what makes "the switchboard is the host" a mechanism rather than an aspiration.

| Call | Who | Contract |
|---|---|---|
| `SendMessage(to, message)` | every agent | One direction, **no reply**. Returns whether it was *delivered*, never a response. Queues per target and merges while the target is mid-turn, so a burst arrives as one prompt |
| `CreateWorker(type)` | Cognition, Reflection | → a session id |
| `SessionStatus(id)` | owners | alive · busy/idle · what it is on · turns. **Meta only** — free to call |
| `SessionMessages(id)` | owners | its actual output. Costs context, so it is a separate call from status |

**`from` is stamped by the registry, never passed by the caller.** The host knows who is
calling; letting an agent name itself is letting it impersonate.

**An address is a session id. Nothing else.** One form, for every hop: a worker's id comes
back from `CreateWorker`, and the ids of the standing rungs are **projected** into the
window of whoever may reach them, the same way open tasks are.

That last part is the point. The alternative — letting an agent name a destination by some
other string and having the registry resolve it — is retrieval: the agent produces a name and
hopes something is behind it. Retrieval can miss, and here a miss is indistinguishable from
"nobody is there". Projecting live ids inverts it: an agent is told who is reachable this
turn, so a rung that is cold is visible as cold *before* a message is sent at it, and the
registry goes back to being a map lookup rather than a search.

Session ids die with the process, so **nothing durable may hold one.** Durable work is
recovered from [Tasks](data.md#tasks) and re-addressed to whatever session is live now; the
projection is rebuilt every turn because that is the only rate at which it is true.

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

## Default tool surfaces

Each role is handed a **default** surface, sized to keep its context small, sized to keep its context small — every tool
definition in the window costs tokens and latency on a turn that is trying to be fast. This
is a default, not a rail: "only workers act" means workers do the actual jobs, not that the
host fences anyone out.

| Role | Default surface | Why that size |
|---|---|---|
| **Reaction** | `say` · `show` · `SendMessage`, and **no built-ins at all** | its expression channels plus the ability to hand work down. It cannot read, fetch, or run anything — that is why it is fast |
| **Deliberation** | `SendMessage` · reads (files, memory, web) · write **only** the conversation's brief | enough to work out what was asked. No shell and no editor, so heavy work has nowhere to go but up |
| **Cognition** | `SendMessage` · `CreateWorker` · session reads · task writes | it delegates rather than does, and it is the **sole writer** of the ledger |
| **Reflection** | as Cognition, minus task writes, plus memory curation | it curates `data/`; duties are not its to record |
| **Workers** | everything — shell, devices, web, build | the job is here, so the surface is wide |

Reaction is the one exception to "default, not rail". Its surface is **enforced at session
open**, because the argument for the rung — that it is fast since it *cannot* wait — is worth
nothing if it can quietly open a file. Restricting our own tools is not enough for that; the
underlying agent's built-ins have to be restricted too, or "cannot" means "was asked not to".

Perception needs no tool of its own. An agent that can read files can open a photo that
arrived, because the signal carried a **ref** and a ref is a path. What it needs is not a
grant but *knowing where things land* — which is prompt, not plumbing.

A **pure transform** changes data; an **effecting tool** changes the world. Workers carry the
effecting ones because that is where the work is, not because anyone above is untrusted — a
role reaching outside its default is a sizing mistake to correct, not an intrusion to block.
The real guardrails are [invariant 9](arch.md#invariants) and the
[gates](#gates) below, and both are judgment.

## Bundled

Ship with the agent runtime: read/write, shell, web search and fetch, plus the vendor-backed
capabilities — TTS, ASR, text→image, text→video, image→text. Assume present. Most work is
done with these.

Vendor-backed capabilities keep two layers independent: a **capability** (the interface and
its adaptation) and a **vendor** (one API implementation of it). No shared-vendor umbrella,
no cross-capability references.

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

Where there *is* a key: the [notebook](data.md#drive) records the endpoint, the calling
convention, and **which environment variable holds the secret**. The secret itself lives in
env or keychain and is decrypted at call time. It never enters a thinking layer.

## Honesty about reach

Before promising anything, consult what is believed about the world's policy surface, not
just our own capabilities. Integrations sort roughly into first-class open APIs,
limited-but-sound official channels, and fragile client automation that violates terms and
gets accounts banned.

When the sound path exists, offer it and equip it. When the person insists on the risky one,
state the risk plainly, leave a trace, and act in a limited way. When something is genuinely
blocked — a captcha, a login wall — hand that step back rather than trying to defeat it, and
never silently retry a blocked automation.

## Gates

Two, and they fire *inside* running work rather than before it starts:

- **Sensitive or irreversible** — payment, deletion, outbound send, account operations. The
  worker suspends, routes a question up, resumes. The call itself is a
  [Decision Maker](agents.md#decision-maker)'s job — a specialized worker, not a checkpoint
  in the path.
- **Outward-facing publication** — staged for explicit approval, always.

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

## See also

[`arch.md`](arch.md) for the authorship rule ·
[`data.md`](data.md) for everything this engine serves ·
[`agents.md`](agents.md#workers) for where the jobs that use these get done ·
[`../data-dir-layout.md`](../data-dir-layout.md) for the concrete tree.
