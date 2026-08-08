// Client for the stage lane — tell the backend the frame this window is showing.
//
// The one consumer is `review_view`: it renders a view at whatever we last
// reported, so a view-builder session composes for the frame the person actually
// has and a reviewer signs off on that same frame. Before this, both worked
// against a hardcoded 1280×800 that matched no real window, which is how a
// composition built at one aspect shipped with its cards overlapping at another.
//
// **Only the desktop window reports.** The same page also runs in the menu-bar
// popover (380×540, portrait) and in a plain browser tab; a review rendered at
// either would be a review of a frame nobody composes for. The window already
// announces itself with `?chrome=titlebar` to claim its titlebar strip, and
// `applyHostChrome` has hoisted that onto `<html data-chrome>` before we run —
// so the same flag gates this, with no second notion of "which host am I".
//
// Fire-and-forget, like the attention lane: a dropped report just means the next
// review uses the previous frame.

/** CSS pixels, plus the display's device pixel ratio. */
interface StageFrame {
  width: number;
  height: number;
  scale: number;
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
  };
}

async function send(frame: StageFrame): Promise<void> {
  try {
    await fetch("/api/stage", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(frame),
    });
  } catch {
    /* a dropped report is harmless — the next review uses the previous frame */
  }
}

/**
 * Start reporting this window's frame: once now, then whenever a resize settles.
 * No-op unless we are the desktop window. Returns a teardown.
 */
export function installStageReport(): () => void {
  if (document.documentElement.dataset.chrome !== "titlebar") return () => {};

  let timer: ReturnType<typeof setTimeout> | undefined;
  // Don't re-post a frame we already reported: a `resize` also fires for things
  // that don't change our box (a display switch at the same size), and the
  // report is only interesting when the number moves.
  let last = "";

  const report = () => {
    const frame = currentFrame();
    const key = `${frame.width}x${frame.height}@${frame.scale}`;
    if (key === last) return;
    last = key;
    void send(frame);
  };

  const onResize = () => {
    clearTimeout(timer);
    timer = setTimeout(report, SETTLE_MS);
  };

  report();
  window.addEventListener("resize", onResize);
  return () => {
    clearTimeout(timer);
    window.removeEventListener("resize", onResize);
  };
}
