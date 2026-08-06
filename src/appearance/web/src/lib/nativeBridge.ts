// Native (desktop) ↔ web bridge.
//
// The macOS app hosts this page in a WKWebView that is *reused*, not torn down:
// closing the window sends the app to the background and hides the view, but the
// page — and its whole React tree — keeps running so a reopen is instant (see
// src/foundation/vendors/macos_window.rs). That warmth is the point, but it means
// the page can't lean on load/unmount to know when it's on screen. The native
// shell tells it instead, by dispatching a `hi:lifecycle` CustomEvent whenever the
// app moves between foreground and background. The page listens and pauses/restores
// things a hidden window shouldn't keep doing — first among them holding the
// microphone and camera open.
//
// In a plain browser tab nobody emits these events, so every subscriber here is
// simply inert and the tab's own unmount handles teardown as before.

/**
 * What the desktop window just did.
 *
 * `background` and `closed` are both "nobody is reading this", and the face
 * treats them identically for its channels — but they are not the same thing to
 * presence, so the shell keeps them apart on the way in. Backgrounding is ambient
 * (covered, hidden, miniaturized); closing is a decision, and reads as away at
 * once rather than after five minutes of inferring it from silence.
 */
export type LifecyclePhase = "foreground" | "background" | "closed";

/** The DOM event the native shell dispatches for a lifecycle transition. */
const LIFECYCLE_EVENT = "hi:lifecycle";

/**
 * Subscribe to native foreground/background transitions. Returns an unsubscribe
 * function (drop it in a React effect cleanup). No-op in a browser tab, where the
 * event is never dispatched.
 */
export function onNativeLifecycle(
  handler: (phase: LifecyclePhase) => void,
): () => void {
  const listener = (e: Event) => {
    const phase = (e as CustomEvent<{ phase?: LifecyclePhase }>).detail?.phase;
    if (phase === "foreground" || phase === "background" || phase === "closed") {
      handler(phase);
    }
  };
  window.addEventListener(LIFECYCLE_EVENT, listener);
  return () => window.removeEventListener(LIFECYCLE_EVENT, listener);
}
