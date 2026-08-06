import { describe, expect, it } from "vitest";

import { splitSpeechLinks } from "./links";

describe("splitSpeechLinks", () => {
  it("extracts a URL without swallowing sentence punctuation", () => {
    const href = "https://accounts.feishu.cn/oauth/v1/device/verify?flow_id=abc&user_code=4PJV-MY7T";
    expect(splitSpeechLinks(`打开它：${href}。`)).toEqual([
      { kind: "text", text: "打开它：" },
      {
        kind: "link",
        text: href,
        href,
        label: "accounts.feishu.cn",
      },
      { kind: "text", text: "。" },
    ]);
  });

  it("leaves incomplete URLs as plain text", () => {
    expect(splitSpeechLinks("not ready: https://")).toEqual([
      { kind: "text", text: "not ready: https://" },
    ]);
  });
});
