// Keeping a surface honest about *when* it was read.
//
// Every review view reads state the agent changes on its own initiative — the drive it
// files into, the skills it writes, the clusters it grows, the tasks it opens and closes.
// A view that fetches once on mount is therefore not showing state; it is showing a
// timestamp nobody can see. That is worse than an error, because a quietly stale reading
// still reads as authoritative: it is the surface someone checks *before* asking "did you
// drop that?", and the wrong answer there is indistinguishable from a lie.
//
// Re-showing is not a refresh path, which is the reason this has to be the view's own job.
// Compiled modules are content-addressed, so identical source is an identical URL, and
// `ViewMount` keys its import on that URL — a `show` of an already-visible view keeps the
// same slot and the same component instance, effects and all. Nothing outside the view can
// make it read again.
//
// So: `useLive` owns the clock, and the view keeps its own state. It is deliberately not a
// store. Views here fan one reload out over several pieces of state (a roster *and* a
// ledger; an index, a selection and a list), and a data-owning hook would have to be fought
// in every one of them.
//
// It reads nothing from the session context on purpose, so a headless review render — which
// fabricates a session rather than mounting `SessionProvider` — can still use a view that
// polls.

import { useEffect, useRef } from "react";

/**
 * How often a surface re-reads, named rather than numbered.
 *
 * There are two tempos here and both were already in the tree before this hook existed,
 * arrived at twice independently: the workers roster on 2s and the task board on 8s. The
 * distinction they found is real and worth keeping as vocabulary — a surface is either
 * something you are *watching happen* or something you are *checking*. Naming them stops
 * the third, fourth and fifth number from being invented per view.
 */
export const TEMPO = {
  /** Watching something happen: a live roster, a pairing you are waiting to complete. */
  watching: 2000,
  /** Checking a ledger: what it has filed, remembered, learned, or is carrying. */
  ledger: 8000,
  /**
   * No clock — read on mount and again whenever the page is looked at afresh.
   *
   * For a reading whose cost is not proportional to a tick. `/api/stats` scans every frame
   * log in the window, so putting it on the ledger clock would spend that scan repeatedly
   * on a page nobody is interacting with. Attention is the honest trigger there, and it
   * still fixes the staleness that matters: a page left open for an hour is re-read when
   * you come back to it, not an hour later on a treadmill.
   */
  onAttention: 0,
} as const;

/** Why a read is being considered. `mount` is the view's first, and is unconditional. */
export type ReadReason = "mount" | "tick" | "attention";

/** What the hook knows at the moment it decides whether to read. */
export interface ReadState {
  /** `document.hidden` — nothing is being read, so nothing needs re-reading. */
  hidden: boolean;
  /** A previous read has not come back. Overlapping reads can land out of order. */
  inFlight: boolean;
  /** The view says now is not the time — see the `hold` option. */
  held: boolean;
}

/**
 * Whether to read, given why we are asking and what is going on.
 *
 * The mount read ignores both `hidden` and `held`, and that is not a shortcut: a view
 * mounted in a background window must still have something to draw when it is brought
 * forward, and holding the first read would leave the skeleton up for as long as the guard
 * lasts. Only `inFlight` can stop it, because a second identical read is pure waste.
 */
export function shouldRead(reason: ReadReason, state: ReadState): boolean {
  if (state.inFlight) return false;
  if (reason === "mount") return true;
  return !state.hidden && !state.held;
}

/**
 * The interval a period asks for, or `null` for no clock at all.
 *
 * `headless` is the review renderer. Its page is a single capture rather than a session:
 * it waits for `__hiRender.ready` — React committed, two frames painted, fonts and images
 * resolved — and then screenshots. A clock on that page cannot make the shot more correct
 * (the mount read already filled the view) and its ticks would keep firing through the
 * capture, so the review of a `watching`-tempo view would re-render itself several times
 * under the camera for nothing.
 */
export function clockFor(period: number, headless: boolean): number | null {
  if (headless) return null;
  if (!Number.isFinite(period) || period <= 0) return null;
  return period;
}

/** True on the standalone render page, which publishes its report before React mounts. */
function isHeadless(): boolean {
  if (typeof window === "undefined") return false;
  return (window as unknown as { __hiRender?: unknown }).__hiRender !== undefined;
}

export interface LiveOptions {
  /** From [`TEMPO`]. Defaults to `TEMPO.ledger`, which is what a review surface is. */
  period?: number;
  /**
   * What is being read, when that can change without the component being replaced.
   *
   * `reload` is called through a ref, so it always closes over the current render — but the
   * hook cannot *tell* that the question changed, and would leave the old answer up until
   * the next tick. Pass a string that identifies the subject (a range, an id, a pair) and a
   * change to it counts as a fresh mount: an immediate read, and the clock restarted.
   *
   * Prefer a React `key` on the component when the subject changing should also throw away
   * the state around it — a detail panel pointed at something else wants its scroll,
   * expansion and "loading" reset, and a remount does all of that for free. Reach for
   * `subject` when the surrounding state is meant to survive, which is what a range switch
   * on a chart wants: the previous reading stays on screen while the new one loads.
   */
  subject?: string;
  /**
   * "Not now" — checked on every tick, never on the mount read.
   *
   * This is the difference between a poll and a poll you can use. A tick that lands
   * mid-write flips a card back under the click that changed it; one that lands mid-drag
   * re-renders the board out from under a held card and cancels the drag; one that lands on
   * an edited-but-unsaved facet throws the edit away. Read the guard from a ref, not from
   * state, when the guard is set imperatively around an `await` — state has not committed
   * yet at that point and a tick in the gap sees the old value.
   */
  hold?: () => boolean;
}

/**
 * Re-run `reload` for as long as someone is looking at the page.
 *
 * Reads once on mount, then on `period`, skipping any tick where the page is hidden, a
 * previous read is still out, or `hold()` says not now — and reads immediately when the
 * page becomes visible again, so coming back to a surface does not mean waiting out a tick
 * in front of a stale one.
 *
 * `reload` is read through a ref, so it does not need to be a `useCallback` and the clock
 * never closes over the first render's copy of it. `reload` keeps owning its own state and
 * its own failure: a read that does not come back should leave the last good reading
 * standing rather than blanking the surface, because empty is a claim ("nobody is on
 * anything") and a failed fetch is not entitled to make it.
 */
export function useLive(reload: () => void | Promise<void>, options: LiveOptions = {}): void {
  const { period = TEMPO.ledger, hold, subject } = options;

  const reloadRef = useRef(reload);
  reloadRef.current = reload;
  const holdRef = useRef(hold);
  holdRef.current = hold;
  const inFlight = useRef(false);

  useEffect(() => {
    let alive = true;

    const read = async (reason: ReadReason) => {
      if (!alive) return;
      const go = shouldRead(reason, {
        hidden: document.hidden,
        inFlight: inFlight.current,
        held: holdRef.current?.() ?? false,
      });
      if (!go) return;
      inFlight.current = true;
      try {
        await reloadRef.current();
      } finally {
        inFlight.current = false;
      }
    };

    void read("mount");

    const clock = clockFor(period, isHeadless());
    const timer = clock === null ? null : setInterval(() => void read("tick"), clock);
    const onShow = () => {
      if (!document.hidden) void read("attention");
    };
    document.addEventListener("visibilitychange", onShow);

    return () => {
      alive = false;
      if (timer !== null) clearInterval(timer);
      document.removeEventListener("visibilitychange", onShow);
    };
  }, [period, subject]);
}
