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
stays the person's until Home is opened again. Initial centring waits for both the task
ledger and the other initial sources to settle, and for the viewport to be measured;
centring an empty intermediate tree must not consume the one initial positioning.

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
seen to be one; a picture drawn large enough to read, below; and a 48px gutter between
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
and two sizes in a rank read as two ranks. Titles use up to three lines at 17px with 1.4
line height, leaving the footer its own space instead of truncating over unused air.
Group headings occupy a narrower 144x56 box, wrap to two lines, and use stronger text
contrast than metadata; they remain unframed labels, not cards.

**Cards separate from the canvas.** A clearer neutral border and a shallow shadow separate
white card surfaces from a barely tinted canvas. This applies equally to the core, tasks,
sessions and picture tiles; it does not add a status-colored border or change their sizes.
Group headings pair their label with a 40px icon, and the label is drawn in its group's
colour. The icon is the group's own when one has been drawn for it and a shared default until
then — see § *Icons* below. Home never picks one from a label or from task text.

**A wire's colour is its category's, and never a status.** Wires used to take the status of
the node they pointed at: accent for in progress, accent-2 for on duty and for a picture, grey
for everything else, all at 70% opacity. That put a second copy of the status word on every
wire, gave accent-2 two unrelated meanings, and drew "on duty" and "to do" as nearly the same
grey. None of it said what a wire is for, which is the branch a card belongs to. Status is the
status word's, and only the word's.

- **Each group on the core takes one of eight hues**, evenly spaced around OKLCH at one
  lightness and one chroma, so any set of them sits together. The label picks its slot by
  hash, and a taken slot moves to the next free one: up to eight groups never share a colour,
  and a group appended to the record moves none that are already drawn. Past eight, a colour
  is reused.
- **Below a group, every node is a shade of it, a group inside it included.** An inner group
  is a child like any other, never a hue of its own. Siblings are spread end to end across a
  band around their parent (±12° of hue, ±0.09 of lightness), so they are as far apart as the
  family allows, and each rank down gets a narrower band inside its parent's. The band is
  mostly lightness because neighbouring hues on the wheel blur into each other at their edges:
  at ±18° of hue, amber's lightest shade and green's darkest drew as the same olive.
- **An only child is its parent's colour exactly.** A shade exists to tell siblings apart.
- **What is in no group is neutral** all the way down: it has no category to show.
- The narrow flow follows the same rule: a rail takes its parent's colour, a tick its node's.

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
- **A task sits in a group when the grouping record puts it in one — under every group that
  group is inside — and otherwise hangs off the core.** See § *Grouping* below. Nothing about a task's own record decides this: there
  was a topic rank named by a task's `project` or else its `systems`, and nothing has ever
  written a `project`, so every topic drawn was a `systems` value — and `systems` names the
  operational records a task touches, not what it belongs to: a birthday deck whose photos
  arrived over Feishu was drawn under "feishu" beside a client brief. Similar wording is not
  evidence of ownership, and neither is a shared system.
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

## Grouping

**A group is a name, an ordered list of task subjects and the groups inside it, and it
belongs to this surface alone.** It is not a field on a task, not a dimension of the memory store, and not a second
ledger. Nothing else in the system reads it; delete the record and Home is what it was.

**No axis is built in.** A group may be a project, a kind of work, a state, who asked for
it, or "this week" — the structure is the same name-and-members either way, so the person
can reorganise along a different axis without anything in the code changing.

**Nor is a depth.** A person who divides their work once often divides one part of it again —
the company's projects as one group, each project a group inside it. So a group can hold
groups, the same shape at every level, and nothing caps how deep. The depth lives in the
record's structure, never in a label: a hierarchy spelled into names by a separator is a
convention only its writer knows, and it splits names that were never meant as two. The alternative
was a `project:` field on the task record, and it is the wrong shape twice over: it commits
to one axis, and it puts a claim that only one surface consumes into a record everything
reads.

**It is written by a mind and never inferred by code.** No matching of titles, systems,
refs, or episodes happens here or on the server. The path that was deleted — grouping by
`systems` — failed on real data, not in principle: `systems` says which operational records
a task touches, and 70% of rows carry one. Evidence like that belongs to whoever *writes*
the grouping, as one input among the others; the moment code turns it into a rule, a
birthday deck is filed under "feishu" again.

