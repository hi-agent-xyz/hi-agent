// The remote's Back button, which is the television's only way back.
//
// A phone has an edge to swipe and a desktop has Escape. A remote has one button
// for the whole idea, and it is the app's, not the page's: Android never delivers
// `KEYCODE_BACK` to a WebView, and the shell — which must answer it synchronously,
// there and then, either consuming the press or letting the system close the
// activity — cannot stop to ask a page a question.
//
// So it is settled by two one-way messages, and each is an existing shape rather
// than a new one:
//
//   face → shell   `hiAgentShell.backDepth(n)`, on the document-start bridge that
//                  already carries `unauthorized()` (`web/CoreWebView.kt`)
//   shell → face   a `hi:back` event, as the desktop shell already dispatches
//                  `hi:lifecycle` (`lib/nativeBridge.ts`)
//
// The shell keeps the last depth it was told, so when Back is pressed it already
// knows whether the face has anything to close. Nothing is awaited and nothing can
// be out of date by more than one render.
//
// Inert everywhere else: no bridge object in a browser tab, and nobody dispatching
// the event.

const BACK_EVENT = "hi:back";

interface ShellBridge {
  backDepth?: (depth: number) => void;
}

function bridge(): ShellBridge | undefined {
  return (window as unknown as { hiAgentShell?: ShellBridge }).hiAgentShell;
}

/**
 * Tell the shell how many things the face would close before it should treat Back
 * as "leave".
 *
 * A count rather than a list, because the shell has no business knowing what the
 * face has open — only whether pressing Back is the face's press or its own. What
 * gets closed is decided here, by whoever handles `hi:back`.
 */
export function reportBackDepth(depth: number): void {
  bridge()?.backDepth?.(depth);
}

/** Handle a Back the shell decided belongs to the face. */
export function onShellBack(handler: () => void): () => void {
  const listener = () => handler();
  window.addEventListener(BACK_EVENT, listener);
  return () => window.removeEventListener(BACK_EVENT, listener);
}
