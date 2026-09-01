import { afterEach, describe, expect, it, vi } from "vitest";
import { goToView } from "./view";

/** The one request `goToView` fires, decoded. */
function sent(mock: ReturnType<typeof vi.fn>) {
  const [target, init] = mock.mock.calls[0]!;
  return { target: String(target), body: JSON.parse(String(init.body)) };
}

function accepted() {
  return vi.fn(() => Promise.resolve(new Response(null, { status: 202 })));
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("taking the screen somewhere", () => {
  it("names a destination the server can key on, and says it is not live", async () => {
    const fetchMock = accepted();
    vi.stubGlobal("fetch", fetchMock);

    await goToView({ viewRef: "factory/drive", id: "drive" });

    const { target, body } = sent(fetchMock);
    // The person's write of the appearance, which is the same route the inventory
    // opens through — there is one way to put the screen somewhere.
    expect(target).toContain("/api/views/open");
    expect(body.ref).toBe("factory/drive");
    expect(body.live).toBe(false);
  });

  it("falls back to the module for an inline view, and still carries a name", async () => {
    const fetchMock = accepted();
    vi.stubGlobal("fetch", fetchMock);

    await goToView({ moduleUrl: "/views/_compiled/abc.mjs", id: "trip" });

    const { body } = sent(fetchMock);
    expect(body.ref).toBeUndefined();
    expect(body.module).toBe("/views/_compiled/abc.mjs");
    // The hash names nothing; the id is what a prompt can say out loud.
    expect(body.id).toBe("trip");
  });

  it("coming back to live needs no destination", async () => {
    const fetchMock = accepted();
    vi.stubGlobal("fetch", fetchMock);

    await goToView({ live: true });

    expect(sent(fetchMock).body.live).toBe(true);
  });

  // A ref whose source is gone, or that no longer compiles. The caller decides what to
  // do about it — the screen stays where it is either way, which beats blanking it.
  it("rejects when the server refuses the destination", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.resolve(new Response("no such view", { status: 404 }))),
    );
    await expect(goToView({ viewRef: "factory/gone" })).rejects.toThrow(/404/);
  });
});
