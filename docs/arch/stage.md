# The stage

**Status:** proposed and built August 11, 2026, on `design/stage`. **Amended August 17,
2026 — the rail is a popover:** the conversation no longer holds a column beside the
agent's view, it opens as a panel over it, out of the corner the controls are in. That
reverses this document's own *Rejected* entry, and the reversal is argued where the rail
was described. **Amended August 18, 2026 — the pill is timed:** it shows the newest line
while that line is worth reading and then fades out, instead of holding it over the view
until the next one replaces it; argued in *The pill is timed* below. **Amended the same
day — the conversation is a card and the line is inside it:** a one-line title, the
messages, and the line being written, in one card that is the same in both placements; and
the text channel's one control moves the whole of it; argued in *The line is inside the
conversation*. **Amended the same day — the band's lower row is bookmarks, and its tiles
are pictures:** the row is the system views plus what the person kept, not the whole
inventory, which reverses this document's own deferral of a pinned subset; and a raise is
captured by the headless renderer the moment it goes up, which reverses *marks, not
screenshots*. Both are argued in *Going back to a view the agent has moved past* and
*The tile is a picture of the raise*. **Amended the same day — the keyboard follows the
planes:** the plane the focus is in owns the keystroke, so a view can no longer take a key
typed into the host's own line or controls; argued in *The keyboard follows the planes*.
**Amended August 19, 2026 — the conversation stands in one box:** the panel keeps the same
corner and the same measure whether or not the agent has a view up, so the `stage`
presentation is gone and with it the jump the person saw whenever something went up;
argued in *One box, whatever else is on the stage*. **Amended the same day — a move is
reported, the cursor still is not:** going to a view posts on the view channel's new
inbound half, so the agent knows where the person is looking when they speak next, while
which entry a window is parked on stays the window's own and out of the appearance;
argued in *Where they went is reported; the cursor still is not*. Everything else stands. Defines what may be on screen at once, and how the conversation, the agent's views
and the host's own surfaces share it. Supersedes the placement half of `core/layout.ts`'s
doc comment and the "every view owns the whole frame" rule in `ui/ViewSlot.tsx`.

## The problem

`floorLayout` marks the captions participant `docked` whenever any view is on the stage
([`core/layout.ts:91`](../../src/appearance/web/src/core/layout.ts)), and `Shell` reads
that flag to render `<SpeechText>` — `messages[messages.length - 1]`, one line — in place
of `<Chat>` ([`ui/Shell.tsx:111`](../../src/appearance/web/src/ui/Shell.tsx)). So showing
anything collapses the conversation to its newest line, and the scrollback, the day
separators and the input all go with it.

Every other surface on the stage composes. The camera shrinks to a pip and keeps playing.
The condition layer sits over the content without evicting it. Only the conversation —
the one durable, append-only, journal-seeded thing in the product
([`text-transcript.md`](text-transcript.md)) — degrades to a caption, and the only way
back is to close the view.

It degrades because it was never a participant. It is host chrome smuggled into the
participant list under a synthetic id, `CAPTIONS_ID = "__captions__"`, whose "placement"
is not a placement but a switch between two different components.

## Decision

**Everything on the stage is a view, the conversation included, and the stage composes
them by role.**

Three parts, in order of how much they change:

### 1. A view's source is bundled or compiled

| Source | What it is | Who has one |
|---|---|---|
| **compiled** | an esbuild artifact behind a content-addressed module URL, mounted by dynamic `import()` | the agent's content view, the condition notice |
| **bundled** | a component inside the host bundle, always present, no compile step and no fetch | the conversation, the camera self-view |

This is what makes it safe for the product's most important surface to be a view.
**Being a view means having a role on the stage — not having gone through esbuild.** A
compiled view that fails to compile or 404s renders `null`
([`ViewSlot.tsx:32`](../../src/appearance/web/src/ui/ViewSlot.tsx)), which is the correct
outcome for a board and an intolerable one for the record of what was said. The
conversation must not be able to fail to appear, so it does not depend on the view
compiler, the module cache, or the ref resolver.

### 2. Four roles, and the write path decides which

| Role | Owner | How it is written | Placement |
|---|---|---|---|
| `conversation` | the host — always present | nothing writes it | one popover panel, in the corner the controls hold · the pill when put away |
| `content` | the agent | `hi_show` / `replace` / `dismiss` → `ViewBus::apply` | the stage, whole |
| `condition` | the process | level-driven `ViewBus::reconcile` | over everything, full-bleed |
| `self` | the person's camera channel | the channel being on | the backdrop alone · a pip otherwise |

This extends the rule the two slots already follow — *which slot a write lands in is
decided by how it was written, not by anything on the wire*
([`view_bus.rs:49`](../../src/foundation/server/view_bus.rs)) — from two roles to four.

**Views still declare no geometry.** The compositor arranges by role, and a role's
placement is fixed and host-decided. Nothing here reopens `region` / `size`, which stay
deleted: a view is handed a frame and owns all of it. Under the amendment the frame is the
whole stage again in every state — the conversation covers it rather than shortening it.

### 3. The conversation has two presentations, one mount

| Presentation | When | What is on screen |
|---|---|---|
| **popover** | the person has not put it away | the chat in a fixed-measure panel in the controls' corner, over the content if there is any and over the room if there is not; full scrollback, and the line being written standing in its foot |
| **pill** | the person put the popover away | the newest line, floating over whatever is behind it while it is fresh, then gone — a caption |

*Three until August 19: a **stage** presentation held the card centred in the room while
`content` was empty, and the panel was what it became when something went up. That is the
box that is gone — see* One box, whatever else is on the stage.

`SpeechText` survives exactly here: **the pill is the conversation collapsed, not a
separate surface.** One source, two presentations, and the person moves between them.

**The input is in the conversation** — the last clause of that sentence started as *the
input follows the conversation*, which was three placements kept in step by hand; it is
one surface now. See *The line is inside the conversation*.

## The popover

