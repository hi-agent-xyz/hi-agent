// `@hi/core` — the session surface a view authors against. Host chrome and
// agent-authored views both import these hooks; the import map (Stage 1-2)
// guarantees every importer shares the one provider instance.
export {
  SessionProvider,
  useMessages,
  usePresence,
  useWake,
  useChannels,
  useSendText,
} from "./session";
export { ViewsProvider, useViews, type ActiveView } from "./views";
// The clock a surface re-reads on. Any view showing state the agent changes on its own
// initiative needs one, because re-showing a view does not remount it and so cannot
// refresh it — see `core/live.ts`.
export { useLive, TEMPO, type LiveOptions } from "./live";
// …and the one that has no clock: a surface whose store can say when it moved parks on
// that instead of asking again. See `core/live.ts` § watching, without a clock.
export { useWatched, type WatchOptions } from "./live";

// Whether an input method owns a keydown. A view with a line of its own — the task panel's
// reply box — answers Enter and Escape the way the conversation's line does, which means
// leaving both to the IME mid-composition. See `lib/keyboard.ts`.
export { inputMethodHasKey } from "../lib/keyboard";
