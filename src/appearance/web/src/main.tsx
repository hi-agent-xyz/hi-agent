import React from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { Inspect } from "./inspect/Inspect";
import { usePath } from "./inspect/router";
import { installAuthGate } from "./lib/authGate";
import { installBase, inCore } from "./lib/base";
import { installKeyPlanes } from "./lib/keyboard";
import { applyHostChrome } from "./lib/chrome";
import { applyLanguage } from "./lib/language";
import { installNativeFeel } from "./lib/nativeFeel";
import { installShape } from "./lib/shape";
import { installSpatialNav } from "./lib/spatial";
import { installSoftKeyboard } from "./lib/softKeyboard";
import { installStageReport } from "./lib/stageReport";
import "./ui/tailwind.css";
import "./ui/global.css";

// Where this core is served from. First, and before anything can make a request:
// under the community's subpath a bare `/api/x` is the community's route, not
// ours. No-op at the core's own root.
installBase();

// If the login gate is on, a 401 (session expired) bounces the tab to sign-in.
// No-op when auth is disabled.
installAuthGate();

// Native chrome the page has to keep clear of (the desktop window's titlebar).
// Read before the first render so nothing paints in the strip and then jumps.
applyHostChrome();

// And which shape of screen this is — narrow AND pointed at with a finger, or
// anything else. Read here for the same reason the chrome flag is: the phone
// arrangement is a different geometry for the whole cover plane, and a face that
// painted itself as a window and then re-laid as a stack of pages would be seen
// doing it. It keeps listening after this, because a phone rotates.
installShape();

// And how much of the window the software keyboard is standing on, which the
// conversation page's foot subtracts so the line stays above it. Zero everywhere
// there is no such keyboard.
installSoftKeyboard();

// Tell the backend how big this window is, so `hi_review_view` renders a view at the
// frame the person actually has. Gated on the titlebar flag `applyHostChrome` just
// hoisted, so only the desktop window reports — hence the ordering. Installed here
// rather than in a component effect: it's a fact about the window, not about
// anything React mounts, and StrictMode would double-invoke an effect.
installStageReport();

// The face reads as an app, not a web page: no double-click word-select, no
// right-click page menu. Only the face — the inspect console below is an
// operator's browser tool, where Reload and "open in new tab" are the point.
if (!inCore().startsWith("/inspect")) {
  installNativeFeel();
  // And the keyboard follows the planes: a key typed into the host's own line or
  // controls never reaches an agent view's window listener. Installed here, ahead
  // of the first view import, because the guard can only silence listeners that
  // have not run yet.
  installKeyPlanes();
  // And on a television, the arrows move the focus, because there is no pointer
  // to move instead. After `installKeyPlanes` and through it: the plane rule is
  // what stands this down while an agent view holds the focus.
  installSpatialNav();
}

// `<html lang>` for the bundled views' copy. The server already stamps this when it
// serves the page in prod; this only covers the dev seam, where Vite serves index.html
// and the Rust injection never runs. Fire-and-forget — nothing below waits on it.
void applyLanguage();

const rootEl = document.getElementById("root");
if (!rootEl) {
  throw new Error("missing #root mount point");
}

// One SPA, two surfaces: the agent "face" at `/` and the operator console under
// `/inspect/*`. A tiny path check picks between them; the inspect section owns its
// own nested routing. (AI-credential config lives in the native tray, not the web.)
function Root() {
  const { path } = usePath();
  if (path.startsWith("/inspect")) return <Inspect />;
  return <App />;
}

createRoot(rootEl).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
);
