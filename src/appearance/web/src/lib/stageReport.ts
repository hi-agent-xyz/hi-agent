import { url } from "./base";
// Client for the stage lane — tell the backend the frame this window is showing.
//
// Both consumers are renderers. `hi_review_view` renders a view at whatever we last
// reported, so a view-builder session composes for the frame the person actually
// has and a reviewer signs off on that same frame. Before this, both worked
// against a hardcoded 1280×800 that matched no real window, which is how a
// composition built at one aspect shipped with its cards overlapping at another.
// The views band's history thumbnails render into that frame *and* the skin we
// report, so the picture on a tile is of the screen this window was showing.
//
// **Every face that can show a view reports, under its own id.** The same page runs
// in the desktop window, in a plain browser tab, and in the iPhone client's web view
// (`CoreWebView.swift`), and one `hi_show` lands on all of them — so any of them can
// be the frame a person is actually reading a view in, and the builder that composes
// for one of them has to be told which. The store keeps an entry per surface and a
// review renders the one that reported most recently.
//
// The single exception is the **menu-bar popover** (380×540, portrait). It is a chat
// panel; a review rendered at its frame would be a review of a frame nobody reads a
// view on. It declares itself with `?chrome=popover`, which `applyHostChrome` has
// hoisted onto `<html data-chrome>` before we run — so the flag gates this, with no
// second notion of "which host am I".
//
// Fire-and-forget, like the attention lane: a dropped report just means the next
// review uses the previous frame.

/** CSS pixels, the display's device pixel ratio, the skin, and which face this is. */
interface StageFrame {
  width: number;
  height: number;
  scale: number;
  theme: "light" | "dark";
  surface: string;
}

/** This face's id, stable across reloads of the same tab and distinct from every
 *  other face on the same core. It is deliberately opaque: which *device* this is
 *  is not something the page can honestly know, and the renderer only wants the
 *  frame. `sessionStorage` rather than a module constant so a refresh keeps the
 *  entry it already had instead of minting a second one beside it. */
const SURFACE_KEY = "hi.surface";

function surfaceId(): string {
  try {
    const kept = sessionStorage.getItem(SURFACE_KEY);
    if (kept) return kept;
    const minted = Math.random().toString(36).slice(2, 10);
    sessionStorage.setItem(SURFACE_KEY, minted);
    return minted;
  } catch {
    // Private mode, or storage denied: an id that lasts as long as the page is
    // still better than every face reporting as the same one.
    return Math.random().toString(36).slice(2, 10);
  }
}

/** The skin this window is actually in: a forced `data-theme` when the person has
 *  pinned one, else whatever `prefers-color-scheme` resolves to — the same two-step
 *  `global.css` paints by, read here rather than restated. */
function currentTheme(): "light" | "dark" {
  const forced = document.documentElement.getAttribute("data-theme");
  if (forced === "light" || forced === "dark") return forced;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

/** Coalesce a resize drag into one report. A drag fires `resize` continuously and
 *  every intermediate frame is noise — only where it comes to rest is a frame
 *  anyone will look at. */
const SETTLE_MS = 250;

function currentFrame(): StageFrame {
  return {
    width: window.innerWidth,
    height: window.innerHeight,
    scale: window.devicePixelRatio || 1,
    theme: currentTheme(),
    surface: surfaceId(),
  };
}

async function send(frame: StageFrame): Promise<void> {
  try {
    await fetch(url("/api/stage"), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(frame),
    });
  } catch {
    /* a dropped report is harmless — the next review uses the previous frame */
  }
}

/**
 * Start reporting this face's frame: once now, then whenever a resize settles.
 * No-op in the popover, which is not a surface anyone reads a view on. Returns a
 * teardown.
 */
export function installStageReport(): () => void {
  if (document.documentElement.dataset.chrome === "popover") return () => {};

  let timer: ReturnType<typeof setTimeout> | undefined;
  // Don't re-post a frame we already reported: a `resize` also fires for things
  // that don't change our box (a display switch at the same size), and the
  // report is only interesting when the number moves.
  let last = "";

  const report = () => {
    const frame = currentFrame();
    const key = `${frame.width}x${frame.height}@${frame.scale}/${frame.theme}`;
    if (key === last) return;
    last = key;
    void send(frame);
  };

  const onResize = () => {
    clearTimeout(timer);
    timer = setTimeout(report, SETTLE_MS);
  };

  // The skin can change without the frame moving — the system flipping at dusk, or
  // the person pinning one in Settings — and a thumbnail rendered in the other skin
  // is a wrong picture of what they were looking at. Both routes are watched: the
  // media query for the system's answer, and the attribute for a pinned one.
  const scheme = window.matchMedia?.("(prefers-color-scheme: dark)");
  scheme?.addEventListener("change", report);
  const pinned = new MutationObserver(report);
  pinned.observe(document.documentElement, { attributeFilter: ["data-theme"] });

  report();
  window.addEventListener("resize", onResize);
  return () => {
    clearTimeout(timer);
    window.removeEventListener("resize", onResize);
    scheme?.removeEventListener("change", report);
    pinned.disconnect();
  };
}
