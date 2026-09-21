# Home: what is going on right now

Home answers what Hi Agent and the person are discussing and what work is in hand.
It is a read projection of existing records, not another task ledger or session
lifecycle store. It remains a normal factory view.

**The model is complete; the chart is not.** The projection holds every row the ledger has
with the whole of its structure — the groups it sits in, the pictures it made, the hands on
it — and the window then draws as much of that as fits, work in hand first. What this surface
refuses is not the record but the claim that all of it is worth the window. `factory/tasks`
still carries the ledger to *read*, `factory/workers` every session, and a card's press hands
off to whichever owns it.

**The two used to be one decision, and that was the mistake.** A row that failed the age and
presence rules was never made a node, so nothing downstream could reach it: pressing into a
group could only ever take away, and the surface had no way to answer *show me this thread*
without a second fetch and a second rule. Those rules are unchanged — they are in § *In hand,
and history* — but what they decide now is **rank**. Everything is in the model; the window
fills from the top; what does not fit is simply not drawn.

A live session is the one thing here that is never history: only live ones are on this surface
at all, and an ended one is `factory/workers`' subject.

## Nodes and relationships

`HomeNode` has a stable identity, a semantic kind, a title, typed data and source
references. Kinds are `core`, `task`, `activity`, `result`, and `overview`.
`HomeEdge` carries `contains`, `works-on`, `produces`, or `explains`, plus whether it is a
primary edge. Every non-root node has one primary parent and reaches `core`. Additional
references do not duplicate the destination. The primary tree is acyclic.

Nodes may have children at any depth. Selection and coordinates belong to the window,
never to a task or a session. Overview nodes are embedded in the core rather than drawn
as peripheral cards. **A group is drawn on the way to a card, or because it holds work in hand.** The second is
what keeps a busy group that lost the fit from vanishing — it is a label and a count, still
holding its rank; the first is what draws a finished group when a branch has room for its
history. A group with neither is not a node: structure standing where its content used to be
is the thing this refuses. The chart opens whole in the window, so it holds what fits
there — see *What the window holds* below.

**A group can be taken as the centre.** Pressing a group heading draws that group's branch
and nothing else — the group where the core stood, a size up, with its tasks, inner groups,
sessions and pictures around it, laid out by the same rules and in the colour the branch wears
on the whole chart. It is a lens the person takes, not a collapse the surface imposes, and **it is the one thing
that adds**: the branch has the whole window, so everything it holds in hand is drawn at any
depth, inner groups included, and its finished work fills the room that is left. Pressing
the group at the centre steps back out one level, and a trail at the top of the window names
every group from the core down, each one a way back; the whole chart has no trail over it,
the way it has no zoom control. Stepping in or out opens the new chart whole, cut on its own
terms, so a branch the whole chart put away is drawn again with the window to itself. Escape is not
the way out: the host already owns it for retreating the panel ([stage.md](stage.md)). A task
is not a centre — a card's press is its handoff.

**It opens whole.** The chart is drawn at the scale that puts all of it in the window — never above
1x, and **never below the overview scale** below: when what may not be cut still does not fit (every
group's label, or a group's own cards once it is the centre), the window opens at that scale and
scrolls, rather than shrinking to whatever the day forces. The drawing's middle — not the core's —
sits at the window's middle. The two sides are rarely the
same height, and centring the core put the taller side's last card half below the window of a
chart that fit it. While the window is whole it stays whole: on the first open, when the sources
answer, when the window is resized, when a card comes or goes. Positioning is therefore not a
one-shot that an empty intermediate tree could use up; it is redone until the person takes the
window. **A zoom, a drag or a plain scroll takes it**, and from then an update keeps their
scroll, until they take another centre — a new chart, which opens whole again.

