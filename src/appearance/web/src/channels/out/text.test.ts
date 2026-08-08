import { describe, expect, it } from "vitest";

import { parseTextCursor, serializeTextCursor } from "./text";

describe("outbound text cursor", () => {
  it("round-trips an epoch-qualified cursor", () => {
    const cursor = { epoch: "2026-08-08T00:00:00Z", id: 12 };
    expect(parseTextCursor(serializeTextCursor(cursor))).toEqual(cursor);
  });

  it("resets legacy numeric and malformed storage", () => {
    expect(parseTextCursor("12")).toBeNull();
    expect(parseTextCursor('{"epoch":"boot","id":-1}')).toBeNull();
    expect(parseTextCursor('{"epoch":"","id":1}')).toBeNull();
    expect(parseTextCursor("not-json")).toBeNull();
  });
});
