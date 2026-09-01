import { describe, expect, it } from "vitest";
import { destinationOf, trailOf } from "./trail";
import type { WireHistoryEntry } from "../channels/out/view";

function entry(over: Partial<WireHistoryEntry> & { at: string }): WireHistoryEntry {
  return {
    id: over.view_ref ?? "inline",
    module_url: "/views/_compiled/abc.mjs",
    label: "Tasks",
    ...over,
  };
}

describe("trailOf", () => {
  it("puts the newest first, because the row scrolls from its start", () => {
    const trail = trailOf([
      entry({ view_ref: "factory/path", label: "Path", at: "2026-08-17T09:00:00Z" }),
      entry({ view_ref: "factory/tasks", label: "Tasks", at: "2026-08-19T11:00:00Z" }),
    ]);
    expect(trail.map((e) => e.label)).toEqual(["Tasks", "Path"]);
  });

  // The server already holds one card per destination, whichever hand put it there, so
  // there is nothing left here to fold together — and nothing of this window's own to
  // fold in. What arrives is the row.
  it("is the server's list and does not dedupe it a second time", () => {
    const trail = trailOf([
      entry({ module_url: "/views/_compiled/one.mjs", at: "2026-08-19T09:00:00Z" }),
      entry({ module_url: "/views/_compiled/two.mjs", at: "2026-08-19T10:00:00Z" }),
      entry({ view_ref: "factory/tasks", at: "2026-08-19T12:00:00Z" }),
    ]);
    expect(trail.map(destinationOf)).toEqual([
      "factory/tasks",
      "/views/_compiled/two.mjs",
      "/views/_compiled/one.mjs",
    ]);
  });

  it("leaves the list it was handed alone", () => {
    const history = [entry({ view_ref: "factory/tasks", at: "2026-08-19T11:00:00Z" })];
    trailOf(history);
    expect(history[0]!.view_ref).toBe("factory/tasks");
    expect(history).toHaveLength(1);
  });
});

describe("destinationOf", () => {
  it("is the ref when there is one and the module when there isn't", () => {
    expect(destinationOf(entry({ view_ref: "factory/tasks", at: "x" }))).toBe("factory/tasks");
    expect(destinationOf(entry({ module_url: "/m/a.mjs", at: "x" }))).toBe("/m/a.mjs");
  });
});
