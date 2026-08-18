# The stage

**Status:** proposed and built August 11, 2026, on `design/stage`. **Amended August 17,
2026 — the rail is a popover:** the conversation no longer holds a column beside the
agent's view, it opens as a panel over it, out of the corner the controls are in. That
reverses this document's own *Rejected* entry, and the reversal is argued where the rail
was described. Everything else stands. Defines what may be on screen at once, and how the
conversation, the agent's views and the host's own surfaces share it. Supersedes the
placement half of `core/layout.ts`'s doc comment and the "every view owns the whole frame"
rule in `ui/ViewSlot.tsx`.

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
| `conversation` | the host — always present | nothing writes it | the stage alone · a popover over content · the pill when put away |
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

### 3. The conversation has three presentations, one mount

| Presentation | When | What is on screen |
|---|---|---|
| **stage** | nothing in `content` | the chat fills the frame — today's default face, unchanged |
| **popover** | `content` is up and the person has not put it away | the chat in a fixed-measure panel over the content, rising out of the controls' corner; full scrollback, own input |
| **pill** | the person put the popover away | the newest line, floating over the content — today's docked caption |

`SpeechText` survives exactly here: **the pill is the conversation collapsed, not a
separate surface.** One source, three presentations, and the person moves between them.

**The input follows the conversation.** On the stage it is the centered line it is today;
with the popover up it sits under the panel as its foot, the same width and the same right
edge, under the messages it is adding to; put away it returns to the floating centered
line. The input is where the conversation is, because typing into a line that is nowhere
near the messages is what makes a chat feel like a command bar.

## The popover

*August 17: this section replaced one called **The rail**, which specified a ~400px column
beside the content, built and shipped that way. What it argued, and why it lost, is below.*

The conversation opens as a panel over the content, pinned to the **bottom right**, rising
out of the button that opens it. Not a column beside the content, which is what this
document originally specified and what was built.

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
carries the full scrollback and the input, and the pill still holds the newest line behind
it. The conversation still never degrades; it now stops charging rent.

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
behind ignores the controls cluster and the input line, which are the panel's own foot and
its own button standing in separate boxes.

## Who decides the presentation

- **By default, nobody:** it is derived — `content` present → popover; no content → stage;
  put away → pill.
- **The person** toggles popover ↔ pill from the channel controls, and dismisses the
  popover with Escape or a press behind it.
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

## What stays on the wire, and what does not

`GET /api/out/view` keeps carrying exactly what it carries today: the `content` and
`condition` slots, in z-order. **The conversation and the camera are not added to it.**

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
  camera: boolean,           // the self view is live
  ownsConversation: boolean, // the topmost content view renders the words itself
  collapsed: boolean,        // the person put the conversation away
}): {
  conversation: "stage" | "popover" | "pill" | "hidden";
  camera: "fill" | "pip";
  input: "center" | "popover";
  demote: number;            // presence fade, unchanged
}
```

**Nothing in the input is a measurement.** `width` left with the rail's threshold, and
`condition` never landed — the notice is a view on the `view` plane and changes no
geometry. What remains is what is on screen and what the person asked for, which is the
whole of what placement depends on.

**Presentation changes are a class flip, never a remount.** The rule
[`Shell.tsx:26`](../../src/appearance/web/src/ui/Shell.tsx) already states for the camera
now covers the chat: `<Chat>` and `<CameraPreview>` are mounted once, above `ViewSlot`, and
the pass only changes their props and classes. Remounting the chat on a stage→popover
transition would throw away the scroll position and every page of scrollback already
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
| 2 | `cover` | everything the host owns and the agent can never occlude | the camera self view, the conversation, the input line, the controls, alerts |

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
  cosmetic to harmful: it would take down the record and the input line at the exact
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