Pinch or ⌘/Ctrl-wheel zooms at the pointer, from 0.25 to 2, and dragging pans from anywhere a tap
would not open. Two fingers pan it too, except the one roll that is the host's: from the room,
fingers moving left bring the panel in rather than panning ([stage.md](stage.md) § *The
trackpad's swipe*). There is no on-screen zoom control — a − / percentage / + stepper sat over
the canvas and was removed as chrome.

**What the window holds is chosen, and only cards are cut.** The two axes are bought
differently, and that is the whole rule. **Width is bought by depth**: a chart is exactly as wide
as its deepest path, whatever the day holds — group → task is 1348px, group → group → task 1732,
group → task → picture 1924. **Height is bought by cards**: every one stacks. So nothing that
makes depth is cut. Every group is drawn where it is; one whose cards are all put away is its
label and a count, *3 more* / *还有 3 项*, 56px tall, still holding its rank — and pressing it is
the way to them. Only cards are cut, and only until the drawing is the window's shape at the
overview scale.

Two other ways of using the width were tried and rejected, both because they changed what the
chart says rather than how much of it fits:

- **Packing siblings into blocks.** A run of cards with nothing under them was laid out two to a
  row, which took one day from 0.39 to 0.57 at whole-chart scale. But a block's second column
  lands exactly where the next rank sits, and position across the chart is how it says depth:
  a sibling beside a card reads as one level below it.
- **Drawing first-level groups alone.** It narrowed the chart to the 1348 it was meant to grow out
  of, and hung inner groups' cards straight off their parent, which is a different tree.

Cards are offered hottest first, each tried against the whole chart laid out afresh:

1. **In a group taken as the centre, everything it holds in hand is drawn** — at any depth below
   it, inner groups included — because the person pressed in to see this thread. It is the one
   thing on this surface that may overflow the window, which is what the overview scale and a
   scroll are for. Ungrouped work on the whole chart gets no such pass. Exempting it was
   watched failing: a render with no transcript held nineteen closed, ungrouped notices, they
   took the whole window, and every group was left a bare label. And someone who has never
   grouped anything has nothing *but* ungrouped work, so nothing would ever be cut for them.
   What the core puts away it counts, *18 more on the task board*, and the count opens the board
   that carries all of it — the same handoff, and the same named loan, as a card's.
2. **Every branch with work in hand draws its hottest card that fits**, so a quiet branch still
   says something besides its count. An ungrouped card is a branch of its own. A branch holding
   nothing but history gets no such floor: it is drawn where there is room and not otherwise.
3. **Then the rest, hottest first, each while the whole still fits.** A card that would not is
   passed over and the next is tried, so the shorter side fills. **History is the tail of this
   pass**: a branch's finished work is drawn in whatever room its live work leaves, which on a
   full day is none, and in a group taken as the centre is most of the window.
4. **Pictures last, in the width that is left.** A picture spends a rank of width, and the width is
   both sides' at once. Offered with its card, one picture on the right kept a card still in
   progress out of its inner group on the left, and the branch drew one closed eight hours
   before.

**Heat is a tier and then a time**: waiting on the person, then anybody running on it, then
open, then closed and in hand, then history — and within a tier, how lately it moved. **The tier
is what keeps a complete model from drawing like an archive.** On time alone the freshest thing
on a busy instance is almost always something that just finished, so a report closed an hour ago
would outrank the to-do nobody has touched in a week. Heat decides whether a card is on the
chart, never where: what is drawn keeps the record's order, so an update moves only the cards it
changes.

**Only work in hand is counted.** *3 more* is an invitation to press into a group; *and 47 things
that finished* is not one, and the number would say how deep the ledger is rather than anything
about the work. History that did not fit is simply not drawn, and pressing in is what asks for
it.

**What the fitting loop costs is the window's, not the ledger's.** Every offer lays the whole
chart out again, and the model can hold hundreds of rows against a window that draws a dozen. So
the candidates are everything in hand — always, however much of it there is — and then history
64 deep, which is several times what any window has drawn. On a 211-row instance the whole pass
costs 19ms against the 16ms it cost when the model held 80 nodes.

**The overview scale is 0.8**, measured against one day's record (22 open cards, nine pictures,
eight groups, two of them inner) in a 1512×855 window:

| Scale | Cards | Pictures | Chart | Title drawn at |
|---|---|---|---|---|
| 1.0 | 8 | 0 | 1348×820 | 17px |
| 0.87 | 11 | 0 | 1732×978 | 14.8px |
| **0.8** | **12** | **2** | 1828×1000 | 14.1px |
| 0.7 | 13 | 3 | 2116×1125 | 12.1px |
| everything | 22 | 9 | 2308×2178 | 6.6px, at the 0.39 it takes |

At 1.0 there is no width for a third rank, so every inner group is a label; the room left over
went to a to-do five days untouched while work in progress sat behind a count. At 0.87 the third
rank opens and the width goes to inner groups — no picture fits. 0.8 is where pictures start,
and 0.7 is the 12px title fitting was rejected for (below). On that record, pressing the
heaviest group drew all nine of its cards and five pictures, at 0.86.

**What is kept is the centre taken.** The window used to keep its scale and the card nearest its
middle too, because the chart was larger than the window and there was a place in it to lose —
every open starting back at 1x on the core was reported as losing it. A chart that opens whole
has no such place. A group that has closed to nothing lets go of the focus once the sources have
answered, rather than pulling the window into it when it reappears. **It is the window's**, kept
in the webview's own storage: another device keeps its own, nothing on the server reads it, and a
webview that refuses storage opens as a first visit does. The narrow flow keeps the centre taken
too; it is a list the page scrolls, and nothing in it is cut.

**Every card can be brought to the middle of the window.** The drawing sits inside half a
window of air on every side. The canvas used to be the drawing plus only what centring the core
needed, so a card on the chart's outer edge stopped at the window's edge: zoomed in, it stayed
pinned there half off screen, with no way to drag it to where it could be read.

