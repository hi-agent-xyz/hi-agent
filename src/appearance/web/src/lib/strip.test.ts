import { describe, expect, it } from "vitest";
import { scrollToShow } from "./strip";

/** The band's own numbers: a 118px card with a 9px gap, in a strip about 690px wide. */
const strip = (scrollLeft: number) => ({ scrollLeft, clientWidth: 690 });
const card = (index: number) => ({ offsetLeft: index * 127, offsetWidth: 118 });

describe("scrollToShow", () => {
  it("leaves a card that is already whole on screen alone", () => {
    expect(scrollToShow(strip(0), card(0))).toBeNull();
    expect(scrollToShow(strip(0), card(4))).toBeNull();
  });

  it("centres one that is off the far end", () => {
    const at = scrollToShow(strip(0), card(9))!;
    expect(at).not.toBeNull();
    // The card sits in the middle of the visible window, not against its edge.
    expect(card(9).offsetLeft - at).toBeCloseTo((690 - 118) / 2);
  });

  it("comes back for one scrolled off the near end", () => {
    expect(scrollToShow(strip(900), card(1))).toBeLessThan(900);
  });

  it("never asks for a negative offset", () => {
    expect(scrollToShow(strip(400), card(0))).toBe(0);
  });

  it("counts a card cut off at the right edge as not on screen", () => {
    // Its left is inside the window, its right is 8px past it.
    const partly = { offsetLeft: 580, offsetWidth: 118 };
    expect(scrollToShow(strip(0), partly)).not.toBeNull();
  });
});
