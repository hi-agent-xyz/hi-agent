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

