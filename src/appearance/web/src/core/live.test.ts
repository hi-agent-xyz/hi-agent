import { describe, expect, it } from "vitest";

import { TEMPO, clockFor, shouldRead, type ReadState } from "./live";

const idle: ReadState = { hidden: false, inFlight: false, held: false };

describe("shouldRead", () => {
  it("always takes the mount read, so a view has something to draw", () => {
    // The failure: a view mounted while its window is in the background skips its first
    // read and is brought forward showing a skeleton.
    expect(shouldRead("mount", { ...idle, hidden: true })).toBe(true);
    expect(shouldRead("mount", { ...idle, held: true })).toBe(true);
  });

  it("never overlaps two reads, mount included", () => {
    // Two reads in flight can land out of order, and the older one wins.
    expect(shouldRead("mount", { ...idle, inFlight: true })).toBe(false);
    expect(shouldRead("tick", { ...idle, inFlight: true })).toBe(false);
    expect(shouldRead("attention", { ...idle, inFlight: true })).toBe(false);
  });

  it("does not tick against a hidden page", () => {
    expect(shouldRead("tick", { ...idle, hidden: true })).toBe(false);
  });

  it("honours the view's hold on a tick", () => {
    // This is what stops a tick flipping a card back under the click that changed it.
    expect(shouldRead("tick", { ...idle, held: true })).toBe(false);
    expect(shouldRead("attention", { ...idle, held: true })).toBe(false);
  });

  it("reads on returning attention, which is the whole point of the listener", () => {
    // Guarding ticks on `hidden` without this leaves a backgrounded surface showing a
    // stale reading for a full period after the person comes back to it.
    expect(shouldRead("attention", idle)).toBe(true);
  });

  it("reads on an ordinary tick", () => {
    expect(shouldRead("tick", idle)).toBe(true);
  });
});

describe("clockFor", () => {
  it("gives each named tempo its interval", () => {
    expect(clockFor(TEMPO.watching, false)).toBe(2000);
    expect(clockFor(TEMPO.ledger, false)).toBe(8000);
  });

  it("runs no clock for the attention tempo", () => {
    expect(clockFor(TEMPO.onAttention, false)).toBe(null);
  });

  it("runs no clock on the review render page", () => {
    // A review is one capture. Ticking through it re-renders the view under the camera.
    expect(clockFor(TEMPO.watching, true)).toBe(null);
    expect(clockFor(TEMPO.ledger, true)).toBe(null);
  });

  it("treats a nonsense period as no clock rather than as a busy loop", () => {
    expect(clockFor(0, false)).toBe(null);
    expect(clockFor(-1, false)).toBe(null);
    expect(clockFor(Number.NaN, false)).toBe(null);
    expect(clockFor(Number.POSITIVE_INFINITY, false)).toBe(null);
  });
});

describe("TEMPO", () => {
  it("keeps the two tempos the tree already arrived at twice", () => {
    // The workers roster found 2s and the task board found 8s independently. Naming them
    // is what stops a third number being invented for the next surface.
    expect(TEMPO.watching).toBe(2000);
    expect(TEMPO.ledger).toBe(8000);
  });
});
