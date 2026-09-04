import { describe, it, expect } from "vitest";

import { nearest, type Box } from "./spatial";

/** A box from its corner and size, so the cases below read as a layout. */
const at = (left: number, top: number, w = 100, h = 40): Box => ({
  left,
  top,
  right: left + w,
  bottom: top + h,
});

describe("choosing what is in that direction", () => {
  const from = at(200, 200);

  it("finds nothing where there is nothing", () => {
    expect(nearest(from, [at(200, 100)], "down")).toBeNull();
    expect(nearest(from, [], "right")).toBeNull();
  });

  it("ignores what is behind, however close", () => {
    // Directly above and touching, while the only thing below is far away. Down
    // must not turn round.
    expect(nearest(from, [at(200, 158), at(200, 600)], "down")).toBe(1);
  });

  // The rule that makes a D-pad feel deliberate rather than approximate. Cross
  // distance counts double, so a control slightly further down the same column
  // beats a nearer one sitting off to the side — which is what "press down" means.
  it("prefers the same column to a nearer thing off to the side", () => {
    const straightDown = at(200, 400);
    const nearerButSideways = at(700, 300);
    expect(nearest(from, [nearerButSideways, straightDown], "down")).toBe(1);
  });

  // A wide control under a narrow one is under it, whatever their centres say —
  // so overlap on the cross axis costs nothing at all rather than a little.
  it("treats any overlap on the cross axis as none", () => {
    const wideBelow = at(0, 400, 900);
    const narrowBelowButFurther = at(200, 420);
    expect(nearest(from, [wideBelow, narrowBelowButFurther], "down")).toBe(0);
  });

  it("reads left and right off the same rule", () => {
    expect(nearest(from, [at(400, 200), at(0, 200)], "right")).toBe(0);
    expect(nearest(from, [at(400, 200), at(0, 200)], "left")).toBe(1);
  });

  // Leading edge to leading edge: a tall neighbour that starts above the current
  // box and merely extends past its bottom is not "below" it.
  it("does not count a tall neighbour that started above as below", () => {
    const tallAlongside = at(200, 100, 100, 400);
    expect(nearest(from, [tallAlongside], "down")).toBeNull();
  });
});
