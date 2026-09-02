// How much of the window the software keyboard is standing on, published as
// `--hi-keyboard` on `<html>`.
//
// **This is the phone page's half of the keyboard.** The other half —
// `lib/keyboard.ts` — is about which plane a *keypress* belongs to and has
// nothing to do with this; what is here is the several hundred pixels of glass
// that stop being the page when someone taps the input line.
//
// It did not matter while the conversation was a panel in a corner: the line sat
// well above the bottom of the window, and a keyboard covering the bottom third
// covered the room, not the line. A page is the whole screen and its line is on
// the last row of it, which is exactly where the keyboard lands — so without this
// the one thing the person just reached for is the one thing they cannot see.
//
// **`dvh` does not answer this on iOS.** The dynamic viewport units track the
// browser's own retracting chrome, not the keyboard: WebKit leaves the layout
// viewport at full height and overlays the keyboard on top of it, so `100dvh` is
// still the whole screen with a keyboard sitting on the bottom of it. The
// `interactive-widget=resizes-content` viewport key says exactly what we want and
// WebKit does not implement it. `visualViewport` is what WebKit does give, and it
// is the only source here that is telling the truth.
//
// `offsetTop` is in the sum because WebKit answers a focused field near the foot
// by sliding the visual viewport up inside the layout viewport rather than by
// scrolling anything — the page did not move, the window looking at it did.
// Subtracting the slide is what keeps the number "pixels of keyboard" rather than
// "pixels of keyboard, sometimes, depending".

/** Where the answer is published. The stylesheet reads it; nothing else should
 * need to. */
const VAR = "--hi-keyboard";

function inset(view: VisualViewport): number {
  return Math.max(0, window.innerHeight - view.height - view.offsetTop);
}

/**
 * Start reporting the keyboard's height. Called once from `main.tsx`.
 *
 * No-op where there is no `visualViewport` — every desktop browser has one and
 * reports 0 forever, which is the right answer there, so this is only the
 * render worker and the tests.
 *
 * **Not yet watched on a device.** It is written against WebKit's documented
 * behaviour and verified in Chromium, where the emulated keyboard is not the
 * real one. Until it has been seen on an iPhone with the conversation open, this
 * is machinery that compiles — see `docs/user-journeys/`.
 */
export function installSoftKeyboard(): void {
  const view = window.visualViewport;
  if (!view) return;
  const write = () =>
    document.documentElement.style.setProperty(VAR, `${Math.round(inset(view))}px`);
  write();
  // Both events: `resize` is the keyboard arriving and leaving, `scroll` is the
  // viewport sliding while it is already up — moving between two fields, or the
  // predictive-text bar appearing above it.
  view.addEventListener("resize", write);
  view.addEventListener("scroll", write);
}
