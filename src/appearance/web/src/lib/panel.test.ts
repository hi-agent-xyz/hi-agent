import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import {
  advance,
  depth,
  initial,
  leftOf,
  measureOf,
  opened,
  pushes,
  reach,
  retreat,
  rolled,
  settle,
  sideways,
  stops,
} from "./panel";
import { PANEL_MS } from "../ui/PanelGesture";

const UI = fileURLToPath(new URL("../ui/", import.meta.url));
const CSS = readFileSync(join(UI, "global.css"), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");

const WIDE = 1512;
const PHONE = 390;
const MEASURE = 420;

describe("the axis", () => {
  it("skips the middle stop on a phone", () => {
    // 390px cannot be split into two usable columns — the same threshold the rail
    // died of. A phone that offered `panel` would offer a 420px panel on a 390px
    // screen, which is `full` with an off-by-one.
    expect(stops("phone")).toEqual(["room", "full"]);
  });

  it("stops a television short of covering itself", () => {
    expect(stops("tv")).toEqual(["room", "panel"]);
  });

  it("runs out rather than wrapping", () => {
    // The failure this guards: an axis that wraps turns "one more step toward the
    // panel" at the last stop into "back to the room", so a person pressing the
    // same edge twice loses what they opened.
    expect(advance("wide", "full")).toBe("full");
    expect(retreat("wide", "room")).toBe("room");
    expect(advance("phone", "room")).toBe("full");
    expect(retreat("phone", "full")).toBe("room");
  });

  it("opens to the first stop that is not the room", () => {
    expect(opened("wide")).toBe("panel");
    expect(opened("phone")).toBe("full");
    expect(opened("tv")).toBe("panel");
  });

  it("starts open only where there is room for both", () => {
    // A wide window boots showing the panel because the corner that used to
    // advertise the way in is empty now.
    expect(initial("wide")).toBe("panel");
    expect(initial("phone")).toBe("room");
    expect(initial("tv")).toBe("room");
  });

  it("counts one Back rung per stop behind it", () => {
    expect(depth("wide", "room")).toBe(0);
    expect(depth("wide", "panel")).toBe(1);
    expect(depth("wide", "full")).toBe(2);
  });

  it("pushes the view only at the middle stop", () => {
    // `full` covers rather than insets, so a view asked to reflow to zero width
    // is a bug, not a stop.
    expect(pushes("panel")).toBe(true);
    expect(pushes("full")).toBe(false);
    expect(pushes("room")).toBe(false);
  });
});

describe("where a stop puts the panel's left edge", () => {
  it("puts the room off the right-hand side and full at zero", () => {
    expect(leftOf("room", WIDE, MEASURE)).toBe(WIDE);
    expect(leftOf("full", WIDE, MEASURE)).toBe(0);
    expect(leftOf("panel", WIDE, MEASURE)).toBe(WIDE - MEASURE);
  });

  it("never puts the edge off the left of the window", () => {
    // A panel measured wider than the window it is in — the menu-bar popover, a
    // dragged-narrow window — is `full`, not a negative offset that tears a gap
    // open on the right.
    expect(leftOf("panel", 380, MEASURE)).toBe(0);
  });
});

describe("how wide the box behind that edge is", () => {
  it("has two measures and no third", () => {
    // The whole of the guarantee: whatever a drag is doing, the panel is being laid
    // out either at the sidebar's measure or at the screen's. A width that tracked
    // the finger is what re-wrapped every line of the conversation on the way in.
    expect(measureOf("panel", "wide", WIDE, MEASURE)).toBe(MEASURE);
    expect(measureOf("full", "wide", WIDE, MEASURE)).toBe(WIDE);
  });

  it("lets the room borrow the measure of the stop it opens to", () => {
    // So pulling the panel in changes no width at all — which is the case the
    // reflow was worst in, the panel arriving from nothing.
    expect(measureOf("room", "wide", WIDE, MEASURE)).toBe(MEASURE);
    expect(measureOf("room", "phone", PHONE, MEASURE)).toBe(PHONE);
    expect(measureOf("room", "tv", WIDE, MEASURE)).toBe(MEASURE);
  });

  it("agrees with where the edge is, at every stop the panel is out at", () => {
    // The two functions are one arrangement said twice, and a drag is the only
    // thing allowed to pull them apart. If this ever fails at rest, the panel is
    // drawn either short of the right-hand edge or past it.
    for (const width of [WIDE, PHONE, 380]) {
      for (const stop of ["panel", "full"] as const) {
        expect(leftOf(stop, width, MEASURE) + measureOf(stop, "wide", width, MEASURE)).toBe(width);
      }
    }
  });

  it("is never wider than the window it is in", () => {
    expect(measureOf("panel", "wide", 380, MEASURE)).toBe(380);
  });
});

describe("letting go", () => {
  it("takes the stop the edge is nearest", () => {
    expect(settle(WIDE - 30, 0, "room", "wide", WIDE, MEASURE)).toBe("room");
    expect(settle(WIDE - MEASURE + 10, 0, "room", "wide", WIDE, MEASURE)).toBe("panel");
    expect(settle(40, 0, "panel", "wide", WIDE, MEASURE)).toBe("full");
  });

  it("takes a flick that never travelled far, one step in its direction", () => {
    // Distance OR speed. And one throw is one step: thrown leftward out of the
    // room on a window the panel stops at its measure, rather than carrying past
    // it to cover the very thing it was opened next to.
    expect(settle(WIDE - 50, -1.2, "room", "wide", WIDE, MEASURE)).toBe("panel");
    expect(settle(60, 1.2, "full", "wide", WIDE, MEASURE)).toBe("panel");
  });

  it("springs back from a short, slow drag", () => {
    // Second thoughts: a hand that started the gesture and stopped. Nothing moves,
    // or the edge becomes a place you cannot touch without losing what you had.
    expect(settle(WIDE - 12, -0.05, "room", "wide", WIDE, MEASURE)).toBe("room");
  });

  it("cannot land on a stop this shape does not have", () => {
    // The middle position is where a phone's finger passes through on its way
    // across, and it is not a place to stop.
    expect(settle(PHONE - MEASURE, 0, "room", "phone", PHONE, MEASURE)).toBe("full");
  });

  it("does nothing on a window it could not measure", () => {
    expect(settle(0, -5, "room", "wide", 0, MEASURE)).toBe("room");
  });
});

describe("two fingers sideways", () => {
  it("reads a delta in whatever unit the browser reported it in", () => {
    // A wheel event is free to arrive in lines or in pages, and a threshold read
    // against the raw number would take a three-line roll on a mouse for a swipe
    // and never fire at all on a trackpad reporting pages.
    expect(rolled(60, 0)).toBe(60);
    expect(rolled(3, 1)).toBe(48);
    expect(rolled(1, 2)).toBe(100);
  });

  it("does not take a vertical scroll that drifted", () => {
    // The failure this guards is the one that would make the face unusable: a
    // person reading down a long conversation, whose two fingers are never
    // perfectly vertical, watching the panel close itself under them.
    expect(sideways(4, 90)).toBe(false);
    expect(sideways(30, 20)).toBe(false);
    expect(sideways(90, 4)).toBe(true);
    expect(sideways(-90, 4)).toBe(true);
    expect(sideways(0, 0)).toBe(false);
  });

  it("stops one run at the neighbouring stop, however hard it was thrown", () => {
    // Momentum keeps delivering frames after the fingers have lifted and there is
    // no honest way to tell those from the ones the hand drove — so the edge is
    // stopped where the person could have meant to stop it. Out of the room on a
    // window that is beside the view, never over it.
    expect(reach("room", "wide", WIDE, MEASURE)).toEqual([WIDE - MEASURE, WIDE]);
    expect(reach("full", "wide", WIDE, MEASURE)).toEqual([0, WIDE - MEASURE]);
  });

  it("leaves the middle stop the whole window to move in", () => {
    // It has a neighbour in both directions, so one run can reach either — and
    // still no further, because there is nothing further.
    expect(reach("panel", "wide", WIDE, MEASURE)).toEqual([0, WIDE]);
  });

  it("gives a phone the one step it has", () => {
    expect(reach("room", "phone", PHONE, MEASURE)).toEqual([0, PHONE]);
  });

  it("cannot be pushed off the end of the axis", () => {
    // A television has no `full`, so a run out of its middle stop can only come
    // back to the room.
    expect(reach("panel", "tv", WIDE, MEASURE)).toEqual([WIDE - MEASURE, WIDE]);
  });

});

// The strip that moves the panel stands on the panel's own left edge — the window's
// right-hand side at `room`, the seam at `panel`, the window's left-hand side at
// `full` — and that is one rule in the stylesheet rather than a position per stop.
// Nailing it back to the window's edges is the regression this guards: it looks
// right at two stops out of three, and the one it is wrong at is the desktop's
// resting arrangement.
describe("the edge the gesture begins on", () => {
  it("rides the same variable the panel does", () => {
    // Anchored, so it is the strip's own rule and not the drag's `transition: none`.
    const rule = CSS.match(/^\.hi-panel-edge \{[^}]*\}/m)?.[0] ?? "";
    expect(rule, "the strip declares its position").toMatch(/left:/);
    expect(rule).toContain("--hi-panel-left");
  });
});

// The settle's animation is played by the stylesheet and timed by `PanelGesture`: it
// hands the panel to CSS at the target position, waits, and only then tells React
// the stop changed. Those are two numbers for one duration, in two files, and
// nothing else would notice them drifting — the symptom is a panel that snaps back
// for a frame before it settles, which reads as a rendering glitch rather than as
// a constant someone changed.
describe("the settle's duration is stated once", () => {
  it("matches every panel transition in the stylesheet", () => {
    const durations = [...CSS.matchAll(/transition:[^;]*?(\d+)ms[^;]*;/g)]
      .filter((m) => /--hi-panel-left|\bleft\b|\bright\b/.test(m[0]!))
      .map((m) => Number(m[1]));
    expect(durations.length, "the panel rules declare a transition").toBeGreaterThan(0);
    for (const ms of durations) expect(ms).toBe(PANEL_MS);
  });
});
