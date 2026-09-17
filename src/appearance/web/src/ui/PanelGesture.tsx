import { useEffect, useRef } from "react";
import { flushSync } from "react-dom";
import {
  advance,
  leftOf,
  pushes,
  reach,
  retreat,
  rolled,
  settle,
  sideways,
  type Stop,
} from "../lib/panel";
import type { Shape } from "../lib/shape";

/**
 * Every way a hand moves the panel: **one strip on its edge, and two fingers
 * anywhere at all.**
 *
 * Both do the same thing to the same number. The panel lives off-screen to the
 * right and comes in at one of three stops (`lib/panel.ts`, `docs/arch/stage.md`
 * § *The panel, and the axis it runs on*); every stop is a position for its left
 * edge; a gesture moves that edge and lets go. What differs between a finger, a
 * mouse and a trackpad is only how the hand says where the edge should be, and how
 * the letting-go is noticed.
 *
 * ## The strip
 *
 * It stands on **the panel's own left edge, wherever it is.** At `room` that edge
 * is the window's right-hand side, so the strip is the way in; at `panel` it is the
 * seam between the view and the panel; at `full` it is the window's left-hand side,
 * so the strip is the way back. **One handle, and it is always the place the two
 * halves of the screen meet.**
 *
 * *This replaced a pair of strips nailed to the window's own two edges — the right
 * one advancing a stop, the left one retreating. The pair was the whole mental
 * model and it was wrong in exactly one place, which happens to be the desktop's
 * resting arrangement: with the panel beside the view, the thing a hand reaches
 * for is the boundary it can see, not the far edge of the window. The seam is that
 * boundary, and at the other two stops it lands precisely where each of the old
 * strips already was. It also gives back the twenty points the pair charged for at
 * the middle stop — a tap on the panel's right-hand edge is the panel's again.*
 *
 * **Direction is read off the drag, not off which strip was grabbed**, which is
 * what lets one handle do both jobs: pulled left the seam advances a stop, pulled
 * right it retreats one, and `settle` was always computing that from the position
 * alone.
 *
 * *What the pair worked out is kept whole: the drag is real — the box tracks the
 * finger pixel for pixel, comes back if you change your mind, and leaves on
 * distance **or** speed. A handler that merely detected a swipe and then played a
 * canned animation is the thing this is not.*
 *
 * **It is a strip and not a listener on the panel**, because on iOS the browser
 * decides who owns a touch at `touchstart` and never revisits it. A pan that
 * begins over the scrollback belongs to the scroller by then, so asking for it
 * afterwards gets nothing. A strip declares `touch-action: none` up front over the
 * ~20px where the gesture can begin — the same width the system's own edge
 * gesture claims.
 *
 * **A mouse's press is claimed the same way, and for the same reason.** Pointer
 * capture moves the pointer's events to the strip; it does not move the browser's
 * own reading of the press, and WebKit reads a press dragged across a page as a
 * text selection. Unclaimed, one drag on the edge selected 31,508 characters of the
 * board and the conversation, highlighted across both halves of the screen.
 * Chromium selected none, which is how it got past a check in Chromium.
 *
 * **A tap in those twenty points may do nothing but move the panel**, and there is
 * now exactly one such band on screen instead of two — the same trade iOS makes
 * for its own edge, charged once.
 *
 * ## The trackpad
 *
 * **A laptop has no edge to swipe from, and hunting for a twenty-point band on a
 * screen with nothing drawn on it is not something anyone should have to do.** The
 * strip answers a mouse and the keyboard answers a typist, but the thin entrance
 * `docs/arch/stage.md` § *The ways in* has always named as its weakest point is the
 * pointer's, and a hairline that appears once you are already at the edge only
 * helps someone who went there. Two fingers sideways is the gesture the platform
 * has already taught, it works anywhere on the screen, and there is nothing to aim
 * at.
 *
 * **It follows the fingers, exactly as the strip follows a thumb.** It shipped once
 * as a step — swipe far enough, get a stop, no tracking — on the grounds that a
 * `wheel` stream has no end event, keeps arriving as momentum after the hand has
 * lifted, and may be reported in lines or pages. Every one of those is true and
 * none of them forces a canned animation; they only mean the *end* of a run has to
 * be inferred from silence rather than announced. So the panel tracks, and the run
 * is over when the frames stop.
 *
 * **What momentum cannot do is carry the panel somewhere nobody asked for.** A run
 * moves the edge to the neighbouring stop and not one pixel further (`reach`), so a
 * hard flick out of the room comes to rest beside the view rather than over it and
 * the surplus is absorbed against the detent. That is the touch flick's *one throw
 * is one step*, enforced as a range because here it has to hold against travel the
 * hand did not make. The pointer is not clamped this way and should not be: every
 * pixel of a drag is the hand's.
 *
 * **The gesture has to be taken hold of before it moves anything.** A few pixels of
 * sideways drift is not a swipe, so a run banks its travel until it passes
 * `GRAB_PX` and only then grips the edge — which also leaves those first frames
 * with the browser, where they belong if it turns out to have been a scroll.
 *
 * **A scroller under the pointer keeps its own sideways gesture.** Any board with a
 * wide table in it scrolls that way; if anything in the path can scroll across, the
 * roll is theirs and this never sees it. It is not even conditional on their having
 * room left to scroll — a strip that has reached its end and hands the whole panel to
 * the next flick is a worse surprise than one that simply stops. The views tab was the
 * other example and is no longer one: its rows wrap now, so a sideways roll over them
 * reaches the axis.
 *
 * ## What they share
 *
 * **The page is moved by a variable, not by React state.** A state update per
 * `pointermove` would re-render the shell — the conversation, the tabs, everything
 * — sixty times a second to move one box, so a gesture writes `--hi-panel-left`
 * straight onto the elements that ride the edge (`[data-rides-edge]`: the panel and
 * this strip) and the stylesheet turns it into geometry for each. React learns the
 * stop when the gesture is over, which is when there is finally something to tell it.
 *
 * **Onto the riders and nowhere above them**, because where a variable is written
 * decides how much of the page every write re-styles. It was written on the root,
 * and in WebKit that cost 33ms of style for each step of the hand with a board up —
 * a drag held near thirty frames before anything was painted. On the riders, with
 * the variable registered as not inheriting, a step costs well under a millisecond
 * (`global.css` § `--hi-panel-left`).
 *
 * **The measure is not that variable, and it moves once a gesture at most.** The
 * box behind the edge is laid out at one of two widths (`measureOf`) and no gesture
 * touches it, so pulling the panel in *reveals* the conversation instead of
 * re-wrapping every line of it sixty times a second. The new measure arrives with
 * the stop at the moment of release, so the content re-lays once, while the box is
 * already on its way.
 *
 * **The content does not reflow under the hand, and the edge is a window onto it
 * too.** The view plane's inset is keyed on the committed stop, so a panel pulled
 * *in* slides over the board and the board re-lays once, on release. A panel pulled
 * back *out* from the middle stop is the other way round: it uncovers the board, and
 * a board still at its inset width would leave bare paper between its edge and the
 * panel's for the whole of the drag — measured at 400px on a seam drag and 252px on
 * a trackpad swipe. So the moment the edge passes the board's, the board takes the
 * width it is being revealed at (`data-revealing`) and re-lays once, behind the
 * glass. A compiled board re-flowing sixty times a second is the one cost of pushing
 * that would have made pushing not worth it, and this is still once a gesture.
 */

