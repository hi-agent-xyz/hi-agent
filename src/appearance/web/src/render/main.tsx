// The headless render page — one agent view, mounted standalone, for a screenshot.
//
// This exists because a compiled view is NOT a self-contained bundle: it keeps
// `react` / `@hi/ui` / `@hi/core` / `motion/react` as bare specifiers that only
// the host page's import map can resolve. Pointing a browser at the `.mjs`
// therefore renders nothing. This page IS a host page — a second Vite entry in
// the same build, so Rollup puts the shared modules in the very chunks the
// import map names, and the Rust route serves it through the same import-map
// injection `GET /` uses. One map, one React, no second copy to drift.
//
// What it deliberately does not do: mount `SessionProvider`. That opens the mic,
// the camera and every channel long-poll, and registers as a live scene
// subscriber — a review render would look like a person in the room. Instead a
// fabricated session is supplied on the same context object, so a view's
// `useSpeech()` / `usePresence()` return plausible sample state.
//
// Everything that goes wrong is collected on `window.__hiRender` for the driver
// to read over CDP: a blank screenshot with an empty error list means the view
// really did render blank, not that the harness swallowed the failure.
import { Component, useEffect, useState, type ComponentType, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { SessionContext } from "../core/session";
import type { AgentSession } from "../hooks/useAgentSession";
import { ActivityMeter } from "../lib/activityMeter";
import { applyHostChrome } from "../lib/chrome";
import "../ui/global.css";

/** What the driver reads back out of the page. */
export interface RenderReport {
  /** Set once the view has mounted and settled (or definitively failed). */
  ready: boolean;
  /** True if the module failed to load or the component threw. */
  failed: boolean;
  /** Console errors, uncaught exceptions, rejections, failed loads. */
  errors: string[];
}

declare global {
  interface Window {
    __hiRender: RenderReport;
  }
}

const report: RenderReport = { ready: false, failed: false, errors: [] };
window.__hiRender = report;

function note(what: unknown): void {
  const text =
    what instanceof Error
      ? `${what.name}: ${what.message}`
      : typeof what === "string"
        ? what
        : (() => {
            try {
              return JSON.stringify(what);
            } catch {
              return String(what);
            }
          })();
  // Cap the list: a view stuck in a render loop can log without bound, and the
  // driver only needs enough to name the fault.
  if (report.errors.length < 50 && text.trim()) report.errors.push(text);
}

// Page-side collectors. The driver ALSO watches CDP's Runtime/Log domains — these
// two channels overlap on purpose. CDP sees things the page cannot (a subresource
// 404 with its status code); the page sees things CDP reports only as an opaque
// rejection (the exact `Failed to resolve module specifier "@hi/ui"` text from a
// dynamic import, which is the single most common real failure).
window.addEventListener("error", (e) => note(e.error ?? e.message));
window.addEventListener("unhandledrejection", (e) => note(e.reason));
const consoleError = console.error.bind(console);
console.error = (...args: unknown[]) => {
  note(args.map((a) => (a instanceof Error ? a.message : String(a))).join(" "));
  consoleError(...args);
};

const params = new URLSearchParams(window.location.search);
const moduleUrl = params.get("module") ?? "";

const theme = params.get("theme");
const lang = params.get("lang");
// The bundled system views read `<html lang>` to pick which of their copies to show —
// the same attribute the served page carries in the real app.
if (lang) {
  document.documentElement.setAttribute("lang", lang);
}
if (theme === "dark" || theme === "light") {
  document.documentElement.setAttribute("data-theme", theme);
}
// The review render stands in for the desktop window, so it reserves the same
// titlebar strip the window's chrome floats in (the driver asks for it in the
// URL). Without this a view lays out to the full height here and only collides
// with the traffic lights once it's on the person's screen — the exact class of
// defect a review is for.
applyHostChrome(window.location.search);

/** A fabricated session: plausible sample state, no devices, no network. */
const stubSession: AgentSession = {
  state: "speaking",
  reactive: false,
  bus: null,
  activity: new ActivityMeter(),
  sentences: [
    { id: 1, text: "How did spending go last month?", speaker: "user" },
    { id: 2, text: "Groceries crept up; everything else held steady.", speaker: "agent" },
  ],
  woken: true,
  audioInput: false,
  audioError: null,
  videoInput: false,
  videoError: null,
  visionStream: null,
  audioOutput: false,
  textInput: false,
  toggleAudio: () => {},
  toggleVideo: () => {},
  toggleAudioOutput: () => {},
  setTextChannel: () => {},
  sendText: () => {},
};

/** Resolve once the browser has actually painted what React just committed:
 * two frames after the commit, plus webfonts and any images the view pulled in
 * from `/views/`. Screenshotting before this reliably captures a half-painted
 * card and reads as a broken view. */
async function settle(): Promise<void> {
  await new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())));
  try {
    await document.fonts.ready;
  } catch {
    /* no font loading API — nothing to wait for */
  }
  const images = Array.from(document.images).filter((img) => !img.complete);
  await Promise.all(
    images.map(
      (img) =>
        new Promise<void>((r) => {
          const done = () => r();
          img.addEventListener("load", done, { once: true });
          img.addEventListener("error", () => {
            note(`image failed to load: ${img.currentSrc || img.src}`);
            r();
          }, { once: true });
        }),
    ),
  );
  await new Promise<void>((r) => requestAnimationFrame(() => r()));
}

