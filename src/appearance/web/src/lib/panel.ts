import type { Shape } from "./shape";

// Where the person's side of the screen is, as one number.
//
// The host used to have three openable things — the conversation, the views
// navigator, the channel controls — each drawn one way on a window and another
// way on a phone, each with its own dismissal. They are one box now (*the
// panel*), and where it is, is a stop on one axis: `docs/arch/stage.md` §
// *The panel, and the axis it runs on*.
//
// This file is the whole of that axis, and it is pure: which stops a shape has,
// and how to step between them. What it deliberately does not know is anything
// about pixels, gestures or the DOM — the stylesheet sizes `panel`, and
// `PanelEdge` turns a finger into a call to `advance` / `retreat`.

/** The three positions. `room` is the panel off-screen, not a fourth surface. */
export type Stop = "room" | "panel" | "full";

/**
 * The stops a shape can actually reach, from most content to least.
 *
 * The phone skips the middle one, and that omission is the one piece of window
 * measurement worth keeping: below ~640px a screen cannot be split into two
 * usable columns, which is the same threshold the rail died of
 * (`docs/arch/stage.md` § *The popover*). The television skips `full` for the
 * opposite reason — it is all room, and covering the whole of it to read a
 * message list is not what a television is for.
 */
export function stops(shape: Shape): Stop[] {
  switch (shape) {
    case "phone":
      return ["room", "full"];
    case "tv":
      return ["room", "panel"];
    case "wide":
      return ["room", "panel", "full"];
  }
}

/** Where a freshly loaded page starts.
 *
 * A wide window opens showing both, because the corner that used to advertise
 * the way in is empty now and bare paper would have no visible entrance at all.
 * The phone and the television have no room to show both, and both have an
 * entrance a person already knows — the edge swipe, the D-pad — so they open on
 * the room. Never persisted: like the collapse it replaces, the stop says what
 * this window is showing right now and does not outlive the page. */
export function initial(shape: Shape): Stop {
  return shape === "wide" ? "panel" : "room";
}

/** One stop toward the panel, or where you already were at the end of the axis. */
export function advance(shape: Shape, from: Stop): Stop {
  const order = stops(shape);
  return order[Math.min(order.indexOf(from) + 1, order.length - 1)] ?? from;
}

/** One stop back toward the room. */
export function retreat(shape: Shape, from: Stop): Stop {
  const order = stops(shape);
  return order[Math.max(order.indexOf(from) - 1, 0)] ?? from;
}

/**
 * The stop a shape lands on when something asks for the panel by name rather
 * than by stepping — a printable key opening the conversation, a D-pad's right
 * press. The first stop that is not the room, so a phone gets its whole screen
 * and a window gets the middle.
 */
export function opened(shape: Shape): Stop {
  return stops(shape)[1] ?? "room";
}

/**
 * How many things the panel would close before Back means "leave the app".
 *
 * The television's ladder (`lib/tvBack.ts`) counts rungs, and the panel is worth
 * as many rungs as it has stops behind it — so Back walks `full` → `panel` →
 * `room` one press at a time rather than dropping the whole thing at once.
 */
export function depth(shape: Shape, at: Stop): number {
  return stops(shape).indexOf(at);
}

/** Whether the view plane has to give up width at this stop. `full` does not
 * inset the view — it covers it — so only the middle stop pushes. */
export function pushes(at: Stop): boolean {
  return at === "panel";
}

// --- The gesture's arithmetic ------------------------------------------
//
// Kept here rather than in `ui/PanelEdge.tsx` so the rules can be tested at a
// table of numbers instead of through a synthesised pointer, which is the same
// reason `lib/spatial.ts` keeps `nearest` out of its listener.

/**
 * Where a stop puts the panel's **left edge**, in px from the window's left.
 *
 * Every stop is a position for that one edge, which is what makes sliding in,
 * sliding out, widening and narrowing a single motion with different endpoints
 * rather than four animations to keep in step.
 */
export function leftOf(stop: Stop, width: number, panelW: number): number {
  switch (stop) {
    case "room":
      return width;
    case "panel":
      return Math.max(0, width - panelW);
    case "full":
      return 0;
  }
}

/** A flick: fast enough at release that the person clearly meant to throw the
 * panel, however little of it they had actually dragged. In px/ms; negative is
 * leftward, which is the direction that brings the panel in. */
const FLICK_PX_PER_MS = 0.5;

/**
 * Which stop letting go here lands on.
 *
 * Distance OR speed, never both — a slow, deliberate drag most of the way means
 * it, and so does a short hard flick; requiring both is what makes a gesture feel
 * like it has to be performed correctly rather than merely meant.
 *
 * A flick takes the neighbouring stop **in the direction of travel**, which is
 * why it is not simply "nearest": thrown leftward from `room` on a window, the
 * panel stops at `panel` rather than carrying on to `full`, because one throw is
 * one step and the axis is what tells you how far you have come.
 */
export function settle(
  left: number,
  vx: number,
  from: Stop,
  shape: Shape,
  width: number,
  panelW: number,
): Stop {
  if (width <= 0) return from;

  if (vx <= -FLICK_PX_PER_MS) return advance(shape, from);
  if (vx >= FLICK_PX_PER_MS) return retreat(shape, from);

  let best = from;
  let bestGap = Infinity;
  for (const stop of stops(shape)) {
    const gap = Math.abs(left - leftOf(stop, width, panelW));
    if (gap < bestGap) {
      bestGap = gap;
      best = stop;
    }
  }
  return best;
}
