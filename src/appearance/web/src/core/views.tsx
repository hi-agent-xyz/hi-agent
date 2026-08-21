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
  traits?: ViewTraits;
}

interface ViewsValue {
  views: ActiveView[];
  /** Clear the screen back to the default empty room. Server-side, so every
   * device + a refresh converge on the cleared screen; the empty state arrives
   * via the same long-poll. */
  clear: () => void;
  /** Where this window can go back to, **newest first**: what the agent raised, plus
   * the places this window went on its own, one entry per destination. */
  trail: WireHistoryEntry[];
  /** The destination the agent has up now — the newest *raise*, which is not
   * necessarily the head of the trail: a bookmark opened after it sits above it. */
  live: string | null;
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
  /** Park on a named view from the inventory, and put it at the head of the trail:
   * going somewhere is arriving somewhere, whether the agent took you or you went. */
  openRef: (viewRef: string, label: string) => void;
}

const ViewsContext = createContext<ViewsValue>({
  views: [],
  clear: () => {},
  trail: [],
  live: null,
  parked: null,
  liveMoved: false,
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
    { id: string; moduleUrl: string; traits?: ViewTraits; slot?: string }[]
  >([]);
  const [history, setHistory] = useState<WireHistoryEntry[]>([]);
  /** Places this window went that the agent did not take it to — a bookmark opened.
   *
   * Local, and dies with the window. Not because the move is a secret — it is posted as
   * a perception the moment it happens, see `reportWentTo` — but because the server's
   * list is the record of *raises*, and its newest entry is what is on the stage:
   * appending to it would tell the desktop the agent had raised something because a
   * phone tapped a bookmark. So that list stays what it is and this rides alongside it
   * into the row. */
  const [visits, setVisits] = useState<WireHistoryEntry[]>([]);
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
    // Clearing is about the room, not only the server's slot. A parked window renders its
    // cursor in preference to the wire, so the control that says "the conversation takes
    // the screen back" cleared the slot and then visibly did nothing — the past raise it
    // was holding stayed up. Dropping the cursor is also this window's way home when there
    // is no live card to tap: a bookmark opened on an instance that has never raised
    // anything parks with nothing in the trail to go back to.
    if (parkedRef.current) {
      setParked(null);
      setLiveMoved(false);
      reportWentTo({ live: true });
    }
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

  const goTo = useCallback((entry: WireHistoryEntry) => {
    const key = destinationOf(entry);
    const live = key === liveKeyRef.current;
    // Told before the mount, not after: the report is what the agent reads to know
    // where they are, and a compile that hangs must not make it late.
    reportWentTo({
      viewRef: entry.view_ref,
      moduleUrl: entry.module_url,
      id: entry.id,
      live,
    });
    // Tapping the card marked *live* is going live, not parking on a copy of it —
    // `parked` means "somewhere other than the live one" and nothing else clears it
    // until the live destination CHANGES. Parking here looked identical on screen and
    // then, on the next raise, the window read as away: it kept the old view and put a
    // dot on the return control instead of following the raise it was standing on.
    if (live) {
      setParked(null);
      setLiveMoved(false);
      return;
    }
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

  const openRef = useCallback((viewRef: string, label: string) => {
    const live = viewRef === liveKeyRef.current;
    reportWentTo({ viewRef, live });
    // Same as `goTo`: opening the bookmark the agent already has up is being live. The
    // server holds that view on the stage, so there is nothing to resolve and nothing to
    // add to the trail — the raise is already a card in it.
    if (live) {
      setParked(null);
      setLiveMoved(false);
      return;
    }
    void openView(viewRef).then(
      (opened) => {
        setParked({
          key: viewRef,
          view: { id: opened.id, moduleUrl: opened.module_url, traits: opened.traits },
        });
        // Only on arrival: a request that failed is not a place anyone went.
        setVisits((was) => [
          ...was.filter((v) => v.view_ref !== viewRef),
          {
            id: opened.id,
            module_url: opened.module_url,
            view_ref: viewRef,
            label,
            at: new Date().toISOString(),
          },
        ]);
      },
      // Nothing the person can act on, and blanking the stage would be worse than
      // leaving them where they are.
      (error) => console.warn("opening a view failed", error),
    );
  }, []);

  // What the agent raised and where this window went, as one row. See `trailOf`.
  const trail = useMemo(() => trailOf(history, visits), [history, visits]);

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
      trail,
      live: liveKey,
      parked: parked?.key ?? null,
      liveMoved,
      goTo,
      openRef,
    };
  }, [wire, parked, clear, trail, liveKey, liveMoved, goTo, openRef]);
  return <ViewsContext.Provider value={value}>{children}</ViewsContext.Provider>;
}

/** The currently mounted layers, in z-order (content first, condition over it). */
export function useViews(): ViewsValue {
  return useContext(ViewsContext);
}
