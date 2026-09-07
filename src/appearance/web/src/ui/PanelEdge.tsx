import { useEffect, useRef } from "react";
import { advance, leftOf, retreat, settle, stops, type Stop } from "../lib/panel";
import type { Shape } from "../lib/shape";

/**
 * The two edges the panel moves on.
 *
 * The panel lives off-screen to the right and comes in at one of three stops
 * (`lib/panel.ts`, `docs/arch/stage.md` § *The panel, and the axis it runs on*).
 * **The right edge advances a stop and the left edge retreats one**, at every
 * stop and in every shape — which is the whole of the mental model, and the
 * reason it is one component rather than a forward gesture and a separate back
 * one.
 *
 * *This replaced `PageEdge`, which did the retreat half for the phone's two
 * pushed pages. What it worked out is kept whole: the drag is real — the box
 * tracks the finger pixel for pixel, comes back if you change your mind, and
 * leaves on distance **or** speed. A handler that merely detected a swipe and
 * then played a canned animation is the thing this is not.*
 *
 * **One variable moves everything, and it is the panel's left edge.** Every stop
 * is a position for that edge — `room` is the window's width, `full` is zero,
 * `panel` is the width less the panel's measure — so sliding in, sliding out,
 * widening and narrowing are one motion with different endpoints instead of four
 * animations to keep in step. Release picks whichever stop the edge is nearest,
 * or the next one along if the finger was still travelling.
 *
 * **The page is moved by a variable, not by React state.** A state update per
 * `pointermove` would re-render the shell — the conversation, the tabs,
 * everything — sixty times a second to move one box, so the drag writes
 * `--hi-panel-left` straight onto the panel element and the stylesheet turns it
 * into geometry. React learns the stop when the gesture is over, which is when
 * there is finally something to tell it.
 *
 * **The content does not reflow under the finger.** The view plane's inset is
 * keyed on the committed stop alone, so during a drag the panel slides *over* the
 * view and the view re-lays once, on release. A compiled board re-flowing sixty
 * times a second is the one cost of pushing that would have made pushing not
 * worth it.
 *
 * **They are strips and not listeners on the panel**, because on iOS the browser
 * decides who owns a touch at `touchstart` and never revisits it. A pan that
 * begins over the scrollback belongs to the scroller by then, so asking for it
 * afterwards gets nothing. A strip declares `touch-action: none` up front over the
 * ~20px where the gesture can begin — the same width the system's own edge
 * gesture claims.
 *
 * **A strip is mounted only when it has somewhere to go.** At the room there is
 * nothing to retreat to, so no left strip stands over the view stealing the
 * leftmost twenty points of every board; at the last stop there is nothing to
 * advance to, so no right strip stands over the panel. The cost is real only in
 * the middle: on a window at `panel`, the panel's rightmost twenty points are the
 * advance strip and a tap there does nothing.
 */

/** How long a settle takes. Named here because both ends need it — the stylesheet
 * animates it and this file has to know when it is over — and `PanelEdge.test.ts`
 * fails if the two ever drift apart. */
export const PANEL_MS = 260;

/** The strip a gesture may begin in. Matches the system edge, so a gesture that
 * starts too far inboard fails the same way here as it does anywhere else. */
const EDGE_PX = 20;

/** Under this much travel the gesture was a click, which is the mouse's way in —
 * there being no button to press (`docs/arch/stage.md` § *The ways in*). */
const CLICK_PX = 4;

interface PanelEdgeProps {
  /** Which arrangement this is; it decides which stops exist at all. */
  shape: Shape;
  /** The stop the panel is resting at. */
  stop: Stop;
  /** Where to go. Called once, when the gesture is over. */
  onStop: (next: Stop) => void;
  /** The panel's box, which is what the drag actually moves. */
  panel: React.RefObject<HTMLElement | null>;
}

export function PanelEdge({ shape, stop, onStop, panel }: PanelEdgeProps) {
  const order = stops(shape);
  const canAdvance = stop !== order[order.length - 1];
  const canRetreat = stop !== order[0];

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
    const box = panel.current;
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
    const box = panel.current;
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
    const box = panel.current;
    const state = drag.current;
    drag.current = null;
    if (!box || !state) return;
    box.removeAttribute("data-dragging");

    const { shape: s, stop: from, onStop: go } = live.current;
    // A press that went nowhere is the mouse's entrance: this strip is the only
    // thing standing where a button used to be, so it answers a click with the
    // step it is for.
    const next =
      state.travelled < CLICK_PX
        ? event.currentTarget.dataset.edge === "right"
          ? advance(s, from)
          : retreat(s, from)
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

    // Finish the throw on the variable the panel is already being moved by, and
    // tell React when it has arrived. Handing straight to `onStop` would swap the
    // resting geometry underneath a box still sitting at its dragged offset.
    box.style.setProperty("--hi-panel-left", `${leftOf(next, state.width, state.panelW)}px`);
    settling.current = setTimeout(() => {
      settling.current = null;
      // Cleared once the stop has landed, so the next gesture starts from the
      // stylesheet's own resting position rather than from a stale pixel value.
      box.style.removeProperty("--hi-panel-left");
      go(next);
    }, PANEL_MS);
  };

  const strip = (edge: "left" | "right") => (
    <span
      key={edge}
      className="hi-panel-edge"
      data-edge={edge}
      aria-hidden="true"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
    />
  );

  return (
    <>
      {canRetreat && strip("left")}
      {canAdvance && strip("right")}
    </>
  );
}

/** The panel's own measure at the middle stop, in px. Read off the element so the
 * stylesheet stays the one place a width is decided; falls back to the window
 * when the panel is currently drawn full-bleed. */
function measure(box: HTMLElement, width: number): number {
  const declared = getComputedStyle(box).getPropertyValue("--hi-panel-width").trim();
  const px = Number.parseFloat(declared);
  return Number.isFinite(px) && px > 0 ? Math.min(px, width) : width;
}
