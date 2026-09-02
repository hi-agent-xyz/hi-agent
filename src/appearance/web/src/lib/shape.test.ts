import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { PHONE } from "./shape";

const SRC = fileURLToPath(new URL("..", import.meta.url));
const CSS = readFileSync(join(SRC, "ui/global.css"), "utf8");

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
