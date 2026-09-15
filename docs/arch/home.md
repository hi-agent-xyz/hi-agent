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

**It opens at 1x, and any other scale is the person's.** A chart larger than the window
scrolls, centred on the core; pinch or ⌘/Ctrl-wheel zooms at the pointer, from 0.25 to 2, and
dragging pans from anywhere a tap would not open. There is no on-screen zoom control — a − /
percentage / + stepper sat over the canvas and was removed as chrome — so a scale once taken
stays the person's until Home is opened again.

**Fitting to the window was tried and lost.** The chart used to open at the largest scale that
put all of it in the window, never above 1x and never below a legibility floor of 0.7. No real
day fit at 1x — on the instance it was measured on, the 20 cards in hand were about 88% of a
1511x727 window's area before a single gap or wire — and the floor was where ordinary days
landed: an eleven-task day measured 1604x1281, needing 0.57. So the default was a chart drawn
at 70% every time, a 17px title at 12px and a picture at 168x95, to buy the one view
of the whole that nobody reads at that size. Reading a card beats seeing all of them, and the
person can still pull back to the whole when that is what they want.

**Height and width are both spent on legibility, not on fitting.** One appearance per kind of
record, so a picture is never a strip inside a card; air graded by rank, so a branch can be
seen to be one; a picture drawn large enough to read, below; and a generous 64px gutter between
ranks, because a wire runs from its parent's edge to the midpoint and arrives flat at its child,
so a narrow gutter makes every curve the same near-vertical kink and a card can no longer be
traced back to the branch that owns it. Each of these was once priced in scale. At 1x they are
priced in scroll, which is cheaper: the cards in view stay readable however far the day runs.

**Air is graded by where two nodes' branches part, not by how deep either one sits.** One gap
for every pair drew every rank as a flat column: a task's own pictures were exactly as far
from each other as from the next task's pictures, so nothing in the spacing said what belonged
to what and the structure had to be read off the wires alone. Two nodes that part at the core
now get the core's gap wherever in the tree they are, and only nodes sharing a parent get the
tight one — at which point tightness reads as belonging together rather than as crowding.
Divergence, not depth, is the quantity: depth would still put two deep nodes from different
branches as close as two siblings.

**Every card and every picture is one 240x135 box.** A shot is rendered at a 16:9 frame and
stored 480x270, so the box draws it at exactly 2x with nothing cropped. Tiles were 120x76 —
a crop of a different shape, too small to tell one page from another — and cards were 240x108.
A card takes the picture's size rather than its own because the two sit in one rank as peers,
and two sizes in a rank read as two ranks.

**A card says what it is without a label, and no side of its border means anything.** There
are two kinds of card, a task and a live session, and each used to open with a line naming
its kind. A session instead wears the dot the core's roles wear — it is the same fact, a live
session, filled while it runs — and a task wears none; where each sits in the tree says the
rest. Status tone, which was a coloured left edge, is carried by the status word. The core has
the same plain 1px border as a card.

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
- **A task's pictures are image nodes one rank below it, and its other results are not on
  Home.** Drawn as sibling cards, results were 60.2% of the canvas on a real instance — 125 of
  them, 106 nothing but a filename. So a task hangs its pictures below itself as `result`
  tiles: image only, no title, no status, alongside the live sessions working on it. Those are
  its sub-steps and sub-results in one rank.
  - **One kind of record has one appearance.** The card used to wear the first picture inside
    itself, beside its text, and hang only the rest as tiles — so the same kind of thing was a
    strip in a bordered card here and a bare tile there, and which one it got turned on nothing
    the person can see, only on whether it happened to be first in `refs`. Mixed on a canvas
    that reads as clutter rather than as rank. The card is now one size whether or not the task
    made anything, and every picture is a tile.
  - **A result with no picture is never a node**, however many there are — twenty-one
    liveness JSONs are a log — **and no count stands in for it.** The card used to print "N
    results", which was a number with nothing to open behind it and the only thing a
    picture-less result ever became here. `factory/tasks` lists them.
  - **The tiles are capped.** A task that makes forty screenshots hangs six.
  - Which six is the task's `refs` order: newest first by when *this task* made each one, so
    a row that has been through three deliverables is about the last one. That is the time on
    the task's own `made` line, not a time on the view — a view record still has none.
- **A result is what the task made, never what its record mentions.** `refs` used to be every
  known view name spelled anywhere in a task's prose, and a mention has no verb: "the screen
  is currently showing `research-two-pairs`" put a shoe report under a KTV task, and the
  shoe report's own note that a polaroid grid had taken the screen hung that grid under the
  shoe research. On the instance that exposed it, of 99 non-system refs across the ledger,
  46 had been rendered by a session serving that task, 23 by a session serving a *different*
  one, and 30 by no session the wire log could place — plus 11 system views. Tightening the
  grammar would only move the line to the next phrasing, so prose is not read for this at all.
  - **The host writes it, at the one moment it sees a view being made for a task.** A view is
    saved by writing a file, and a write says nothing about who made it; but a builder
    renders what it is about to hand over with `hi_review_view`, and that call arrives from a
    session whose registry entry names the task it serves. The first such render appends
    ``made — `ref` `` to that task's timeline, the way a transition appends `moved`: a fact
    the store witnessed, which no mind has to remember. One line per view per task, however many times it is re-rendered.
    The bet is the tool's own description — render what you are about to hand over — so a
    session that renders another task's view only to inspect it records it too.
  - **A system view or a name with a `_` segment is never a result.** `factory/*` ships with
    the app, and `_`-led names are the tree's tooling and a builder's probes (`_qa-…`,
    `_tmp-…/…`) — the same rule `view_watch` already applies. `view_refs` refuses both on
    read as well, so a hand-written `made` line cannot put one back.
  - **What is not rendered for a task does not hang under it.** A task that only arranged for
    someone else's page to go up, or a record older than the stamp, shows no pictures. An
    absent tile is a gap; a tile under the wrong task is a confident wrong answer.

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
- **Nothing marks which result the task is *for*.** A task has a report and it has the
  material it gathered on the way, and that difference is real to the person and absent
  from every record. Ordering is not the distinction and cannot stand in for it: `refs` is
  newest-made first, and the log a research task keeps beside its report is made alongside
  it. `latest` is the task's most recent note rather than a pointer, and no field says "this
  view is what the task is for". So a research task in flight cannot show an empty report
  slot above a rank of process pictures. Naming the face is a judgment and belongs to
  whatever writes the task, not to a reader ranking what it made.
- **A view record carries no time.** A task's `made` line says when *that task* first
  rendered a view, which orders its results, but nothing says when a view last changed, so
  no surface can show a view's newest state as newer than another's. `bookmarked` and
  `shared` exist on the record and are set on none of 195, so neither can pick a
  representative one either.
- **An ordinary day does not fit a laptop window, and scrolls on both axes.** Every open task
  is present however old, and a two-sided chart is about 1604px wide before its height is
  counted — an eleven-task day with ten pictures measures 1604x1281, a fourteen-task one with
  twelve pictures more. What is clear is where the height goes: **a task's tiles stack one per
  row**, so four pictures cost four 135px tile heights. Packing a task's tiles into a block
  rather than a column is the largest single lever on how much a day scrolls — it is unbuilt
  because flextree gives each child its own row, and a block is a placement rule layered on
  top of the tree rather than something the tree can express.
