import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LONG_PRESS_MS, LongPress, placeMenu } from "./longPress";

const touch = (clientX: number, clientY: number) => ({ pointerType: "touch", clientX, clientY });

describe("a press held still", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("opens where the finger is, and swallows the click it lifts into", () => {
    const fire = vi.fn();
    const press = new LongPress(fire);
    press.down(touch(40, 60));
    vi.advanceTimersByTime(LONG_PRESS_MS - 1);
    expect(fire).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(fire).toHaveBeenCalledWith(40, 60);

    press.up();
    expect(press.takeClick()).toBe(true);
    expect(press.takeClick()).toBe(false);
  });

  it("is a tap when it lifts early, and the tap still opens the card", () => {
    const fire = vi.fn();
    const press = new LongPress(fire);
    press.down(touch(40, 60));
    vi.advanceTimersByTime(200);
    press.up();
    vi.advanceTimersByTime(LONG_PRESS_MS);
    expect(fire).not.toHaveBeenCalled();
    expect(press.takeClick()).toBe(false);
  });

  // A finger scrolling the row goes down on a card first; drifting past the slop is a
  // scroll, not a press.
  it("is a scroll once the finger has moved", () => {
    const fire = vi.fn();
    const press = new LongPress(fire);
    press.down(touch(40, 60));
    press.move(touch(43, 63));
    press.move(touch(40, 75));
    vi.advanceTimersByTime(LONG_PRESS_MS);
    expect(fire).not.toHaveBeenCalled();
  });

  it("leaves a mouse to its right button", () => {
    const fire = vi.fn();
    const press = new LongPress(fire);
    press.down({ pointerType: "mouse", clientX: 1, clientY: 1 });
    vi.advanceTimersByTime(LONG_PRESS_MS * 2);
    expect(fire).not.toHaveBeenCalled();
  });
});

describe("where a menu goes", () => {
  it("opens below and to the right of the point when there is room", () => {
    expect(placeMenu(100, 100, 160, 120, 800, 600)).toEqual({ left: 100, top: 100 });
  });

  it("flips to the other side of the point at the right and bottom edges", () => {
    expect(placeMenu(780, 590, 160, 120, 800, 600)).toEqual({ left: 620, top: 470 });
  });

  it("never runs off a screen too small to flip on", () => {
    expect(placeMenu(10, 10, 300, 200, 200, 150)).toEqual({ left: 8, top: 8 });
  });
});
