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
// Where this page is served from. A view that calls the agent's own API needs it
// for the same reason the host does: under the community's subpath, `/api/x` is
// the community's route and `/ana/api/x` is the agent's. Empty everywhere else,
// so a view that ignores it is only wrong in the one shape.
export { base, url } from "../lib/base";

