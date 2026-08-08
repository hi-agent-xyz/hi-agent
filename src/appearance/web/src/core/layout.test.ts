import { describe, it, expect } from "vitest";
import { floorLayout, CAPTIONS_ID, CAMERA_ID, type Participant } from "./layout";

// The compositor arranges the host's own surfaces — the words and the camera —
// around whatever is on screen. It does not place views: those fill the frame,
// one at a time, so the only thing a view contributes here is whether it renders
// the words itself.

const captions = (): Participant => ({ id: CAPTIONS_ID, kind: "captions" });
const camera = (): Participant => ({ id: CAMERA_ID, kind: "camera" });
const view = (id: string, ownsCaptions?: boolean): Participant => ({
  id,
  kind: "view",
  ownsCaptions,
});

describe("floorLayout", () => {
  it("no views, camera off → captions centered, presence undimmed", () => {
    const { demote, placements } = floorLayout([captions()]);
    expect(demote).toBe(0);
    const cap = placements.get(CAPTIONS_ID)!;
    expect(cap.region).toBe("center");
    expect(cap.docked).toBe(false);
    expect(cap.hidden).toBe(false);
  });

  it("no views, camera on → camera fills, captions dock bottom-center", () => {
    const { demote, placements } = floorLayout([captions(), camera()]);
    expect(demote).toBe(0);
    const cam = placements.get(CAMERA_ID)!;
    expect(cam.region).toBe("fill");
    expect(cam.pip).toBe(false);
    const cap = placements.get(CAPTIONS_ID)!;
    expect(cap.region).toBe("bottom");
    expect(cap.docked).toBe(true);
  });

  it("a view on screen → captions dock bottom, demote 0.72, camera→pip", () => {
    const { demote, placements } = floorLayout([view("v1"), captions(), camera()]);
    expect(demote).toBe(0.72);
    const cap = placements.get(CAPTIONS_ID)!;
    expect(cap.region).toBe("bottom");
    expect(cap.docked).toBe(true);
    expect(cap.hidden).toBe(false);
    const cam = placements.get(CAMERA_ID)!;
    expect(cam.pip).toBe(true);
    expect(cam.region).toBe("bottom_left");
  });

  it("views are not placed — the compositor only arranges the host surfaces", () => {
    const { placements } = floorLayout([view("v1"), captions()]);
    expect(placements.has("v1")).toBe(false);
    expect([...placements.keys()]).toEqual([CAPTIONS_ID]);
  });

  it("a view that owns the captions makes the host's pills stand down", () => {
    const { placements } = floorLayout([view("v1", true), captions()]);
    expect(placements.get(CAPTIONS_ID)!.hidden).toBe(true);
  });

  // The condition layer (a vendor outage) is the only thing that ever sits over
  // the content, and it renders the words itself. Whoever is on top decides.
  it("the top-most layer decides who owns the captions", () => {
    const overContent = floorLayout([view("content"), view("vendor-outage", true), captions()]);
    expect(overContent.placements.get(CAPTIONS_ID)!.hidden).toBe(true);

    // ...and the reverse: a caption-owning view underneath does not hide them.
    const underContent = floorLayout([view("vendor-outage", true), view("content"), captions()]);
    expect(underContent.placements.get(CAPTIONS_ID)!.hidden).toBe(false);
  });
});