*August 17: this section replaced one called **The rail**, which specified a ~400px column
beside the content, built and shipped that way. What it argued, and why it lost, is below.*

The conversation opens as a panel pinned to the **bottom right**, rising out of the button
that opens it — over the content when there is content, over the room when there is not.
Not a column beside the content, which is what this document originally specified and what
was built.

**Why the rail lost.** The rail was priced as "the conversation narrows rather than
degrades", and that price was right for the surface and wrong for the window. It cost the
view a permanent 320–460px — a third of a 1512px window, and the same third whether the
conversation was being read or had been idle for an hour — and the view is *the thing being
looked at* for as long as it is up. A view that reflows into two thirds of the window is a
view composed for a frame nobody designed for, and every board we ship reads better across
the whole window than across most of it. A popover charges the same width only while it is
open, and the person opens it exactly when they want to read.

What the rail was actually protecting — *the record must never disappear because the agent
decided to show something* — is untouched: the panel is up by default when content appears,
carries the full scrollback and the input, and the pill carries the newest line behind it
for as long as it is worth reading. The conversation still never degrades; it now stops
charging rent.

| | |
|---|---|
| Measure | ~420px — a readable line, not a column of two-word wraps |
| Bounds | the window minus its margins on anything narrower than that |
| Height | bounded (`min(58vh, 560px, …)`), so it reads as a panel and not as a drawer that reached the titlebar |
| Threshold | **none.** The rail needed one — below ~760px a window cannot be split into two usable columns — but a popover splits nothing. A narrow window gets the same panel as a wide one, sized to it, and keeps the whole scrollback instead of dropping to the newest line |

Losing the threshold is the quiet win. It took the last piece of window measurement out of
the compositor: `stage()` no longer reads `width`, `RAIL_MIN_WIDTH` is gone, and
`useViewportWidth` — a resize listener that existed for that one number — is deleted. Size
is the stylesheet's business again, and the pass is a function of what is on screen.

**Dismissal is what makes it a popover and not a panel.** Escape, a press anywhere behind
it, or the toggle again. Escape defers to whoever already handled it (`defaultPrevented`),
so clearing a half-typed line closes the line and leaves the conversation up; the press-
behind ignores the controls cluster, which is the panel's own button standing in its own
box. The line needs no exclusion any more: it is inside the panel, so *behind it* already
excludes it.

**The two dismissals answer to different conditions (August 19).** Escape is live whenever
the conversation is up — including with nothing on the stage, where it is the keyboard's way
to the same put-away the control does. The press-behind is live only while there *is*
something behind it: with an empty stage the press lands on bare paper, and the room is not
a thing anyone reaches past the conversation for — a click to focus the window would put the
record away. Until August 19 that condition came for free, because the panel only existed
while a view did. **Host chrome is never "behind"**: the controls cluster and the views band
it opens are both excluded, since picking a view in the band is *how a view gets on the
stage*, and putting one up must not take the conversation down with it.

## One box, whatever else is on the stage

*August 19: the `stage` presentation is deleted. The panel above is where the conversation
is, always — same corner, same measure, whether the agent has a view up or nothing at all.*

Until now there were two boxes, and the pass chose between them by *what else was on
screen*: an empty stage got the card centred in the room at up to 880px; a view or a live
camera moved it to the corner at ~420px. The trigger for that move is the problem. **The
person is not the one who fires it** — the agent is, by showing something — and it fires
onto a surface they may be in the middle of reading or typing into. A slide goes up and the
conversation halves its measure, rewraps every line, relocates its scroller and re-lays the
line being written, all under the eye. Every account of what this surface is for says the
opposite: it is the durable one, the one that *keeps*
([`text-transcript.md`](text-transcript.md)), the one thing the agent must not be able to
disturb by deciding to show something.

Two boxes also cost twice everywhere else. Two geometries in the stylesheet with a
`:not(…)` narrow-window override to keep them apart; a card sized `min(880px, 100%)` in one
and `100%` in the other; an entrance animation that fired on a *class change*, so it played
when a view appeared and never when someone actually opened the conversation; and a
`camera` input to the pass that existed only to say "the room is busy, move the card".
All of it is gone with the second box.

**What the empty room loses.** Grandeur: with nothing up, the face is now paper, presence,
and a 420px panel in the corner rather than a wide card in the middle. That is a real loss
and it is the price. It buys a conversation that is exactly where it was left — and the
room was never the thing that had to be defended; the record was.

**What did not change:** the panel is still up by default, still carries the whole
scrollback and the line, still dismissable, still comes back on any printable key. The pill
is unchanged. A view that owns the conversation still stands the whole surface down.

## The pill is timed

*August 18: the pill used to hold the newest line indefinitely — until another line
replaced it, or the person opened the popover. It now fades a few seconds after that line
settles, and the dock sits empty until the next thing is said.*

**Why it held in the first place, and why that reason is gone.** It held because the pill
was written when it was the whole of the conversation on that screen: let the line go and
the words were gone, since there was nowhere else to look. Since August 11 there is —
the popover, one press away, carrying the full journal-seeded scrollback. What the pill
shows is a *copy*. A copy has no claim on the frame it is sitting over once it has been
read, and holding one there means the ordinary resting state of a view is a sentence from
some earlier minute lying across its bottom edge.

**This does not reinstate the caption band's timer**, which *What this reverses* (below)
argues against and is right about. The band's timer was *spending* the words: it revealed
an utterance the buffer had already deleted, so whatever it advanced past was lost, and it
advanced whether or not anyone was looking (`arch-refactor.md`, the half-spent-text
finding). This one hides a copy while the original stays in the list. That is the whole
distinction — a timer over a durable list costs nothing; a timer over a queue costs the
message. It is also why no presence check belongs here: whether anyone was looking does
not matter, because nothing is spent by their not having been.

