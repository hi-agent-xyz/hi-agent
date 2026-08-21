import { describe, expect, it } from "vitest";
import { destinationOf, trailOf } from "./trail";
import type { WireHistoryEntry } from "../channels/out/view";

function show(over: Partial<WireHistoryEntry> & { at: string }): WireHistoryEntry {
  return {
    id: over.view_ref ?? "inline",
    module_url: "/views/_compiled/abc.mjs",
    label: "Tasks",
    ...over,
  };
}

describe("trailOf", () => {
  it("puts the newest first, because the row scrolls from its start", () => {
    const trail = trailOf(
      [
        show({ view_ref: "factory/path", label: "Path", at: "2026-08-17T09:00:00Z" }),
        show({ view_ref: "factory/tasks", label: "Tasks", at: "2026-08-19T11:00:00Z" }),
      ],
      [],
    );
    expect(trail.map((e) => e.label)).toEqual(["Tasks", "Path"]);
  });

  it("counts a window's own open as being there, and dates the card by it", () => {
    const trail = trailOf(
      [show({ view_ref: "factory/tasks", at: "2026-08-17T09:00:00Z", shot_url: "/s/a.png" })],
      [show({ view_ref: "factory/tasks", at: "2026-08-19T12:00:00Z" })],
    );
    expect(trail).toHaveLength(1);
    expect(trail[0]!.at).toBe("2026-08-19T12:00:00Z");
    // The visit knows where it went, not what the picture of it was.
    expect(trail[0]!.shot_url).toBe("/s/a.png");
  });

  it("keeps a show that came after the open as the card's time", () => {
    const trail = trailOf(
      [show({ view_ref: "factory/tasks", at: "2026-08-19T12:00:00Z" })],
      [show({ view_ref: "factory/tasks", at: "2026-08-19T09:00:00Z" })],
    );
    expect(trail).toHaveLength(1);
    expect(trail[0]!.at).toBe("2026-08-19T12:00:00Z");
  });

  it("holds two inline views apart and folds two shows of one ref together", () => {
    const trail = trailOf(
      [
        show({ module_url: "/views/_compiled/one.mjs", at: "2026-08-19T09:00:00Z" }),
        show({ module_url: "/views/_compiled/two.mjs", at: "2026-08-19T10:00:00Z" }),
        show({ view_ref: "factory/tasks", at: "2026-08-19T11:00:00Z" }),
        show({ view_ref: "factory/tasks", at: "2026-08-19T12:00:00Z" }),
      ],
      [],
    );
    expect(trail.map(destinationOf)).toEqual([
      "factory/tasks",
      "/views/_compiled/two.mjs",
      "/views/_compiled/one.mjs",
    ]);
  });
});