**Fitting to the window was tried and lost, and is back on different terms.** The chart used to
open at the largest scale that put all of it in the window, never above 1x and never below a
legibility floor of 0.7. No real day fit at 1x — on the instance it was measured on, the 20 cards
in hand were about 88% of a 1511x727 window's area before a single gap or wire — and the floor was
where ordinary days landed: an eleven-task day measured 1604x1281, needing 0.57. So the default
was a chart drawn at 70% every time, a 17px title at 12px and a picture at 168x95, to buy the one
view of the whole that nobody reads at that size. It then opened at 1x and scrolled, which kept
every card legible and put most of them outside the window.

What lost was not fitting but fitting *everything*: the scale was whatever the day's size forced.
With the chart cut to the window first, the scale is a constant that was chosen — 0.8, a title at
14px — rather than a floor that was hit.

**Height and width are both spent on legibility, not on fitting.** One appearance per kind of
record, so a picture is never a strip inside a card; air graded by rank, so a branch can be
seen to be one; a picture drawn large enough to read, below; and a gutter between ranks wide
enough for the wires to bend in, because a wire runs from its parent's edge to the midpoint and
arrives flat at its child, so a narrow gutter makes every curve the same near-vertical kink and
a card can no longer be traced back to the branch that owns it. Each of these was once priced
in scale, then in scroll. They are now priced in cards put away, which is the cheapest of the
three: a press on the group gets them back, and the cards that are drawn are the ones most in
hand.

**The gutter is the rank's, and the first one is not like the others.** It was one 48px run
everywhere. But every group on the chart parts from the core, so that first gutter carries
the entire fan — a dozen wires spreading over the full height of the drawing inside 48px of
horizontal run, which draws them as a near-vertical bundle leaving one edge rather than as a
branch each. A deeper rank fans two or three ways over about a card's height, where 48px is
ample. So the gutter is indexed by the rank it leaves — 104px off the core, 56 off a
first-level group, 48 below that. Width is the axis the chart is already long on, and this
spends it once, at the one rank where the bend is the whole reading.

**Air is graded by where two nodes' branches part, not by how deep either one sits.** One gap
for every pair drew every rank as a flat column: a task's own pictures were exactly as far
from each other as from the next task's pictures, so nothing in the spacing said what belonged
to what and the structure had to be read off the wires alone. Two nodes that part at the core
now get the core's gap wherever in the tree they are, and only nodes sharing a parent get the
tight one — at which point tightness reads as belonging together rather than as crowding.
Divergence, not depth, is the quantity: depth would still put two deep nodes from different
branches as close as two siblings.

**Every card and every picture is one 240x135 box.** A shot is rendered at a 16:9 frame and
stored 960x540, so the box draws it with nothing cropped and four image pixels to a CSS
pixel — which is 1:1 *device* pixels at full zoom on a retina screen. Stored 480x270 it was
1:1 at 1x and a plain upscale from there, so zooming in to look at a picture, which is what
the zoom range reaches in for, was the one act that blurred it. Tiles were 120x76 —
a crop of a different shape, too small to tell one page from another — and cards were 240x108.
A card takes the picture's size rather than its own because the two sit in one rank as peers,
and two sizes in a rank read as two ranks. Titles use up to three lines at 17px with 1.4
line height, leaving the footer its own space instead of truncating over unused air.
Group headings occupy a narrower 144x56 box, wrap to two lines, and use stronger text
contrast than metadata; they remain unframed labels, not cards.

**Cards separate from the canvas by material, not by being lighter than it.** The wash behind
them is a real warm-to-cool colour — two corner radials over a diagonal sweep — and a card is
a pane of frosted glass over it: a translucent, top-lit surface that blurs and saturates what
is under it, with a neutral hairline border, an inset edge highlight and three shadow layers.

