import { describe, expect, it } from "vitest";
import { parseFrame, parseMessage } from "./text";

const msg = {
  id: "0199a000-0000-7000-8000-000000000001",
  ts: "2026-08-11T09:31:04.000Z",
  role: "user",
  text: "What day is it?",
};

describe("message parsing", () => {
  it("accepts a message", () => {
    expect(parseMessage(msg)).toEqual(msg);
  });

  it("carries an attachment when one came with it", () => {
    expect(
      parseMessage({ ...msg, attachment: { ref: "file/2026-08-11/09/31-04.png", mime: "image/png" } }),
    ).toEqual({ ...msg, attachment: { ref: "file/2026-08-11/09/31-04.png", mime: "image/png" } });
  });

  it("rejects a role the conversation has no end for", () => {
    expect(parseMessage({ ...msg, role: "system" })).toBeNull();
  });

  it("rejects a message with no id, since the id is what scrollback stitches on", () => {
    const { id: _id, ...rest } = msg;
    expect(parseMessage(rest)).toBeNull();
  });
});

describe("frames", () => {
  it("opens with the whole window", () => {
    expect(parseFrame({ reset: { messages: [msg], interim: null } })).toEqual({
      kind: "reset",
      conversation: { messages: [msg] },
    });
  });

  it("keeps a live interim on the opening window", () => {
    expect(parseFrame({ reset: { messages: [], interim: "what day" } })).toEqual({
      kind: "reset",
      conversation: { messages: [], interim: "what day" },
    });
  });

  it("appends one message", () => {
    expect(parseFrame({ append: msg })).toEqual({ kind: "append", message: msg });
  });

  it("reads an interim and its expiry", () => {
    expect(parseFrame({ interim: "wait" })).toEqual({ kind: "interim", text: "wait" });
    expect(parseFrame({ interim: null })).toEqual({ kind: "interim" });
  });

  /** A malformed message inside a reset must not take the whole window with it. */
  it("drops an unreadable message rather than the frame", () => {
    const frame = parseFrame({ reset: { messages: [msg, { id: 7 }], interim: null } });
    expect(frame).toEqual({ kind: "reset", conversation: { messages: [msg] } });
  });

  it("rejects anything that is not a frame", () => {
    expect(parseFrame({ user: "old wire shape" })).toBeNull();
    expect(parseFrame(null)).toBeNull();
    expect(parseFrame([msg])).toBeNull();
  });
});
