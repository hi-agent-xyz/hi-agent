import { useEffect, useRef } from "react";
import {
  advance,
  leftOf,
  measureOf,
  retreat,
  rolled,
  settle,
  sideways,
  swiped,
  type Stop,
} from "../lib/panel";
import type { Shape } from "../lib/shape";

/**
 * Every way a hand moves the panel: **one strip on its edge, and two fingers
 * anywhere at all.**
 *
 * ## The strip
 *
 * It stands on **the panel's own left edge, wherever it is.**
 *
 * The panel lives off-screen to the right and comes in at one of three stops
 * (`lib/panel.ts`, `docs/arch/stage.md` § *The panel, and the axis it runs on*),
 * and this strip stands on the edge that every stop is a position for. At `room`
 * that edge is the window's right-hand side, so the strip is the way in; at
 * `panel` it is the seam between the view and the panel; at `full` it is the
 * window's left-hand side, so the strip is the way back. **One handle, and it is
 * always the place the two halves of the screen meet.**
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
 * **One variable moves everything, and it is the panel's left edge.** Every stop
 * is a position for that edge — `room` is the window's width, `full` is zero,
 * `panel` is the width less the panel's measure — so sliding in, sliding out,
 * widening and narrowing are one motion with different endpoints instead of four
 * animations to keep in step. Release picks whichever stop the edge is nearest,
 * or the next one along if the finger was still travelling.
 *
 * **The measure is not that variable, and it moves twice a gesture at most.** The
 * box behind the edge is laid out at one of two widths (`measureOf`) and the drag
 * never touches it, so pulling the panel in *reveals* the conversation instead of
 * re-wrapping every line of it sixty times a second. The new measure is written at
 * the moment of release, together with the position it is settling to, so the
 * content re-lays once, while the box is already on its way.
 *
 * **The page is moved by a variable, not by React state.** A state update per
 * `pointermove` would re-render the shell — the conversation, the tabs,
 * everything — sixty times a second to move one box, so the drag writes
 * `--hi-panel-left` straight onto the root element and the stylesheet turns it
 * into geometry for the panel *and* for this strip, which rides the same edge.
 * React learns the stop when the gesture is over, which is when there is finally
 * something to tell it.
 *
 * **The content does not reflow under the finger.** The view plane's inset is
 * keyed on the committed stop alone, so during a drag the panel slides *over* the
 * view and the view re-lays once, on release. A compiled board re-flowing sixty
 * times a second is the one cost of pushing that would have made pushing not
 * worth it.
 *
 * **It is a strip and not a listener on the panel**, because on iOS the browser
 * decides who owns a touch at `touchstart` and never revisits it. A pan that
 * begins over the scrollback belongs to the scroller by then, so asking for it
 * afterwards gets nothing. A strip declares `touch-action: none` up front over the
 * ~20px where the gesture can begin — the same width the system's own edge
 * gesture claims.
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
 * **It is a step, not a drag, and the asymmetry is honest.** A finger on the edge is
 * holding the panel, so the panel follows it. Two fingers on a trackpad are holding
 * nothing: a `wheel` stream has no end event, it keeps arriving as momentum after
 * the hand has lifted, and the browser may report it in lines or pages rather than
 * pixels. Direction and amount are what can be read off that honestly, and those are
 * one step on an axis (`lib/panel.ts` § *The trackpad's arithmetic*).
 *
 * **A scroller under the pointer keeps its own sideways gesture.** Any board with a
 * wide table in it scrolls that way; if anything in the path can scroll across, the
 * roll is theirs and this never sees it. It is not even conditional on their having
 * room left to scroll — a strip that has reached its end and hands the whole panel to
 * the next flick is a worse surprise than one that simply stops. The views tab was the
 * other example and is no longer one: its rows wrap now, so a sideways roll over them
 * reaches the axis.
 *
 * **One swipe is one stop**, and the rest of the run — every frame of momentum after
 * the step is taken — is swallowed rather than acted on, so a hard flick out of the
 * room lands beside the view rather than covering it. That is the same rule the
 * flick already follows.
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

/** How long a `wheel` stream has to stop for before the run counts as over. There
 * is no end event to wait for: momentum keeps delivering frames after the hand has
 * lifted, and the only thing that marks the end of a swipe is the frames stopping.
 * Long enough to sit through the ragged tail of a slow one, short enough that two
 * deliberate swipes in a row are two. */
const SWIPE_REST_MS = 140;

interface PanelGestureProps {
  /** Which arrangement this is; it decides which stops exist at all. */
  shape: Shape;
  /** The stop the panel is resting at. */
  stop: Stop;
  /** Where to go. Called once, when the gesture is over. */
  onStop: (next: Stop) => void;
  /** The face's root box. It carries the axis — `--hi-panel-left` and
   * `--hi-panel-measure` — so the panel and this strip read one edge from one
   * place rather than each being told where it is. */
  root: React.RefObject<HTMLElement | null>;
}

