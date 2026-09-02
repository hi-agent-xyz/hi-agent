import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { PAGE_MS, inEdge, popsBack } from "./PageEdge";

const UI = fileURLToPath(new URL(".", import.meta.url));
const CSS = readFileSync(join(UI, "global.css"), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");

const PHONE_WIDE = 390;

describe("letting go of a page", () => {
  it("finishes the pop once most of the page has been dragged across", () => {
    expect(popsBack(220, PHONE_WIDE, 0)).toBe(true);
  });

  it("springs back from a short, slow drag", () => {
    // Second-thoughts: a hand that started the gesture and stopped. The page has
    // to come back, or the edge becomes a place you cannot touch without losing
    // what you were reading.
    expect(popsBack(40, PHONE_WIDE, 0.05)).toBe(false);
  });

  it("takes a flick that never travelled far", () => {
    // Distance OR speed. Requiring both is what makes a gesture feel like it has
    // to be performed correctly rather than merely meant.
    expect(popsBack(50, PHONE_WIDE, 1.2)).toBe(true);
  });

  it("does nothing on a page it could not measure", () => {
    // A zero-width box is a page that has not been laid out. Answering "yes" here
    // would dismiss the conversation on the first touch after it opened.
    expect(popsBack(0, 0, 5)).toBe(false);
  });

  it("starts only at the edge", () => {
    expect(inEdge(4, 0)).toBe(true);
    expect(inEdge(120, 0)).toBe(false);
    // And the edge is the page's, not the screen's — a page pushed part-way off
    // by a drag still has its own left edge.
    expect(inEdge(104, 100)).toBe(true);
  });
});

// The gesture's exit animation is played by the stylesheet and timed by
// `PageEdge`: it hands the page to CSS at 100%, waits, and only then tells React
// the surface is gone. Those are two numbers for one duration, in two files, and
// nothing else would notice them drifting — the symptom is a page that snaps back
// into view for a frame before it disappears, which reads as a rendering glitch
// rather than as a constant someone changed.
describe("the page's duration is stated once", () => {
  it("matches every page transition in the stylesheet", () => {
    const durations = [...CSS.matchAll(/transition:\s*transform\s+(\d+)ms/g)].map((m) =>
      Number(m[1]),
    );
    expect(durations.length, "the page rules declare a transform transition").toBeGreaterThan(0);
    for (const ms of durations) expect(ms).toBe(PAGE_MS);
  });

  it("matches the entrance keyframe's own duration", () => {
    const [, ms] = CSS.match(/animation:\s*hi-page-in\s+(\d+)ms/) ?? [];
    expect(Number(ms)).toBe(PAGE_MS);
  });
});