**The three layers are what makes a card hover rather than rest, and which one is heaviest is
the whole of it.** A contact shadow tight to the edge says an object is lying on the surface;
height is said by the *distance* between a card and the dark under it. So the contact layer
stays a hairline and the two below it are pushed down and pulled in — a large y-offset with a
negative spread — leaving a lit gap under the card's lower edge and the weight further out.
Depth is what the wash cost: colour under a translucent card narrows the contrast between
card and canvas, and the shadow is where that contrast is bought back.
The wash and the glass are one decision. Near-opaque cards over a hint of a wash was the same
idea attempted with neither half doing its part: on a white theme the middle of the window —
where the core and most of the cards sit — came out flat white, so a white card on it was
separated by a hairline alone, and the wash, the one thing saying this is a canvas and not a
page, stopped at every card's edge. Blur over colour separates by depth instead: the branch
hue and the wash carry *through* a card, softened, so it reads as sitting above the canvas
rather than as hiding a patch of it. The core is the same glass one step more solid and one
step warmer, and carries a little more elevation — it holds the most text on the chart, so it
is the one pane where the wash behind is a cost. Tasks, sessions and picture tiles share the
same 12px corners; nothing adds a status-colored border or changes their sizes. Dark mode
softens the highlight, deepens the shadow, dims the wash and leans the panes more opaque: a
blur of a dark wash returns almost no colour, so transparency there costs legibility and buys
nothing.
Cards enter with a short fade and 10px rise, staggered at 28ms intervals capped at 224ms.
Stable node identities keep live refreshes from replaying entry animations. Animation never
owns layout or the canvas zoom transform, and releases opacity back to the model's emphasis
when finished. Pointer hover raises actionable cards 2px; keyboard focus has an explicit
accent outline. Reduced-motion preferences disable entry and movement effects.
Group headings pair their label with a 40px icon, and the label is drawn in its group's
colour. The icon is the group's own when one has been drawn for it and a shared default until
then — see § *Icons* below. Home never picks one from a label or from task text. The icon is
the one flat thing on a chart of glass, and it is kept that way deliberately: no shadow and no
gradient under it, only a hairline ring in the group's own colour, which closes the hard square
its own baked background would otherwise cut out of the wash and ties it to the label beside
it.
**A heading's content is centred in its box**, and the side it sits on decides only which
end its icon takes: leading on the chart's right, trailing on its left, so the icon is always
in the half of the heading that faces the core. One box width for every group leaves a short
label some 50px of slack, and that slack is split evenly rather than pushed outward. What
that gives up is exactness the eye was getting for free. A wire lands on the box edge, and the
icon it points at now sits 6-30px inside that edge where it sat 5px in for every group, so the
wire stops short of the thing it reaches. And siblings no longer line their icons up: across
labels of three to five characters the column spreads about 24px, each icon moved by half of
its own label's slack. A group taken as the centre follows the same rule, rather than being
the one exception to a rule about sides.

**A wire is drawn like structure, because that is what it is.** At a 1.8px stroke and 0.8
opacity the branch hues came out as pastel threads that a card's own hairline border
out-weighed, so the one thing a wire says — which branch a card belongs to — was the faintest
mark on the chart. 2.6px at 0.95 is what makes a branch followable across a gutter; the narrow
flow's rails and ticks take the same weight, since they are these wires on a list.

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
- **Everything below a first-level group is that one colour, at any depth**: its tasks, the
  groups inside it, their tasks, sessions and pictures. A group inside a group never has a
  colour of its own. Shades were tried first — siblings spread across a band of hue and
  lightness around their parent, an only child taking its parent's colour — and lost on the
  first real nested arrangement: they read as a scatter of near-colours instead of as one
  branch, and telling siblings apart is what the cards are for.
- **What is in no group is neutral** all the way down: it has no category to show.
- Wires are 2.2px, and the narrow flow's rails and ticks 2px, so a colour reads as a line.

**A card says what it is without a label, and no side of its border means anything.** There
are two kinds of card, a task and a live session, and each used to open with a line naming
its kind. A session instead wears the dot the core's roles wear — it is the same fact, a live
session, filled while it runs — and a task wears none of its own; where each sits in the tree
says the rest. Status tone, which was a coloured left edge, is carried by the status word. The
core has the same plain 1px border as a card.

**A task with more than one live session draws them as cards one rank below it, joined by
`works-on`. A task with one draws no card for it, and either way the row keeps one status
line: the ledger's word, or *Working* while a hand on it is in a turn.**

The sessions were marks in that status line: a dot each, filled while running, three at most
and a `+1` past that. **A mark is a poor way to say a thing a word can say.** The row read *On
duty* with two hollow marks in front of it, and a reader had to have been told what a hollow
mark was before the card said anything at all; the sessions' own titles were hover text, which
on a touch screen is nothing.

**But the lone session is the one that really had nothing to add, and that is why it is drawn
nowhere.** A session's title is the errand as it was handed out, and when it is the only errand
on the task it is the task's own title said back — *做每周总结页第一版* under *每周五自动出一页
总结 view*, *VICTOR 球拍智联传感器能统计哪些信息* under *把 VICTOR VI-01 能统计什么做成一页*.
A card for it costs a node to restate the row above it, which is what a reader feels as clutter
without being able to name it. That is the same objection that took session cards off the chart
as peers of their tasks; it survives the move onto the branch, because it was never about where
the card sat.

