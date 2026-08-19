import { url } from "../../lib/base";
// Client for the outbound view channel — the conversation's retained appearance state.
//
// GET /api/out/view serves the conversation's whole appearance (active views in
// z-order, plus a version) and long-polls on `?since=<version>`: the first
// request returns the current state immediately (even when empty), each
// following one is held until the state changes. Refresh, a second device, or
// a server restart all converge on the same screen — the server retains and
// persists the state; the client just mirrors the latest snapshot.

/** What a view declared about itself, carried verbatim from the Rust
 * `ViewTraits`. Views are full-bleed and one at a time, so there is no placement
 * to declare — this is all that's left. Field names match the wire. */
export interface ViewTraits {
  /** This view renders the live words itself; the host's caption pills stand
   * down. Absent reads as `false` — the safe default. */
  owns_conversation?: boolean;
}

/** One active layer in the conversation's appearance, in z-order (first = bottom).
 *
 * At most two arrive: the agent's content view, and the host's condition layer
 * over it (a vendor outage). The server holds them in fixed slots, so this list
 * can no longer grow into the stacks it used to — see the Rust `Appearance`. */
export interface WireView {
  id: string;
  /** URL of the compiled ESM module to import and mount under `id`. */
  module_url: string;
  /** What the view declared; absent = host-owned captions. */
  traits?: ViewTraits;
  /** Which server slot this layer came out of. A window showing a past view puts it in
   * place of `content` and keeps `condition` over it — an outage still covers whatever
   * the person went back to. Absent on a server older than the field. */
  slot?: "content" | "condition";
}

/** One past raise, as the server recorded it.
 *
 * `view_ref` decides what re-opening means. A named view is re-resolved from its
 * current source, so `factory/tasks` comes back as today's board — reopen it through
 * `openView(ref)`. An inline view has no ref and can only come back as the artifact it
 * compiled to, so mount its `module_url` directly. */
export interface WireHistoryEntry {
  id: string;
  module_url: string;
  view_ref?: string;
  /** Derived server-side from the ref's last segment, or the id for an inline view. */
  label: string;
  /** ISO timestamp of the raise. */
  at: string;
  /** A picture of this entry, served from `/views/_shots/`. For a named view it is a
   * picture of the surface as it currently stands — the card leads to today's board, so
   * the tile is of today's board — and for an inline view it is the artifact's own,
   * captured the moment it went up. Absent while the capture is still running, and for
   * good on a view that did not render cleanly; the tile falls back to its mark either
   * way. */
  shot_url?: string;
}

/** The conversation's full appearance state — one GET /api/out/view response. */
export interface ViewState {
  version: number;
  views: WireView[];
  /** The recent raises, oldest first; the last is what is on the stage now. Absent on
   * a server older than the history — read as empty, which reads as "nowhere to go
   * back to", which is the truthful answer from a server that isn't recording. */
  history?: WireHistoryEntry[];
}

/** One view that exists on disk and can be opened by name — `GET /api/views`. */
export interface ListedView {
  view_ref: string;
  label: string;
  /** One of the surfaces we ship, under `factory/`. These are the standing floor of
   * the bookmarks row: always there, and not removable. */
  system: boolean;
  /** The person put this one in the row. Never true for a system view, which is in
   * the row by being system. */
  bookmarked: boolean;
  /** A picture of this surface as it currently stands, or absent until one has been
   * taken. Named views only — an inline view has no ref to file a picture under. This
   * is the fresher of the two answers about a named view's tile, because the band
   * re-reads the inventory while it is open. */
  shot_url?: string;
}

/** Every named view in the views tree — the whole inventory, each entry saying
 * whether it is a system surface and whether the person kept it. The band's lower row
 * is the two of those together; the rest of the tree is the agent's working files and
 * is deliberately not a place a person is offered. */
export async function listViews(): Promise<ListedView[]> {
  const res = await fetch(url("/api/views"), {
    method: "GET",
    headers: { Accept: "application/json" },
    cache: "no-store",
  });
  if (!res.ok) throw new Error(`/api/views failed: ${res.status} ${res.statusText}`);
  return (await res.json()) as ListedView[];
}

/** What one window needs to mount a named view locally. */
export interface OpenedView {
  id: string;
  module_url: string;
  traits?: ViewTraits;
}

