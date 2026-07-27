# Foundation

## Goal

Be everything the agent stands on and reaches with: the engine that runs it, and the tools
and devices it acts through.

Nothing here decides anything. It has to keep working while the thinking layers are slow,
wrong, or gone — and [`data/`](data.md), not this, is where the agent itself lives.

## The line

> **Producing this requires no judgment.**

A signal landing in raw, a line appended to the journal, a config value resolving, an
embedding being computed, a shell command running — none of these need an agent to be
correct. Everything requiring a decision belongs to an agent; foundation only carries it out.

## What it writes into `data/`

Foundation holds the pen for the mechanical subtrees: `raw/`, `journal/`, the factory layer
of `prompts/`, and the factory seeds under `skills/` and `views/`. Those are described where
they live, in [`data.md`](data.md#who-holds-the-pen).

Wherever a subtree has both a factory and a learnt layer, the two stay physically separate,
so an upgrade replaces one and never the other.

## Engine

| | |
|---|---|
| Runtime | process management, the bundled toolchain |
| ACP / MCP | the agent wire and the tool surface, scene-routed |
| Gateway + vendors | model access, credentials, energy accounting |
| Config cascade | layered configuration resolution |
| Store I/O | the read and write paths under every part of `data/` |
| Build pipeline | esbuild and headless-browser rendering for views |
| Logger / observatory | operational visibility — **distinct from the journal**, which is a durability mechanism, not a debug surface |

---

# Tools

There is no separate "effector" layer. From the agent's side everything is a tool call, and
`Bash` is the effector for most of the world.

## Decisions

| Decision | Reasoning |
|---|---|
| The side-effect boundary is a **grant table**, not a layer | Roles already differ only by prompt + tool surface; a table is testable, a drawing is not |
| Classify by **who had to be present** | That, not what the tool is, predicts whether a job stalls |
| Operate UI where no API exists | A person with an account can do it; so can we |
| A missing tool is part of the task | Degrading to a web search when the answer was "install it" is a failure |
| Irreversible or outward-facing → ask | Publishing cannot be undone; caches and indexes outlive deletion |
| General vision over per-app scripting | See-and-click works on any app; a script works on one |

## Who may act

"Only workers act" is enforced here, by what each role is granted:

| Role | Tools granted |
|---|---|
| **Reaction** | none — it cannot fetch, and that is why it is fast |
| **Deliberation** | memory reads · pure transforms (image→text, …) |
| **Cognition** | memory read/write · task operations · dispatch |
| **Workers** | everything — shell, devices, web, build |

A **pure transform** changes data; an **effecting tool** changes the world. Only workers get
the second kind.

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

---

# Devices

Machines and phones hi-agent can reach — an Android handset, a Mac, whatever gets added. All
of them are equipped rather than bundled; none arrives for free.

A device is **both a surface and an effector**, told apart only by who moved first: a surface
when someone messages hi-agent's account on it, an effector when hi-agent opens an app on it
to get something done.

Their state lives in [`drive/devices`](data.md#drive): **reachability** (can we get to it,
and how), **grants** (what the OS will permit), **accounts** (what it is signed into).

Two things about devices that no amount of code fixes:

- **The same tool works or not depending on how the host was launched.** A process started
  over SSH has no window server and no granted permissions; identical code driven from a
  desktop session does. That is an environment property, so it belongs in the registry, not
  in the agent's beliefs.
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
  worker suspends, routes a question up, resumes. See
  [Decision Maker](agents.md#decision-maker).
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
[`agents.md`](agents.md#workers) for who may call these ·
[`../data-dir-layout.md`](../data-dir-layout.md) for the concrete tree.
