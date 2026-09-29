import { afterEach, describe, expect, it, vi } from "vitest";
import { deleteView, goToView, shareState, shareView, ShareRefused, unshareView } from "./view";

/** The one request `goToView` fires, decoded. */
function sent(mock: ReturnType<typeof vi.fn>) {
  const [target, init] = mock.mock.calls[0]!;
  return {
    target: String(target),
    headers: init.headers as Record<string, string>,
    body: JSON.parse(String(init.body)),
  };
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

    await goToView({ viewRef: "factory/drive" });

    const { target, body } = sent(fetchMock);
    // The person's write of the appearance, which is the same route the inventory
    // opens through — there is one way to put the screen somewhere.
    expect(target).toContain("/api/views/open");
    expect(body.ref).toBe("factory/drive");
    expect(body.live).toBe(false);
  });

  // The server refuses a move no window made: on loopback this header is the only thing
  // that tells a person's move from a script that wanted a module.
  it("says which face made the move", async () => {
    const fetchMock = accepted();
    vi.stubGlobal("fetch", fetchMock);

    await goToView({ viewRef: "factory/drive" });

    expect(sent(fetchMock).headers["X-HI-Face"]).toMatch(/\S/);
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

describe("the card's menu, on the wire", () => {
  it("deletes a view from a face, by its ref", async () => {
    const fetchMock = vi.fn(() => Promise.resolve(new Response(null, { status: 204 })));
    vi.stubGlobal("fetch", fetchMock);

    await deleteView("trip/plan");

    const { target, headers, body } = sent(fetchMock);
    expect(target).toContain("/api/views/delete");
    expect(body).toEqual({ ref: "trip/plan" });
    expect(headers["X-HI-Face"]).toMatch(/\S/);
  });

  it("rejects a delete the server refused", async () => {
    vi.stubGlobal("fetch", vi.fn(() => Promise.resolve(new Response("no", { status: 400 }))));
    await expect(deleteView("factory/tasks")).rejects.toThrow(/400/);
  });

  it("shares unlisted when asked to, and hands back the server's link as it is", async () => {
    const fetchMock = vi.fn(() =>
      Promise.resolve(
        Response.json({ path: "/trip/plan", key: "k", link: "/trip/plan?key=k", reachable: "this_machine" }),
      ),
    );
    vi.stubGlobal("fetch", fetchMock);

    const published = await shareView("trip/plan", { unlisted: true });

    expect(sent(fetchMock).body).toEqual({ ref: "trip/plan", on: true, unlisted: true });
    expect(published.link).toBe("/trip/plan?key=k");
    expect(published.reachable).toBe("this_machine");
  });

  // The check's reasons are the owner's to read, worded by the server.
  it("carries the check's refusals as they were worded", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.resolve(Response.json({ refusals: ["it reads /api/tasks"] }, { status: 422 }))),
    );
    const refused = await shareView("trip/plan", { unlisted: true }).catch((e: unknown) => e);
    expect(refused).toBeInstanceOf(ShareRefused);
    expect((refused as ShareRefused).refusals).toEqual(["it reads /api/tasks"]);
  });

  it("reads a view that is not shared as null", async () => {
    vi.stubGlobal("fetch", vi.fn(() => Promise.resolve(new Response("not shared", { status: 404 }))));
    expect(await shareState("trip/plan")).toBeNull();
  });

  it("withdraws a share by turning it off", async () => {
    const fetchMock = vi.fn(() => Promise.resolve(new Response(null, { status: 204 })));
    vi.stubGlobal("fetch", fetchMock);
    await unshareView("trip/plan");
    expect(sent(fetchMock).body).toEqual({ ref: "trip/plan", on: false });
  });
});
