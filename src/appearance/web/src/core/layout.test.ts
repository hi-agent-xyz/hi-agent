import { describe, it, expect } from "vitest";
import { stage, type StageInput } from "./layout";

// The compositor divides the stage between the agent's view and the host's own
// surfaces. It decides geometry only — planes are static and live in the
// stylesheet, so there is nothing here about who covers whom.

const at = (over: Partial<StageInput> = {}): StageInput => ({
  content: false,
  camera: false,
  ownsConversation: false,
  collapsed: false,
  ...over,
});

describe("stage", () => {
  it("nothing on the stage → the conversation is the face", () => {
    const s = stage(at());
    expect(s.conversation).toBe("stage");
    expect(s.demote).toBe(0);
  });

  // The defect this whole design exists for: a view used to collapse the
  // conversation to its newest line, and the only way back was to close the view.
  it("a view on screen → the conversation moves over it, it does not collapse", () => {
    const s = stage(at({ content: true }));
    expect(s.conversation).toBe("popover");
    expect(s.demote).toBe(0.72);
  });

  it("the person puts it away → the pill, and the line being written goes with it", () => {
    // There is no separate answer for the input any more: it is inside the
    // conversation, so putting the conversation away puts the line away too.
    const s = stage(at({ content: true, collapsed: true }));
    expect(s.conversation).toBe("pill");
  });

  // Reversed on purpose. The pill is timed now, so what is left behind is the room
  // and a line that fades rather than a shelf — and the control that does this is
  // the text channel's, which cannot be the one button in the cluster that goes
  // dead in the state where its channel is the whole face.
  it("putting it away with nothing up leaves the room, not the stage", () => {
    expect(stage(at({ collapsed: true })).conversation).toBe("pill");
  });

  // The rail's width threshold went with the rail: a popover splits no window, so
  // a narrow one gets the whole scrollback rather than only the newest line, and
  // the pass has no `width` input left to answer with.
  it("the presentation is the person's toggle alone, at every window size", () => {
    expect(stage(at({ content: true })).conversation).toBe("popover");
    expect(stage(at({ content: true, collapsed: true })).conversation).toBe("pill");
  });

  // The camera is full-bleed as a backdrop, so leaving the conversation on the
  // stage over it would cover the thing the person turned on to see.
  it("the camera backdrop occupies the stage, so the conversation floats over it", () => {
    const s = stage(at({ camera: true }));
    expect(s.conversation).toBe("popover");
    expect(s.camera).toBe("fill");
    expect(s.demote, "the camera is the person's own surface, not content").toBe(0);
  });

  it("the camera shrinks to a pip once a view leads", () => {
    expect(stage(at({ content: true, camera: true })).camera).toBe("pip");
    expect(stage(at({ camera: true })).camera).toBe("fill");
  });

  it("a view that renders the words itself stands the host down", () => {
    // Including the line: a view that owns the conversation owns the writing of
    // it too, and a host line floating over one would be a second place to type.
    expect(stage(at({ content: true, ownsConversation: true })).conversation).toBe("hidden");
  });

  it("a view's claim on the words means nothing once it is gone", () => {
    expect(stage(at({ ownsConversation: true })).conversation).toBe("stage");
  });

  // The claim is the view's, not the person's: standing the host down is not the
  // same as being asked for the room, and it outranks it while the view is up.
  it("a view's claim outranks the person's put-away", () => {
    expect(stage(at({ content: true, ownsConversation: true, collapsed: true })).conversation).toBe(
      "hidden",
    );
  });
});
