import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  subscribeViewState,
  clearViewState,
  goToView,
  type WireHistoryEntry,
} from "../channels/out/view";
import { destinationOf, trailOf } from "./trail";
import { usePresence, useWake } from "./session";

// How long a newly appearing view waits for the voice before showing anyway.
// The view is paced to its narration, but the /view and /audio channels have
// very different latencies, so a view tends to land a beat ahead of the speech
// it belongs to. Holding it until the voice is audibly playing closes that gap;
// this fallback ensures a silent or text-only turn (no voice to wait for) still
// shows the view promptly rather than stalling on a beat that never sounds.
const VOICE_GATE_FALLBACK_MS = 1000;

/** One mounted layer: a stable id, the compiled module URL to import, and what
 * the view declared about itself. Every layer is full-bleed — there is no
 * placement here because there is no longer any placement to carry. */
export interface ActiveView {
  id: string;
  moduleUrl: string;
}

interface ViewsValue {
  views: ActiveView[];
  /** Clear the screen back to the default empty room. Server-side, so every
   * device + a refresh converge on the cleared screen; the empty state arrives
   * via the same long-poll. */
  clear: () => void;
  /** Where the screen can go back to, **newest first** — the agent's shows and the
   * person's own moves, one entry per destination. The server's list, whole. */
  trail: WireHistoryEntry[];
  /** The destination the agent has up in the content slot, or `null` for an empty room.
   * Not necessarily the head of the trail: a bookmark opened after it sits above it. */
  live: string | null;
  /** The destination the screen is parked on, or `null` when it is live. **The
   * server's, not this window's**: going back on the phone is going back on the
   * desktop, because there is one screen and both hands reach it. */
  parked: string | null;
  /** Take the screen to one past destination. */
  goTo: (entry: WireHistoryEntry) => void;
  /** Take the screen to a named view from the inventory, which puts it at the head of
   * the trail if it has never been there: going somewhere is arriving somewhere,
   * whether the agent took you or you went. */
  openRef: (viewRef: string) => void;
}

const ViewsContext = createContext<ViewsValue>({
  views: [],
  clear: () => {},
  trail: [],
  live: null,
  parked: null,
  goTo: () => {},
  openRef: () => {},
});

/**
 * Runs the /api/out/view long-poll ABOVE the view slot, so the stream — like the
 * session's channel loops — survives any view swap. Mirrors the server's retained
 * retained appearance state: each response is the whole set of active views in
 * z-order, so a fresh page, a second device, or a reconnect all converge on the
 * same screen. A view persists until the agent dismisses or replaces it — there
 * is no client-side expiry; the next snapshot is the only lifecycle driver.
 */
