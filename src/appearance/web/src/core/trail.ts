import type { WireHistoryEntry } from "../channels/out/view";

/**
 * The trail — where this window can go back to.
 *
 * Two lists reach it and they are different kinds of fact. The server's history is the
 * record of **shows**: what the agent put up, oldest first, shared by every window and
 * persisted across restarts. The visits are this window's own **opens**: a bookmark
 * tapped here, which is this window's to remember and dies with it — the *move* is
 * reported to the agent as it happens, but the record of shows is not a place to write
 * it. Both are the same thing to a person looking at the row — a place they have been —
 * so the row is the two of them merged, and the design's rule for what counts as one
 * place applies across both: the ref when there is one, the module when there isn't.
 */

/** The same destination identity the server dedupes history by and the cursor is keyed
 * on: two shows of `factory/tasks` are one place, because both re-resolve to the same
 * recompiled board; two different inline views are two artifacts and both stay. */
export function destinationOf(entry: { view_ref?: string; module_url: string }): string {
  return entry.view_ref ?? entry.module_url;
}

/**
 * One card per destination, **newest first**.
 *
 * Newest first because the row overflows within an afternoon and a strip only ever
 * scrolls from its start: oldest-first put the live view — the one entry certain to be
 * wanted — reliably off the right-hand edge.
 *
 * Where both lists know a destination, the later fact wins the timestamp and the
 * earlier one still supplies anything it was alone in carrying: a visit knows a view's
 * ref and label but not the picture the server took of it.
 */
export function trailOf(
  history: WireHistoryEntry[],
  visits: WireHistoryEntry[],
): WireHistoryEntry[] {
  const byDestination = new Map<string, WireHistoryEntry>();
  for (const entry of [...history, ...visits]) {
    const key = destinationOf(entry);
    const seen = byDestination.get(key);
    if (seen && Date.parse(seen.at) > Date.parse(entry.at)) continue;
    byDestination.set(key, { ...seen, ...entry, shot_url: entry.shot_url ?? seen?.shot_url });
  }
  return [...byDestination.values()].sort((a, b) => Date.parse(b.at) - Date.parse(a.at));
}