| | |
|---|---|
| Dwell | a floor plus reading time, capped — `captionDwell` in [`ui/caption.ts`](../../src/appearance/web/src/ui/caption.ts). The cap is the pill's own three-line clamp: past that there is no more of the line on screen to read |
| Clock | the line's own `ts`, never this window's mount. One rule covers reload, resync and a second device joining mid-conversation: a line said an hour ago opens already spent, so a window never flashes a stale sentence over the view |
| Rolling speech | no deadline while an interim is in flight — it is still being said |
| Exit | a fade, then `visibility: hidden` at the end of it, so the spent line stops being reachable by pointer or screen reader exactly when it stops being visible |
| Reaching for it | a link in the line holds the fade off while it is hovered or focused (`:has(.hi-speech-link:hover)`) — a link is an offer, not a record, and a caption that dissolves under the cursor takes it away |

**The presentation is still `pill`.** Whether the pill has anything on it right now is not
a placement question, so `stage()` does not answer it and does not learn a clock: the pass
says where the pill goes, the shell says how long it stays. Nor is the line unmounted when
it expires — the dock fades and the line stays in it, so the next thing said cross-fades
with it instead of appearing into a box that just collapsed.

## The line is inside the conversation

*August 18: the line the person writes on used to be a box of its own in the bottom band
(`.hi-kbd`), positioned to read as the panel's foot — the panel's width, the panel's right
edge, and the panel's own floor lifted by exactly that row to make space. It is rendered
inside the panel now, and the control that shows the conversation is the control that
shows the line.*

**Two toggles for one surface was the tell.** A keyboard button showed the line; a separate
conversation button opened the panel over a view. Neither state a person could reach by
moving only one of them was a state worth having: a conversation opened to read with no way
to answer in it, or a line floating over a view with the thread it is adding to put away.
So the cluster is one control per channel now — mic · speaker · **text** · camera — and the
text one moves the whole of its channel: the record, the scrollback, and the line.

That also ends the control that appeared and disappeared. The conversation toggle had to:
it opened a popover, and there is nothing to open one over unless something else is on the
stage. The text control is a channel's control, so it is there whatever is on screen.

What it deletes, and the deletions are the argument:

| Gone | Why it existed |
|---|---|
| the conversation toggle | to move the half of the surface the keyboard button did not |
| `--hi-pop-width` / `--hi-pop-right` read by a second box, and the `:has(.hi-kbd)` rules lifting the panel and the pill by the line's row | to hold two boxes flush enough to read as one |
| `Stage.input` (`"center" \| "popover"`) | the pass had to place the line in every state; it goes where the conversation goes, which is not a question |

**Put away means away, with nothing else on the stage too** — a reversal of *"collapsing
with nothing up is not a way to hide the conversation"*, which the compositor enforced and
a test named. It was right when the pill was a shelf that would have been left holding a
sentence over an empty room, and when the button doing it was the conversation's own, so
refusing simply meant not showing it. Neither holds now: the pill is timed, so what is left
is the room and a line that fades; and the button is the **text channel's**, which cannot be
the one control in the cluster that goes dead in the state where its channel is the whole
face.

### The conversation is a card

