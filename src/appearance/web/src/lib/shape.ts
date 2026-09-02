import { useSyncExternalStore } from "react";

// What shape of screen this is, as far as *presentation* has to care.
//
// The face is one build running in four places: the desktop window, the ~380px
// menu-bar popover, a browser tab, and the iPhone client's web view
// (`CoreWebView.swift`). Three of them are pointed at with a mouse and one is
// held in a hand, and only the held one wants the host's surfaces to be pages
// pushed onto a stack that a thumb swipes back out of, rather than panels
// floating in a corner of a window.
//
// **Width alone cannot tell those two groups apart, and assuming it could is what
// shrank the controls on the phone.** The menu-bar popover is about 380x540, so a
// `max-width: 420px` block written for it — tighten everything, the whole face has
// to fit — was also the block a 393px iPhone matched, and it took the channel
// discs from 38px down to 32 on the one host where the pointer is a finger and
// 44 is the floor. Pointer type is the half of the question that width does not
// answer: the popover hovers, a phone does not.
//
// So the phone is `narrow AND coarse`, and it is published as
// `<html data-shape="phone">` rather than left as a media query each rule repeats.
// Same reason `data-chrome` is a flag on `<html>` (`lib/chrome.ts`), plus one this
// one has on its own: **the gesture that goes with the shape is JavaScript**
// (`ui/PageEdge.tsx`), and a query written once in CSS and again in `matchMedia`
// is two answers waiting to disagree — a page that swipes back on a screen the
// stylesheet is still drawing as a popover.
export const PHONE = "(max-width: 640px) and (pointer: coarse)";

/** The live match, or `false` where there is no `matchMedia` (the render worker,
 * tests). Kept as one object so both the flag on `<html>` and the hook below read
 * the same list. */
function query(): MediaQueryList | null {
  return typeof window === "undefined" || !window.matchMedia ? null : window.matchMedia(PHONE);
}

const phoneQuery = query();

/**
 * Hoist the shape onto `<html>`, and keep it there. Called once from `main.tsx`
 * before the first render, alongside the other host facts.
 *
 * It re-reads on change rather than only at boot, because every input to the
 * answer moves under a running page: a phone rotates into landscape (still
 * coarse, now 852px wide, so the room is a room again), an iPad splits its screen
 * down to a 507px column, a desktop window is dragged narrow. The flag would
 * otherwise be a fact about how the app happened to open.
 */
export function installShape(): void {
  if (!phoneQuery) return;
  const write = () =>
    document.documentElement.setAttribute("data-shape", phoneQuery.matches ? "phone" : "wide");
  write();
  phoneQuery.addEventListener("change", write);
}

function subscribe(onChange: () => void): () => void {
  phoneQuery?.addEventListener("change", onChange);
  return () => phoneQuery?.removeEventListener("change", onChange);
}

/** Whether this is the held-in-a-hand shape. The components that read it decide
 * *what to render* — a back chevron, the controls as the page's bar — while the
 * stylesheet decides how it looks off the same flag. */
export function useIsPhone(): boolean {
  return useSyncExternalStore(
    subscribe,
    () => phoneQuery?.matches ?? false,
    // The server/prerender answer. A view rendered off-screen has no window to
    // measure, and the wide shape is the one that needs no gesture attached.
    () => false,
  );
}
