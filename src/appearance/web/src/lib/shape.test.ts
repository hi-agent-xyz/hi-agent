import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { COARSE, PHONE, VIEW_TARGET, declaredTv, viewZoom } from "./shape";

const SRC = fileURLToPath(new URL("..", import.meta.url));
const CSS = readFileSync(join(SRC, "ui/global.css"), "utf8");
const BUILDER = readFileSync(join(SRC, "../../../identity/workers/view-builder.md"), "utf8");

describe("the phone shape", () => {
  // The bug this whole flag exists to close: a `max-width` block written for the
  // ~380px menu-bar popover shrank the channel discs to 32px, and a phone matched
  // it. Width is half the question. If this ever drops the pointer clause, the
  // popover and the phone are one host again.
  it("is narrow AND coarse, never width alone", () => {
    expect(PHONE).toContain("pointer: coarse");
    expect(PHONE).toContain("max-width");
  });

  // The stylesheet reads the flag, not the query. Two copies of the same media
  // query — one in CSS, one in `matchMedia` — is how a page ends up swiping back
  // on a screen the stylesheet is still drawing as a popover.
  it("is the stylesheet's only way to ask", () => {
    expect(CSS).toContain('[data-shape="phone"]');
    expect(CSS).not.toContain("pointer: coarse");
  });

  // The popover's tightening had to be excluded, or it would still win inside its
  // own media block on a 390px phone.
  it("holds the popover's shrink off the phone", () => {
    expect(CSS).toContain(':root:not([data-shape="phone"]) .hi-channel {');
  });
});

describe("the pointer", () => {
  // The half of the question that is not about width at all. Rules that follow
  // the *input method* — the browser's touch gestures, the 16px floor under which
  // iOS zooms a focused field — hold on a phone turned sideways, where the shape
  // flag has already gone back to `wide`.
  it("asks about the pointer alone, never the width", () => {
    expect(COARSE).toContain("pointer: coarse");
    expect(COARSE).not.toContain("width");
  });

  // Same rule as the shape flag: the stylesheet reads the answer, it does not
  // re-ask the question. Two copies of one media query is two answers waiting to
  // disagree.
  it("is the stylesheet's only way to ask", () => {
    expect(CSS).toContain('[data-pointer="coarse"]');
    expect(CSS).not.toContain("pointer: coarse");
  });

  // The line the whole flag was added for: below 16px, tapping it zooms the face.
  it("puts the text line at the iOS zoom floor", () => {
    expect(CSS).toContain(':root[data-pointer="coarse"] .hi-composer textarea {');
  });
});

describe("the television shape", () => {
  // The one shape that is not measured. A television is wide like a monitor and
  // its WebView's `pointer` is whatever the device felt like reporting, so there
  // is no query that finds it — the host says so in the URL it loads, exactly as
  // the desktop window declares its titlebar. If this ever becomes a media query,
  // a browser full-screened on a TV-shaped panel becomes a television.
  it("is declared by the host, never matched", () => {
    expect(declaredTv("?shape=tv")).toBe(true);
    expect(declaredTv("?shape=tv&chrome=titlebar")).toBe(true);
    expect(declaredTv("")).toBe(false);
    expect(declaredTv("?shape=phone")).toBe(false);
  });

  it("is the stylesheet's only way to ask", () => {
    expect(CSS).toContain('[data-shape="tv"]');
  });

  // Overscan is the one inset nothing reports, so it is written onto the tokens
  // every surface already pads by rather than onto the surfaces themselves. A
  // surface added later inherits the safe area without knowing televisions exist.
  it("holds the whole face clear of the overscan margin at once", () => {
    expect(CSS).toContain(':root[data-shape="tv"] {');
    expect(CSS).toContain("--hi-safe-left: 48px");
    expect(CSS).toContain("--hi-safe-bottom: 27px");
  });

  // There is no pointer on this host, so there is no click that leaves an unwanted
  // ring behind and nothing for `:focus-visible` to protect against. A heuristic
  // that guesses wrong here makes the cursor invisible and the remote look dead.
  it("shows focus unconditionally, not on the browser's guess", () => {
    expect(CSS).toContain(':root[data-shape="tv"] :focus {');
  });
});

describe("the room a view is handed", () => {
  // A window dragged narrow is a choice, and the person can drag it back. A phone's
  // width is the device. Only the second one is worth overriding.
  it("leaves alone any room whose pointer can resize it", () => {
    expect(viewZoom(852, false)).toBeNull();
  });

  // 393px against the target is a factor of 0.31, and no viewing distance rescues
  // that. Excluded by the floor rather than by a rule naming phones — and the same
  // floor is what excludes a slot sharing an 852px phone with the 420px panel.
  it("leaves a room too small to be worth scaling alone", () => {
    expect(viewZoom(393, true)).toBeNull();
    expect(viewZoom(432, true)).toBeNull();
  });

  // Telling a screen that already fits the layout a bigger number would only make
  // its text smaller for nothing.
  it("never scales a room that is already big enough", () => {
    expect(viewZoom(VIEW_TARGET, true)).toBeNull();
    expect(viewZoom(1600, true)).toBeNull();
  });

  // The case this exists for: the same iPhone turned sideways.
  it("hands a phone turned sideways the room a view is composed for", () => {
    expect(viewZoom(852, true)).toBeCloseTo(852 / VIEW_TARGET, 6);
  });

  // A page can report a zero width mid-teardown, and dividing by it would publish
  // a factor that scales the slot to nothing.
  it("refuses a width a page cannot actually have", () => {
    expect(viewZoom(0, true)).toBeNull();
    expect(viewZoom(-1, true)).toBeNull();
    expect(viewZoom(Number.NaN, true)).toBeNull();
  });

  // Same rule as the two flags above: one place decides, the stylesheet reads.
  it("is the stylesheet's only way to ask", () => {
    expect(CSS).toContain("var(--hi-view-zoom)");
    expect(CSS).toContain(".hi-view-fill[data-view-zoom]");
  });

  // The scale is only meaningful to a view that asks its slot, so the slot has to
  // *be* a container — and it has to be the one the builder is told to name. A
  // prompt naming a container the stylesheet does not declare is a rule that
  // silently never fires.
  it("declares the container the builder is told to query", () => {
    expect(CSS).toContain("container-type: inline-size");
    expect(CSS).toContain("container-name: hi-view");
    expect(BUILDER).toContain("@container hi-view");
    expect(BUILDER).toContain("cqi");
  });
});