/** How long a settle takes. Named here because both ends need it — the stylesheet
 * animates it and this file has to know when it is over — and `panel.test.ts`
 * fails if the two ever drift apart. */
export const PANEL_MS = 260;

/** The strip a gesture may begin in. Matches the system edge, so a gesture that
 * starts too far inboard fails the same way here as it does anywhere else. */
const EDGE_PX = 20;

/** Under this much travel the gesture was a click, which is the mouse's way in —
 * there being no button to press (`docs/arch/stage.md` § *The ways in*). */
const CLICK_PX = 4;

/** How much sideways travel a wheel run banks before it takes hold of the edge.
 * A hand is never perfectly straight and neither is a trackpad, so this is the
 * difference between a swipe and a scroll that leaned; while it is unspent the
 * frames are still the browser's. */
const GRAB_PX = 24;

/** How long a `wheel` stream has to stop for before the run counts as over. There
 * is no end event to wait for: momentum keeps delivering frames after the hand has
 * lifted, and the only thing that marks the end of a run is the frames stopping.
 * Long enough to sit through the ragged tail of a slow swipe, short enough that
 * letting go does not feel like waiting. */
const REST_MS = 120;

interface PanelGestureProps {
  /** Which arrangement this is; it decides which stops exist at all. */
  shape: Shape;
  /** The stop the panel is resting at. */
  stop: Stop;
  /** Where to go. Called once, when the gesture is over. */
  onStop: (next: Stop) => void;
  /** The face's root box. It carries the stop and the gesture's own attributes, and
   * the riders of the edge are found under it, so a gesture names them as one set. */
  root: React.RefObject<HTMLElement | null>;
}

