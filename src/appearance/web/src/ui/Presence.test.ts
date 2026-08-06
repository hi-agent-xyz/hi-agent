import { describe, expect, it } from "vitest";
import { combineStatus, type ActivityState } from "./Presence";

describe("combineStatus", () => {
  it.each<ActivityState>(["waking", "listening", "thinking", "typing", "speaking"])(
    "keeps active %s activity",
    (activity) => {
      expect(combineStatus(activity)).toBe(activity);
    },
  );

  it("shows rest while idle", () => {
    expect(combineStatus("idle")).toBe("rest");
  });
});
