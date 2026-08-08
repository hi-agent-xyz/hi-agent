import { describe, expect, it } from "vitest";
import { projectActivityState, statusLabel, type ActivitySignals } from "./Presence";

const idle: ActivitySignals = {
  ready: true,
  listening: false,
  speaking: false,
  reactionBusy: false,
  delegatedBusy: false,
};

describe("projectActivityState", () => {
  it("uses the agreed priority order", () => {
    expect(projectActivityState({ ...idle, delegatedBusy: true })).toBe("working");
    expect(projectActivityState({ ...idle, delegatedBusy: true, reactionBusy: true })).toBe(
      "typing",
    );
    expect(
      projectActivityState({
        ...idle,
        delegatedBusy: true,
        reactionBusy: true,
        speaking: true,
      }),
    ).toBe("speaking");
    expect(
      projectActivityState({
        ...idle,
        delegatedBusy: true,
        reactionBusy: true,
        speaking: true,
        listening: true,
      }),
    ).toBe("listening");
    expect(
      projectActivityState({
        ready: false,
        listening: true,
        speaking: true,
        reactionBusy: true,
        delegatedBusy: true,
      }),
    ).toBe("starting");
  });

  it("is idle when no activity remains", () => {
    expect(projectActivityState(idle)).toBe("idle");
  });

  it("uses user-facing labels directly", () => {
    expect(statusLabel("starting")).toBe("Starting");
    expect(statusLabel("listening")).toBe("Listening");
    expect(statusLabel("speaking")).toBe("Speaking");
    expect(statusLabel("typing")).toBe("Typing");
    expect(statusLabel("working")).toBe("Working");
    expect(statusLabel("idle")).toBe("Idle");
  });
});
