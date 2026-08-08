// Client for the attention lane.
//
// `reportAttention` tells the backend what our window is doing (POST
// /api/in/attention). It's the first-party presence signal: strictly about *our
// own* window, never anything about other apps. Fire-and-forget; a failed beat is
// harmless and swallowed.
//
// Three states, because the backend cannot see two of them. It derives reach from
// which out-channels are open, and we drop those for `background` and `closed`
// alike — so from its side the two are the same silence. Only we know whether the
// window drifted behind something or was shut, and that decides whether the person
// reads as away now or after the ordinary decay.

/** What our window is doing. */
export type WindowState = "active" | "background" | "closed";

/** Report our window's state. Defaults to `active` — the original meaning of a
 *  body-less beat, kept so callers that only ever say "I'm here" can stay silent
 *  about it. */
export async function reportAttention(opts: {
  state?: WindowState;
  signal?: AbortSignal;
}): Promise<void> {
  try {
    await fetch("/api/in/attention", {
      method: "POST",
            body: opts.state ?? "active",
      signal: opts.signal,
    });
  } catch {
    /* a dropped beat is harmless — presence just misses one report */
  }
}