**The one thing it did have to add is the row's own word, and the row now says it.** This
section used to go on to say that the lone hand's state "is what *In progress* already implied",
and that was false: *In progress* is where the ledger row got to, not what is happening in it.
On one day's record, *买鞋：把之前那几份合成一页综合对比* had been `doing` for two days with no
live session at all and *游戏截图→结构化数据 POC* had a worker mid-turn, and the two cards read
the same word — the only difference on the chart was the age of the row's status, which says
nothing about whether anybody is on it. The one place *Working* appeared was Upkeep, the agent's
own housekeeping, whose sessions are activity cards of their own.

**So an open row with a hand in a turn says *Working*, and its clock is that turn's.** The word
is the same one that hand would wear on a card, and the time beside it is how long the turn has
run — *Working · 3m ago* — because a card's time is the age of what its word says, which is
already why a wait is timed from its own line rather than from the row's. The row's clock there
would say how long it has been open and read as three days of work. When no hand is in a turn
the ledger's word stands, unchanged, with the row's own clock.

**A mark was tried here first, and it is the wrong instrument twice over.** A single dot on the
status line, filled while a hand ran, adds the fact at no width and keeps the ledger's word — but
it is the same mark this line threw out two paragraphs up, making a smaller claim, and a reader
still has to be told what it means before the card says anything. *A mark is a poor way to say a
thing a word can say* decides this one too.

**What the word costs is *On duty* for the length of a turn**, and that is taken deliberately. A
`serving` row is a standing assignment and *On duty* is how it says so, but Home answers what is
going on right now; between the two facts, the slot goes to the one that changes, and the
standing one comes back the moment the turn ends. `factory/tasks` is where a duty is a duty
whatever it is doing this minute.

Two or more is a different fact, and not a bigger version of the same one: *which* hands, how
many, and that one of them has stalled while another runs are things no word on the row can say.

- **The task's word is the ledger's — to do, in progress, on duty, completed, cancelled —
  unless a hand on it is in a turn, when it is *Working***. The time beside it is whichever of
  those the word says: how long the row has held its status, or how long the turn has run. Both
  are facts about the row, true however many hands are on it; a closed row keeps its own word
  whatever is still warm beside it, and so does a row waiting on the person.
- **Past two, nothing is capped and nothing collapses into a number**: four hands draw four
  cards, and the length of that branch IS the news.
- **Each card says its own state**: *Working*, *Work queued*, *Idle*, or that its last turn
  failed or was cut off, in the danger tone. That last one used to replace the task's word,
  since it was the one thing a session knew that the ledger could not. It no longer does, which
  leaves a **known gap**: a task whose single hand has stalled falls back to *In progress*,
  which is what the row says when no hand is on it at all. The word separates *a hand is in a
  turn* from *nothing is in flight*; it does not separate the two reasons for the second.
  `factory/tasks` and `factory/workers` both show which it is.

**A task waiting on the person says *Needs you*, in the danger tone, in place of its status.**
The test is the board's: the task is open and the newest line a mind wrote on it is a
`waiting` one, read from the same `latest` field, so the two cannot disagree. Its time is that
line's. It is the one status that asks the reader to act, and Home is where somebody comes
back to once the conversation has stopped saying where each thing got to — three messages go
out between one of theirs and the next ([legibility.md](legibility.md) § *F*). *In progress*
on a row that is waiting on them says the opposite of what is true, and so does *Working*: a
hand may be in a turn on some other part of the row, but the step the record is stopped on is
theirs.

## Internal mapping

- A task is `task:<subject>` from the task ledger, **all of them**, whatever their status and
  age. Its status and lifecycle timestamps stay authoritative even when its worker is idle,
  fails a turn, or disappears. Whether it is in hand or history is a tier on the node, read by
  the window's cut and by nothing else.
- An activity is `session:<run>:<session>`, and only a live one. The run is required
  because session slugs can recur after restart. A session that has ended belongs to
  `factory/workers`, not here.
- Reaction and Cognition sessions compose the single core. Every other live session is a
  visible activity.
- A session's `subject` joins it to a task. A subject whose task is not drawn connects the
  session to the core, where it keeps its own title. **A session with no subject, and
  Reflection, are the agent's own upkeep** and sit in the built-in group § *Grouping*
  describes. Its technical `owner` remains inspectable but does not determine semantic
  placement or create a new task grouping.
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
  tiles: image only, no title, no status. **Where a task has session cards too, that rank holds
  the hands before the pictures**: a session card is the live half of what hangs off a task and
  a picture the finished half, and running work reads before its leftovers.
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
activity only while its session is running. Each of these states is a word on that session's
own card, where it has one — the task's row carries none of them. What stays `factory/workers`' is everything
underneath a state: the tail, the retained action, the turn count, the window.

## Grouping

**A group is a name, an ordered list of task subjects and the groups inside it, and it
belongs to this surface alone.** It is not a field on a task, not a dimension of the memory store, and not a second
ledger. Nothing else in the system reads it; delete the record and Home is what it was.

