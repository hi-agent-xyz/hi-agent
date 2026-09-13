# Home: what is going on right now

Home answers what Hi Agent and the person are discussing and what work is in hand.
It is a read projection of existing records, not another task ledger or session
lifecycle store. It remains a normal factory view.

**It is deliberately not complete, and that is the design.** `factory/tasks` carries the
whole ledger and `factory/workers` carries every session; Home repeats neither. What it
draws is the open work, the live sessions, and the conversation. Anything closed, ended
or produced is reached by handing off to the surface that owns it.

## Nodes and relationships

`HomeNode` has a stable identity, a semantic kind, a title, typed data and source
references. Kinds are `core`, `topic`, `task`, `activity`, `result`, and `overview`.
`HomeEdge` carries `contains`, `works-on`, `produces`, or `explains`, plus whether it is a
primary edge. Every
non-root node has one primary parent and reaches `core`. Additional references do not
duplicate the destination, and a dissolved topic never promotes one into a second primary
parent. The primary tree is acyclic.

Nodes may have children at any depth. Selection and coordinates belong to the window,
never to a task or a session. Overview nodes are embedded in the core rather than drawn
as peripheral cards. **The whole tree is drawn**: there is no collapse, no zoom and no
fit, because a surface carrying only the work in hand has nothing to hide from.

## Internal mapping

- A task is `task:<subject>` from the task ledger. Its status and lifecycle timestamps
  stay authoritative even when its worker is idle, fails a turn, or disappears.
- An activity is `session:<run>:<session>`, and only a live one. The run is required
  because session slugs can recur after restart. A session that has ended belongs to
  `factory/workers`, not here.
- Reaction, Cognition and Reflection sessions compose the single core. Other live
  sessions remain visible activities regardless of owner, role, or task binding.
- A session's `subject` joins it to a task. Without a resolvable task it connects to the
  core and keeps its own title. Its technical `owner` remains inspectable but does not
  determine semantic placement or create a new task grouping.
- Explicit project/system metadata supplies topic membership. Similar wording is not
  evidence of ownership. **A topic is drawn only where it groups at least two children**;
  one that groups fewer dissolves and its children re-link to the core. A rank that
  groups one thing is a level that groups nothing.
- **A task's results are a count, one picture on the card, and the rest one rank below.**
  Drawn as sibling cards they were 60.2% of the canvas on a real instance — 125 of them,
  106 nothing but a filename. A count is right for what a duty writes and wrong for what a
  deck is, so a task carries its count, wears its first picture, and hangs the remaining
  pictures below itself as `result` tiles: image only, no title, no status, alongside the
  live sessions working on it. Those are its sub-steps and sub-results in one rank.
  - **A result with no picture is never a node**, however many there are. Twenty-one
    liveness JSONs are a log, and a filename is what a count is for.
  - **The tiles are capped.** A task that makes forty screenshots hangs six and says forty.
  - The picture on the card is the first shot-bearing view in the task's `refs` order, and
    that order carries intent: `view_refs` walks the task's timeline newest entry first so
    that "a row that has been through three deliverables is about the last one". It is the
    most recently *mentioned* view, never the most recently made — a view record has no
    time on it, and the `?v=` cache-buster on its shot URL is not declared to mean one.
- **A ref is a mention, and a system view is never a product.** A task whose prose named
  `factory/home` had it counted as a result and drawn as its picture. The app's own
  surfaces are excluded. A mention of *another task's* result is the same confusion with
  no flag to catch it and is left to the server-side matching that produced it.

`running`, `waiting` and `idle` are registry states of a live session. In particular,
`waiting` means queued work, not a request for the person to answer. Last-turn outcome
and session termination are separate facts. A retained tool action describes current
activity only while its session is running.

## Core overview

The core presents the shared context and significant updates, each with source
references, an update time and freshness. It does not expose private reasoning or
interpret a busy flag as a plan. Internal output tails are not public statements.

User-visible conversation excerpts and factual task transitions are valid grounded
overview content. A previous response is earlier context after a new user message.

## Retention

**Age decides, and an unknown age reads as old.**

- All `todo`, `doing` and `serving` tasks are present however long they have been open.
- A `done` or `cancelled` task is present only while its own closure is inside 24 hours.
  Its visual emphasis decreases with age, not its readability.
- **A closed record with no usable closure time is treated as old and is not drawn.** Not
  fabricating a missing timestamp is right; concluding from a missing one that the record
  is current is not. A task that IS drawn may still print its time as unknown.
- **A live session does not re-admit its own expired task.** That task is gone; the
  session connects to the core and keeps its own title.
- All live sessions are present. Ended sessions are not on this surface at all.

Missing sources are distinct from empty sources; refresh failure keeps the last
successful snapshot with a visible stale-source indication.

## Handing off

A card is a glance and its whole surface is one handoff: a task opens `factory/tasks`,
an activity opens `factory/workers`. Home owns no detail panel of its own.

Narrow views use a connected, recursively expandable flow of the same nodes and edges.

## Open

- **A view-open carries no target.** `openRef(viewRef)` takes a view reference and
  nothing else, and `factory/tasks` reads no incoming selection, so a card opens the
  board rather than its own row on it and the person finds the row themselves. Giving
  the view-open a target means changing the wire, the server and the view contract
  together. That work takes this loan back; until it lands the handoff is imprecise.
- **`direction` and `decision` have never existed.** This document used to list public
  direction and decisions-needed among what the core presents, and the copy table
  carried strings for both, but `buildHome` has only ever produced `context` and
  `update`. The strings are deleted. What a published, source-backed direction or
  decision request would be grounded in is undesigned, and inferring either from a
  worker's narrative is explicitly not it.
- **`view_refs` matches mentions, not outputs.** Excluding system views removes the
  clearly-wrong class, but a task that merely discussed another task's result still counts
  it as its own — on the instance this was written against, `research-two-pairs` is a
  counted result of an unrelated KTV task. The fix belongs where the match is made
  (`view_refs` in `foundation/server/tasks.rs`), not in a reader of it.
- **Nothing marks which result the task is *for*.** A task has a report and it has the
  material it gathered on the way, and that difference is real to the person and absent
  from every record. Ordering is not the distinction and cannot stand in for it: `refs` is
  ordered newest-mention-first on purpose, but a log is mentioned every time it is
  appended, so the most recently mentioned result is the one least likely to be the point.
  `latest` is the task's most recent note rather than a pointer, and no field says "this
  view is what the task is for". So a research task in flight cannot show an empty report
  slot above a rank of process pictures. Naming the face is a judgment and belongs to
  whatever writes the task, not to a reader ranking mentions.
- **A view record carries no time.** Nothing on it says when it was made or last changed,
  so no surface can order a task's results or show its newest. `bookmarked` and `shared`
  exist on the record and are set on none of 195, so neither can pick a representative one
  either.
- **The chart is wider than a laptop.** Core plus two ranks measures about 1936px
  across, so on a 1512px viewport the outer rank is reached by dragging. Scaling it to
  fit would put it under the legibility floor, and the controls that used to offer that
  choice were removed along with the need for them.
