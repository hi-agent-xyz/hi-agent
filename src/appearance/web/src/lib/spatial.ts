import { onHostKey } from "./keyboard";
import { declaredTv } from "./shape";

// Moving the focus with a D-pad.
//
// On every other host the focus is a fallback — the pointer says what is being
// addressed, and `:focus-visible` is a courtesy to the rare keyboard user. On a
// television the focus *is* the cursor, and the only instrument for moving it is
// four arrows. The browser does not do this: `Tab` walks the document in source
// order, which is a list, and a face laid out in a room is not a list. Pressing
// right on a remote has to reach the thing that is to the right.
//
// **The plane rule already settles whose arrows these are**, so this file inherits
// it instead of restating it. Registration goes through `onHostKey`, which only
// runs for keys that started in the cover plane or the room (`lib/keyboard.ts`),
// so an agent view that binds the arrows to page itself keeps them the moment its
// own content holds the focus. `preventDefault` on a move claims the key, and the
// same rule then stops it reaching the view underneath.
//
// The way back out of a view is therefore *not* a key: the arrows belong to the
// view while it holds focus, and taking one back would break the rule this file
// depends on. It is the remote's Back button, which never reaches the page at all
// and arrives from the shell instead — see `lib/tvBack.ts`.

/** A rectangle, as much of one as choosing a direction needs. */
export interface Box {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export type Direction = "up" | "down" | "left" | "right";

const KEYS: Record<string, Direction> = {
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
};

/**
 * Which candidate is the one in that direction, or null if there is nothing
 * there.
 *
 * Two distances decide it, and they are deliberately not weighted equally. The
 * **primary** distance is the gap along the direction travelled; the **cross**
 * distance is how far the candidate sits off to the side of it. Cross distance
 * counts double, because a person pressing down means "the next thing down this
 * column", and treating a near-but-sideways control as closer than a slightly
 * further one directly below is the thing that makes D-pad navigation feel like
 * it is guessing.
 *
 * Overlapping on the cross axis scores zero for it — a wide control directly
 * below a narrow one is directly below it, whatever their centres say.
 *
 * Pure, and exported, so the rule can be tested at a table of rectangles rather
 * than through a browser.
 */
export function nearest(from: Box, candidates: Box[], dir: Direction): number | null {
  const vertical = dir === "up" || dir === "down";
  let best: number | null = null;
  let bestScore = Infinity;

  candidates.forEach((box, index) => {
    // Beyond, on the axis travelled. Compared leading-edge to leading-edge so a
    // tall neighbour that starts above and extends past does not count as below.
    const primary = vertical
      ? dir === "down"
        ? box.top - from.bottom
        : from.top - box.bottom
      : dir === "right"
        ? box.left - from.right
        : from.left - box.right;
    if (primary < 0) return;

    const cross = vertical
      ? gap(from.left, from.right, box.left, box.right)
      : gap(from.top, from.bottom, box.top, box.bottom);

    const score = primary + cross * 2;
    if (score < bestScore) {
      bestScore = score;
      best = index;
    }
  });

  return best;
}

/** How far two spans are apart, or 0 where they overlap. */
function gap(aStart: number, aEnd: number, bStart: number, bEnd: number): number {
  if (bEnd < aStart) return aStart - bEnd;
  if (bStart > aEnd) return bStart - aEnd;
  return 0;
}

const FOCUSABLE = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled])",
  "textarea:not([disabled])",
  "select:not([disabled])",
  "[contenteditable]",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

/** Everything on screen the focus is allowed to land on. */
function candidates(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => {
    if (el.hasAttribute("disabled") || el.getAttribute("aria-hidden") === "true") return false;
    // A surface put away is `visibility: hidden` rather than unmounted, so it is
    // still in the document and still matches the selector. Measuring is what
    // tells the two apart: a hidden box has no size.
    const rect = el.getBoundingClientRect();
    if (rect.width === 0 && rect.height === 0) return false;
    return el.closest('[aria-hidden="true"]') === null;
  });
}

/** Whether a key aimed at this element is text, not navigation. */
function isTyping(el: Element | null): boolean {
  if (!el) return false;
  const name = el.tagName;
  return name === "INPUT" || name === "TEXTAREA" || (el as HTMLElement).isContentEditable;
}

/**
 * Arm the arrows. No-op unless the host declared itself a television, so every
 * other client keeps the browser's own focus behaviour untouched.
 */
export function installSpatialNav(): void {
  if (!declaredTv()) return;

  onHostKey((event) => {
    const dir = KEYS[event.key];
    if (!dir || event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey) return;

    // Inside a text field the arrows move the caret, and on a television they are
    // in any case the on-screen keyboard's rather than the page's — the IME is
    // full screen and the page is not receiving keys at all while it is up.
    const active = document.activeElement as HTMLElement | null;
    if (isTyping(active)) return;

    const boxes = candidates();
    const here = active && boxes.includes(active)
      ? active.getBoundingClientRect()
      : // Nothing focused yet: enter from the edge the person is pressing away
        // from, so the first press lands on the nearest control rather than on
        // whatever happens to be first in the document.
        edge(dir);

    const rects = boxes.map((el) => el.getBoundingClientRect());
    const pick = nearest(here, rects, dir);
    if (pick === null) return;
    const target = boxes[pick];
    if (!target) return;

    target.focus();
    event.preventDefault();
  });
}

/** A zero-height line along the edge the focus is arriving from. */
function edge(dir: Direction): Box {
  const w = window.innerWidth;
  const h = window.innerHeight;
  switch (dir) {
    case "down":
      return { left: 0, right: w, top: -1, bottom: -1 };
    case "up":
      return { left: 0, right: w, top: h + 1, bottom: h + 1 };
    case "right":
      return { left: -1, right: -1, top: 0, bottom: h };
    case "left":
      return { left: w + 1, right: w + 1, top: 0, bottom: h };
  }
}

/**
 * Take the focus back out of the agent's plane and put it on host chrome.
 *
 * The one move the arrows cannot make, because while a view holds the focus the
 * arrows are the view's. Called from the shell's Back button.
 */
export function leaveViewPlane(): boolean {
  const active = document.activeElement as HTMLElement | null;
  if (!active?.closest?.(".hi-plane--view")) return false;

  const [firstCover] = candidates().filter((el) => el.closest(".hi-plane--cover"));
  if (!firstCover) {
    active.blur();
    return true;
  }
  firstCover.focus();
  return true;
}

/** Whether the focus is currently inside a view. */
export function focusInViewPlane(): boolean {
  const active = document.activeElement as Element | null;
  return active?.closest?.(".hi-plane--view") != null;
}