export function PanelGesture({ shape, stop, onStop, root }: PanelGestureProps) {
  // Read at gesture time rather than closed over, so a re-render between the
  // press and the release cannot leave a gesture committing a stale stop.
  const live = useRef({ shape, stop, onStop });
  live.current = { shape, stop, onStop };
  const settling = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (settling.current) clearTimeout(settling.current);
    },
    [],
  );

  /**
   * The gesture in flight, whichever kind of hand has it. Null between gestures.
   *
   * One record for both, because they are one gesture with two ways in: something
   * says where the panel's left edge should be, and something says when that has
   * stopped. Whichever hand grips first keeps it until it lets go.
   */
  const drag = useRef<{
    kind: "pointer" | "wheel";
    /** The stop it began at. Held rather than re-read, so a keystroke landing
     * mid-gesture cannot move the ground under a run already in flight. */
    from: Stop;
    /** Where the edge was when the gesture began. The pointer measures its travel
     * from here; the wheel accumulates onto it. */
    startLeft: number;
    /** Where the pointer was when it took hold. Only a pointer has one. */
    startX: number;
    /** Where the edge is now. */
    left: number;
    /** The last sample, for the release velocity — one segment rather than the
     * whole gesture, because a slow drag that ends in a flick is a flick. */
    lastT: number;
    vx: number;
    width: number;
    panelW: number;
    /** The furthest the hand has been from where it started, for the click test. */
    travelled: number;
    /** What the edge is written onto, found once at grip. */
    riders: HTMLElement[];
  } | null>(null);

  // These three close over refs alone and read `live.current` when they run, so the
  // copy the wheel listener captures on its first render behaves identically to the
  // one a later render would hand it.

  /** Take hold of the edge. */
  const grip = (box: HTMLElement, kind: "pointer" | "wheel", at: number, x = 0) => {
    const width = window.innerWidth;
    const panelW = measure(box, width);
    const from = live.current.stop;
    const startLeft = leftOf(from, width, panelW);
    drag.current = {
      kind,
      from,
      startLeft,
      startX: x,
      left: startLeft,
      lastT: at,
      vx: 0,
      width,
      panelW,
      travelled: 0,
      riders: [...box.querySelectorAll<HTMLElement>("[data-rides-edge]")],
    };
    box.setAttribute("data-dragging", "true");
  };

  /** Put the edge here, and remember how fast it got there. */
  const moveTo = (box: HTMLElement, left: number, at: number, floor = 0, ceiling = Infinity) => {
    const state = drag.current;
    if (!state) return;
    const next = Math.min(Math.max(Math.max(0, floor), left), Math.min(state.width, ceiling));
    const dt = at - state.lastT;
    if (dt > 0) state.vx = (next - state.left) / dt;
    state.left = next;
    state.lastT = at;
    for (const rider of state.riders) rider.style.setProperty("--hi-panel-left", `${next}px`);
    // Past the board's own edge, the panel is uncovering the board rather than
    // covering it. Once a gesture: pulled back in again, it stays revealed until the
    // release says where it lands.
    if (pushes(state.from) && next > state.startLeft) box.setAttribute("data-revealing", "");
  };

  /**
   * Let go, and land on a stop.
   *
   * **React is told immediately, not when the animation is over.** The view plane's
   * inset is keyed on the committed stop, so every millisecond between the panel
   * setting off and `onStop` being called is a millisecond the board sits at its old
   * width while the panel slides over it — and it showed: measured from the room on
   * a trackpad, the panel reached its stop at 399ms and the board did not begin
   * giving up its width until 2282ms.
   *
   * **And told synchronously, so the gesture's own values go in the same breath.**
   * The settle used to be written inline as a target in px and cleared on a
   * quarter-second timer, on the reasoning that the panel could not tell the inline
   * target from the stop's. It could: the transition starts at the next style pass,
   * a frame or more after the write, so the timer fired before it had finished and
   * clearing the target started a fresh quarter-second from wherever it had got to.
   * The panel's tail trailed the board's, and the seam opened. With the stop committed
   * inside `flushSync`, `data-stop` and the removal of every value the gesture wrote
   * land in one style change, so the panel's `left` and the board's one step of
   * `right` are timed from the same frame.
   *
   * The timer that remains is a guard and nothing else: a gesture gripped mid-settle
   * would start from the stop's resting position, not from where the box is drawn.
   */
  const land = (box: HTMLElement, next: Stop) => {
    const state = drag.current;
    drag.current = null;
    box.removeAttribute("data-dragging");
    if (!state) return;

    if (next !== state.from) {
      const { onStop: go } = live.current;
      flushSync(() => go(next));
      if (settling.current) clearTimeout(settling.current);
      settling.current = setTimeout(() => {
        settling.current = null;
      }, PANEL_MS);
    }
    // Where the gesture put the edge gives way to where the stop puts it — or, for a
    // hand that changed its mind, back to where it began — animated by the transition
    // the gesture switched off. A board that was being revealed is told the same stop.
    for (const rider of state.riders) rider.style.removeProperty("--hi-panel-left");
    box.removeAttribute("data-revealing");
  };

  const onPointerDown = (event: React.PointerEvent<HTMLSpanElement>) => {
    // Before any guard: a press on the strip is never the start of a selection,
    // including one that lands mid-settle and grips nothing. Focus does not move
    // either, which costs nothing — the panel goes `inert` at the room and takes
    // the caret out with it.
    event.preventDefault();
    const box = root.current;
    if (!box || !event.isPrimary || settling.current || drag.current) return;
    const strip = event.currentTarget;
    const rect = strip.getBoundingClientRect();
    // The press has to land in the strip's own twenty points. It always does —
    // the strip is that wide — but reading it from the element rather than from
    // the window keeps the rule true if the strip is ever inset.
    if (event.clientX < rect.left - EDGE_PX || event.clientX > rect.right + EDGE_PX) return;

    grip(box, "pointer", event.timeStamp, event.clientX);
    // Capture, so the drag survives the finger leaving the strip — which it does
    // immediately, since the whole gesture is moving away from it.
    strip.setPointerCapture(event.pointerId);
  };

  const onPointerMove = (event: React.PointerEvent<HTMLSpanElement>) => {
    const box = root.current;
    const state = drag.current;
    if (!box || !state || state.kind !== "pointer") return;
    // Pixel for pixel: the edge goes where it was, plus however far the hand has
    // moved since it took hold.
    state.travelled = Math.max(state.travelled, Math.abs(event.clientX - state.startX));
    moveTo(box, state.startLeft + (event.clientX - state.startX), event.timeStamp);
  };

  const onPointerUp = (event: React.PointerEvent<HTMLSpanElement>) => {
    const box = root.current;
    const state = drag.current;
    if (!box || !state || state.kind !== "pointer") return;

    const { shape: s } = live.current;
    const from = state.from;
    const left = state.startLeft + (event.clientX - state.startX);
    // A press that went nowhere is the mouse's entrance: this strip is the only
    // thing standing where a button used to be, so it answers a click with a step
    // toward the room — and from the room, where there is nothing to step back to,
    // with the step that brings the panel in.
    const back = retreat(s, from);
    land(
      box,
      state.travelled < CLICK_PX
        ? back === from
          ? advance(s, from)
          : back
        : settle(left, state.vx, from, s, state.width, state.panelW),
    );
  };

  // The trackpad's half of the axis, on the root rather than on the strip: the
  // whole point is that there is nothing to aim at.
  //
  // Non-passive, because taking the roll means taking it away from the browser —
  // `html, body { overscroll-behavior: none }` already stops a sideways overscroll
  // from becoming a back-navigation, and `preventDefault` keeps the rest of the run
  // from being handed anywhere else either.
  useEffect(() => {
    const box = root.current;
    if (!box) return;

    /** Sideways travel banked before the run takes hold of the edge. */
    let slop = 0;
    /** The run has done all it can and the frames still arriving are momentum:
     * swallowed, so the tail cannot grip the edge a second time and walk on to the
     * stop after the one that was asked for. */
    let spent = false;
    let resting: ReturnType<typeof setTimeout> | null = null;

    const rest = () => {
      resting = null;
      slop = 0;
      spent = false;
    };

    /** The frames stopped, so the hand is off. */
    const stopped = () => {
      const state = drag.current;
      rest();
      if (!state || state.kind !== "wheel") return;
      land(box, settle(state.left, state.vx, state.from, live.current.shape, state.width, state.panelW));
    };

    const onWheel = (event: WheelEvent) => {
      // A pinch arrives as a wheel with `ctrlKey`, a settle already owns the axis,
      // and a hand on the strip is a hand this one must not fight.
      if (event.ctrlKey) return;
      if (drag.current && drag.current.kind !== "wheel") return;

      const x = rolled(event.deltaX, event.deltaMode);
      if (!sideways(x, rolled(event.deltaY, event.deltaMode))) {
        // Reading down a conversation is not a half-finished swipe: it ends the run
        // rather than leaving its drift banked against the next one. A run that has
        // already taken hold keeps its grip and is left to the silence to end.
        if (!drag.current && !spent) {
          if (resting) clearTimeout(resting);
          rest();
        }
        return;
      }
      // A spent run is still a run: keep taking its frames away from the browser and
      // keep waiting for them to stop, but do nothing with them. Checked before the
      // settle guard below, because the settle a spent run just started is its own.
      if (spent) {
        event.preventDefault();
        if (resting) clearTimeout(resting);
        resting = setTimeout(rest, REST_MS);
        return;
      }
      if (settling.current) return;
      // Asked once, at the start: mid-run the pointer has not moved, and a gesture
      // that has taken hold is not up for reassignment.
      if (!drag.current && scrollsAcross(event, box)) return;

      event.preventDefault();
      if (resting) clearTimeout(resting);
      resting = setTimeout(stopped, REST_MS);

      if (!drag.current) {
        slop += x;
        if (Math.abs(slop) < GRAB_PX) return;
        grip(box, "wheel", event.timeStamp);
        slop = 0;
      }
      const state = drag.current;
      if (!state) return;
      // Scrolling right is fingers moving left, which is the direction the same
      // hand would drag the edge — so the edge moves against the roll.
      const [floor, ceiling] = reach(state.from, live.current.shape, state.width, state.panelW);
      moveTo(box, state.left - x, event.timeStamp, floor, ceiling);

      // **The edge has run out of reach, so this swipe is already decided** — land it
      // now instead of sitting on it until the momentum dies. Waiting for silence is
      // the right way to end a run that stopped somewhere in between, and the wrong
      // way to end one that has arrived: the panel is visibly at the stop while the
      // tail burns off, and until the tail stops the board has not been told to give
      // up its width. Measured before this: the edge was there at 399ms and the stop
      // was committed at 2282ms.
      const s = live.current.shape;
      const forward = advance(s, state.from);
      const back = retreat(s, state.from);
      const arrived =
        state.left <= floor && forward !== state.from
          ? forward
          : state.left >= ceiling && back !== state.from
            ? back
            : null;
      if (arrived) {
        spent = true;
        land(box, arrived);
      }
    };

    box.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      box.removeEventListener("wheel", onWheel);
      if (resting) clearTimeout(resting);
    };
  }, [root]);

  return (
    <span
      className="hi-panel-edge"
      data-rides-edge=""
      aria-hidden="true"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
    />
  );
}

/**
 * Whether something under the pointer scrolls sideways, in which case the roll is
 * its own and never reaches the axis.
 *
 * Deliberately not "and has room left to scroll": a strip that reaches its end and
 * then hands the next flick to the whole panel is a worse surprise than one that
 * simply stops, and it is the surprise `overscroll-behavior: contain` is already
 * written into those scrollers to prevent.
 */
function scrollsAcross(event: WheelEvent, root: HTMLElement): boolean {
  for (const node of event.composedPath()) {
    if (node === root) return false;
    if (!(node instanceof HTMLElement)) continue;
    if (node.scrollWidth <= node.clientWidth) continue;
    const across = getComputedStyle(node).overflowX;
    if (across === "auto" || across === "scroll") return true;
  }
  return false;
}

/** The panel's own measure at the middle stop, in px. Read off the root so the
 * stylesheet stays the one place a width is decided; falls back to the window
 * when it cannot be read. */
function measure(box: HTMLElement, width: number): number {
  const declared = getComputedStyle(box).getPropertyValue("--hi-panel-width").trim();
  const px = Number.parseFloat(declared);
  return Number.isFinite(px) && px > 0 ? Math.min(px, width) : width;
}
