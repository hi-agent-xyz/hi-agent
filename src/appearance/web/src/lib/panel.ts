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
// `PanelGesture` turns a finger, a click or a two-finger swipe into a call to
// `advance` / `retreat`.

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
// Kept here rather than in `ui/PanelGesture.tsx` so the rules can be tested at a
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

/**
 * The panel's own measure at a stop, in px. **Two values and never a third:** the
 * sidebar's measure, or the whole window.
 *
 * `leftOf` says where the panel's left edge is and this says how wide the box
 * behind that edge is laid out, and the two agree at rest — `leftOf(stop) +
 * measureOf(stop) === width` at every stop but `room`, where the same box is
 * pushed off the right-hand side. They come apart only under a finger: a drag
 * moves the edge pixel by pixel and the measure does not move at all, so the
 * panel is *revealed* rather than re-laid. That is the whole of why this function
 * exists — the content used to be laid out in whatever width the drag had
 * uncovered so far, which wrapped and re-wrapped every line sixty times a second
 * on the way in.
 *
 * The room borrows the measure of the stop it opens to, so pulling the panel in
 * changes no width at all: on a window that is the sidebar's, on a phone the
 * screen's.
 */
export function measureOf(stop: Stop, shape: Shape, width: number, panelW: number): number {
  const at = stop === "room" ? opened(shape) : stop;
  return at === "full" ? width : Math.min(panelW, width);
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

// --- The trackpad's arithmetic -----------------------------------------
//
// A laptop has no edge to swipe from. The strip answers a mouse, but finding a
// twenty-point band on a screen with nothing drawn on it is the one entrance this
// face has never been able to defend (`docs/arch/stage.md` § *The ways in, and the
// one that is thin*) — and it is defended here by the gesture the platform already
// taught: **two fingers sideways, anywhere at all, with nothing to aim at.**
//
// It is a step rather than a drag, and that is not a compromise. A finger on the
// edge is holding the panel, so the panel must follow it; two fingers on a trackpad
// are holding nothing, and a `wheel` stream has no end event — the deltas simply
// keep arriving as momentum, in units the browser is free to report in lines or in
// pages. What can be read off that honestly is a direction and an amount, which is
// one step on an axis.

/** A wheel delta in px, whatever unit the browser chose to report it in. Lines are
 * a rough text row and pages a rough screenful; both are guesses, and both only
 * have to be close enough to reach a threshold at a human speed. */
export function rolled(delta: number, mode: number): number {
  if (mode === 1) return delta * 16;
  if (mode === 2) return delta * 100;
  return delta;
}

/**
 * Whether a wheel event is meant sideways at all.
 *
 * A two-finger scroll down a long conversation is never perfectly vertical, and a
 * face that took every stray pixel of sideways drift as an intention would close
 * itself while someone was reading. Twice as much across as down is the line: a
 * deliberate sideways swipe is nearly all `deltaX`, and a drifting vertical one is
 * nearly none.
 */
export function sideways(x: number, y: number): boolean {
  return Math.abs(x) >= 1 && Math.abs(x) > Math.abs(y) * 2;
}

/** How much sideways travel makes one step. Deliberate enough that a flick of drift
 * cannot reach it, short enough to be one comfortable swipe. */
export const SWIPE_PX = 60;

/**
 * Which stop a run of sideways travel lands on, and it is never more than one away.
 *
 * The sign follows the panel rather than the fingers: the panel lives off the right
 * of the screen, so scrolling **right** — which on a trackpad is fingers moving
 * left — brings it in, the same direction the same hand would drag the edge.
 */
export function swiped(travel: number, from: Stop, shape: Shape): Stop {
  if (travel >= SWIPE_PX) return advance(shape, from);
  if (travel <= -SWIPE_PX) return retreat(shape, from);
  return from;
}
