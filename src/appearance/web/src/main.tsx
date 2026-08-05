import React from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { Inspect } from "./inspect/Inspect";
import { usePath } from "./inspect/router";
import { installAuthGate } from "./lib/authGate";
import { applyHostChrome } from "./lib/chrome";
import { applyLanguage } from "./lib/language";
import "./ui/global.css";

// If the login gate is on, a 401 (session expired) bounces the tab to sign-in.
// No-op when auth is disabled.
installAuthGate();

// Native chrome the page has to keep clear of (the desktop window's titlebar).
// Read before the first render so nothing paints in the strip and then jumps.
applyHostChrome();

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
