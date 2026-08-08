import { afterEach, describe, expect, it, vi } from "vitest";

import { FileUploadError, postInFiles } from "./file";

const originalFetch = globalThis.fetch;

afterEach(() => {
  vi.restoreAllMocks();
  globalThis.fetch = originalFetch;
});

function response(body: string, status: number, contentType = "application/json") {
  return new Response(body, {
    status,
    headers: { "content-type": contentType },
  });
}

describe("postInFiles", () => {
  it("returns the structured batch outcome", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(
      response(
        JSON.stringify({ attempted: 2, received: 2, failed: [] }),
        200,
      ),
    );

    await expect(
      postInFiles({
        files: [new File(["a"], "a.txt"), new File(["b"], "b.txt")],
      }),
    ).resolves.toEqual({ attempted: 2, received: 2, failed: [] });
  });

  it("preserves partial results for retry routing", async () => {
    const result = {
      attempted: 2,
      received: 1,
      failed: [{ index: 1, name: "b.txt", error: "inbound channel closed" }],
    };
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(response(JSON.stringify(result), 207));

    const error = await postInFiles({
      files: [new File(["a"], "a.txt"), new File(["b"], "b.txt")],
    }).catch((caught: unknown) => caught);

    expect(error).toBeInstanceOf(FileUploadError);
    expect(error).toMatchObject({ status: 207, result });
  });

  it("keeps plain-text server details after reading the body", async () => {
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(response("request body too large", 413, "text/plain"));

    await expect(
      postInFiles({
        files: [new File(["a"], "a.txt")],
      }),
    ).rejects.toThrow("request body too large");
  });
});
