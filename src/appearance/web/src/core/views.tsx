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
  openView,
  reportWentTo,
  type ViewTraits,
  type WireHistoryEntry,
} from "../channels/out/view";
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
  traits?: ViewTraits;
}

/** What identifies a *destination* — the same rule the server dedupes history by: the
 * durable ref when there is one, else the content-addressed module. Two raises of
 * `factory/tasks` are one place; two different inline views are two. */
function destinationOf(entry: { view_ref?: string; module_url: string }): string {
  return entry.view_ref ?? entry.module_url;
}

interface ViewsValue {
  views: ActiveView[];
  /** Clear the screen back to the default empty room. Server-side, so every
   * device + a refresh converge on the cleared screen; the empty state arrives
   * via the same long-poll. */
  clear: () => void;
  /** The recent raises, oldest first — the server's record of what it put up. */
  history: WireHistoryEntry[];
  /** The destination this window is parked on, or `null` when it is on the live one.
   * Local to this window and never reported, exactly like the conversation's scroll
   * position: a phone that went back must not move the desktop. **The move that put it
   * here is reported** — that is a different fact, and the agent needs it to read what
   * the person says next; see `reportWentTo`. */
  parked: string | null;
  /** A raise landed while this window was parked. The signal that replaces yanking. */
  liveMoved: boolean;
  /** Park on one past raise. */
  goTo: (entry: WireHistoryEntry) => void;
  /** Park on a named view from the inventory. */
  openRef: (viewRef: string) => void;
  /** Back to what the agent has up now. */
  returnToLive: () => void;
}

const ViewsContext = createContext<ViewsValue>({
  views: [],
  clear: () => {},
  history: [],
  parked: null,
  liveMoved: false,
  goTo: () => {},
  openRef: () => {},
  returnToLive: () => {},
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
    { id: string; moduleUrl: string; traits?: ViewTraits; slot?: string }[]
  >([]);
  const [history, setHistory] = useState<WireHistoryEntry[]>([]);
  /** Where this window is looking, when that is not the live view. */
  const [parked, setParked] = useState<{ key: string; view: ActiveView } | null>(null);
  const [liveMoved, setLiveMoved] = useState(false);
  const parkedRef = useRef<{ key: string; view: ActiveView } | null>(null);
  useEffect(() => {
    parkedRef.current = parked;
  }, [parked]);

  useEffect(() => {
    playingRef.current = reactive;
    if (reactive && voiceWaitersRef.current.size > 0) {
      const waiters = [...voiceWaitersRef.current];
      voiceWaitersRef.current.clear();
      waiters.forEach((wake) => wake());
    }
  }, [reactive]);

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
                traits: v.traits,
                slot: v.slot,
              })),
            );
            setHistory(state.history ?? []);
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

  // The live destination is the newest raise, because every raise appends one. When
  // this window is parked and that changes, the person is told rather than moved: a
  // raise arriving must not yank the thing they went back to read out from under
  // them, the same refusal the conversation makes by not auto-scrolling to a new
  // message. If the agent happens to raise exactly what they went back to, they are
  // simply live again and there is nothing to signal.
  const liveKey = history.length > 0 ? destinationOf(history[history.length - 1]!) : null;
  const prevLiveKeyRef = useRef<string | null>(null);
  // Read by the navigation callbacks, which must not re-create on every raise: going to
  // the newest card is going *live*, and the report says so rather than telling the agent
  // the person wandered off to the thing it just put up.
  const liveKeyRef = useRef<string | null>(null);
  useEffect(() => {
    liveKeyRef.current = liveKey;
    const prev = prevLiveKeyRef.current;
    prevLiveKeyRef.current = liveKey;
    const p = parkedRef.current;
    if (!p) return;
    if (liveKey && liveKey === p.key) {
      setParked(null);
      setLiveMoved(false);
      return;
    }
    if (liveKey !== prev) setLiveMoved(true);
  }, [liveKey]);

  const returnToLive = useCallback(() => {
    setParked(null);
    setLiveMoved(false);
    reportWentTo({ live: true });
  }, []);

  const goTo = useCallback((entry: WireHistoryEntry) => {
    const key = destinationOf(entry);
    // Told before the mount, not after: the report is what the agent reads to know
    // where they are, and a compile that hangs must not make it late.
    reportWentTo({
      viewRef: entry.view_ref,
      moduleUrl: entry.module_url,
      id: entry.id,
      live: key === liveKeyRef.current,
    });
    // An inline view has no durable name and is only ever the artifact it compiled
    // to, so it mounts straight from the record.
    if (!entry.view_ref) {
      setParked({ key, view: { id: entry.id, moduleUrl: entry.module_url } });
      return;
    }
    // A named view is re-resolved, so going back to `factory/tasks` lands on today's
    // board rather than a module compiled against a schema the app has moved past.
    void openView(entry.view_ref).then(
      (opened) =>
        setParked({
          key,
          view: { id: opened.id, moduleUrl: opened.module_url, traits: opened.traits },
        }),
      () =>
        // Source gone, or no longer compiling. The artifact it was shown as is still
        // on disk and still mounts: a stale view beats an empty room, which is the
        // same call `ViewBus::refresh_sources` makes on the server.
        setParked({ key, view: { id: entry.id, moduleUrl: entry.module_url } }),
    );
  }, []);

  const openRef = useCallback((viewRef: string) => {
    reportWentTo({ viewRef, live: viewRef === liveKeyRef.current });
    void openView(viewRef).then(
      (opened) =>
        setParked({
          key: viewRef,
          view: { id: opened.id, moduleUrl: opened.module_url, traits: opened.traits },
        }),
      // Nothing the person can act on, and blanking the stage would be worse than
      // leaving them where they are.
      (error) => console.warn("opening a view failed", error),
    );
  }, []);

  const value = useMemo<ViewsValue>(() => {
    const bare = ({ id, moduleUrl, traits }: (typeof wire)[number]): ActiveView => ({
      id,
      moduleUrl,
      traits,
    });
    const views = parked
      ? [parked.view, ...wire.filter((v) => v.slot === "condition").map(bare)]
      : wire.map(bare);
    return {
      views,
      clear,
      history,
      parked: parked?.key ?? null,
      liveMoved,
      goTo,
      openRef,
      returnToLive,
    };
  }, [wire, parked, clear, history, liveMoved, goTo, openRef, returnToLive]);
  return <ViewsContext.Provider value={value}>{children}</ViewsContext.Provider>;
}

/** The currently mounted layers, in z-order (content first, condition over it). */
export function useViews(): ViewsValue {
  return useContext(ViewsContext);
}
