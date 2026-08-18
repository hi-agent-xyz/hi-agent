import { describe, expect, it } from "vitest";

import { hostHearsKey, keyOrigin, viewHearsKey, type KeyOrigin } from "./keyboard";

/** A target that answers `closest` for the plane it is in, like a real element. */
function inPlane(plane: string | null): EventTarget {
  const el = { closest: (selector: string) => (selector === plane ? {} : null) };
  return el as unknown as EventTarget;
}

const COVER = inPlane(".hi-plane--cover");
const VIEW = inPlane(".hi-plane--view");
const ROOM = inPlane(null);

describe("keyOrigin", () => {
  it("reads the plane the focus is in", () => {
    expect(keyOrigin(COVER)).toBe("cover");
    expect(keyOrigin(VIEW)).toBe("view");
    expect(keyOrigin(ROOM)).toBe("room");
  });

  it("calls the room anything that is not an element", () => {
    // The document and the window are both legal `event.target`s and neither has
    // `closest`; the old code would have thrown on them.
    expect(keyOrigin(null)).toBe("room");
    expect(keyOrigin({} as unknown as EventTarget)).toBe("room");
  });
});

describe("the keyboard follows the planes", () => {
  // The bug this exists for: a deck view binds Space and the arrows on the
  // window, and the person types a message. Every Space they type pages the deck
  // and never lands in the line.
  it("keeps a view out of the line the person is writing on", () => {
    expect(viewHearsKey("cover", false)).toBe(false);
  });

  it("leaves the view its own keys while it holds the focus", () => {
    expect(hostHearsKey("view")).toBe(false);
    expect(viewHearsKey("view", false)).toBe(true);
  });

  // With focus nowhere in particular — the person clicked the deck's background —
  // both hear it. That is what keeps Space paging the deck, and what keeps
  // start-typing-to-open working from a blank stage.
  it("shares the room, host first", () => {
    expect(hostHearsKey("room")).toBe(true);
    expect(viewHearsKey("room", false)).toBe(true);
  });

  // …unless the host took it: a printable key that opened the conversation and
  // seeded the line must not also page the deck behind it.
  it("ends the key where the host claims it", () => {
    expect(viewHearsKey("room", true)).toBe(false);
  });

  it("never lets a claim leak the other way", () => {
    const origins: KeyOrigin[] = ["cover", "view", "room"];
    for (const origin of origins) {
      expect(hostHearsKey(origin)).toBe(origin !== "view");
    }
  });
});
