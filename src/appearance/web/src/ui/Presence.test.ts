import { describe, expect, it } from "vitest";
import { projectActivityState, type ActivitySignals } from "./Presence";

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

  // The one state anything is drawn for, and the one signal it may be drawn from.
  // Everything else on this list resolves to no UI, so a projection that silently
  // widened `typing` would put dots at the foot of the conversation while the
  // agent was off running an errand nobody asked to watch.
  it("says typing for a reply being composed, and for nothing else", () => {
    expect(projectActivityState({ ...idle, reactionBusy: true })).toBe("typing");
    expect(projectActivityState({ ...idle, delegatedBusy: true })).not.toBe("typing");
    expect(projectActivityState({ ...idle, reactionBusy: true, listening: true })).not.toBe(
      "typing",
    );
  });
});
