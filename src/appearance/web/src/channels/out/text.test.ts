import { describe, expect, it } from "vitest";
import { parseTextAppearanceState } from "./text";

describe("outbound text appearance state", () => {
  it("accepts a whole current-state snapshot", () => {
    expect(
      parseTextAppearanceState({
        user: "What day is it?",
        agent: { text: "Sunday.", final: true },
        interim: "wait",
      }),
    ).toEqual({
      user: "What day is it?",
      agent: { text: "Sunday.", final: true },
      interim: "wait",
    });
  });

  it("accepts the empty initial appearance", () => {
    expect(parseTextAppearanceState({})).toEqual({});
  });

  it("rejects malformed snapshots", () => {
    expect(parseTextAppearanceState(null)).toBeNull();
    expect(parseTextAppearanceState({ user: 3 })).toBeNull();
    expect(parseTextAppearanceState({ agent: { text: "partial" } })).toBeNull();
    expect(parseTextAppearanceState({ agent: { text: 1, final: false } })).toBeNull();
  });
});
