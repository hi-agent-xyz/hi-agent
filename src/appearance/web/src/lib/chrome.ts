// Which host is showing this page, as far as layout has to care.
//
// The face runs in two places with the same markup: a plain browser tab and the
// desktop window. Only the window spans a native
// titlebar — its content view runs under the strip so the paper paints it
// (macos_window.rs), which leaves the traffic lights and the centred title
// floating over the top of the page. That strip is background-only space: the
// page may paint through it, but nothing readable may sit in it.
//
// The page can't detect that on its own, so each host declares itself in the URL it
// loads. The window asks for `/?chrome=titlebar`; here we hoist the flag onto
// `<html>`, where global.css turns it into `--hi-chrome-top` and every occupant of
// the stage pads by it.
//
// The default (no flag) is a full-page face with no chrome at all: a browser tab,
// or the iPhone client's web view.
const HOSTS = ["titlebar"];

export function applyHostChrome(search: string = window.location.search): void {
  const chrome = new URLSearchParams(search).get("chrome");
  if (!chrome || !HOSTS.includes(chrome)) return;
  document.documentElement.setAttribute("data-chrome", chrome);
}
