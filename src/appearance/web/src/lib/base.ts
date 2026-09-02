// Where this page is served from.
//
// A core is usually at the root of its own address — `http://localhost:12358`,
// or an app's local proxy. When the community routes by subpath it is at
// `https://hi-agent.xyz/ana` instead, and every absolute path the page uses has
// to start there or it lands on the community's own routes.
//
// The backend knows the prefix (it arrives as `X-Forwarded-Prefix`) and stamps
// it onto the page; this reads it back. Empty is the normal case and costs
// nothing.
//
// Through the *desktop* app this is always empty: `crates/hi-app` proxies from
// its own root, holds the credential, and the face never learns where the core
// actually is. **The iPhone app does not.** `CoreWebView` loads the paired
// address itself — which is what the QR carried, `https://hi-agent.xyz/ana` —
// with the session cookie set on that origin, so on a phone the prefix is not
// the exceptional case, it is the normal one. Anything below that only holds
// "when the prefix is empty" is broken on every phone, and only on phones.

declare global {
  interface Window {
    __HI_BASE__?: string;
  }
}

/** The path prefix this page is served under — `""` or `"/ana"`, never trailing. */
export function base(): string {
  const raw = (typeof window !== "undefined" && window.__HI_BASE__) || "";
  return raw.replace(/\/+$/, "");
}

/**
 * Resolve a root-absolute path against the prefix.
 *
 * `url("/api/in/text")` is `/api/in/text` locally and `/ana/api/in/text` behind
 * the community. Anything already absolute (a full URL) or already prefixed is
 * returned untouched, so this is safe to apply to a path the backend handed us.
 */
export function url(path: string): string {
  const b = base();
  if (!b || !path.startsWith("/")) return path;
  if (path === b || path.startsWith(b + "/")) return path;
  // The core's root is the prefix itself, with no slash after it: that is the
  // address the community hands out, and a link home should read as that address
  // rather than a variant of it.
  if (path === "/") return b;
  return b + path;
}

/**
 * A browser path as the core reads it — `/inspect` whether this page is served
 * at `/inspect` or at `/ana/inspect`.
 *
 * The inverse of {@link url}: routing decisions are made against the path the
 * core owns, and only turned back into a browser path when one is written.
 */
export function inCore(pathname: string = window.location.pathname): string {
  const b = base();
  if (!b || !pathname.startsWith(b)) return pathname;
  return pathname.slice(b.length) || "/";
}

/**
 * Apply the prefix to every root-absolute `fetch` and `EventSource` the page
 * makes, so a path names this core wherever the core happens to be served.
 *
 * **Why a seam and not a rule.** `url()` alone asks every call site to remember,
 * and most of the views on this page are written by the agent at runtime — there
 * is no review pass in which a forgotten one gets caught. The failure is also
 * invisible until the one shape that matters: at the core's own root the prefix
 * is empty and a bare `/api/x` is right, so nothing local ever shows the bug.
 * This is the same rule as `url()`, applied once where the request leaves —
 * `url()` is idempotent, so a call site that already used it is still correct.
 *
 * A no-op when there is no prefix, which is every ordinary shape.
 *
 * Not covered, because there is no interception point: what a view puts in an
 * attribute (`<img src>`, `<a href>`), a CSS `background-image`, a `new Audio(…)`,
 * and a `WebSocket` built from `location.host`. Those call `url()` by hand.
 *
 * That gap is not theoretical. Three call sites shipped without it — the face
 * crops and voice clips in `people-review`, the picture on every tile of the
 * views band, and the task-file links — and each was invisible on a desktop and
 * blank on a phone, where the prefix is the ordinary case. A regex over the
 * sources would not have caught the band: the path came from the backend and sat
 * in a variable (`src={shot}`), which reads like every correct call site. The
 * only fix that closes the class is for the phone to stop being served under a
 * prefix at all — an iOS-side local proxy, the way `crates/hi-app` already works.
 */
export function installBase(): void {
  if (!base()) return;

  const originalFetch = window.fetch.bind(window);
  window.fetch = (input, init) => {
    if (typeof input === "string") return originalFetch(url(input), init);
    if (input instanceof URL && input.origin === location.origin) {
      return originalFetch(new URL(url(input.pathname) + input.search + input.hash, input), init);
    }
    return originalFetch(input, init);
  };

  const NativeEventSource = window.EventSource;
  window.EventSource = class extends NativeEventSource {
    constructor(source: string | URL, init?: EventSourceInit) {
      super(typeof source === "string" ? url(source) : source, init);
    }
  } as typeof EventSource;
}
