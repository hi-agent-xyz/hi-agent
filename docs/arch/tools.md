# Tools

How the agent comes to have a capability, where that capability is written down, and what it
costs to be able to find it again.

[`foundation.md`](foundation.md#tools) already sorts tools by **who had to be present** to
equip them — bundled, user-added, agent-learnt — and covers devices, gates and honesty about
reach. That classification is unchanged and is not restated here. This doc is the other
question: the workshop is meant to grow without bound, and a growing workshop that is loaded
eagerly is a context leak that gets worse the more the agent learns.

## The shape of the problem

A tool the agent might use costs nothing. A tool whose interface is **resident in a window**
costs on every turn of every session that holds it, forever.

Those numbers are far apart. One attached MCP server publishes tens of schemas — thousands of
tokens. A one-line index entry is roughly twelve. So an index is cheap, and **the invariant is
not "keep the tool set small"**.

But an index being cheap is not the binding constraint, because the workshop does not grow one
learnt tool at a time. [MCP is a command](#carriers), and that makes adding tools nearly free:
someone who points at twenty servers contributes hundreds of tools in an afternoon, with no
learning loop at all. The cheapest way to add a tool is therefore also what forces this doc's
shape. At a few hundred, an index is affordable to hold. At ten thousand it is not affordable
to *fetch* — and there is no size at which curating one by hand is honest.

So the invariant is about **residency**, and it has two halves:

> **What a session holds is a cut line, not a list — recomputed each time, never stored.**
>
> **What is not in hand is reached by asking. Never by consulting a partial list.**

Everything below follows from those, and from one more thing: [`arch.md`
invariant 2](arch.md#invariants) already settled that per-role surfaces are a *context
optimization, not a rail*. Nothing here is about who is allowed what. Every rung can reach
every tool. The only question is what is already in hand.

## Residency

Three levels — and they are not kinds of tool. They are one monotone ladder of what a tool
costs to hold, cut wherever the budget falls:

| Level | Resident | Membership decided by |
|---|---|---|
| **Bundled** | full schema | **authored** — ships with the role, static, per-role |
| **Hot** | full schema | **derived** — recency-weighted use, per install |
| **Inventory** | nothing | everything else; reached by [asking](#the-tool-manager) |

A tool has no level of its own and nothing stores one. Hot is a *ranking* — one number per
tool, cut at a budget — so a tool used yesterday climbs on its own and one untouched for a
season falls off without anyone deciding it should. That is the difference between this and the
thing [`views/`](data.md#views) already refused: a stored tier is bookkeeping kept by judgment,
and bookkeeping kept by judgment drifts silently.

Bundled and hot differ by **who writes the level**, not by how important a tool is. Bundled is
ours, fixed, the same on every install. Hot is the install's own and changes weekly. Framing
the split as *critical vs. frequent* would draw a boundary nothing observes — same cost, same
reach, same call site.

**Hot makes usage counting a design object rather than a policy.** *Tuned from usage, not
argued* is the standard already set for the worker specialists, and it has never had an
instrument. Here that gap is load-bearing: if nothing counts calls, hot is either empty or
hand-curated, and hand-curated is the drift this whole doc is written against.

### Why there is no layer between hot and asking

The obvious middle tier — the next N tools at a line each, or a summary of what kinds of thing
the workshop covers — was designed and cut. It is the first thing anyone will try to add back,
so the reason is here rather than in a commit message.

**A partial list is a surface that can be consulted and found empty.** The agent checks it,
does not see what it needs, and concludes it cannot — and absence from a truncated list is
indistinguishable, from the inside, from absence from the workshop. That is precisely how
[journey 07](../user-journeys/07-browser-errand.md) failed live: the agent reported having no
browser while a provisioned Chromium sat on the same disk. A middle tier does not fix that
failure. It gives it a respectable-looking place to happen.

Two states have no such stopping point. *What is in hand does not do it → ask.* Unconditional,
no boundary to misjudge, no premature negative available. A third state puts the agent in the
business of deciding whether the middle layer settled the question — a judgment it will
sometimes get wrong, in the one direction that costs.

The same argument kills the categorised version. Grouping is how a list stays readable as it
grows, and it buys that by making a tool that fits no group invisible. It is also the shape
this system rejects everywhere else: every time judgment-read material was put in a frame here,
it decayed.

Two costs, accepted rather than discovered later:

- **Every miss is a round trip.** Affordable by construction — hot absorbs the common path, so
  what is left is by definition uncommon. But there is no degraded mode: a slow or wedged
  lookup leaves a session blind past its hot set.
- **No proactive recognition.** Asking fires only once the agent knows it wants something. It
  cannot notice that a better way exists for a job it believes it is already handling. If that
  ever matters, the place for it is reflection noticing afterwards — not a summary read before.

## A tool is a note

```
---
purpose: drive a real Chrome — navigate, click, fill, read the page, screenshot
use: chrome-devtools-cli
---

How to actually use it, the traps, what good looks like — and how to rebuild it.
```

Named by its filename: `skills/factory/browser.md` is the tool `browser`. A `name:` key would
restate the path the tree is already addressed by.

It lives in [`skills/`](data.md#skills), which already has everything this needs — a factory
layer rewritten on upgrade, a learnt layer never touched by one, a review API, and
delete-learnt-only. A tool is a skill with an invocation contract; the front matter is what
makes it one. There is no `tools/` tree.

### The format rule

> **Front matter is only what code reads. Everything a mind reads is prose.**

The two keys sit differently under that rule, and it is worth being exact about how.

`purpose` is read by code: the [registry](#the-registry) scan takes it verbatim, and it is the
only key that costs anything at scale because it alone is in the scan's output. A note missing
it degrades to a bare filename — unhelpful, never a confident wrong answer, the same bargain
[`views/`](data.md#views) already makes.

`use` is read by a mind. Nothing parses the value; the agent runs it. **What code reads is the
key's presence** — a note with `use` is a tool, a note without one is an ordinary skill, and
that boolean is the whole discriminator between the two kinds of note sharing one tree. Its
value stays out of the body for one reason only: a mind about to run something should find the
command in one place rather than mid-paragraph. If that ever stops holding, the value goes to
prose and the rule wins.

That is the whole of why there are two keys and no more. Whether a tool acts on the agent's own
machine or the person's screen, whether it returns now or mails a result back later, whether it
should stop and ask before doing something irreversible — all of that matters, and all of it is
prose in the body.

Not because it is unimportant. Because every time judgment-read material was put in a schema
here it decayed: facets stayed prose on the grounds that *forms get filled*, and
`proactivity.md`'s four-value standing collapsed into eleven `muted` beside sentences that were
still specific and useful. Gravity in particular is journey 07's open question — a soft list, or
judge each time — and the answer everywhere else in this system is guidance plus judgment.

### The note records a call that succeeded

Not a plan that should work. `foundation.md`'s verification rule, pointed at tools: research
it, ask the person once and concretely if only they can do a step, install, configure, and
**exercise a real call** — *then* write the note. This is the same discipline as "an artifact
is not shipped until it has been looked at", and it is what keeps the workshop from filling
with capabilities that were never tried.

## Running one

`use` is a command. `<data_dir>/bin` is on every worker's PATH, so something the agent wrote
and something the system already has are **indistinguishable at the call site**. Nothing
records which it was, and a tool may migrate between them — system Chrome today, a downloaded
Chromium tomorrow — with no edit to the note. [`runtime/`](foundation.md#engine) already runs
exactly this policy for Node, esbuild and Chromium: prefer what the system has, fall back to a
pinned download, and let callers not care.

**Readiness is running it.** Whatever fails — the binary is absent, its interpreter is absent,
its payload is half-unpacked — the shell says so, and the message names its own fix. There is
no dependency model and no failure taxonomy, because a shebang already declares its interpreter
unforgeably and a `requires:` list would be a second copy of that, drifting.

**Read the note first; never call `use` from memory.** `command not found` names no fix — the
rebuild is in the note. Readiness-is-running-it is what happens once the note is open, not a
way to skip opening it.

Which is the same rule as everything else here: **nothing is stored that can be derived.** No
index file, no readiness flag, no dependency list, no copied signature, no stored level. Every
stored claim about the world goes stale; the world does not. `foundation.md` already forbids
the registry version of this — reachability *changes without telling us, and a stale registry
is worse than no registry*.

### Signatures come from the carrier, at call time

The note carries an entry point, never an argument shape. A CLI publishes its own through
`--help` — so a shim the agent writes into `bin/` needs one. An MCP server publishes its own on
attach. Hand-copying either into a note is the index-beside-the-file failure one level down.

This is also the real difference between the two: **MCP front-loads every signature; a CLI
fetches them on demand.** [Residency](#residency) softens that — schemas can arrive when a tool
enters the hot set rather than at thread open — but does not remove it, because an MCP server
cannot be attached to a thread that is already running. Progressive disclosure is native to the
CLI carrier; for MCP it is something the levels have to arrange.

## `bin/`

The agent's own PATH. `skills/` is what it knows; `bin/` is what it can run; the command name
in `use` is the whole link between them.

**`bin/` is machine-local and disposable.** A binary built on one machine does not run on
another, so unlike [`drive/`](data.md#drive) it is not portable and must never sync. It is a
build artifact — neither foundation's pen nor an agent's judgment — and it is the one tree in
`data/` that can be deleted whole with no loss.

That is affordable because **the note carries how to rebuild**, which is the *perishable half
must be marked* rule pointed at implementation instead of facts. So a fresh install has the
entire workshop's knowledge and none of its binaries, and recovers each one the first time it
is actually needed. Nothing merges, nothing conflicts, and there is no upgrade story to get
wrong.

### What must not live there

Disposable is only true of what a note can put back, and there is one class it cannot: **state
the person had to create.** `foundation.md` names it — *the logged-in session **is** the
credential*, for every platform reached by driving an app that is already signed in. A browser
profile holding those sessions cannot be reconstructed from prose. It needs the person, on
their machine, again — the one class that [stalls jobs](foundation.md#user-added) in a way no
amount of agent capability fixes.

So a tool's **binaries** go in `bin/`; its **profile, session and cache state does not**. That
state is ordinary durable data and lives in `drive/` with everything else the person would be
upset to lose. The line is not *what the tool needs in order to run* — it is *what a note can
put back*.

### One namespace, and the factory/learnt split does not cover it

`skills/` nests, so `skills/factory/browser.md` and `skills/browser.md` are two names and can
coexist — that is how the [factory layer](data.md#skills) is rewritten on upgrade without
touching what the agent wrote. **`bin/` is flat, and two `browser` commands cannot coexist.**

The knowledge layer is path-scoped; the execution layer is not. The split this doc inherits as
solved was solved for *notes*, so a collision between a seeded tool and a learnt one of the
same command name is a real state with no rule yet. It is [open](#open) rather than answered
quietly, because the two candidate answers — the factory command wins and the learnt note is
told, or learnt wins and an upgrade never shadows — differ in who gets surprised.

## The registry

Scan the notes, take each `purpose`, emit `name — purpose`. One line per tool.

**Derived, never an index file.** [`views/`](data.md#views) already argued this and the
argument transfers exactly: an index is bookkeeping kept by judgment, and it drifts silently —
a missed entry reads as *never built* while the file sits right there, so the workshop would
get less trustworthy the more it accumulated. Derived from the tree, a missing line degrades to
a bare filename. It never becomes a confident wrong answer.

**It is read by the [Tool Manager](#the-tool-manager), not by every session.** That is what
[residency](#residency) buys: the scan can be as long, as slow, or as paged as it needs to be,
because it runs inside an agent with its own window and reaches the asking session as one
question and one answer. Flat therefore stays right at any size — grouping was only ever a way
to make a list readable by whoever had to hold all of it, and now nobody does.

## Bundled and hot

**What a rung carries without asking** — the two resident levels, in one window. See the
[default surfaces](foundation.md#default-tool-surfaces) table for today's bundled half.

Membership is decided by how often a tool is used against what it costs to sit in a window, and
by nothing else. So:

- It is **tuned from usage, not argued**, and hot cannot be tuned at all until something
  counts calls.
- It is per-role because *use* differs per role, not because reach does.
- It needs a **cap expressed as a failing test**, and the honest unit is serialized bytes
  rather than a count — one tool with a two-hundred-word description costs what five terse ones
  do. "Add tools continuously" is pressure on exactly this tier, and a budget that is not a
  test is not a budget.

Reaction is the one rung whose surface is enforced rather than sized, and that is a fact about
what a voice is for — a voice holding a shell writes code reviews as message text instead of
speaking. It constrains Reaction, not the ladder.

## Carriers

**How a call travels is a per-tool decision, revisable at any time, and changing it touches one
line.** No level owns a carrier: a bundled tool may be a CLI, a hot one may be MCP.

The default ranks rather than forbids: **prefer a CLI**, because a script is something the
agent can actually author, and because a tool can then be picked up in the middle of an errand
— an MCP server cannot be added to a thread that is already open. Reach for something else
where schemas genuinely earn being resident, or where there is no shell to use.

**MCP is a command, not a carrier class.** A service that speaks only MCP is reached through one
small program — `hi mcp <endpoint> call <tool> <json>` — which turns every such server into an
ordinary note. That is a better trade than a loader, a per-carrier dispatch and a
dead-server-kills-the-thread failure mode.

Two costs of that, stated rather than discovered later:

- **Consent loses its mechanical backstop.** The agent runtime gates MCP calls and does not gate
  shell commands. That is consistent with [invariant 9](arch.md#invariants) being guidance the
  agent follows rather than a gate the host enforces, and with the
  [gates](foundation.md#gates) firing inside running work. It is a real change — and a smaller
  one than it reads, since the rungs already run with approvals off and answer for themselves
  what the runtime still asks.
- **One-shot calls drop the protocol's push half.** A shim is request/response; MCP servers can
  emit progress and can ask the client for things mid-call. Through a command, none of that
  arrives. For a system built on continuous-not-batch that is a real amputation, and it is
  where "reach for something else" has teeth: a server whose value *is* the stream is not
  served by a note.

## The Tool Manager

Takes *"here is what I want to do"* and answers with a tool or with **"nothing serves that
yet"**. It reads the same derived registry — not a second index — so a miss is a judgment error
you can recover from by asking differently, never a coverage hole.

Its second job is why it is worth having: **it owns the pen for new notes.** It is the only rung
holding both halves of the question — what was wanted, and that nothing serves it — so *nothing
yet* is a work order rather than a dead end. That is how the workshop grows without a human
curating each entry, and it gives the learnt layer a designated writer the way
[`person-reader`](agents.md#reflection--background) owns the `people` dimension.

### Two jobs, two tempos

Lookup and authorship are not the same work. Lookup sits in the critical path of a job and is
the *only* route to anything outside the hot set, so it runs constantly and has to be quick.
Authorship is the slow loop — research, ask the person once and concretely, install, configure,
exercise a real call, write the note — measured in minutes, and sometimes in someone's
availability.

**So they are separated, along the seam this architecture already runs on everywhere else.**
Lookup answers now. When the answer is *nothing yet*, that answer is itself the work order,
handed off rather than waited on: the asking session carries on with what it can do, exactly as
it does with any other [worker](agents.md#workers) it dispatched. Folding the two together puts
every *"do I have something for this?"* behind a rung sized for installing software.

**A no has to say which no it is.** *Nothing matched* and *I did not search well* read as the
same sentence to the session receiving them. At a hundred tools they collapse; at ten thousand
they do not. So the answer carries how hard it looked — because a false negative here is
journey 07 with an extra hop.

In an earlier shape of this doc it was the last piece to build: a convenience over a registry
any session could scan for itself. [Residency](#residency) makes it the only route to anything
outside the hot set, so it is load-bearing instead. Until it exists, the workshop is whatever
fits in a window.

## Decisions

| Decision | Reasoning |
|---|---|
| A tool is a **note**, not a registry row | The knowledge is the durable half; the binary is a cache |
| **Two keys**, everything else prose | Front matter is only what code reads; forms get filled |
| `use` is front matter for its **presence**, not its value | The key is the tool/skill discriminator, and that is a boolean; the value is read by a mind |
| Named by **filename** | The tree is already addressed by path |
| It lives in **`skills/`** | A tool is a skill with an invocation contract — same tree, installer, split and review API |
| **Three residency levels**, nothing between hot and asking | A partial list is a surface that can be consulted and found empty |
| The cut line is **derived per session**, never stored | A stored level is bookkeeping kept by judgment, and it drifts |
| The registry is **derived** | An index kept by judgment drifts silently and reads as "never built" |
| The registry is read **inside the Tool Manager** | A scan in an agent's own window costs the asker one question — so flat scales |
| Readiness is **running it**, after reading the note | A stored claim goes stale; but `command not found` names no fix |
| Signatures come from the **carrier at call time** | A copied schema is a second truth, drifting |
| **Prefer CLI** | Authorable by the agent, and pickable up mid-errand |
| **MCP is a command** | One shim turns every MCP server into a note; a carrier class would need a loader |
| `bin/` is **machine-local and disposable** | A binary is not portable; the note says how to rebuild |
| **Session state never lives in `bin/`** | Disposable is only true of what a note can put back |
| Lookup and authorship are **separate tempos** | Lookup is in every job's critical path; installing software is not |
| Residency is **economy, not permission** | Everything reaches everything; only *in hand* differs |

## What this deliberately does not have

Named so they are not reintroduced as oversights: a `tools/` tree, carriers as a class, an
attach/loader layer, per-carrier dispatch, declared signatures, dependency keys, readiness
flags, a taxonomy of failure modes, a stored level per tool, **a middle index tier between hot
and asking, and any grouping of tools by kind**. Each was designed and cut — in each case
because it stored something derivable, or gave a wrong answer a place to look right.

## Open

- **What counts a tool call.** The hot level is derived from usage, and nothing measures usage.
  Until something does, the ladder has two rungs, not three.
- **The resident cap** has no number yet. It needs one, as a test, in bytes.
- **`bin/` name collisions** between a seeded tool and a learnt one: the factory command wins
  and the learnt note is told, or learnt wins and an upgrade never shadows.
- **Consent has no mechanical backstop** on the shell path. Guidance only, as above.
- **`hi_look` / `hi_act` name the person's screen.** Once the agent has a machine of its own,
  the same verbs mean two things, and *whose body* becomes something a note must say.

## See also

[`foundation.md`](foundation.md#tools) for how tools are equipped and gated ·
[`data.md`](data.md#skills) for the tree this lives in ·
[`agents.md`](agents.md#workers) for who runs them ·
[`arch.md`](arch.md#invariants) for invariants 2 and 9.
