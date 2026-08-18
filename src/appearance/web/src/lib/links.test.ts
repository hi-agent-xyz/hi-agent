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

  it("ends the URL where an unspaced CJK sentence resumes", () => {
    const href = "http://127.0.0.1:19070/sample.mp3";
    expect(splitSpeechLinks(`${href}。听完告诉我`)).toEqual([
      { kind: "link", text: href, href, label: "127.0.0.1" },
      { kind: "text", text: "。听完告诉我" },
    ]);
  });

  it("ends the URL at a CJK glyph even with no punctuation between", () => {
    const href = "https://example.com/a";
    expect(splitSpeechLinks(`请开${href}看看`)).toEqual([
      { kind: "text", text: "请开" },
      { kind: "link", text: href, href, label: "example.com" },
      { kind: "text", text: "看看" },
    ]);
  });

  it("leaves incomplete URLs as plain text", () => {
    expect(splitSpeechLinks("not ready: https://")).toEqual([
      { kind: "text", text: "not ready: https://" },
    ]);
  });
});