/** Compile a named view for this window to mount.
 *
 * Deliberately does not touch the content slot: the stage stays what the agent raised,
 * so a second device is unaffected and the agent's "as you can see here" stays true of
 * what it put up. Where this window is looking is this window's own, like scroll
 * position. */
export async function openView(viewRef: string): Promise<OpenedView> {
  const res = await fetch(url("/api/views/open"), {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-HI-Surface": "1" },
    body: JSON.stringify({ ref: viewRef }),
  });
  if (!res.ok) throw new Error(`/api/views/open failed: ${res.status} ${res.statusText}`);
  return (await res.json()) as OpenedView;
}

/** Keep a named view in the bookmarks row, or drop it out of it.
 *
 * Server-side because a bookmark has to be the same on the desktop and the phone —
 * unlike the cursor, which is each window's own. Only a named, non-system view can be
 * bookmarked: an inline view is only ever the disposable artifact it compiled to, and
 * a system view is in the row already. */
export async function setBookmark(viewRef: string, on: boolean): Promise<void> {
  const res = await fetch(url("/api/views/bookmarks"), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ ref: viewRef, on }),
  });
  if (!res.ok) throw new Error(`/api/views/bookmarks failed: ${res.status} ${res.statusText}`);
}

/** Where this window just went — the inbound half of the view channel.
 *
 * **This is not the cursor, and reporting it is not reporting the cursor.** Which entry
 * a window is parked on stays the window's own and never leaves it: nothing here moves
 * the stage, bumps the appearance version or reaches a second device. What goes is the
 * *move*, as something the agent perceives, because the next thing the person says is
 * usually about what they are looking at — an agent that thinks its own last raise is in
 * front of them answers confidently about the wrong board.
 *
 * Fire-and-forget, like the stage lane: a dropped report costs the next turn one line of
 * context, and nothing else. It does not drive a turn either, so walking the band never
 * makes the agent pipe up.
 */
export function reportWentTo(dest: {
  /** The durable ref, for a named view. */
  viewRef?: string;
  /** The compiled module, for an inline view that has no ref — the same fallback the
   * history dedupes destinations by. */
  moduleUrl?: string;
  /** What the view was raised under, to name an inline destination in a prompt. */
  id?: string;
  /** They are back on what the agent has up. Clears the fact rather than recording one. */
  live?: boolean;
}): void {
  void fetch(url("/api/in/view"), {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-HI-Surface": "1" },
    body: JSON.stringify({
      ref: dest.viewRef,
      module: dest.moduleUrl,
      id: dest.id,
      live: dest.live ?? false,
    }),
  }).catch(() => {
    /* the next turn goes without the line; nothing else depends on it */
  });
}

export interface SubscribeViewOpts {
  signal: AbortSignal;
}

export async function* subscribeViewState(
  opts: SubscribeViewOpts,
): AsyncGenerator<ViewState, void, void> {
  let since: number | undefined;
  while (!opts.signal.aborted) {
    const query = since === undefined ? "" : `?since=${since}`;
    const res = await fetch(url(`/api/out/view${query}`), {
      method: "GET",
      headers: { Accept: "application/json" },
      signal: opts.signal,
      cache: "no-store",
    });
    if (!res.ok) {
      throw new Error(`/api/out/view subscribe failed: ${res.status} ${res.statusText}`);
    }
    const state = (await res.json()) as ViewState;
    if (!state || !Array.isArray(state.views)) continue;
    since = state.version;
    yield state;
  }
}

/** Clear the conversation's appearance — close all views, back to the default room.
 * The server bumps the version, so every device's long-poll converges on empty
 * (there is no optimistic local change; the next snapshot drives the UI). */
export async function clearViewState(): Promise<void> {
  // A bodyless DELETE carries no content type, which is indistinguishable
  // here from `text/plain`; the header is how it says it is not a
  // cross-site form. See the note in `in/text.ts`.
  const res = await fetch(url("/api/out/view"), {
    method: "DELETE",
    headers: { "X-HI-Surface": "1" },
  });
  if (!res.ok) {
    throw new Error(`/api/out/view clear failed: ${res.status} ${res.statusText}`);
  }
}
