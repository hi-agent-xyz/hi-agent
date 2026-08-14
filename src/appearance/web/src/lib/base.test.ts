import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { base, inCore, installBase, url } from "./base";

// These tests run in node, where there is no DOM — and `base.ts` needs only the
// three window members it actually reads, so a stub is enough and honest.
const g = globalThis as unknown as { window?: unknown; location?: unknown };

let fetched: Array<unknown>;

beforeEach(() => {
  fetched = [];
  const location = { origin: "https://hi-agent.xyz", pathname: "/" };
  g.location = location;
  g.window = {
    location,
    fetch: (input: unknown) => {
      fetched.push(input);
      return Promise.resolve(undefined);
    },
    EventSource: class {
      constructor(readonly source: unknown) {}
    },
  };
});

afterEach(() => {
  delete g.window;
  delete g.location;
});

/** Serve this page under `prefix`, the way the backend's injection does. */
function servedUnder(prefix: string): void {
  (g.window as { __HI_BASE__?: string }).__HI_BASE__ = prefix;
}

describe("base()", () => {
  it("is empty when nothing was stamped, and never keeps a trailing slash", () => {
    expect(base()).toBe("");
    servedUnder("/ana/");
    expect(base()).toBe("/ana");
  });
});

describe("url()", () => {
  it("leaves every path alone at the core's own root", () => {
    expect(url("/api/in/text")).toBe("/api/in/text");
    expect(url("/")).toBe("/");
  });

  it("starts a root-absolute path at the prefix", () => {
    servedUnder("/ana");
    expect(url("/api/in/text")).toBe("/ana/api/in/text");
    expect(url("/views/factory/hi-mark.svg")).toBe("/ana/views/factory/hi-mark.svg");
  });

  it("is idempotent, so a path the backend already prefixed is untouched", () => {
    servedUnder("/ana");
    expect(url(url("/api/in/text"))).toBe("/ana/api/in/text");
    expect(url("/ana")).toBe("/ana");
  });

  it("makes the core's root the prefix itself — the address, not a variant of it", () => {
    servedUnder("/ana");
    expect(url("/")).toBe("/ana");
  });

  it("does not touch a relative path or another origin", () => {
    servedUnder("/ana");
    expect(url("api/session")).toBe("api/session");
    expect(url("https://hi-agent.xyz/pricing")).toBe("https://hi-agent.xyz/pricing");
  });
});

describe("inCore()", () => {
  it("reads a browser path as the route the core owns", () => {
    servedUnder("/ana");
    expect(inCore("/ana/inspect/sessions")).toBe("/inspect/sessions");
    expect(inCore("/ana")).toBe("/");
  });

  it("is the identity at the core's own root", () => {
    expect(inCore("/inspect/sessions")).toBe("/inspect/sessions");
  });
});

describe("installBase()", () => {
  it("does nothing at all when the page is at a root", () => {
    const w = g.window as { fetch: unknown };
    const before = w.fetch;
    installBase();
    expect(w.fetch).toBe(before);
  });

  it("sends a root-absolute fetch to the core, not to the site it is served from", async () => {
    servedUnder("/ana");
    installBase();
    const w = g.window as { fetch: (input: unknown, init?: unknown) => Promise<unknown> };

    await w.fetch("/api/in/text", { method: "POST" });
    // A view that remembered `url()` itself is not prefixed twice.
    await w.fetch("/ana/api/out/text");
    // Somewhere else entirely is left alone.
    await w.fetch("https://example.test/x");

    expect(fetched).toEqual([
      "/ana/api/in/text",
      "/ana/api/out/text",
      "https://example.test/x",
    ]);
  });

  it("prefixes an EventSource, which is how a channel arrives", () => {
    servedUnder("/ana");
    installBase();
    const w = g.window as { EventSource: new (source: string) => { source: string } };
    expect(new w.EventSource("/api/activity").source).toBe("/ana/api/activity");
  });
});
