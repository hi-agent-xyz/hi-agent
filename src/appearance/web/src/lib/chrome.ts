// Which host is showing this page, as far as layout has to care.
//
// The face runs in three places with the same markup: a plain browser tab, the
// menu-bar popover, and the desktop window. Only the window spans a native
// titlebar — its content view runs under the strip so the paper paints it
// (macos_window.rs), which leaves the traffic lights and the centred title
// floating over the top of the page. That strip is background-only space: the
// page may paint through it, but nothing readable may sit in it.
//
// The page can't detect that on its own, so the window declares it in the URL it
// loads (`/?chrome=titlebar`). Here we hoist the flag onto `<html>`, where
// global.css turns it into `--hi-chrome-top` and every occupant of the stage
// pads by it. The default (no flag) is a page with no chrome at all, so a browser
// tab and the popover lose no space.
export function applyHostChrome(search: string = window.location.search): void {
  if (new URLSearchParams(search).get("chrome") !== "titlebar") return;
  document.documentElement.setAttribute("data-chrome", "titlebar");
}
