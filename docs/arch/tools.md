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
tokens. A one-line index entry is roughly twelve. **A hundred tools, indexed, costs less than
one server attached eagerly.**

So the invariant is not "keep the tool set small". It is:

> **What a session carries is a pointer. The inventory is fetched.**

Everything below follows from that, and from one more thing: [`arch.md`
invariant 2](arch.md#invariants) already settled that per-role surfaces are a *context
optimization, not a rail*. Nothing here is about who is allowed what. Every rung can reach
every tool. The only question is what is already in hand.

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

It lives in [`skills/`](data.md#skills), which already has everything this needs — a
factory layer rewritten on upgrade, a learnt layer never touched by one, a review API, and
delete-learnt-only. A tool is a skill with an invocation contract; the front matter is what
makes it one. There is no `tools/` tree.

**`purpose` is the only key that costs anything at scale**, because it alone is in the index.
A note missing it degrades to a bare filename — unhelpful, never a confident wrong answer,
the same bargain [`views/`](data.md#views) already makes.

### The format rule

> **Front matter is only what code reads. Everything a mind reads is prose.**

That is the whole of why there are two keys. Whether a tool acts on the agent's own machine
or the person's screen, whether it returns now or mails a result back later, whether it
should stop and ask before doing something irreversible — all of that matters, and all of it
is prose in the body.

Not because it is unimportant. Because every time judgment-read material was put in a schema
here it decayed: facets stayed prose on the grounds that *forms get filled*, and
`proactivity.md`'s four-value standing collapsed into eleven `muted` beside sentences that
were still specific and useful. Gravity in particular is [journey
07](../user-journeys/07-browser-errand.md)'s open question — a soft list, or judge each time
— and the answer everywhere else in this system is guidance plus judgment.

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
no dependency model and no failure taxonomy, because a shebang already declares its
interpreter unforgeably and a `requires:` list would be a second copy of that, drifting.

Which is the same rule as everything else here: **nothing is stored that can be derived.** No
index file, no readiness flag, no dependency list, no copied signature. Every stored claim
about the world goes stale; the world does not. `foundation.md` already forbids the registry
version of this — reachability *changes without telling us, and a stale registry is worse
than no registry*.

### Signatures come from the carrier, at call time

The note carries an entry point, never an argument shape. A CLI publishes its own through
`--help`. An MCP server publishes its own on attach. Hand-copying either into a note is the
index-beside-the-file failure one level down.

This is also the real difference between the two, and the reason for the default below:
**MCP front-loads every signature; a CLI fetches them on demand.** Progressive disclosure is
not something built on top of the CLI carrier — it is native to it.

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

## The registry

Scan the notes, take each `purpose`, emit `name — purpose`. One line per tool.

**Derived, never an index file.** [`views/`](data.md#views) already argued this and the
argument transfers exactly: an index is bookkeeping kept by judgment, and it drifts silently
— a missed entry reads as *never built* while the file sits right there, so the workshop
would get less trustworthy the more it accumulated. Derived from the tree, a missing line
degrades to a bare filename. It never becomes a confident wrong answer.

Flat is right while it fits. When it stops fitting, the fix is grouping or search — but the
scan stays the floor underneath, because a retrieval miss on a tool reads as *"I can't do
that"*, which is precisely how journey 07 failed live: the agent reported having no browser
while a provisioned Chromium sat on the same disk.

## The kit

**What a rung carries without asking.** Everything else is one lookup away and equally
reachable — see the [default surfaces](foundation.md#default-tool-surfaces) table for today's.

Membership is decided by how often a tool is used against what it costs to sit in a window,
and by nothing else. So:

- It is **tuned from usage, not argued** — the standard already set for the worker
  specialists.
- It is per-role because *use* differs per role, not because reach does.
- It needs a **cap expressed as a failing test**. "Add tools continuously" is pressure on
  exactly this tier, and a budget that is not a test is not a budget.

Reaction is the one rung whose kit is enforced rather than sized, and that is a fact about
what a voice is for — a voice holding a shell writes code reviews as message text instead of
speaking. It constrains Reaction, not the kit.

## Carriers

**How a call travels is a per-tool decision, revisable at any time, and changing it touches
one line.** No tier owns a carrier: a kit member may be a CLI, a workshop tool may be MCP.

The default ranks rather than forbids: **prefer a CLI**, because signatures arrive on demand
instead of being front-loaded, because a script is something the agent can actually author,
and because a tool can then be picked up in the middle of an errand — an MCP server cannot be
added to a thread that is already open. Reach for something else where schemas genuinely earn
being resident, or where there is no shell to use.

**MCP is a command, not a carrier class.** A service that speaks only MCP is reached through
one small program — `hi mcp <endpoint> call <tool> <json>` — which turns every such server
into an ordinary note. That is a better trade than a loader, a per-carrier dispatch and a
dead-server-kills-the-thread failure mode.

The cost of this, stated rather than discovered later: the agent runtime gates MCP calls and
does not gate shell commands, so **consent loses its mechanical backstop**. That is consistent
with [invariant 9](arch.md#invariants) being guidance the agent follows rather than a gate the
host enforces, and with the [gates](foundation.md#gates) firing inside running work. It is
still a real change.

## The Tool Manager

Takes *"here is what I want to do"* and answers with a tool or with **"nothing serves that
yet"**. It reads the same derived registry — not a second index — so a miss is a judgment
error you can recover from by asking differently, never a coverage hole.

Its second job is why it is worth having: **it owns the pen for new notes.** It is the only
rung holding both halves of the question — what was wanted, and that nothing serves it — so
*nothing yet* is a work order rather than a dead end. That is how the workshop grows without a
human curating each entry, and it gives the learnt layer a designated writer the way
[`person-reader`](agents.md#reflection--background) owns the `people` dimension.

It is the last piece to build, because it reads the registry and the registry is inside it
either way.

## Decisions

| Decision | Reasoning |
|---|---|
| A tool is a **note**, not a registry row | The knowledge is the durable half; the binary is a cache |
| **Two keys**, everything else prose | Front matter is only what code reads; forms get filled |
| Named by **filename** | The tree is already addressed by path |
| It lives in **`skills/`** | A tool is a skill with an invocation contract — same tree, installer, split and review API |
| The registry is **derived** | An index kept by judgment drifts silently and reads as "never built" |
| Readiness is **running it** | A stored claim about the world goes stale; the world does not |
| Signatures come from the **carrier at call time** | A copied schema is a second truth, drifting |
| **Prefer CLI** | Signatures on demand, authorable by the agent, pickable up mid-errand |
| **MCP is a command** | One shim turns every MCP server into a note; a carrier class would need a loader |
| `bin/` is **machine-local and disposable** | A binary is not portable; the note says how to rebuild |
| The kit is **economy, not permission** | Everything reaches everything; only *in hand* differs |

## What this deliberately does not have

Named so they are not reintroduced as oversights: a `tools/` tree, carriers as a class, an
attach/loader layer, per-carrier dispatch, declared signatures, dependency keys, readiness
flags, and a taxonomy of failure modes. Each was designed and cut, in each case because it
stored something derivable.

## Open

- **Consent has no mechanical backstop** on the shell path. Guidance only, as above.
- **The kit's cap** has no number yet. It needs one, as a test.
- **When the flat registry stops fitting** — grouping or search, with the scan kept as the
  floor.
- **`hi_look` / `hi_act` name the person's screen.** Once the agent has a machine of its own,
  the same verbs mean two things, and *whose body* becomes something a note must say.

## See also

[`foundation.md`](foundation.md#tools) for how tools are equipped and gated ·
[`data.md`](data.md#skills) for the tree this lives in ·
[`agents.md`](agents.md#workers) for who runs them ·
[`arch.md`](arch.md#invariants) for invariants 2 and 9.
