import { afterEach, describe, expect, it, vi } from "vitest";
import { reportWentTo } from "./view";

/** The one request `reportWentTo` fires, decoded. */
function sent(mock: ReturnType<typeof vi.fn>) {
  const [target, init] = mock.mock.calls[0]!;
  return { target: String(target), body: JSON.parse(String(init.body)) };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("reporting where this window went", () => {
  it("names a destination the server can key on, and says they are not live", () => {
    const fetchMock = vi.fn(() => Promise.resolve(new Response(null, { status: 202 })));
    vi.stubGlobal("fetch", fetchMock);

    reportWentTo({ viewRef: "factory/drive", id: "drive" });

    const { target, body } = sent(fetchMock);
    expect(target).toContain("/api/in/view");
    expect(body.ref).toBe("factory/drive");
    expect(body.live).toBe(false);
  });

  it("falls back to the module for an inline view, and still carries a name", () => {
    const fetchMock = vi.fn(() => Promise.resolve(new Response(null, { status: 202 })));
    vi.stubGlobal("fetch", fetchMock);

    reportWentTo({ moduleUrl: "/views/_compiled/abc.mjs", id: "trip" });

    const { body } = sent(fetchMock);
    expect(body.ref).toBeUndefined();
    expect(body.module).toBe("/views/_compiled/abc.mjs");
    // The hash names nothing; the id is what a prompt can say out loud.
    expect(body.id).toBe("trip");
  });

  it("coming back to live needs no destination", () => {
    const fetchMock = vi.fn(() => Promise.resolve(new Response(null, { status: 202 })));
    vi.stubGlobal("fetch", fetchMock);

    reportWentTo({ live: true });

    expect(sent(fetchMock).body.live).toBe(true);
  });

  // Fire-and-forget: a dropped report costs the next turn one line of context. It must
  // never surface as an unhandled rejection in a window someone is using.
  it("swallows a failed report", async () => {
    vi.stubGlobal("fetch", vi.fn(() => Promise.reject(new Error("offline"))));
    expect(() => reportWentTo({ viewRef: "factory/drive" })).not.toThrow();
    await Promise.resolve();
  });
});
