import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

// The frame a view is handed is fixed in both axes — it is the window minus the
// insets, and the window does not grow (docs/arch/stage.md, "The frame is fixed,
// and overflow scrolls"). So the layer has to say where a too-tall view goes, and
// the failure this guards is exactly the one that shipped: it said nothing, the
// only clipping ancestor was `.hi-root { overflow: hidden }`, and everything past
// the bottom edge was cut with no scrollbar and no gesture that could reach it.
//
// Silent in every way that matters — it renders, it screenshots (the review shot is
// viewport-bounded), and it only bites on the person's window at their size. A
// stylesheet test is the only place it can be caught early.

const UI = fileURLToPath(new URL(".", import.meta.url));
const CSS = readFileSync(join(UI, "global.css"), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");

/** The declarations of the first top-level rule whose selector list is exactly `selector`. */
function block(selector: string): string {
  const at = CSS.indexOf(`\n${selector} {`);
  expect(at, `${selector} is declared`).toBeGreaterThan(-1);
  const open = CSS.indexOf("{", at);
  return CSS.slice(open + 1, CSS.indexOf("}", open));
}

describe("the view frame", () => {
  it("scrolls vertically, so no part of a view is unreachable", () => {
    expect(block(".hi-view-fill")).toMatch(/overflow-y:\s*auto\s*;/);
  });

  // A view wider than its frame is a layout that failed. A horizontal scrollbar
  // would present that as a feature, and hide it from the review screenshot too.
  it("clips horizontally rather than scrolling", () => {
    expect(block(".hi-view-fill")).toMatch(/overflow-x:\s*hidden\s*;/);
  });

  // A floor, not a ceiling. The regression this guards is subtle and was real: with
  // the root stretched to a `1fr` grid track instead, a view writing `min-height: 100%`
  // on its own root — which is what they write — pinned the track at one frame, and the
  // root's ground and its transparent bottom border both stopped at the first screen
  // while the content scrolled on past them.
  it("floors the view's root at the frame without capping it", () => {
    expect(block(".hi-view-fill > :only-child")).toMatch(/min-height:\s*100%\s*;/);
    expect(block(".hi-view-fill:has(> :only-child)")).not.toMatch(/grid-template-rows/);
  });

  // The width had no floor at all, on the assumption that a block box fills its parent
  // anyway — which it does right up until the view declares a width, and a view that
  // declares one is building a card: `width: min(1180px, 96vw)` held 1180px of a 1512px
  // window and left the right-hand side of the frame as bare paper. `min-width` is the
  // one property that outranks a declared `width`; the natural fill does not.
  it("floors the root across the width too, over a width the view declared", () => {
    expect(block(".hi-view-fill > :only-child")).toMatch(/min-width:\s*100%\s*;/);
  });

  // The whole reason the layer must carry its own scroll: there is no scrollable
  // ancestor to fall back on. If this ever stops being true, the rule above can be
  // reconsidered — until then, removing it loses the bottom of every tall view.
  it("has no scrollable ancestor to fall back on", () => {
    expect(block(".hi-root")).toMatch(/overflow:\s*hidden\s*;/);
    expect(block("body")).toMatch(/overflow:\s*hidden\s*;/);
  });
});
