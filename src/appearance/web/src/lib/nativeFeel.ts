// The face is an app surface, not a document.
//
// WebKit hands every page two affordances that read as wrong inside a window
// with no address bar. A double-click selects the word under the pointer, so a
// stray double-tap leaves a phrase in a view highlighted like a search hit. A
// right-click opens the browser's page menu — Reload, Back, Print — none of
// which a native app offers over its own content. Both are cancelled here, once
// at the document, so every view inherits it without asking.
//
// What stays is the part a person actually uses: dragging still selects, so any
// text on screen is still copyable. And a real text field keeps BOTH — word
// select and the Cut/Copy/Paste menu are the native behaviour there, so
// editable targets are exempt rather than special-cased away.
//
// Both listeners capture, because the views are agent-authored: one that stops
// propagation on its own mousedown must not be able to take the page's feel
// with it. Cancelling the default in the capture phase still leaves a view's own
// handler free to run (and to draw its own menu, if it ever wants one).
export function installNativeFeel(): void {
  // `detail` counts the clicks in the streak — 2 is the double-click, 3 the
  // triple. Cancelling the default on THAT mousedown is what drops the word /
  // paragraph selection; the opening click, and the drag it can start, never
  // reach this branch.
  document.addEventListener(
    "mousedown",
    (e) => {
      if (e.detail > 1 && !isEditable(e.target)) e.preventDefault();
    },
    { capture: true },
  );

  document.addEventListener(
    "contextmenu",
    (e) => {
      if (!isEditable(e.target)) e.preventDefault();
    },
    { capture: true },
  );
}

// A target the person types into: a text field, or anything inside a
// contenteditable subtree.
function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable;
}