**No axis over the person's work is built in.** A group may be a project, a kind of work, a state, who asked for
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

The basis is what the person asked for, in their own words, dated — that two pieces of work
are one thing, that a kind of work goes on its own. It is standing, so a task filed next week
obeys an instruction given today without the person repeating it. **Only a line quoted there
is theirs.** This document and the prompts once illustrated it with sentences written as if a
person had said them, and a grouping mind took one of those for an instruction and named a
group after it; an illustration of what a person might say is described, never quoted. Regularising it into fields would compress the
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
{ "groups": [ { "label": "<client>",
                "note": "<when and what the person said makes these one thing>",
                "icon": "drive/home/icons/0199568a….png",
                "members": ["<task subject>", "<task subject>"],
                "groups": [ { "label": "<project>", "members": ["<task subject>"] } ] } ] }
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

**The style is flat, one ink, filled, and the picture is what has to say so.** Two colours
and no more — one background, one ink. The object is a solid silhouette in that ink rather
than a line drawing of one, and where two of its parts overlap they are separated by a gap of
the background colour, never by a second colour, an outline or a shade. No gradient, no
shading, no highlight, no drop shadow, nothing suggesting a light source.

Carrying that means the shipped picture must itself hold nothing to copy that the rule
forbids. The first one was three sheets in three colours with a soft drop shadow under them:
at 40px the shadow was invisible and the palette was a smudge, and at 1024 both were an
invitation. It is now the same three sheets in the one ink, told apart by background-coloured
gaps — which is the mono rule demonstrated rather than described, since the separation
problem is the first one any object with parts will hit.

It also means the words naming the style are there to *refuse* what the picture cannot: an
image model reaches for depth and for a palette unasked, so the drawer is told to look at what
came back and send it round again if it came out lit or coloured. One dimensional or
many-coloured icon among eight flat ones is what breaks a set, and nothing downstream can
flatten it after the fact — the write files an icon-sized copy, and a copy of a shaded picture
is a shaded picture.

- **An icon belongs to its label, not to a write.** The arrangement is replaced whole on every
  pass, and a writer that leaves `icon` out keeps the one that label already had. Rearranging
  is not redrawing, and a forgotten field must not cost a generation per group to put back. A
  renamed group is a new label and starts on the default unless the writer passes the old ref.
- **What is recorded is an icon-sized copy, cut to the size it is ever drawn at.** A
  generation comes back at 1024px or more and a megabyte or two. The write crops the offered
  picture to its centred square, scales it to 360px, files it under `drive/home/icons/`, and
  records that ref; the original stays where it was made. 360 is not the usual draw — a
  heading's icon is 40px — it is the ceiling: **a group taken as the centre wears a 56px icon
  and the chart zooms to 2**, so 56 × 2 × 3 = 336 device pixels on the densest screen. It was
  120, "40px at 3x", which is the heading on a dense screen and nothing else, so the one act
  that reaches in to look at an icon was the act that blurred it. That is the same rule the
  picture tiles already hold and the icons were never held to — see *Every card and every
  picture is one 240x135 box* above, where a 480x270 store was found to be 1:1 at 1x and an
  upscale from there. The cost is about 21 KB an icon against 5.
- **An icon that cannot be used is refused on its own.** It might not be a `drive/` ref,
  might name no file, or might not decode. The arrangement still lands, the label keeps what it
  had, and the answer says why.
- **Every answer carries the ref, and every write refiles the anchor**; the list of groups
  still wearing the default is a second, separate line. Both used to be conditional on that
  list being non-empty, which quietly made a *redraw* impossible: an arrangement where every
  label already had an icon got no ref, so a writer asked to draw the set again had no picture
  to match — and the copy on disk was never refreshed, so a binary shipping a new default
  could sit behind a year-old anchor and every icon drawn from it would carry the style that
  had been replaced. Wanting the picture and having no icon are different things, and only the
  second is a state of the record. The filing compares before it writes, so the ordinary pass
  is a read. A kept icon whose file has since gone counts as the default again, which is what
  gets it redrawn.

### What the surface does with it

- A task named by a group is drawn under it, and a group inside another is drawn under that
  one: core, group, inner group, card. A task in no group hangs off the core, beside the
  groups, and that is an ordinary state rather than a fault.
- **A group is drawn only on the way to a drawn task.** One whose tasks have all aged out
  draws nothing, and neither does a group holding nothing else — at any depth.
- **A member that names no drawn task is ignored.** Tasks close and age out while the record
  stands; the record is not the ledger and never resurrects one.
- **Activities are not grouped, they follow.** A live session joined to a task is already
  inside that task's branch, so it is in the task's group. One whose task is not drawn stays
  at the core: its identity is run-scoped, so a durable record naming it would be a dangling
  reference by the next restart.