export function PanelGesture({ shape, stop, onStop, root }: PanelGestureProps) {
  // Read at gesture time rather than closed over, so a re-render between the
  // press and the release cannot leave the drag committing a stale stop.
  const live = useRef({ shape, stop, onStop });
  live.current = { shape, stop, onStop };
  const settling = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (settling.current) clearTimeout(settling.current);
    },
    [],
  );

  /** The drag in flight. Null between gestures. */
  const drag = useRef<{
    startX: number;
    /** Where the panel's left edge was when the finger landed. */
    startLeft: number;
    /** The last sample, for the release velocity — one segment rather than the
     * whole gesture, because a slow drag that ends in a flick is a flick. */
    lastX: number;
    lastT: number;
    vx: number;
    width: number;
    panelW: number;
    travelled: number;
  } | null>(null);

  const onPointerDown = (event: React.PointerEvent<HTMLSpanElement>) => {
    const box = root.current;
    if (!box || !event.isPrimary || settling.current) return;
    const strip = event.currentTarget;
    const rect = strip.getBoundingClientRect();
    // The press has to land in the strip's own twenty points. It always does —
    // the strip is that wide — but reading it from the element rather than from
    // the window keeps the rule true if the strip is ever inset.
    if (event.clientX < rect.left - EDGE_PX || event.clientX > rect.right + EDGE_PX) return;

    const width = window.innerWidth;
    drag.current = {
      startX: event.clientX,
      startLeft: leftOf(live.current.stop, width, measure(box, width)),
      lastX: event.clientX,
      lastT: event.timeStamp,
      vx: 0,
      width,
      panelW: measure(box, width),
      travelled: 0,
    };
    // Capture, so the drag survives the finger leaving the strip — which it does
    // immediately, since the whole gesture is moving away from it.
    strip.setPointerCapture(event.pointerId);
    box.setAttribute("data-dragging", "true");
  };

  const onPointerMove = (event: React.PointerEvent<HTMLSpanElement>) => {
    const box = root.current;
    const state = drag.current;
    if (!box || !state) return;
    const left = Math.min(Math.max(0, state.startLeft + (event.clientX - state.startX)), state.width);
    const dt = event.timeStamp - state.lastT;
    if (dt > 0) state.vx = (event.clientX - state.lastX) / dt;
    state.travelled = Math.max(state.travelled, Math.abs(event.clientX - state.startX));
    state.lastX = event.clientX;
    state.lastT = event.timeStamp;
    box.style.setProperty("--hi-panel-left", `${left}px`);
  };

  const onPointerUp = (event: React.PointerEvent<HTMLSpanElement>) => {
    const box = root.current;
    const state = drag.current;
    drag.current = null;
    if (!box || !state) return;
    box.removeAttribute("data-dragging");

    const { shape: s, stop: from, onStop: go } = live.current;
    // A press that went nowhere is the mouse's entrance: this strip is the only
    // thing standing where a button used to be, so it answers a click with a step
    // toward the room — and from the room, where there is nothing to step back to,
    // with the step that brings the panel in.
    const back = retreat(s, from);
    const next =
      state.travelled < CLICK_PX
        ? back === from
          ? advance(s, from)
          : back
        : settle(
            Math.min(Math.max(0, state.startLeft + (event.clientX - state.startX)), state.width),
            state.vx,
            from,
            s,
            state.width,
            state.panelW,
          );

    if (next === from) {
      // Changed their mind, or it was a tap on a strip with nowhere to go. Back
      // to the resting position, animated by the transition the drag switched off.
      box.style.removeProperty("--hi-panel-left");
      return;
    }

    // Finish the throw on the variables the panel is already being drawn by, and
    // tell React when it has arrived. Handing straight to `onStop` would swap the
    // resting geometry underneath a box still sitting at its dragged offset.
    //
    // The measure lands here rather than with the stop, so the one reflow this
    // gesture costs happens as the box starts moving to where it is going, not a
    // quarter-second later when it is already sitting there.
    box.style.setProperty("--hi-panel-left", `${leftOf(next, state.width, state.panelW)}px`);
    box.style.setProperty(
      "--hi-panel-measure",
      `${measureOf(next, s, state.width, state.panelW)}px`,
    );
    settling.current = setTimeout(() => {
      settling.current = null;
      // Cleared once the stop has landed, so the next gesture starts from the
      // stylesheet's own resting position rather than from a stale pixel value.
      box.style.removeProperty("--hi-panel-left");
      box.style.removeProperty("--hi-panel-measure");
      go(next);
    }, PANEL_MS);
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

    /** Sideways travel so far in this run, in px. */
    let travelled = 0;
    /** Whether this run has already moved a stop. One swipe is one step; the
     * momentum that follows the hand lifting is swallowed, not acted on. */
    let spent = false;
    let resting: ReturnType<typeof setTimeout> | null = null;

    const rest = () => {
      resting = null;
      travelled = 0;
      spent = false;
    };

    const onWheel = (event: WheelEvent) => {
      // A pinch arrives as a wheel with `ctrlKey`, and a drag or a settle already
      // owns the axis. None of those are this.
      if (event.ctrlKey || drag.current || settling.current) return;

      const x = rolled(event.deltaX, event.deltaMode);
      if (!sideways(x, rolled(event.deltaY, event.deltaMode))) {
        // Reading down a conversation is not a half-finished swipe: it ends the run
        // rather than leaving its drift banked against the next one.
        if (resting) clearTimeout(resting);
        rest();
        return;
      }
      if (scrollsAcross(event, box)) return;

      event.preventDefault();
      if (resting) clearTimeout(resting);
      resting = setTimeout(rest, SWIPE_REST_MS);
      if (spent) return;

      travelled += x;
      const { shape: s, stop: from, onStop: go } = live.current;
      const next = swiped(travelled, from, s);
      if (next === from) return;
      spent = true;
      go(next);
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