/** Imports the compiled view module and renders its default export inside the
 * same full-bleed layer the real stage uses (`.hi-view-fill`), so the screenshot
 * shows exactly the frame a person would see. There is nothing to negotiate here
 * any more: one frame on the stage means one frame in the review. */
function Preview() {
  const [Comp, setComp] = useState<ComponentType | null>(null);

  useEffect(() => {
    let alive = true;
    if (!moduleUrl) {
      note("no `module` query parameter — nothing to render");
      report.failed = true;
      report.ready = true;
      return;
    }
    // Runtime URL; Vite must not try to analyze it.
    import(/* @vite-ignore */ moduleUrl)
      .then(async (mod: { default?: unknown }) => {
        if (!alive) return;
        if (typeof mod.default !== "function") {
          note(`${moduleUrl} has no default-exported component`);
          report.failed = true;
          report.ready = true;
          return;
        }
        setComp(() => mod.default as ComponentType);
      })
      .catch((err) => {
        note(err);
        report.failed = true;
        report.ready = true;
      });
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    if (!Comp) return;
    let alive = true;
    settle().then(() => {
      if (alive) report.ready = true;
    });
    return () => {
      alive = false;
    };
  }, [Comp]);

  if (!Comp) return null;
  return (
    <div className="hi-view-fill">
      <Comp />
    </div>
  );
}

/** A view that throws during render must be reported as FAILED, not left to hang
 * until the driver's readiness timeout — "it took 10s" and "it crashed" are very
 * different diagnoses. */
class RenderBoundary extends Component<{ children: ReactNode }, { crashed: boolean }> {
  constructor(props: { children: ReactNode }) {
    super(props);
    this.state = { crashed: false };
  }
  static getDerivedStateFromError() {
    return { crashed: true };
  }
  override componentDidCatch(error: Error) {
    note(error);
    report.failed = true;
    report.ready = true;
  }
  override render() {
    return this.state.crashed ? null : this.props.children;
  }
}

const rootEl = document.getElementById("root");
if (!rootEl) {
  note("missing #root mount point");
  report.failed = true;
  report.ready = true;
} else {
  // No StrictMode: this page renders once for a screenshot, and StrictMode's
  // double-invoke would double every logged error.
  createRoot(rootEl).render(
    <SessionContext.Provider value={stubSession}>
      <RenderBoundary>
        <Preview />
      </RenderBoundary>
    </SessionContext.Provider>,
  );
}
