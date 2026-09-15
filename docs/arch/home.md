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
references. Kinds are `core`, `task`, `activity`, `result`, and `overview`.
`HomeEdge` carries `contains`, `works-on`, `produces`, or `explains`, plus whether it is a
primary edge. Every non-root node has one primary parent and reaches `core`. Additional
references do not duplicate the destination. The primary tree is acyclic.

Nodes may have children at any depth. Selection and coordinates belong to the window,
never to a task or a session. Overview nodes are embedded in the core rather than drawn
as peripheral cards. **The whole tree is drawn**: there is no collapse, because a surface
carrying only the work in hand has nothing to hide from.

**It opens fitted to the window, and zoom is the person's.** By default the chart is scaled
to the largest size, never above 1x, at which all of it is inside the window, and that scale
follows the window as it resizes. The default stops at a legibility floor of 0.7, past which
the chart scrolls on the axis that overflows instead — a chart that fits and cannot be read
has not fitted. The person can take the scale over: pinch or ⌘/Ctrl-wheel zooms at the
pointer, a − / percentage / + control zooms about the centre, and dragging pans from anywhere
a tap would not open. Their range, 0.25 to 2, is wider than the default's, because the floor
limits what is chosen *for* them, not what they may choose. Pressing the percentage hands the
scale back to the window. This is not the zoom that was removed: that was the only way to see
a canvas that could never fit, and this one starts from a chart that already does. No arrangement fits at 1x: on the
instance this was measured on, the 20 cards in hand were about 88% of a 1511x727 window's
area before a single gap or wire. So the geometry is shaped to need as little scale as it can
— but **two things outrank scale, and both are paid for in height.** One appearance per kind
of record, so a picture is never a strip inside a card; and air graded by rank, so a branch
can be seen to be one. Together they took a measured instance from 1652x766 to 1604x982, or
0.91 to 0.74 in that window. Height is what a landscape window runs out of first and this
spends it deliberately: a chart that fits and cannot be read has not fitted, and neither has
one that fits and cannot be parsed.

**Width, by the same accounting, is nearly free, so the gutter between ranks is generous.**
Both measured instances are bound on height — at their scale they draw about 1078 of a 1511px
window's width — so widening the gutter changes no scale at all. What it buys is the wires: one
runs from its parent's edge to the midpoint and arrives flat at its child, so a narrow gutter
makes every curve the same near-vertical kink and a card can no longer be traced back to the
branch that owns it. Air between ranks is legibility bought with the axis that has it to spend.

**Air is graded by where two nodes' branches part, not by how deep either one sits.** One gap
for every pair drew every rank as a flat column: a task's own pictures were exactly as far
from each other as from the next task's pictures, so nothing in the spacing said what belonged
to what and the structure had to be read off the wires alone. Two nodes that part at the core
now get the core's gap wherever in the tree they are, and only nodes sharing a parent get the
tight one — at which point tightness reads as belonging together rather than as crowding.
Divergence, not depth, is the quantity: depth would still put two deep nodes from different
branches as close as two siblings.

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
- **Every task is its own branch off the core, and nothing groups tasks.** There was a topic
  rank, named by a task's `project` or else its `systems`. Nothing has ever written a
  `project`, so every topic drawn was a `systems` value — and `systems` names the operational
  records a task touches, not what it belongs to: a birthday deck whose photos arrived over
  Feishu was drawn under "feishu" beside a client brief. Similar wording is not evidence of
  ownership, and neither is a shared system. A grouping is a claim somebody makes; if tasks
  are ever to be grouped, the grouping has to be written by whoever files the task, and Home
  reads it — it is not inferred here.
- **A task's results are a count, and every picture among them is an image node one rank
  below.** Drawn as sibling cards they were 60.2% of the canvas on a real instance — 125 of
  them, 106 nothing but a filename. A count is right for what a duty writes and wrong for what
  a deck is, so a task carries its count and hangs its pictures below itself as `result` tiles:
  image only, no title, no status, alongside the live sessions working on it. Those are its
  sub-steps and sub-results in one rank.
  - **One kind of record has one appearance.** The card used to wear the first picture inside
    itself, beside its text, and hang only the rest as tiles — so the same kind of thing was a
    strip in a bordered card here and a bare tile there, and which one it got turned on nothing
    the person can see, only on whether it happened to be first in `refs`. Mixed on a canvas
    that reads as clutter rather than as rank. The card is now one size whether or not the task
    made anything, and every picture is a tile.
  - **A result with no picture is never a node**, however many there are. Twenty-one
    liveness JSONs are a log, and a filename is what a count is for.
  - **The tiles are capped.** A task that makes forty screenshots hangs six and says forty.
  - Which six is the task's `refs` order, and that order carries intent: `view_refs` walks the
    task's timeline newest entry first so that "a row that has been through three deliverables
    is about the last one". They are the most recently *mentioned* views, never the most
    recently made — a view record has no time on it, and the `?v=` cache-buster on its shot URL
    is not declared to mean one.
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
- **An ordinary day now reaches the floor, not just a busy one.** Every open task is present
  however old, so a ledger of forty was always going to open at 0.7 and scroll — but a
  fourteen-task instance with nine live sessions and twelve pictures measures 1604x1438 and
  opens there too, and that is a normal Tuesday. What should give way first by default is
  still undecided, and handing the person a zoom does not decide it. What is now clear is
  where the height goes: **a task's tiles stack one per row**, so four pictures cost four tile
  heights of a window that has width to spare and is short of height. Packing a task's tiles
  into a block rather than a column is the largest single lever and the one that costs no
  legibility — it is unbuilt because flextree gives each child its own row, and a block is a
  placement rule layered on top of the tree rather than something the tree can express.
