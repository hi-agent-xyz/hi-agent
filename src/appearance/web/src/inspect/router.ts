// A tiny history-API router — enough for the inspect section's nested routes
// (/inspect, /inspect/sessions, …) without pulling in react-router. Components
// read the current path with usePath() and move with its navigate().
//
// Routes are the core's, not the browser's: `path` and `navigate` both speak in
// paths this core owns (`/inspect/sessions`), and the community's subpath is put
// on and taken off at this edge. Otherwise a relayed console would match no route
// at all — its every path would start `/ana`.

import { useCallback, useEffect, useState } from "react";

import { inCore, url } from "../lib/base";

export interface Router {
  path: string;
  navigate: (to: string, opts?: { replace?: boolean }) => void;
}

export function usePath(): Router {
  const [path, setPath] = useState(() => inCore());

  useEffect(() => {
    const onPop = () => setPath(inCore());
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);

  const navigate = useCallback((to: string, opts?: { replace?: boolean }) => {
    if (to === inCore()) return;
    if (opts?.replace) window.history.replaceState({}, "", url(to));
    else window.history.pushState({}, "", url(to));
    setPath(to);
  }, []);

  return { path, navigate };
}

/**
 * The selected id under a tab base, or null. `/inspect/sessions/7` with base
 * `/inspect/sessions` → `7`. Ids are URL-encoded in links, so they are decoded
 * here.
 */
export function selectedUnder(path: string, base: string): string | null {
  const prefix = `${base}/`;
  if (!path.startsWith(prefix)) return null;
  const raw = path.slice(prefix.length).replace(/\/$/, "");
  if (!raw) return null;
  try {
    return decodeURIComponent(raw);
  } catch {
    return raw;
  }
}
