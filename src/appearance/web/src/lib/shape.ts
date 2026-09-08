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
// (`ui/PanelGesture.tsx`), and a query written once in CSS and again in `matchMedia`
// is two answers waiting to disagree — a page that swipes back on a screen the
// stylesheet is still drawing as a popover.
export const PHONE = "(max-width: 640px) and (pointer: coarse)";

// The other half on its own. Some of what a finger changes is not about the
// arrangement at all and so does not follow the shape: the browser's own touch
// gestures, and the 16px floor under which iOS zooms the page when a text field
// is focused (`.hi-composer textarea` in `ui/global.css`). Those are facts about
// the input method, and they hold at any width — **the same iPhone turned
// sideways is 852px and is no longer `phone`, and it still zooms.** Hanging them
// off the shape flag would have handed the browser's behaviour back on rotation.
//
// Published as `<html data-pointer="coarse|fine">` for the same reason the shape
// is: one place asks the question, everywhere else reads the answer.
export const COARSE = "(pointer: coarse)";

// The third shape, and **the only one that is not measured.**
//
// A television is wide, like the desktop window, and it is pointed at with
// nothing at all — the cursor is focus itself, moved a control at a time by a
// D-pad. No media query separates it from a large monitor: it is not narrow, its
// `pointer` is whatever the device's WebView decides to report, and a browser
// opened full-screen on a TV-shaped panel is genuinely not a television. Asking
// harder would repeat the mistake this file exists to record — `max-width` was
// also a plausible way to find a phone.
//
// So the host says so, the way the desktop window says it spans a titlebar by
// asking for `/?chrome=titlebar` (`lib/chrome.ts`). The Android TV client loads
// `/?shape=tv`, and nothing else ever will.
//
// Being declared rather than matched has a second consequence worth stating: it
// does not change. A phone rotates and a window is dragged narrow, so those flags
// keep listening; a television is the same television all evening, so this one is
// written once and no listener is installed for it.
const TV = "tv";

/** Whether this page was opened by a host that calls itself a television. */
export function declaredTv(search: string = window.location.search): boolean {
  return new URLSearchParams(search).get("shape") === TV;
}

// The room a view is composed for. `view_render`'s `DEFAULT_WIDTH` is this number —
// the frame a review falls back to when no face has reported one — so it is the width
// the workshop's output is implicitly aimed at.
//
// **A room narrower than this is told this number anyway, and drawn to fit.** The shape
// flag above already decided that a phone turned sideways stops being a phone and the
// room is a room again; the host rearranges on it, and the view did not, because the
// view was reading raw pixels. 852 of them land above 33 of the breakpoints written
// across this workshop's views and below 37, so which arrangement a person got by
// turning their phone was a coin flip per view — and the one arrangement they could
// never get was the one the view was composed for.
//
// **The scale is affordable precisely on the devices this applies to**, because they are
// the dense ones. 1280 CSS px across an iPhone's 2556 device pixels in landscape is
// still 2 device pixels per CSS pixel — retina density — and a phone is held at about
// 30cm against a laptop's 50cm, so the text lands within a few percent of the same
// *angular* size. Nothing is shrunk; it is viewed closer.
export const VIEW_TARGET = 1280;

// How far the slot may be scaled down before the trade stops paying. The argument for
// scaling at all is that a phone is dense and held close, so a factor near ⅔ lands within
// a few percent of a laptop's angular text size — that is a claim about ⅔, not about any
// factor. Below this the text is simply smaller than the person's screen can justify, and
// a view laying itself out honestly for the room it really has is the better answer.
//
// It is also the only condition that has to name the phone: `0.6` admits a slot of 768px
// and up, so a portrait phone (393px, and a factor of 0.31) is excluded by arithmetic
// rather than by a second rule about phones. One rule, one reason.
const MIN_SCALE = 0.6;

/**
 * How much to scale a view slot `width` CSS pixels wide so it can be told
 * [`VIEW_TARGET`] — or `null` to leave it alone, which is most rooms.
 *
 * Pure, and exported, because it is the whole of the policy and the only part worth
 * testing without a browser.
 *
 * **`width` is the slot's, not the window's**, and the difference is not academic: with
 * the panel open beside a view the slot is 432 of an 852px landscape phone. Measured on a
 * live instance, an earlier version of this took the window's width and scaled a
 * half-width slot by the whole window's factor, so the view was told 649 rather than
 * either number meaning anything.
 *
 * **Coarse** is the other condition, and it keeps the pointer out of it: a desktop window
 * dragged narrow is a choice its owner can undo, while a phone's width is the device.
 */