### Two records, because they have two consumers

| | The basis | The result |
|---|---|---|
| Read by | a mind, when it groups | the server, on every poll |
| Shape | prose | JSON |
| Written | file write | tool call, validated and atomic |
| Path | `data/home/grouping.md` | `data/home/groups.json` |
| Parsed by code | never | strictly |

The basis is what the person asked for, in their own words, dated — "粤语解说表和 KT8 是一摊
事", "自己身上的毛病单独一组". It is standing, so a task filed next week obeys an instruction
given today without the person repeating it. Regularising it into fields would compress the
ask before a model reads it, and a model is its only reader.

The result is a snapshot: which task is in which group, and in what order. Home renders it
directly, so it is JSON — the one shape a renderer should not be guessing at — and it is
written only through a tool, which validates and replaces the file atomically. A surface
polling every few seconds must never read half a file, and a mistyped subject must fail
where the mind can still fix it rather than go silently missing minutes later.

**The third input is the state of the agent, and it is never stored.** What is open, what is
running, what each task is called: the mind grouping has the active-task projection and the
switchboard in front of it already. Those are facts about now, so they are read at the
moment of grouping rather than kept — a stored copy is a second ledger, going stale.

### The shape of the result

```json
{ "groups": [ { "label": "KTV",
                "note": "9/16 说粤语解说表和 KT8 是一摊事",
                "icon": "drive/home/icons/0199568a….png",
                "members": ["cantonese-…-20260916", "kt8-046-content-management"],
                "groups": [ { "label": "Content", "members": ["kt8-051-content-review"] } ] } ] }
```

Array order is draw order: groups outward from the core, members top to bottom, and inside a
group its own members before the groups it holds. `groups` is optional and omitted when
empty, so a record written before groups could nest is still the same record. `label` is
the group's identity — there is no separate id, because renaming a group *is* renaming it —
and labels must be unique across every depth. A task claimed twice stays where it was
first claimed, reading a group's own members before the groups inside it. `note` is one line
saying why this group exists; it is optional and it is read, as the label's hover text, so a
person reviewing the arrangement can see what it was based on. `icon` is optional too, and
belongs to its label at whatever depth the label sits; see § *Icons*. There is no `version`
and no `updated_at`: the writer and the reader ship in one binary, and the file's mtime is
already the time it was written.

### Icons

**A group's icon is drawn by the mind that groups, and every icon is an edit of one
picture.** The default icon ships in the binary. Home draws it for any group without its own
(`GET /api/home/group-icon`), and it is also filed at `drive/home/group-icon.png` as the source
for `hi_image_to_image`. Choosing the object that stands for a group is a judgment about what
the group is to the person, so it lives in the `task-manager`'s prompt, beside the arrangement
it belongs to; no code maps a label to a picture.

**The source picture carries the style, not a description of it.** Icons are drawn one at a
time, weeks apart, whenever a label is new. A style described in words drifts a little with
every call and every model, and eight icons drawn that way do not look like a set. An edit of
one shared picture, keeping its background, palette, flat shapes and framing and changing only
the object, does. So `hi_text_to_image` is never the way an icon is made.

- **An icon belongs to its label, not to a write.** The arrangement is replaced whole on every
  pass, and a writer that leaves `icon` out keeps the one that label already had. Rearranging
  is not redrawing, and a forgotten field must not cost a generation per group to put back. A
  renamed group is a new label and starts on the default unless the writer passes the old ref.
- **What is recorded is an icon-sized copy.** A generation comes back at 1024px or more and a
  megabyte or two, and Home draws every icon at 40px on every open. The write crops the offered
  picture to its centred square, scales it to 120px (40px at 3x), files it under
  `drive/home/icons/`, and records that ref. The original stays where it was made.
- **An icon that cannot be used is refused on its own.** It might not be a `drive/` ref,
  might name no file, or might not decode. The arrangement still lands, the label keeps what it
  had, and the answer says why.