- **The agent's own upkeep is one more group, and code draws it.** Reflection, a
  `task-manager` sweeping the ledger, a `person-reader` reading a person's record, a
  `skills-manager` keeping the shelf: nobody asked for any of it, so the ledger holds no row for
  it ([data.md](data.md#tasks)) and dispatch refuses it a `subject` ([agents.md](agents.md)).
  That refusal is what lets code draw the group without inferring anything: a live session with
  no subject is upkeep by construction, and Reflection is upkeep by role. So Reflection is a
  card here rather than a role in the core's strip, and the core is the conversation and the
  coordination. The group is titled in the reader's language (「自身维护」, *Upkeep*), sits
  after the person's own groups, takes a hue like any first-level group, and is drawn only
  while something in it is live. It is not in the arrangement record, and no mind places
  anything in it — **nor does any mind make a group of its own for the agent's own work**:
  every ledger row is something a person asked for, a change to hi-agent they asked for
  included, so it is grouped along their axis like the rest of their work.
  - *Why it exists.* Before it, upkeep sessions hung off the core beside the person's work.
    Asked to organise "the rest", the grouping mind coined 「自己身上的毛病」 for the agent's
    own faults — a phrase this document and its prompt had quoted as if a person had said it —
    and then filed a person's own open-source project there, because they had called its
    code "ours".
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

## In hand, and history

**These rules decide rank, never existence.** Every row the ledger has is a node with its whole
branch; what follows is how the surface tells what is going on right now from what a thread has
been through. The tier feeds three readers and nothing else: which cards fill the window first,
which undrawn ones a group counts as *N more*, and which closures are news on the core.

**What puts a closed card in hand is the work it belongs to, not the clock. An unknown age reads
as old.**

A closed row is on a surface that draws the work in hand for one of two reasons, and they
are not the same reason: **it just finished and the person may not know yet**, or **its
result is still in play for something unfinished**. One window on closure time was a coarse
proxy for the first and no proxy at all for the second, so it did both jobs badly. Measured
on a real instance: of the 15 closed cards it was drawing, 7 were cancellations a
`task-manager` sweep had closed in one batch 13 hours earlier, and a whole first-level group
(4 members, none open) was drawing nothing but history. At the same time it was about to drop
a shoe-research report the person was still working against, because the report was 24 hours
old. The clock had no way to tell those apart.

- All `todo`, `doing` and `serving` tasks are in hand however long they have been open.
- **A `done` task whose thread is still in hand stays in hand with it**: an open task shares its
  innermost group. The person's own words for this were *the parent has not disappeared*,
  and the innermost group is that parent. **Not the first-level branch** — a merged-video row
  sat in `北控视频` with nothing else open while the `KNQ` branch above it was busy, and
  testing the branch would have kept it.
- **Everything else closed is a notice, and a notice is in hand for a glance the person
  actually gets**: until the first message they send *after* it closed, and an hour past that.
  Nobody back yet means nobody has had their glance, so the card stays — work that closes at
  3am is still there when they wake, which a fixed window could never promise.
  - **The hour runs from that row's own collection, never from the newest message.** Measuring
    it from the last thing the person said lets one message resurrect every closed row at once:
    on the instance this was measured against, a message 0.3 hours old turned 14 closed cards
    into 37, the oldest of them seven days closed.
  - **This is not a read receipt, and it is not one being smuggled in.** It is the transcript's
    own inbound timestamps, which `buildHome` already holds for the overview. No client
    identity, no cursor, no acknowledgement — the text channel carries none of those and never
    will ([text-transcript.md](text-transcript.md)).
- **A cancellation is a notice whatever is running beside it.** It has nothing to come back
  to — what it made on the way is process, and the row's own word says the work is not
  happening — so a live sibling never keeps one.
- **Seven days is the ceiling, and it is a backstop rather than a window.** Nothing else
  bounds a thread that stays open, and the arrangement record cannot be the bound: it still
  named 17 closed members, the oldest 45 hours past its closure, and
  [nothing tidies it](#open). Past the ceiling a row is history, which is a rank and not a
  disappearance: the branch it belongs to still draws it where there is room.
- **A closed record with no usable closure time reads as old, so it is history.** Not
  fabricating a missing timestamp is right; concluding from a missing one that the record
  is current is not. A card that IS drawn may still print its time as unknown.
- Visual emphasis decreases over the first 24 hours, and **the fade decides nothing**. It and
  the tier were one constant when one clock did both; a card in hand because the work it
  serves is still open should not read as a day-old leftover, and a history card drawn in a
  branch's spare room should read as exactly what it is.

**Whether the person has *seen* a result is deliberately not the test, and the measurement is
why.** Their screen moves are recorded durably — 509 `went to "<ref>"` observations on that
instance, each with a sender — but the only host-written task↔view join is the `made` line,
and there were 17 of those across 12 of 208 tasks. A signal that can answer for 6% of rows
cannot decide a tier. Nor is seen the axis that was asked for: the report the person had
already read is the one they wanted kept, because the work it serves was still open. Seen
would have deleted it.
- **A live session does not lift its own task out of history.** The row is in the model like
  every other, but a session hangs off a task only while that task is in hand; otherwise it
  connects to the core and keeps its own title. Hanging it under a history row would make the
  row drawable through its child, which is the leak the rule was written for — fifteen of the
  twenty-five closed tasks on the old canvas arrived that way, the oldest closed twenty-six
  days earlier. The join is not lost: the session's `subject` still names it, and
  `factory/workers` has both ends.
- All live sessions are present. Ended sessions are not on this surface at all.

Missing sources are distinct from empty sources; refresh failure keeps the last
successful snapshot with a visible stale-source indication.

## Handing off

A card is a glance and its whole surface is one handoff: a task opens `factory/tasks`,
an activity opens `factory/workers`. Home owns no detail panel of its own.

Narrow views use a connected, recursively expandable flow of the same nodes and edges.

## Open

- **The grace has never been watched on a live instance.** An hour past collection
  was chosen because it made no difference to the measured board — 1 hour and 6 hours drew the
  same 6 cards, because inbound messages are bursty and the first one after a closure is
  usually hours later. That says the number is not sensitive on *that* day's traffic, not that
  it is right. What would move it: a closure landing mid-conversation, where the person's next
  message is a minute later and the hour is the whole of the glance they get.
- **A closed row with pictures, in a finished thread, falls to history as fast as one with
  nothing.** `在那段篮球 clip 上全自动标出一张正确的场地` was the measured instance of it — two
  result tiles, its own group finished, so it is in hand only for the grace. Whether a result
  tile should itself hold a row in hand is undecided; the reason it does not is that a picture
  says nothing about whether anything still needs it, and § *Open* below already records that
  nothing marks which result a task is *for*. It costs less than it did: the row is still in
  the model, and its branch draws it again wherever there is room.
- **A new default does not redraw the icons made from the old one.** The anchor in the drive
  is refiled whenever the binary's default differs, so everything drawn afterwards matches the
  new one, but icons already recorded were edits of the old picture and keep its look. Nothing
  forgets them: a set drawn across that change is two sets until a person asks for a redraw.

- **Nothing tidies the grouping record, and something now reads what is left in it.** A task
  closes and its line stays in `groups.json` until a mind next rewrites the file; a group whose
  members have all finished draws nothing on a busy day, but is still written down. That was
  harmless to look at, and it is now load-bearing: those lines are what hangs a branch's
  history under the right heading, so **a sweep, if one is ever written, keeps closed
  members** — dropping them would file finished work back onto the core. What is still unowned
  is the opposite case, a record that outgrows what one rewrite can hold.
- **What the change costs was measured; how it reads over months was not.** On a live instance
  (211 rows, 222 views, 140 inbound messages, 13 groups, a 1512×855 window) the model went from
  80 nodes to 264 and **the whole chart did not move**: the same 10 cards, 13 groups, 2 pictures
  and 20 counted away, at the same size and scale, for 19ms of fitting against 16ms. The
  branches are where it shows — `KNQ` 4 cards → 7 and 2 pictures → 3; `hi-agent 内部维护`
  1 → 4; `学习类`, `生活类` and `买鞋` unchanged, because their windows were already full of
  work in hand. Unmeasured: a day quiet enough that history fills the *whole chart*, which is
  now possible and has never been seen; and a group with a year of finished work pressed into,
  where the in-hand pass may overflow the window and the first and last cards sit half off it.
- **A branch still has to be on the chart to be pressed into.** A group whose work is all
  finished is drawn only where some branch had room for it, so on a busy day there is no
  heading to press. Its rows are in the model and unreachable from here; `factory/tasks` is
  where they are read. Whether the chart should keep a way into a finished group — a heading
  with no card under it — is undecided, and the reason it does not today is the same one that
  refuses an empty heading anywhere else.
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
  **The panel's reply box makes the loan cost more**: a *Needs you* row can now be answered
  in the panel it opens, and pressing its card still lands on the board, where the person has
  to find the row before they can answer it. The conversation's mark on a reply typed on a task waits on the
  same work, and opens the board too.
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
- **0.8 was chosen on one day's record.** The instance was down when it was measured, so the day
  was rebuilt read-only from `data/` — the task records with their `made` lines, the arrangement,
  the conversation journal. It has since been watched once in the render page against a live
  instance: 14 groups, the chart whole at 0.82, the core counting 18 ungrouped cards onto the
  board, and a running session's card the one its branch drew. That render has no transcript,
  so no closed row aged out; a day read through the real face, and a day with twice the groups,
  are not measured.
