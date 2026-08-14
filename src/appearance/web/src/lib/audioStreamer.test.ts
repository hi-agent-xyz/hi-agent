import { describe, expect, it } from "vitest";

import { reconnectDelayMs } from "./audioStreamer";

describe("reconnectDelayMs", () => {
  it("retries quickly, then backs off, while audio is still flowing", () => {
    const fresh = 100; // a frame arrived a moment ago
    expect(reconnectDelayMs(0, fresh)).toBe(500);
    expect(reconnectDelayMs(1, fresh)).toBe(1000);
    expect(reconnectDelayMs(2, fresh)).toBe(2000);
    expect(reconnectDelayMs(9, fresh)).toBe(5000); // ceiling
  });

  it("waits the ceiling when the audio thread has gone quiet", () => {
    // No frame for seconds: the capture is dead, so a reopened socket would only
    // hand the upstream STT another session to time out. Even the first retry
    // waits — the point of the eager one is audio that has somewhere to go.
    expect(reconnectDelayMs(0, 8000)).toBe(5000);
  });
});
