import type { WireHistoryEntry } from "../channels/out/view";

/**
 * The trail — where the screen can go back to.
 *
 * **One list, and the server holds it.** The agent's shows and the person's own moves
 * are appended to the same history, so this window has nothing of its own to merge in:
 * a bookmark tapped on the phone is a card on the desktop, because it is a card on the
 * server. The row is that list, read head-first.
 *
 * It used to be two — the server's record of shows, and this window's private visits —
 * because appending a visit would have told the desktop the agent had shown something.
 * An entry carrying the hand that moved the screen answers that without a second list.
 */

/**
 * One card per destination, **newest first**.
 *
 * Newest first because the row overflows within an afternoon and a strip only ever
 * scrolls from its start: oldest-first put the live view — the one entry certain to be
 * wanted — reliably off the right-hand edge.
 *
 * The server already holds one entry per destination, so this is a reversal and not a
 * merge. It stays a function of its own because the ordering is the row's rule rather
 * than the list's: the history is stored oldest-first, and appending is the only thing
 * that ever happens to it.
 */
export function trailOf(history: WireHistoryEntry[]): WireHistoryEntry[] {
  return [...history].reverse();
}

/**
 * The trail as the open tab draws it: in the order it had when the tab opened.
 *
 * The server orders the row by when each place was last in front of them, so tapping a
 * card makes it the newest — and at the stops where the tab stays up beside the room,
 * that card would jump to the head under the finger that tapped it, and every card
 * before it would slide one along. So while the tab is open the cards it is already
 * showing keep their places; a card that was not there when it opened (the agent showed
 * something new) goes in at the head, and one trimmed off the end goes away. The next
 * time the tab opens, it reads the server's order again.
 *
 * `order` is the refs as last drawn, `null` on the tab's first draw.
 */
export function held(
  order: readonly string[] | null,
  trail: readonly WireHistoryEntry[],
): WireHistoryEntry[] {
  if (order === null) return [...trail];
  const drawn = new Set(order);
  const byKey = new Map(trail.map((entry) => [entry.view_ref, entry]));
  const arrived = trail.filter((entry) => !drawn.has(entry.view_ref));
  const kept = order.flatMap((key) => {
    const entry = byKey.get(key);
    return entry ? [entry] : [];
  });
  return [...arrived, ...kept];
}