The shape is [shadcn's own chat](https://ui.shadcn.com/docs/components/base/message-scroller):
**a card — a one-line title, the messages, the line being written.** It fills the one box
the compositor keeps for it (*One box, whatever else is on the stage*), so the card has a
single measure and the recipe — border, glass, shadow, radius — is stated once, on
`.hi-chat`.

*Written August 18 as "a card in both placements", when the card had a lead position in the
middle of the room and a popover position over a view, and the point was that the two looked
the same. There is one placement now, which is the same point taken further.*

**The title names the surface, and holds nothing else.** It reads *Conversation*. There is
no thread to title — the record is one append-only list
([`text-transcript.md`](text-transcript.md)) — so there is no "new chat" to offer beside it
either, and the agent's current state is the status button's job in the controls cluster. A
header that repeated any of those would be chrome charging rent on a panel that is already
bounded in height.

*It named the other side until August 19 — the app's mark and "Hi Agent" — which is a
messenger's habit and reads wrong here. There is exactly one agent; its name is already in
the window's title and its face is the room. A card standing over the agent's own views does
not need to introduce it, and a mark on a panel that is always the same panel is decoration
on a row whose whole job is to say what the panel is.*

**The foot draws no rule above the line.** The line being written is already a bounded box
on the card's glass, so a divider over it drew the same boundary twice and cut the card in
half to do it. The messages run into the space the line stands in.

**Any printable key brings it back** and seeds the first character, which is the safety net
under all of this — the way in never depends on finding the control, so putting the
conversation away is never a corner someone is stuck in.

**Escape is still the popover's, not the line's.** In the line it clears a half-written
draft and stops there; on an empty line it passes through to the shell, which puts the
popover away. Dismissal is what makes a popover a popover, and clearing what you typed is
not dismissal.

**A view that owns the conversation owns the writing of it too.** `owns_conversation` used
to leave the host's line floating over the view; a view rendering the words and a host line
under it are two places to type into one conversation. The trait now stands the whole
surface down.

## Who decides the presentation

- **By default, nobody:** it is derived — up → the popover, wherever the person left it;
  put away → pill; a view claiming the words → neither.
- **The person** shows the conversation or puts it away from the text channel's control in
  the cluster — one control, in every state — and dismisses the popover with Escape or a
  press behind it.
- **The agent cannot.** It has no tool for it and gains none. The one thing that must
  never disappear because the agent decided so is the record of what it said.
- **A view may stand the host down** by declaring `owns_conversation` — it renders the
  words itself, so the host shows neither popover nor pill. This is today's `owns_captions`
  under the vocabulary the rest of this document uses; the rename carries a
  `#[serde(default)]` so snapshots written under the old name reload as `None`, which
  reads as host-owned, which is the safe default it already was.

**The collapse is a property of the window, not of the conversation, and it is not
server state.** Every other thing on the stage is backend-owned so that a second device
and a refresh converge — but that is about *what the agent has expressed*, and this is
about how one window renders it. A window where someone put the conversation away must not
put away the one next to it. It is also the same refusal [`text-transcript.md`](text-transcript.md) makes
about scroll position: a window's own view of the conversation stays in the window and is
never reported.

## Going back to a view the agent has moved past

`replace` destroys what was up, and until now the only way back was asking the agent to
show it again — asking someone to redraw a whiteboard they just erased. The screen keeps
a **history**: the raises, oldest first, carried in the same `GET /api/out/view` state and
persisted in the same snapshots. The newest entry is what is on the stage, which is what
makes *the person is at the end* mean *the person is live*.

**One list, and appending is the only thing that happens to it.** A browser's back stack
destroys its forward entries when you navigate from a back position, and can afford to
because you are its only navigator. Here the agent raises views too, so losing the entry
someone was on their way back to because the agent spoke would be indefensible. The agent
appends; the person moves a cursor. There is no branch, so nothing can be truncated, and
the stack and the history are the same object.

**The cursor is the window's, and is never reported.** Which entry a window is parked on
is that window's own, exactly like the conversation's scroll position — a phone that went
back must not move the desktop. The content slot stays the agent's: what it raised is
still what a second device shows and still what it will refer to out loud.

**A raise signals; it never yanks.** Landing a new view on someone who went back to read
something is the same mistake as auto-scrolling the conversation to a new message. The
return-to-live control carries a dot instead. If the agent happens to raise exactly what
they went back to, they are simply live again and there is nothing to signal.

### Where they went is reported; the cursor still is not

Amended August 19, 2026. Those two read as one fact and are opposite in kind, which is why
the paragraph above could be right and still leave the agent blind.

*Which entry this window is parked on* is **state**, it is the window's, and reporting it
would let a phone move the desktop. That stands exactly as written. *That the person went
somewhere* is an **event**, and an event on the agent's own surface is something the agent
should perceive — because the next thing they say is usually about it. "这个数字不对" is
unreadable if the agent believes its own last raise is what is in front of them, and the
failure is silent: it answers confidently, about the wrong board.

**So a move posts, and posts as a perception.** `POST /api/in/view` is the inbound half of
a channel that until now only went out. It carries *where they went* and never *which
window went there*, and it changes nothing about the appearance: `GET /api/out/view` is
what it was, no version bump, no snapshot, and a second device still shows what the agent
raised. The report is read, not applied — the content slot is still the agent's alone, and
this is still not a second writer of it.

**It does not drive a turn.** Someone walking the band through five tiles must not produce
five turns, and an agent that remarks on every tile you touch is unusable. The move lands
in the log and is read into the next turn's context, which is exactly when it is needed:
the moment the person speaks. Same shape as the frame on the stage lane — the window tells
the backend something true about itself, and nothing happens until something else asks.

**The newest move wins, across every window.** One person owns an install
([`topology.md`](topology.md#identity)), so two windows are two of their eyes and not two
people, and the last place they went is the best available answer to where they are
looking. A raise onto that same destination clears it, because that is the client's own
rule for going live again — made in the same place the raise is recorded, so the two
cannot drift.

**It goes stale, and says so instead of pretending.** A window that reloads is live again
and never announces it, so the fact can outlive the looking. The turn therefore reads it
with its age attached — *they went to `factory/drive` 40s ago* — which the agent can weigh,
unlike a bare assertion about where someone is. And it is in-memory: after a restart the
agent has no idea where anybody is looking, which is the truth.

**Same destination, one entry** — the ref when there is one, the module when there isn't.
Two raises of `factory/tasks` are one place, because both re-resolve to the same
recompiled board; two different inline views are two artifacts and both stay. This is the
same named/inline split `refresh_sources` turns on, and it decides what re-opening means:
a named view comes back as what it *is now*, an inline one only ever as what it *was*.

**The person may go to a place; the agent decides what to raise.** A dozen views ship with
no way to reach any of them except asking, which is the interaction cost of a chatbot
sitting on top of what is otherwise an app. `GET /api/views` is the inventory and
`POST /api/views/open` compiles one for a window to mount — deliberately not a third
writer of the appearance. The condition view is not in the inventory: it is the host's,
and offering it would let a person summon an outage that isn't happening.

**The row is a person-owned subset, not the inventory** — amended August 18, 2026,
because the condition this document set for adding one arrived. It said there was no
pinned subset yet, that a dozen views fit one row, and to add it when the row got long
enough to be a problem. What is in the tree after a fortnight of building is that dozen
plus every one-off any builder ever wrote — `entry`, `entry b`, `entry mlat`, `mount b`
— so the row became a list of the agent's scratch files with the surfaces a person
actually wants buried among them, which is worse than the asking it replaced.

So the floor is the **system views** (`factory/`), which are how a person reaches their
tasks, their memory and their files at all and are therefore always in the row and never
removable. Everything above the floor is there because the person put it there: the star
on a history card keeps a view, the cross on a chip drops it. `GET /api/views` still
reports the whole tree — the inventory is the truth about what exists — and now marks
each entry `system` and `bookmarked`; `POST /api/views/bookmarks` is the one write.

**A bookmark is server state, and the cursor is not.** They look alike and are opposite:
which entry a window is parked on is that window's own, like scroll position, while a
bookmark is a thing the person decided once and must find again on the phone. Kept refs
live in the config store rather than the views tree, which is disposable and re-seeded on
every boot — an upgrade replaces `factory/` wholesale and must not take the person's row
with it.

**Only a named view can be kept.** An inline view is only ever the content-addressed
artifact it compiled to, in a cache that prunes; a bookmark to one would be a bookmark to
a hash that stops resolving. This is the same named/inline split that decides what
re-opening means, applied to the same question one step earlier.

## The tile is a picture of the raise

Amended August 18, 2026, reversing *marks, not screenshots*. The history row's box was a
coloured initial derived from the view's identity, on the argument that a view is a live
React app with no moment at which its pixels are available: capturing at replace time
reaches a module that may already be unmounted, and re-mounting a named view offscreen
renders *today* rather than the record it stands in.

That argument assumed the browser had to be the person's. It doesn't. `view_render`
already drives a headless Chromium over the `/render/view` host page for
`hi_review_view`, so a raise can be rendered **at the instant it is raised** — the same
module, the same moment, the same frame the window reported. It is not a reconstruction
of the past; it is a second camera on the present.

Three properties keep it affordable, and each is load-bearing:

- **Content-addressed.** The key is the compiled module's own hash, so an artifact
  renders once no matter how often it is raised, and a recompile is correctly a
  different picture. `views/_shots/` sits beside `_compiled/` and is disposable in
  exactly the same way.
- **One at a time.** A `show, say, show, say` walk-through would otherwise put a browser
  per beat on the machine already running the agent.
- **Off the write path, and silent.** `apply` has returned before the browser opens. A
  capture that fails, blanks or times out writes nothing and the tile stays on its mark
  — which is the whole of the old design, still there as the floor.

The capture bumps the appearance version when it lands, so the picture reaches the
windows already watching, but **writes no snapshot**: a picture of a raise that already
happened changes nothing about what was on screen, and a state identical to its
predecessor and dated later is exactly the noise reflection has to read past.

The window reports its **skin** on the stage lane beside its frame, for the same reason
it reports the frame: the page is the only thing that knows, and a light picture of a
view the person saw dark is a wrong record. `hi_review_view` still renders both skins —
a review exists to catch the colour that resolves in one and not the other.

## What stays on the wire, and what does not

`GET /api/out/view` carries the `content` and `condition` slots in z-order, each tagged
with the slot it came out of, plus the history above. The tag is what lets a window
showing a past view keep the condition layer over it — an outage must still cover what
the person went back to, and z-order alone could not say which of the two layers that
was. **The conversation and the camera are not added to it.**

They are constant participants — never absent, never dismissable, nothing to converge on
— so a slot for them would be state with one value, and `on_screen()` would start
reporting an id the agent cannot act on, which only invites it to try
([`view_bus.rs:333`](../../src/foundation/server/view_bus.rs)). The appearance state stays
the record of what the agent has expressed. The client materializes the constant
participants locally, and the compositor sees a uniform list either way.

## The compositor

`floorLayout` is replaced by one pure pass over roles, keeping the existing shape (pure,
deterministic, no solver, unit-tested in `core/layout.test.ts`):

```ts
stage({
  content: boolean,          // the agent's view is up
  ownsConversation: boolean, // the topmost content view renders the words itself
  collapsed: boolean,        // the person put the conversation away
}): {
  conversation: "popover" | "pill" | "hidden";
  camera: "fill" | "pip";
  demote: number;            // presence fade, unchanged
}
```

**Nothing in the input is a measurement.** `width` left with the rail's threshold, and
`condition` never landed. `camera` left with the second box: it was read only to decide the
stage was *occupied* and so push the conversation out of the middle of the room, and there
is no middle of the room to be pushed out of any more — the notice is a view on the `view` plane and changes no
geometry. What remains is what is on screen and what the person asked for, which is the
whole of what placement depends on.

**Presentation changes are a class flip, never a remount.** The rule
[`Shell.tsx:26`](../../src/appearance/web/src/ui/Shell.tsx) already states for the camera
now covers the chat: `<Chat>` and `<CameraPreview>` are mounted once, above `ViewSlot`, and
the pass only changes their props and classes. Remounting the chat on a presentation
change would throw away the scroll position and every page of scrollback already
fetched through `/api/messages?before=`, which would make moving between presentations
cost a round trip and land the person somewhere they did not ask to be. The popover is a
class on that same element for exactly this reason — a portal or a `<dialog>` would be a
second mount of the one surface that must not lose its place.

`.hi-view-fill` keeps its job — it insets the frame the content view is handed — and its
insets are now the window chrome and the bottom band, both constant. The frame no longer
changes size with anything the person does to the conversation.

## The frame is fixed, and overflow scrolls

The compositor gives a view a frame, and the frame never grows: it is the window minus the
insets, in both axes. So the document has to say what happens to a view whose content is
taller than that, and it did not — the planes below settle *who covers whom* down to the
digit and said nothing at all about *where the rest of it goes*. What the code did in the
silence was lose it: `.hi-root` is `overflow: hidden`, `.hi-view-fill` declared no overflow,
and everything past the frame's bottom edge was cut with no scrollbar and no gesture that
could reach it. The bottom of a view did not exist.

**The host guarantees the whole of a view is reachable; the view is still composed for the
first screen.** Two halves, and both are load-bearing:

- **`.hi-view-fill` scrolls vertically, `auto`.** A view that fits — the poster, the shape
  everything else here is written for — is untouched and shows no scrollbar. Only one that
  overruns becomes scrollable. Horizontal stays clipped: a view wider than its frame is a
  layout that failed, and a horizontal scrollbar dresses that up as a feature.
- **Nothing about this relaxes the composition.** The person is being *talked through* a
  view and may never scroll it, so what matters still belongs above the fold. The scroll is
  a floor under the failure, not a canvas that grew.

The reason this cannot be pushed onto the view author is that the frame moves under them.
It changes when the window is resized, and between the viewport
`hi_review_view` renders at and the one the person actually has. "Make it fit" is advice
about a frame you know the size of. Every bundled review surface had already worked this out
alone and hand-rolled the same `height: 100%; min-height: 0; overflow-y: auto` root, six
times; agent-authored views mostly had not, and clipped.

The scroll alone was not enough, and the second half is the more interesting one. The layer
stretched a view's root to the frame with a grid track — `grid-template-rows: 1fr`, chosen
so that stretching the *used* height would leave a view's own `min-height` intact. `1fr` is
`minmax(auto, 1fr)`, and a grid item's `auto` minimum is content-based only while its
`min-height` is `auto`. A view writing `min-height: 100%` on its own root — which is what
they write, because the guide tells them to fill the frame — replaced that with a definite
one-frame-and-no-more, so the track stopped at the frame and the item was *stretched to* it.
The root's box then ended at the first screen while its content ran on past: its ground
stopped there, and its transparent bottom border — the band that holds the last line clear
of the controls — stayed up there with it, so the final row of a scrolled view sat flush
under the mic and camera buttons. The frame is now a plain block and the root is floored
with `min-height: 100%` instead, which is a floor and not a ceiling: it fills the frame with
less than a frame's worth and grows past it with more.

**Consequence, accepted:** a ground pinned with `position: absolute; inset: 0` scrolls away
with the content, because an absolutely positioned box in a scroll container scrolls with
it. A ground set as the root's own `background` does not — it paints on the root's border
box, which now grows with the content. The second is already what the view guide tells
authors to write, and it is the one that stays right at any content height.

## Layers

Placement has two authorities today and they disagree. `floorLayout` decides docked / pip
/ hidden; the stylesheet decides who covers whom, in eleven `z-index` declarations across
`ui/global.css` and one inline `zIndex: 50` in `ViewSlot.tsx`. The conversation vanishing
is exactly that disagreement — `layout.ts` calls it a participant, `.hi-stage { z-index: 2 }`
parks it under the view plane at 50, and neither file is wrong on its own terms.

The numbers are also ties and gaps: `.hi-stage` and `.hi-selfview` both sit at 2, and the
pill, the input line and the controls all sit at 60 — so among those, covering is settled
by the order of JSX in `Shell.tsx`. The gaps (2 → 8 → 50 → 55 → 60 → 80) are the reflex to
leave room for the next surface, which is how there came to be eleven. One of them,
`.hi-history` at 8, belongs to a History drawer no component renders.

### The stack

Three planes. A surface belongs to exactly one, always, and each plane carries its own
internal order.

| | Plane | What it is | Inside it, bottom to top |
|---|---|---|---|
| 0 | `ground` | the room | the paper (`.hi-presence`), the grain (`.hi-atmosphere`) |
| 1 | `view` | everything the agent put on screen | the content view, the condition notice over it |
| 2 | `cover` | everything the host owns and the agent can never occlude | the camera self view, the conversation (the line being written included, in its foot), the controls, alerts |

**The order has a meaning, not just a value: the agent's plane is below the person's.**
Nothing the agent shows can rise above the record of what was said or the controls to
answer it. That one sentence settles the next ten of these questions without another
argument.

`cover` names the invariant, not the geometry. It was written when the conversation was a
rail that covered nothing — it tiled with the content, and was on `cover` only because a
view must never be able to occlude it. As a popover it does cover, which changes nothing
here: **the plane says who *may* cover whom; the compositor decides whether they overlap at
all.** The invariant was always the point, and the geometry was always the compositor's.

Two things that were previously settled by accident now fall out instead of being declared:

- **The condition notice is a view, so it cannot cover the conversation.** It is a view in
  every other sense already — the `condition` slot, mounted through `ViewSlot`, second in
  the wire's array — so putting it on the `view` plane needs no new rule and still gets the
  right answer: an outage, or an exhausted-energy gate, takes the agent's half of the screen
  and leaves the record, the input and the controls alone. Previously it was full-bleed over
  everything except whatever happened to sit at 60.

  Its `vendor-outage.geom.json` sidecar is deleted with this, and that is the interesting
  half. It declared `owns_captions: true` — but the notice renders a fixed bilingual
  message, not the conversation, so under the trait's own meaning the claim was never
  true; it was a way to suppress the caption band behind a notice that read badly with one
  floating over it. Carried forward as `owns_conversation` it would have gone from
  cosmetic to harmful: it would take down the record and the line at the exact
  moment the person needs to read what happened and go fix it. No bundled view declares
  anything now, and a test says so.
- **Order inside `view` is the wire's order.** `ViewBus::wait_state` already emits content
  first and condition second, and paint order inside a stacking context is DOM order — so
  the view plane needs no `z-index` at all.

The camera self view moves to `cover` in both its presentations and stops changing plane
with its shape. It is the person's own surface. Nothing is lost: it is only ever the
backdrop when no view is up, so it never had a view to be behind.

**Nothing needs to sit between the planes**, which is the test of whether three is enough.
The two candidates were the condition notice, now inside `view`, and the camera backdrop,
now inside `cover` and only ever present when the `view` plane is empty.

### What keeps it honest

- **Three global numbers, and they are 0, 1, 2.** One `:root` block declares `--z-ground`,
  `--z-view`, `--z-cover`, and the three plane wrappers are the only rules that read them.
  `.hi-root` takes `isolation: isolate`, so nothing outside can interleave with them.
- **Each plane is a stacking context, so ordering inside one is local.** A tie inside
  `cover` — the camera pip against the controls — becomes a small question with an obvious
  answer that nothing outside `cover` can be affected by. Today that same tie is at 60,
  against the whole application. **This is what makes three planes stricter than six
  layers rather than looser: the numbers do not go away, they stop being global.**
- **Inside a plane, `z-index` is a single digit.** A contained context never needs 50, so
  any value above 9 is evidence that someone was fighting a stack they could not see. That
  is the vitest: outside the token block, no literal `z-index` above 9, and no inline
  `zIndex` in a `.tsx` at all — which is exactly what `ViewSlot`'s 50 is today.
- **An agent view cannot escape its plane.** A view writing `z-index: 9999` climbs to the
  top of `view` and no further. Already true, by accident of `ViewSlot`'s `position: fixed`
  + `zIndex` wrapper; it becomes a stated rule with a test, because the code running there
  is agent-authored.
- **The stack is static; only geometry is computed.** `stage()` returns presentations and
  frames, never a plane, because a surface's plane never depends on what else is on screen.
  That is what ends the two-authorities problem: CSS owns *over whom*, fixed and declared
  once; the compositor owns *where*, computed per state. They can no longer disagree
  because they no longer overlap.

The conversation's own flip disappears here. Today it moves between z 2 as the chat and z
60 as the pill; on three planes it is on `cover` in all three presentations, and the only
thing that changes between them is the box the compositor hands it.

### The keyboard follows the planes

*August 18: the stack settled who covers whom and `pointer-events` settled whose a click
is. Keys had no rule at all, and the plane order turned out to be the answer for them too.*

**A keystroke does not travel down the stack.** It starts at whatever holds the focus and
bubbles up through the document to the window — so `z-index` cannot reach it, and neither
can the cover plane's `pointer-events: none`. An agent view that writes
`window.addEventListener("keydown", …)` hears every key in the window, whichever plane it
was typed into. A deck binds Space and the arrows to page itself; the person clicks into
the line to write a message; every Space they type pages the deck and never lands in the
line, because the view called `preventDefault()` on a keystroke aimed at the host's own
input. The controls are in the same position — Space on a focused button.

**So: the keyboard belongs to the plane the focus is in**, in the same order as paint. The
agent's plane is below the person's there too.

| Focus is in | Who hears the key |
|---|---|
| `cover` — the line, the controls, any host surface | the host alone; the view is never told |
| `view` | the view alone; the host's *global* affordances stand down rather than firing alongside it |
| neither — the room | both, the host first, and a host claim (a `preventDefault` from one of its handlers) ends it there |

The third row is what keeps a deck working: Space and the arrows are not printable by
start-typing-to-open's test, so with the focus nowhere in particular they pass through
untouched, and a letter that *does* open the conversation is claimed and stops.

**Enforced once, at the document** ([`lib/keyboard.ts`](../../src/appearance/web/src/lib/keyboard.ts)),
for the reason `nativeFeel.ts` installs itself there: the code on the other side is
agent-authored and cannot be asked to check whose key it is. The guard sits one node short
of the window, which is the whole trick — React's delegated handlers have already run
(React attaches at the root container, below the document), so chrome behaves exactly as
before, and `stopImmediatePropagation` then ends the key before any listener on the
document or the window sees it.

- **Host chrome's own global keys go through the same guard**, not through `window`: an
  `onHostKey` registry the guard runs itself. Escape closing the popover and
  start-typing-to-open are the two. Registration order is preserved and `defaultPrevented`
  still reads true from a surface below, so the Escape ladder — clear the half-written
  line, and only an empty line closes the panel — is untouched.
- **`keyup` and `keypress` are routed with `keydown`**, so a view counting a key down and
  up is never handed half a press whose start it never saw.
- **A sensor is not a handler.** The one listener that wants every interaction regardless
  of plane — the audio context's resume-on-first-gesture — moves to the capture phase,
  ahead of the guard, rather than being excepted by it.

A view binding on the window is *not* the mistake here, and is not asked to change: bubble
listeners are exactly what the guard is in front of, so a deck keeps its Space and its
arrows for every key the person did not aim at chrome. **What this does not reach** is a
view that registers in the *capture* phase on the window — capture runs before the target,
so nothing at the document can get in front of it. That one line is written where view
authors read it (`identity/workers/view-builder.md`), as the only key-binding rule they
have to keep.

## What this reverses

*"A stage has room for a line, not a transcript"*
([`Shell.tsx:101`](../../src/appearance/web/src/ui/Shell.tsx)) is wrong, and it is wrong
for a reason worth writing down: it was true of the **caption band**, which was a timed
reveal of an utterance that had already been spent and evicted — a band of that kind gets
worse the longer it is, and the note in `arch-refactor.md` that *"an N large enough to
never lose anything makes the face a chat log"* was correct about it.

It stopped being true on August 11, when the conversation became an append-only list that
keeps. Nothing about a list gets worse for being visible longer. The line survived its own
premise.

The pill's timer (*The pill is timed*, above) is not that premise coming back. It is the
same argument read the other way: the band was timed because its words were *spent*, and
the pill is timed because its words are *not* — the copy can go precisely because the
original stays. What a stage has room for was never the question; what the screen owes a
sentence nobody can lose was.

`ViewSlot`'s *"every view owns the whole frame"* narrows to *"every view owns the frame it
is handed"*. The z-ordered stack it replaced stays abolished — this is composition between
**roles**, which are four and fixed, not between views, which were unbounded and piled up
fourteen deep.

## Rejected

**~~A drawer over the content.~~ Reversed August 17 — this is what we build.** The original
argument: *"the moment you open the conversation you cannot see the board, and the moment
you read the board you cannot see what was said about it. Simultaneous is the whole
point."* What it got wrong was how much simultaneity is worth paying for. Simultaneous
matters in the seconds around a hand-off — you read a line, you look at the board — and the
popover is simultaneous in exactly those seconds, over the right-hand ~420px of the board
rather than instead of it. The rail bought the rest of the hour, at a third of the window,
for a conversation nobody was looking at. See [The popover](#the-popover).

**The conversation as a compiled `_builtin/conversation.jsx`.** It is the uniform answer
and it puts the chat behind esbuild provisioning, a content-hash fetch, and
`refresh_sources` — for a surface whose failure mode must not exist. Bundled vs compiled is
the distinction that lets the conversation be a view without inheriting an artifact's
failure modes.

**A `conversation` slot in `ViewBus`.** State with one value, and an id on the wire that
the agent can read but not act on.

## Accepted consequences

- **The popover occludes the lower-right of the view while it is open.** That is the trade
  taken knowingly: a board's own right-hand column can be behind it, and the way to see it
  is the same gesture that opened the panel. The alternative was occluding nothing and
  paying a third of the window all the time.
- **The empty room is no longer a wide card in the middle of it** — the panel keeps its
  corner and its ~420px whether or not anything is behind it. The grandeur is the price of
  the conversation never being moved by something the agent did (*One box, whatever else is
  on the stage*).
- The content view is handed the whole frame in every state, so it no longer reflows when
  the conversation opens. (Under the rail this was the opposite consequence: *"a view built
  assuming the full window reflows"*.)
- Two windows on the same conversation can be in different presentations. That is intended
  — it is the same conversation, drawn to fit.
- The pill still exists, so there are two chat renderers to keep honest. They share
  `useMessages` and the pill renders one message, so the divergence is bounded to styling.
- Popover width is a host constant, not a preference. If it becomes one, it is a window
  preference like the collapse — never appearance state.

## Work

Built on `design/stage`, in this order:

1. **Compositor** — `stage()` replaces `floorLayout`, ten cases in `core/layout.test.ts`.
   No server change, no wire change.
2. **Planes** — three wrappers in `Shell`, three `--z-*` tokens in one `:root` block,
   `isolation: isolate` on `.hi-root`. Every `z-index` on the cover plane turned out to be
   removable rather than reducible: DOM order inside a stacking context is paint order, and
   the surfaces were already in the right order. `ViewSlot`'s inline `zIndex: 50` is gone,
   the orphaned `.hi-history` (82 lines, no component) is deleted, and `ui/planes.test.ts`
   holds the line at three global numbers and no `zIndex` in a style prop.
3. **Shell** — one `<Chat>` mount across all four presentations, hidden by a `data-shown`
   visibility flip rather than a branch; `SpeechText` becomes the pill; the input moves
   with the conversation.
4. **CSS** — the rail column, `--hi-rail` as the one number every surface insets past
   (`.hi-view-fill`, the camera in both shapes, the railed input), the narrow breakpoint.
   *(Undone by the amendment: `--hi-rail` is deleted and nothing insets past the
   conversation any more — see step 7.)*
5. **Traits** — `owns_captions` → `owns_conversation` through `types::ViewTraits`, the
   render URL, the sidecar contract and `channels/out/view.ts`, with `#[serde(alias)]` so
   sidecars and snapshots already on disk keep loading. The outage view's sidecar deleted.
6. **Controls** — the conversation toggle in `ChannelControls`, shown only when there is a
   rail to collapse; the reset button retitled from "back to the calm room" to what it now
   does, which is close the view.

7. **The amendment (August 17)** — `rail` → `popover` through the pass, the shell, the
   stylesheet and the controls, as one rename plus three deletions. Deleted: `--hi-rail`
   and every inset that stepped past it (`.hi-view-fill`'s left border, both camera
   shapes), `RAIL_MIN_WIDTH` with `stage()`'s `width` input, and `hooks/useViewport.ts`
   whole — a resize listener whose only reader was that threshold. Added: the panel's box
   (`--hi-pop-width` / `--hi-pop-right` / `--hi-pop-floor`, shared with the input line so
   the two land flush), a 180ms rise, and the dismissals — Escape, and a capture-phase
   `pointerdown` that spares the controls cluster so the toggle does not close-then-reopen.
   The toggle's glyph stops drawing a divided frame and draws a panel over a corner of it.

8. **The card, and the line inside it (August 18)** — `KeyboardFallback` becomes `ui/Composer.tsx`,
   rendered as `<Chat>`'s child and drawn in its foot; it is a `shadcn` `InputGroup`
   (`ui/shadcn/input-group.tsx`), so the single-line `<input>` becomes a textarea that
   grows, Shift+Enter breaks a line, and the send button lives in the box. Deleted:
   `.hi-kbd` and its four rules, the two `:has(.hi-kbd)` lifts, `Stage.input` with its
   `Input` type, `ChannelControls`' conversation button and glyph, and the text channel's
   `localStorage` pref — it starts on, and a put-away is this screen's for a minute rather
   than a setting still in force tomorrow. `.hi-chat` takes the card recipe off
   `.hi-stage--popover`, which is left holding placement alone, and gains the title row.

9. **The keyboard (August 18)** — `lib/keyboard.ts`: `keyOrigin` reads the plane off the
   event target, `hostHearsKey` / `viewHearsKey` are the two-line routing (unit-tested
   without a DOM, which is why they are separate from the listener), and `installKeyPlanes`
   is called from `main.tsx` beside `installNativeFeel`, before the first view import.
   `Shell`'s Escape and `Composer`'s start-typing-to-open stop being `window` listeners and
   become `onHostKey` registrations; the audio resume-on-gesture listener moves to capture.
   Checked against a real browser as well as the unit tests — a page holding both planes, a
   deck bound to the window, and trusted key events over CDP — because the load-bearing
   claim is about event order, which a unit test of the routing cannot see: a Space typed
   into the line lands in the line and does not page the deck, and the same Space with the
   focus anywhere else still pages it.

10. **One box, and the card names itself (August 19)** — `stage()` loses the `stage`
   answer and the `camera` input; `Conversation` is three words, not four. The stylesheet
   loses `.hi-stage--popover` (its geometry folds into `.hi-stage`, which is now the panel),
   the `min(880px, 100%)` card width, and the narrow-viewport `:not(…)` override that
   existed only to nudge the lead position's margins. The rise moves onto
   `[data-shown="true"]`, so it plays on an *open* rather than on the class change that used
   to mean "a view appeared". The captions dock stops borrowing `.hi-stage` and becomes
   `.hi-captions`, dropping the seven declarations that undid the box it never wanted. The
   card's title row loses the mark and reads *Conversation*; its foot loses the rule above
   the line. Escape arms whenever the conversation is up; the press-behind keeps its old
   condition — something actually behind the panel — and now spares the views band as well
   as the controls cluster.

**Left:** the camera is placed by `stage()` but is not yet described as the `self` role in
the wire vocabulary — it has no server-side existence to name, so this is naming, not
mechanism. And the whole thing wants eyes on a real window: the tests cover the pass and
the plane discipline, not whether the panel sits well over a five-column board, or which
part of a board people find themselves wanting back from under it.

## See also

[`text-transcript.md`](text-transcript.md) for what the conversation *is* ·
[`surfaces.md`](surfaces.md#carriers) for `hi_show` and why it is a call ·
[`view_bus.rs`](../../src/foundation/server/view_bus.rs) for the two slots and why the
write path decides which.
