import { describe, expect, it } from "vitest";
import { combineStatus, type ActivityState } from "./Presence";

describe("combineStatus", () => {
  it.each<ActivityState>(["waking", "listening", "thinking", "typing", "speaking"])(
    "keeps active %s activity ahead of availability",
    (activity) => {
      expect(combineStatus(activity, "out_of_energy")).toBe(activity);
    },
  );

  it("shows out of energy while idle", () => {
    expect(combineStatus("idle", "out_of_energy")).toBe("out_of_energy");
  });

  it("shows rest while idle and available", () => {
    expect(combineStatus("idle", "available")).toBe("rest");
  });
});