export function ViewsProvider({ children }: { children: ReactNode }) {
  const { woken } = useWake();
  // Whether the agent's voice is audibly playing right now — the gate signal
  // for a newly appearing view (see VOICE_GATE_FALLBACK_MS). Mirrored into a ref
  // (read by the subscription loop without re-subscribing) plus a waiter set the
  // sync effect flushes the instant the voice starts.
  const { reactive } = usePresence();
  const playingRef = useRef(false);
  const voiceWaitersRef = useRef<Set<() => void>>(new Set());
  /** The live layers, in wire order (= z-order), each tagged with the slot it came
   * out of so a parked window can keep the condition layer over what it went back to. */
  const [wire, setWire] = useState<
    { id: string; moduleUrl: string; slot?: string }[]
  >([]);
  const [history, setHistory] = useState<WireHistoryEntry[]>([]);
  /** Where the screen is parked, and what the agent has in the content slot. Both come
   * off the wire, so this window holds no opinion of its own about either: it renders
   * the screen rather than keeping one. */
  const [cursor, setCursor] = useState<string | null>(null);
  const [liveKey, setLiveKey] = useState<string | null>(null);

  useEffect(() => {
    playingRef.current = reactive;
    if (reactive && voiceWaitersRef.current.size > 0) {
      const waiters = [...voiceWaitersRef.current];
      voiceWaitersRef.current.clear();
      waiters.forEach((wake) => wake());
    }
  }, [reactive]);

  // Clearing is about the room, not only the agent's slot — so the server drops the
  // cursor with it, and this is one call. It used to take two, and the two could
  // disagree: the slot cleared and the past view the window was holding stayed up.
  const clear = useCallback(() => {
    void clearViewState();
  }, []);

  useEffect(() => {
    if (!woken) return;
    const ctrl = new AbortController();
    let cancelled = false;

    // Resolve when the voice starts playing, or after `ms` (a silent/text turn,
    // or muted output — no voice to wait for), or on teardown. Used to hold a
    // newly appearing view until its narration is actually sounding.
    const waitForVoice = (ms: number) =>
      new Promise<void>((resolve) => {
        if (playingRef.current || cancelled) return resolve();
        let settled = false;
        const finish = () => {
          if (settled) return;
          settled = true;
          clearTimeout(timer);
          voiceWaitersRef.current.delete(finish);
          ctrl.signal.removeEventListener("abort", finish);
          resolve();
        };
        const timer = setTimeout(finish, ms);
        voiceWaitersRef.current.add(finish);
        ctrl.signal.addEventListener("abort", finish, { once: true });
      });

    void (async () => {
      // Ids currently applied to the screen. Only this loop applies snapshots,
      // so a local set is authoritative — and lets us tell a view *appearing*
      // (gate it on the voice) from a removal or a swap of an on-screen view
      // (apply at once). Persists across reconnects within this effect.
      const applied = new Set<string>();
      while (!cancelled) {
        try {
          for await (const state of subscribeViewState({ signal: ctrl.signal })) {
            if (cancelled) break;
            // A snapshot that brings up a view id not on screen yet is held
            // until the voice is audibly playing (or the fallback elapses), so
            // it doesn't pop in a beat ahead of the speech it's paced to.
            // Removals and replaces of already-shown views apply immediately.
            const introducesView = state.views.some((v) => !applied.has(v.id));
            if (introducesView && !playingRef.current) {
              await waitForVoice(VOICE_GATE_FALLBACK_MS);
              if (cancelled) break;
            }
            applied.clear();
            for (const v of state.views) applied.add(v.id);
            // Mirror the snapshot wholesale: array order = z-order. ViewSlot
            // keys by id, so unchanged views keep their mounted component.
            setWire(
              state.views.map((v) => ({
                id: v.id,
                moduleUrl: v.module_url,
                slot: v.slot,
              })),
            );
            setHistory(state.history ?? []);
            // The cursor arrives with the slots, in the same snapshot and under the
            // same version, so there is no second sync path that could disagree with
            // the first about where the screen is.
            setCursor(state.cursor ?? null);
            setLiveKey(state.live ?? null);
          }
        } catch {
          if (cancelled || ctrl.signal.aborted) break;
          await new Promise((r) => setTimeout(r, 1500));
        }
      }
    })();

    return () => {
      cancelled = true;
      ctrl.abort();
    };
  }, [woken]);

  // Both navigations are the same act — put the screen here — and neither mounts
  // anything itself. The server writes the cursor and the long-poll delivers it, to this
  // window like any other, which is what makes the phone and the desktop agree. Going to
  // the card marked *live* is going live; the server settles that, because "live" is a
  // fact about the content slot and the slot is what holds it.
  const goTo = useCallback((entry: WireHistoryEntry) => {
    void goToView({
      viewRef: entry.view_ref,
      moduleUrl: entry.module_url,
      id: entry.id,
      // Nothing the person can act on, and blanking the screen would be worse than
      // leaving them where they are. A named view whose source is gone keeps its card:
      // the artifact it was shown as is still on disk, which is the same call
      // `ViewBus::refresh_sources` makes on the server.
    }).catch((error) => console.warn("going to a view failed", error));
  }, []);

  const openRef = useCallback((viewRef: string) => {
    void goToView({ viewRef }).catch((error) => console.warn("opening a view failed", error));
  }, []);

  // The server's list, head-first. See `trailOf`.
  const trail = useMemo(() => trailOf(history), [history]);

  const value = useMemo<ViewsValue>(() => {
    const bare = ({ id, moduleUrl }: (typeof wire)[number]): ActiveView => ({ id, moduleUrl });
    // Parked: the cursor's card takes the content layer's place, and the condition layer
    // stays over it — an outage must still cover whatever the screen went back to.
    const parkedOn = cursor
      ? history.find((entry) => destinationOf(entry) === cursor)
      : undefined;
    const views = parkedOn
      ? [
          { id: parkedOn.id, moduleUrl: parkedOn.module_url },
          ...wire.filter((v) => v.slot === "condition").map(bare),
        ]
      : wire.map(bare);
    return {
      views,
      clear,
      trail,
      live: liveKey,
      parked: cursor,
      goTo,
      openRef,
    };
  }, [wire, cursor, history, clear, trail, liveKey, goTo, openRef]);
  return <ViewsContext.Provider value={value}>{children}</ViewsContext.Provider>;
}

/** The currently mounted layers, in z-order (content first, condition over it). */
export function useViews(): ViewsValue {
  return useContext(ViewsContext);
}
