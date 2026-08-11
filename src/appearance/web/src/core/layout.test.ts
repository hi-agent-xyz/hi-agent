import { describe, it, expect } from "vitest";
import { stage, RAIL_MIN_WIDTH, type StageInput } from "./layout";

// The compositor divides the stage between the agent's view and the host's own
// surfaces. It decides geometry only — planes are static and live in the
// stylesheet, so there is nothing here about who covers whom.

const WIDE = 1440;
const NARROW = RAIL_MIN_WIDTH - 1;

const at = (over: Partial<StageInput> = {}): StageInput => ({
  content: false,
  camera: false,
  ownsConversation: false,
  width: WIDE,
  collapsed: false,
  ...over,
});

describe("stage", () => {
  it("nothing on the stage → the conversation is the face", () => {
    const s = stage(at());
    expect(s.conversation).toBe("stage");
    expect(s.input).toBe("center");
    expect(s.demote).toBe(0);
    expect(s.rail).toBe(false);
  });

  // The defect this whole design exists for: a view used to collapse the
  // conversation to its newest line, and the only way back was to close the view.
  it("a view on screen → the conversation narrows to the rail, it does not collapse", () => {
    const s = stage(at({ content: true }));
    expect(s.conversation).toBe("rail");
    expect(s.input).toBe("rail");
    expect(s.rail).toBe(true);
    expect(s.demote).toBe(0.72);
  });

  it("the person collapses the rail → the pill, and the input goes back to centre", () => {
    const s = stage(at({ content: true, collapsed: true }));
    expect(s.conversation).toBe("pill");
    expect(s.input).toBe("center");
    expect(s.rail).toBe(false);
  });

  it("collapsing with nothing up is not a way to hide the conversation", () => {
    expect(stage(at({ collapsed: true })).conversation).toBe("stage");
  });

  it("too narrow for two things → the pill, whatever the person asked for", () => {
    expect(stage(at({ content: true, width: NARROW })).conversation).toBe("pill");
    expect(stage(at({ content: true, width: RAIL_MIN_WIDTH })).conversation).toBe("rail");
  });

  it("a narrow window with nothing up still gives the conversation the stage", () => {
    expect(stage(at({ width: NARROW })).conversation).toBe("stage");
  });

  // The camera is full-bleed as a backdrop, so leaving the conversation on the
  // stage over it would cover the thing the person turned on to see.
  it("the camera backdrop occupies the stage, so the conversation takes the rail", () => {
    const s = stage(at({ camera: true }));
    expect(s.conversation).toBe("rail");
    expect(s.camera).toBe("fill");
    expect(s.demote, "the camera is the person's own surface, not content").toBe(0);
  });

  it("the camera shrinks to a pip once a view leads", () => {
    expect(stage(at({ content: true, camera: true })).camera).toBe("pip");
    expect(stage(at({ camera: true })).camera).toBe("fill");
  });

  it("a view that renders the words itself stands the host down", () => {
    const s = stage(at({ content: true, ownsConversation: true }));
    expect(s.conversation).toBe("hidden");
    expect(s.input, "the person can still type").toBe("center");
  });

  it("a view's claim on the words means nothing once it is gone", () => {
    expect(stage(at({ ownsConversation: true })).conversation).toBe("stage");
  });
});
