import { useEffect, useRef } from "react";

/**
 * Swipe a pushed page back off the stack.
 *
 * On the phone the host's surfaces are pages rather than panels
 * (`docs/arch/stage.md` — *The phone stacks pages*), and a page that arrived by
 * sliding in from the right has exactly one way out that a thumb already knows:
 * drag from the left edge and let go. This is that gesture, and it is a real
 * drag — the page tracks the finger pixel for pixel, comes back if you change
 * your mind, and leaves on either distance or speed. A handler that merely
 * *detected* a swipe and then played a canned animation is the thing this is
 * not: the whole of what makes the gesture read as the page being moved is that
 * it is being moved.
 *
 * **The page is moved by a variable, not by this component's state.** A React
 * state update per `pointermove` would re-render the shell — the conversation,
 * the controls, everything — sixty times a second to move one box. So the drag
 * writes `--hi-page-x` straight onto the page element and the stylesheet turns
 * it into the transform; React learns nothing until the gesture is over, which
 * is when there is finally something to tell it.
 *
 * **It is a strip and not a listener on the page**, because on iOS the browser
 * decides who owns a touch at `touchstart` and never revisits it. A pan that
 * begins over the scrollback belongs to the scroller by then, so asking for it
 * afterwards gets nothing. The strip declares `touch-action: none` up front over
 * the ~20px where a back-swipe can begin, which is the same width the system's
 * own edge gesture claims.
 *
 * *The cost, stated: a tap inside that strip does nothing.* It lands on the
 * strip, moves nothing, and springs back — so the leftmost 20px of a page is not
 * a place a control can be reached. That is the same trade iOS makes for its own
 * edge, and the page's own content starts inboard of it.
 */

/** How long the page takes to finish leaving, or to spring back. Named here
 * because both ends need it — the stylesheet animates it and this file has to
 * know when the animation is over — and `page-transition.test.ts` fails if the
 * two ever drift apart. */
export const PAGE_MS = 260;

/** The strip a back-swipe may begin in. Matches the system edge, so a gesture
 * that starts too far inboard fails the same way here as it does anywhere else
 * on the phone rather than being a surprise. */
const EDGE_PX = 20;

/** Past this much of the page's width, letting go finishes the pop rather than
 * springing back — the halfway-ish point UIKit's own interactive pop commits at.
 * Below it, the flick test still applies. */
const COMMIT_FRACTION = 0.4;

/** A flick: fast enough rightward at release that the person clearly meant to
 * throw the page away, however little of it they had actually dragged. In CSS
 * pixels per millisecond. */
const FLICK_PX_PER_MS = 0.5;

/**
 * Whether letting go here finishes the pop.
 *
 * Distance OR speed, never both: a slow, deliberate drag most of the way across
 * means it, and so does a short hard flick. Requiring both is what makes a
 * gesture feel like it has to be performed correctly.
 */
export function popsBack(dx: number, width: number, vx: number): boolean {
  if (width <= 0) return false;
  return dx >= width * COMMIT_FRACTION || vx >= FLICK_PX_PER_MS;
}

/** Where in the page a press may start a back-swipe. */
export function inEdge(clientX: number, pageLeft: number): boolean {
  return clientX - pageLeft <= EDGE_PX;
}

export function PageEdge({ onBack }: { onBack: () => void }) {
  const strip = useRef<HTMLSpanElement | null>(null);
  // Read at gesture time rather than closed over, so a re-render between the
  // press and the release cannot leave the drag calling a stale dismissal.
  const back = useRef(onBack);
  back.current = onBack;
  const leaving = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (leaving.current) clearTimeout(leaving.current);
    },
    [],
  );

  const page = () => strip.current?.parentElement ?? null;

  /** The drag in flight. Null between gestures. */
  const drag = useRef<{
    startX: number;
    /** The last sample, for the release velocity — one segment rather than the
     * whole gesture, because a slow drag that ends in a flick is a flick. */
    lastX: number;
    lastT: number;
    vx: number;
    width: number;
  } | null>(null);

  const onPointerDown = (event: React.PointerEvent<HTMLSpanElement>) => {
    const box = page();
    if (!box || !event.isPrimary || leaving.current) return;
    const rect = box.getBoundingClientRect();
    if (!inEdge(event.clientX, rect.left)) return;
    drag.current = {
      startX: event.clientX,
      lastX: event.clientX,
      lastT: event.timeStamp,
      vx: 0,
      width: rect.width,
    };
    // Capture, so the drag survives the finger leaving the strip — which it does
    // immediately, since the whole gesture is moving away from it.
    strip.current?.setPointerCapture(event.pointerId);
    box.setAttribute("data-dragging", "true");
  };

  const onPointerMove = (event: React.PointerEvent<HTMLSpanElement>) => {
    const box = page();
    const state = drag.current;
    if (!box || !state) return;
    // Never negative: dragging left is not a gesture here, and letting the page
    // follow past its own edge would tear a gap open on the right.
    const dx = Math.min(Math.max(0, event.clientX - state.startX), state.width);
    const dt = event.timeStamp - state.lastT;
    if (dt > 0) state.vx = (event.clientX - state.lastX) / dt;
    state.lastX = event.clientX;
    state.lastT = event.timeStamp;
    box.style.setProperty("--hi-page-x", `${dx}px`);
  };

  const onPointerUp = (event: React.PointerEvent<HTMLSpanElement>) => {
    const box = page();
    const state = drag.current;
    drag.current = null;
    if (!box || !state) return;
    box.removeAttribute("data-dragging");
    const dx = Math.min(Math.max(0, event.clientX - state.startX), state.width);

    if (!popsBack(dx, state.width, state.vx)) {
      // Changed their mind, or it was a tap. Back to where it was, animated by
      // the transition the drag had switched off.
      box.style.removeProperty("--hi-page-x");
      return;
    }

    // Finish the throw here rather than handing straight to `onBack`: the two
    // surfaces this rides on leave differently — the conversation flips to
    // hidden, the views band unmounts — and only one animation should exist. So
    // the page is sent the rest of the way out on the variable it is already
    // being moved by, and the dismissal lands when it has arrived.
    box.style.setProperty("--hi-page-x", "100%");
    leaving.current = setTimeout(() => {
      leaving.current = null;
      // Cleared while the page is off-screen, so the next open starts from the
      // stylesheet's own resting transform rather than from 100% — which would
      // open it onto nothing.
      box.style.removeProperty("--hi-page-x");
      back.current();
    }, PAGE_MS);
  };

  return (
    <span
      ref={strip}
      className="hi-page-edge"
      aria-hidden="true"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
    />
  );
}
