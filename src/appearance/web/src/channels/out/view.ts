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
}

/** The conversation's full appearance state — one GET /api/out/view response. */
export interface ViewState {
  version: number;
  views: WireView[];
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
    const res = await fetch(`/api/out/view${query}`, {
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
  const res = await fetch("/api/out/view", {
    method: "DELETE",
    headers: { "X-HI-Surface": "1" },
  });
  if (!res.ok) {
    throw new Error(`/api/out/view clear failed: ${res.status} ${res.statusText}`);
  }
}
