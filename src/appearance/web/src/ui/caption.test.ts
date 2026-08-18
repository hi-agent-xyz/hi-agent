import { describe, expect, it } from "vitest";

import { captionDeadline, captionDwell } from "./caption";

describe("captionDwell", () => {
  it("holds a short line long enough to be noticed", () => {
    expect(captionDwell("好")).toBe(4_000);
    expect(captionDwell("")).toBe(4_000);
  });

  it("gives a longer line longer to be read", () => {
    expect(captionDwell("x".repeat(100))).toBeGreaterThan(captionDwell("x".repeat(30)));
  });

  // The pill clamps to three lines, so a wall of text has no more to read than a
  // long one — and a caption that outstays that is just a caption in the way.
  it("caps, however long the line is", () => {
    expect(captionDwell("x".repeat(10_000))).toBe(10_000);
  });
});

describe("captionDeadline", () => {
  // The rule that makes reload, resync and a second window all one case: the
  // clock is the line's, so a line said an hour ago opens already spent.
  it("counts from when the line was said, not from now", () => {
    const said = Date.parse("2026-08-18T09:00:00Z");
    expect(captionDeadline({ ts: "2026-08-18T09:00:00Z", text: "在" })).toBe(said + 4_000);
  });

  it("has nothing to count down without a line", () => {
    expect(captionDeadline(undefined)).toBeNull();
  });

  // A timestamp we cannot read is a bug somewhere upstream; the pill's answer to
  // it is to keep showing the line rather than to hide the words over a parse.
  it("never expires a line whose timestamp is unreadable", () => {
    expect(captionDeadline({ ts: "not a time", text: "在" })).toBeNull();
  });
});