- **The answer names every group still wearing the default**, with the ref to draw from, and
  the anchor is filed at that moment, so the ref it hands out can always be read. A kept icon
  whose file has since gone counts as the default again, which is what gets it redrawn.

### What the surface does with it

- A task named by a group is drawn under it, and a group inside another is drawn under that
  one: core, group, inner group, card. A task in no group hangs off the core, beside the
  groups, and that is an ordinary state rather than a fault.
- **A group is drawn only on the way to a drawn task.** One whose tasks have all aged out
  draws nothing, and neither does a group holding nothing else — at any depth.
- **A member that names no drawn task is ignored.** Tasks close and age out while the record
  stands; the record is not the ledger and never resurrects one.
- **Activities are not grouped, they follow.** A live session joined to a task is already
  inside that task's branch, so it is in the task's group. One with no task stays at the
  core: its identity is run-scoped, so a durable record naming it would be a dangling
  reference by the next restart.
- **A record that cannot be read leaves no groups.** A missing file, malformed JSON, or an
  unusable shape degrades to every task on the core — the surface as it was — with the fault
  in the server's log and nothing about it on screen.

### Who holds which half

| | Writes | Because |
|---|---|---|
| Reaction | nothing | it relays the ask in the person's words, like anything it cannot do itself |
| Cognition | the basis | it is the rung that *hears* it, and the sentence must survive a worker that fails on the way to the screen |
| `task-manager` | the result, icons included | one judgment over every open row, and *one manager, never one per row* is what keeps one hand on a file that is replaced whole |

**The verb is the task-manager's and nothing else's**, enforced where the type is knowable
rather than advertised away: the tool surface has no grain finer than `worker`, so every
worker is offered it and dispatch refuses all but the manager.

This is the split the ledger already has — Cognition opens a row, a manager rules on it —
and it is here for the second of its two reasons. The first, that the rung handing work out
is the worst-placed one to rule that its own errand ended, does not apply to grouping. The
second does: a whole-ledger judgment is an errand, and Cognition hands errands out.

### How it forms and changes

Events, and **no timer** — nothing in this host fires on a period, and grouping has no case
for being the exception.

1. **The person says something.** Reaction relays it; Cognition writes the words into the
   basis **in that turn**, then starts a `task-manager` (or hands the ask to the one already
   running) to rearrange. Basis first: it is what makes the ask stand for next month's task,
   and it must not depend on the worker getting that far.
2. **A manager runs.** It reads the basis whole — standing instruction, not a log — and
   replaces the arrangement. A first arrangement is written the first time this happens.
3. **A task is filed.** It is **not** grouped then. Placing a row is not part of opening one,
   and a spun-up session per filing buys a placement nobody is waiting on. A new task sits by
   itself until the next pass, which is also the honest picture: it is new, and nobody has
   sorted it yet.

A read never triggers a write: opening Home must not cost a turn.

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

- **A new default does not redraw the icons made from the old one.** The anchor in the drive
  is refiled whenever the binary's default differs, so everything drawn afterwards matches the
  new one, but icons already recorded were edits of the old picture and keep its look. Nothing
  forgets them: a set drawn across that change is two sets until a person asks for a redraw.

- **Nothing tidies the grouping record.** A task closes and ages out, and its line stays in
  `groups.json` until a mind next rewrites the file; a group whose members have all gone
  closes to nothing and draws nothing, but is still written down. Ignoring what it cannot
  draw makes this harmless to look at, and the open work in hand is a dozen rows, so the
  sweep it would take is not worth owning. It becomes worth owning if the record ever
  outgrows what one rewrite can hold.
- **A group is a label, not a handoff.** Every other box on this surface opens something;
  the group opens nothing, because the thing it would open is a `factory/memories` subject
  that the grouping deliberately does not have to name. It reads as inert next to cards that
  respond, and the fix is either a target for the view-open above or a group that knows what
  record it stands for — neither is decided.
- **The order within a side is the record's, but which side is not.** Groups and ungrouped
  tasks are fed to the layout in the order the record gives, and the two-sided balance then
  takes them alternately as weight allows, so "first in the file" means near the core rather
  than a place a person can predict. Making a branch's side its own property is the work that
  would make position learnable, and it is not started.
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
