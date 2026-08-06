import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { SpeechText } from "./SpeechText";

describe("SpeechText", () => {
  it("renders a compact, external caption link", () => {
    const href = "https://accounts.feishu.cn/oauth/v1/device/verify?flow_id=abc&user_code=4PJV-MY7T";
    const html = renderToStaticMarkup(
      <SpeechText items={[{ id: 1, text: `打开它：${href}`, speaker: "agent" }]} />,
    );

    expect(html).toContain(`<a class="hi-speech-link" href="${href.replace("&", "&amp;")}"`);
    expect(html).toContain('target="_blank"');
    expect(html).toContain(">accounts.feishu.cn</span>");
  });
});
