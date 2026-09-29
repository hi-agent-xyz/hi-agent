// A press held still — the touch screen's right-click.
//
// A card's menu opens on `contextmenu`, which a mouse's right button fires everywhere.
// A finger is not so lucky: WKWebView on iOS does not fire `contextmenu` for a long
// press at all, and Android fires it only sometimes. So a touch or a pen that goes
// down and stays down for `LONG_PRESS_MS` opens the menu itself, from pointer events,
// which every engine delivers the same way.
//
// Kept apart from the component so the timing can be tested without a DOM.

/** How long a press has to be held. The platforms' own long-press is about this. */
export const LONG_PRESS_MS = 500;

/** How far a finger may drift and still be holding still. Past this it is a scroll. */
export const LONG_PRESS_SLOP = 8;

interface PointerLike {
  pointerType: string;
  clientX: number;
  clientY: number;
}

type Schedule = (run: () => void, ms: number) => unknown;
type Cancel = (handle: unknown) => void;

export class LongPress {
  private handle: unknown = null;
  private from: { x: number; y: number } | null = null;
  private fired = false;

  constructor(
    private readonly fire: (x: number, y: number) => void,
    private readonly schedule: Schedule = (run, ms) => setTimeout(run, ms),
    private readonly cancel: Cancel = (handle) => clearTimeout(handle as number),
  ) {}

  /** A pointer went down. A mouse is left to its right button. */
  down(event: PointerLike): void {
    this.stop();
    this.fired = false;
    if (event.pointerType === "mouse") return;
    const at = { x: event.clientX, y: event.clientY };
    this.from = at;
    this.handle = this.schedule(() => {
      this.handle = null;
      this.from = null;
      this.fired = true;
      this.fire(at.x, at.y);
    }, LONG_PRESS_MS);
  }

  /** The pointer moved. Far enough, and the press was a scroll or a drag. */
  move(event: PointerLike): void {
    if (!this.from) return;
    if (Math.hypot(event.clientX - this.from.x, event.clientY - this.from.y) > LONG_PRESS_SLOP) {
      this.stop();
    }
  }

  /** The pointer came up, or the browser took it for a gesture of its own. */
  up(): void {
    this.stop();
  }

  /** Whether the click the lifted finger is about to produce belongs to a long press —
   *  and so must not also open the card. Answers once per press. */
  takeClick(): boolean {
    const fired = this.fired;
    this.fired = false;
    return fired;
  }

  private stop(): void {
    if (this.handle !== null) this.cancel(this.handle);
    this.handle = null;
    this.from = null;
  }
}

/** Where to put a menu of `width` × `height` opened at (`x`, `y`) so it stays on a
 *  `viewW` × `viewH` screen: to the right of and below the point when it fits, flipped
 *  to the other side when it does not, and never closer than `margin` to an edge. */
export function placeMenu(
  x: number,
  y: number,
  width: number,
  height: number,
  viewW: number,
  viewH: number,
  margin = 8,
): { left: number; top: number } {
  const left = x + width + margin <= viewW ? x : x - width;
  const top = y + height + margin <= viewH ? y : y - height;
  return {
    left: Math.max(margin, Math.min(left, viewW - width - margin)),
    top: Math.max(margin, Math.min(top, viewH - height - margin)),
  };
}