export function viewZoom(width: number, coarse: boolean): number | null {
  if (!coarse || !(width > 0) || width >= VIEW_TARGET) return null;
  const scale = width / VIEW_TARGET;
  return scale >= MIN_SCALE ? scale : null;
}

/** The three arrangements the face knows how to be. */
export type Shape = "phone" | "tv" | "wide";

/** A live match, or `null` where there is no `matchMedia` (the render worker,
 * tests). Kept as objects so the flags on `<html>` and the hook below read the
 * same lists. */
function query(list: string): MediaQueryList | null {
  return typeof window === "undefined" || !window.matchMedia ? null : window.matchMedia(list);
}

const phoneQuery = query(PHONE);
const coarseQuery = query(COARSE);

/**
 * Hoist the shape — and the pointer — onto `<html>`, and keep them there. Called
 * once from `main.tsx` before the first render, alongside the other host facts.
 *
 * They re-read on change rather than only at boot, because every input to the
 * answer moves under a running page: a phone rotates into landscape (still
 * coarse, now 852px wide, so the room is a room again), an iPad splits its screen
 * down to a 507px column, a desktop window is dragged narrow, a 2-in-1 laptop has
 * its keyboard folded back. The flags would otherwise be facts about how the app
 * happened to open.
 */
export function installShape(): void {
  if (declaredTv()) {
    // Both flags, and no listeners. `data-pointer="none"` is not decoration: the
    // coarse rules are about a *finger* — the browser's touch gestures, the 16px
    // floor under which iOS zooms a focused field — and a remote control is not a
    // finger. A television that reported itself coarse would inherit a phone's
    // compensations for a pointer it does not have.
    document.documentElement.setAttribute("data-shape", TV);
    document.documentElement.setAttribute("data-pointer", "none");
    return;
  }
  write(phoneQuery, "data-shape", "phone", "wide");
  write(coarseQuery, "data-pointer", "coarse", "fine");
}

/**
 * Keep `slot`'s scale current: measure the slot, ask [`viewZoom`], write the answer onto
 * the slot. Returns the teardown.
 *
 * **On the slot and not on `<html>`, because the question is about the slot.** The two
 * flags above are facts about the device and belong to the document; this one is a fact
 * about one box, and the box is not the window — the panel takes 420 of an 852px
 * landscape phone. A `resize` listener would miss that entirely: the window does not
 * change when the panel opens.
 *
 * `ResizeObserver` rather than a media query for the same reason, and it covers the
 * rotation and the window drag for free, since both reach the slot as a resize.
 */
export function watchViewZoom(slot: HTMLElement): () => void {
  const apply = () => {
    const zoom = declaredTv()
      ? null
      : viewZoom(slot.clientWidth, coarseQuery?.matches ?? false);
    if (zoom === null) {
      slot.removeAttribute("data-view-zoom");
      slot.style.removeProperty("--hi-view-zoom");
      return;
    }
    slot.setAttribute("data-view-zoom", "");
    slot.style.setProperty("--hi-view-zoom", String(zoom));
  };
  apply();
  if (typeof ResizeObserver === "undefined") return () => {};
  const observer = new ResizeObserver(apply);
  observer.observe(slot);
  // The pointer can change under a running page — a tablet gains a trackpad, a 2-in-1 is
  // folded — and that changes the answer without changing the slot's size.
  coarseQuery?.addEventListener("change", apply);
  return () => {
    observer.disconnect();
    coarseQuery?.removeEventListener("change", apply);
  };
}

function write(list: MediaQueryList | null, attr: string, on: string, off: string): void {
  if (!list) return;
  const set = () => document.documentElement.setAttribute(attr, list.matches ? on : off);
  set();
  list.addEventListener("change", set);
}

function subscribe(onChange: () => void): () => void {
  phoneQuery?.addEventListener("change", onChange);
  return () => phoneQuery?.removeEventListener("change", onChange);
}

/**
 * Which arrangement this is. The components that read it decide *what to render*
 * — a back chevron, the controls as the page's bar — while the stylesheet decides
 * how it looks off the same flag.
 *
 * The television is checked first and wins outright: it is a declaration and the
 * other two are measurements, so there is no case where both could be true and a
 * precedence to argue about.
 */
export function useShape(): Shape {
  const phone = useSyncExternalStore(
    subscribe,
    () => phoneQuery?.matches ?? false,
    // The server/prerender answer. A view rendered off-screen has no window to
    // measure, and the wide shape is the one that needs no gesture attached.
    () => false,
  );
  if (typeof window !== "undefined" && declaredTv()) return TV;
  return phone ? "phone" : "wide";
}

/** Whether this is the held-in-a-hand shape — the one whose surfaces are pages. */
export function useIsPhone(): boolean {
  return useShape() === "phone";
}
