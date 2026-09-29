import { describe, expect, it } from "vitest";
import { held, trailOf } from "./trail";
import type { WireHistoryEntry } from "../channels/out/view";

function entry(over: Partial<WireHistoryEntry> & { view_ref: string; at: string }): WireHistoryEntry {
  return {
    id: over.view_ref,
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
      entry({ view_ref: "trip/plan", at: "2026-08-19T09:00:00Z" }),
      entry({ view_ref: "spend/august", at: "2026-08-19T10:00:00Z" }),
      entry({ view_ref: "factory/tasks", at: "2026-08-19T12:00:00Z" }),
    ]);
    expect(trail.map((e) => e.view_ref)).toEqual(["factory/tasks", "spend/august", "trip/plan"]);
  });

  it("leaves the list it was handed alone", () => {
    const history = [entry({ view_ref: "factory/tasks", at: "2026-08-19T11:00:00Z" })];
    trailOf(history);
    expect(history[0]!.view_ref).toBe("factory/tasks");
    expect(history).toHaveLength(1);
  });
});
describe("held", () => {
  const tasks = entry({ view_ref: "factory/tasks", at: "2026-09-29T09:00:00Z" });
  const drive = entry({ view_ref: "factory/drive", at: "2026-09-29T10:00:00Z" });
  const home = entry({ view_ref: "factory/home", at: "2026-09-29T11:00:00Z" });
  const keys = (list: WireHistoryEntry[]) => list.map((e) => e.view_ref);

  it("draws the server's order on the tab's first draw", () => {
    expect(keys(held(null, [home, drive, tasks]))).toEqual([
      "factory/home",
      "factory/drive",
      "factory/tasks",
    ]);
  });

  it("keeps a tapped card where it was while the tab is open", () => {
    const order = ["factory/home", "factory/drive", "factory/tasks"];
    // Tasks was just looked at, so the server now has it first.
    const fresh = { ...tasks, at: "2026-09-29T12:00:00Z" };
    expect(keys(held(order, [fresh, home, drive]))).toEqual(order);
    expect(held(order, [fresh, home, drive])[2]!.at).toBe(fresh.at);
  });

  it("puts a card that was not there when it opened at the head, and drops a trimmed one", () => {
    const order = ["factory/drive", "factory/tasks"];
    const shown = entry({ view_ref: "factory/people", at: "2026-09-29T13:00:00Z" });
    expect(keys(held(order, [shown, drive]))).toEqual(["factory/people", "factory/drive"]);
  });
});
