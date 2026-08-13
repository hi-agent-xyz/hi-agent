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
// Through an app this is always empty: the app proxies from its own root, holds
// the credential, and the face never learns where the core actually is. The
// prefix exists for the one case where a browser is pointed straight at a
// relayed core.

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
  return b + path;
}
