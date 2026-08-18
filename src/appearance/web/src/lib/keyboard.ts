// Who the keyboard belongs to.
//
// The three planes settle who covers whom, and `pointer-events` on the cover
// plane settles who a click belongs to (`docs/arch/stage.md`). Keys had no such
// rule, and they need one of their own because a keystroke does not travel down
// the stack: it starts at whatever has focus and bubbles up through the document
// to the window, where an agent view's `window.addEventListener("keydown", …)`
// hears it whichever plane it came from.
//
// That is the failure this file exists for. A slide view binds Space and the
// arrows to page itself; the person clicks into the line to write a message; and
// every Space they type pages the deck instead of landing in the line, because
// the view called `preventDefault()` on a keystroke that was aimed at the host's
// own input. The controls are in the same position — Space on a focused button.
//
// **The keyboard belongs to the plane the focus is in**, in the same order as
// paint: the agent's plane is below the person's.
//
//   focus in `cover` — host chrome — the view never hears the key
//   focus in `view`  — the view owns it, and the host's global affordances stand
//                      down rather than firing alongside it
//   focus in neither — the room: both hear it, the host first, and a host claim
//                      (a `preventDefault` from one of its handlers) ends it
//
// Enforced in one place, at the document, because the code on the other side is
// agent-authored and cannot be asked to check whose key it is — the same reason
// `nativeFeel.ts` installs itself once at the document rather than per view.

/** The plane a keystroke started in. */
export type KeyOrigin = "cover" | "view" | "room";

type HostKeyHandler = (event: KeyboardEvent) => void;

const COVER = ".hi-plane--cover";
const VIEW = ".hi-plane--view";

const hostHandlers = new Set<HostKeyHandler>();

/**
 * Register one of host chrome's global key affordances — Escape putting the
 * conversation away, a printable key opening it. They were `window` keydown
 * listeners until the guard below existed, and the guard would now cut them off
 * before they ran: it stops a chrome-originated key at the document, one node
 * short of the window.
 *
 * Handlers run in registration order and *after* React's own delegated handlers
 * (React attaches at the root container, which is a descendant of the document),
 * so one can still read `defaultPrevented` to defer to the surface that already
 * acted. That ordering is the Escape ladder: the line clears a half-written
 * draft, and only an empty line lets Escape through to close the popover.
 */
export function onHostKey(handler: HostKeyHandler): () => void {
  hostHandlers.add(handler);
  return () => {
    hostHandlers.delete(handler);
  };
}

/**
 * Which plane the keystroke started in. Duck-typed on `closest` rather than
 * `instanceof Element` so the routing can be tested without a DOM, and so a
 * target that is the document or the window (no `closest`) reads as the room
 * instead of throwing.
 */
export function keyOrigin(target: EventTarget | null): KeyOrigin {
  const el = target as { closest?: (selector: string) => unknown } | null;
  if (typeof el?.closest !== "function") return "room";
  if (el.closest(COVER)) return "cover";
  if (el.closest(VIEW)) return "view";
  return "room";
}

/** The host's global affordances stand down while the view holds the focus. */
export function hostHearsKey(origin: KeyOrigin): boolean {
  return origin !== "view";
}

/** A view hears nothing typed into the person's chrome, and nothing the host claimed. */
export function viewHearsKey(origin: KeyOrigin, hostClaimed: boolean): boolean {
  return origin !== "cover" && !hostClaimed;
}

const KEY_EVENTS = ["keydown", "keyup", "keypress"] as const;

/**
 * Install the rule. Once, at startup, before any view module can be imported —
 * `stopImmediatePropagation` only silences listeners that have not run yet, and
 * on the document that means every later one, whoever registered it.
 */
export function installKeyPlanes(): void {
  for (const type of KEY_EVENTS) {
    document.addEventListener(type, route as EventListener);
  }
}

function route(event: KeyboardEvent): void {
  const origin = keyOrigin(event.target);

  // Only keydown carries the host's affordances. keyup and keypress are routed
  // so they can be *stopped* alongside their keydown: a view that counts a key
  // down and up must never be handed half a press it never saw the start of.
  let claimed = false;
  if (event.type === "keydown" && hostHearsKey(origin)) {
    const handledBelow = event.defaultPrevented;
    for (const handler of [...hostHandlers]) handler(event);
    claimed = !handledBelow && event.defaultPrevented;
  }

  if (!viewHearsKey(origin, claimed)) event.stopImmediatePropagation();
}
