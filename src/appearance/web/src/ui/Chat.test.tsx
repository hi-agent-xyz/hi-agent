import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { Chat, groupMessages } from "./Chat";
import { Composer } from "./Composer";
import { readsAsName } from "./Avatar";
import type { Message, Sender } from "../channels/out/text";

const T0 = Date.parse("2026-08-16T14:00:00Z");

function said(id: string, text: string, sender?: Sender, offsetMs = 0): Message {
  return {
    id,
    ts: new Date(T0 + offsetMs).toISOString(),
    role: "user",
    text,
    ...(sender ? { sender } : {}),
  };
}

describe("groupMessages", () => {
  it("does not stack two people under one avatar", () => {
    // The failure the sender exists to stop: everyone in the room is `role: "user"`,
    // so grouping on role alone puts a colleague's line under 赵力's face.
    const groups = groupMessages([
      said("1", "其实在我预想中", { subject: "赵力", basis: "cluster" }),
      said("2", "我觉得也行", { subject: "7j2wa4r8", basis: "cluster" }, 1000),
      said("3", "那就这样", { subject: "赵力", basis: "cluster" }, 2000),
    ]);

    expect(groups.map((g) => g.sender?.subject)).toEqual(["赵力", "7j2wa4r8", "赵力"]);
    expect(groups.map((g) => g.messages.length)).toEqual([1, 1, 1]);
  });

  it("keeps one person's run together", () => {
    const zhao: Sender = { subject: "赵力", basis: "cluster" };
    const groups = groupMessages([
      said("1", "哎", zhao),
      said("2", "么意思?", zhao, 1000),
    ]);

    expect(groups).toHaveLength(1);
    expect(groups[0]!.messages.map((m) => m.text)).toEqual(["哎", "么意思?"]);
  });

  it("groups consecutive unattributed lines, claiming nothing about who spoke", () => {
    const nobody: Sender = { basis: "unknown" };
    const groups = groupMessages([said("1", "…", nobody), said("2", "…", nobody, 1000)]);

    expect(groups).toHaveLength(1);
    expect(groups[0]!.sender?.subject).toBeUndefined();
  });

  it("breaks the agent's replies away from the person's lines as before", () => {
    const groups = groupMessages([
      said("1", "哎", { subject: "赵力", basis: "cluster" }),
      { ...said("2", "你接着说就行"), role: "agent" },
    ]);

    expect(groups.map((g) => g.role)).toEqual(["user", "agent"]);
  });
});

describe("the avatar column", () => {
  /** Which side each rendered group sits on, in order. */
  function sides(html: string): string[] {
    return [...html.matchAll(/data-slot="message"[^>]*data-align="(\w+)"/g)].map((m) => m[1]!);
  }

  it("puts one avatar on each group, on that group's own side", () => {
    const html = renderToStaticMarkup(
      <Chat
        messages={[
          said("1", "其实在我预想中", { subject: "赵力", basis: "cluster" }),
          { ...said("2", "你接着说就行", undefined, 1000), role: "agent" },
        ]}
      />,
    );

    expect(sides(html)).toEqual(["end", "start"]);
    // The person's group is titled with who the recognition named…
    expect(html).toContain('title="赵力"');
    // …and the agent's is the app's own mark, not somebody from the store.
    expect(html).toContain("/icon.svg");
  });

  it("says a name it only assumed is assumed", () => {
    const html = renderToStaticMarkup(
      <Chat messages={[said("1", "帮我看下", { subject: "赵力", basis: "owner" })]} />,
    );
    expect(html).toContain('title="赵力 (assumed)"');
  });

  it("draws a silhouette rather than a name for a voice nobody placed", () => {
    const html = renderToStaticMarkup(
      <Chat messages={[said("1", "…", { basis: "unknown" })]} />,
    );
    expect(html).toContain('title="someone — not recognized"');
  });

});

describe("the card", () => {
  // The shape borrowed from shadcn's own chat: a title line, the messages, the
  // line being written. One card in one box — where that box is is the
  // compositor's business and not this component's.
  it("names the surface, above the messages", () => {
    const html = renderToStaticMarkup(<Chat messages={[said("1", "帮我看下")]} />);

    const title = html.indexOf('class="hi-chat-head"');
    const scroller = html.indexOf('data-slot="message-scroller"');
    expect(title).toBeGreaterThanOrEqual(0);
    expect(html).toContain("Conversation");
    expect(title, "the title is the card's first row").toBeLessThan(scroller);
  });

  // It named the other side — the app's mark and "Hi Agent" — which is a
  // messenger's habit: there is one agent, its face is the window's own title,
  // and a badge on a panel that is always the same panel is decoration.
  it("carries no mark beside the title", () => {
    const html = renderToStaticMarkup(<Chat messages={[said("1", "帮我看下")]} />);
    expect(html).not.toContain("icon.svg");
  });
});

describe("the line being written", () => {
  // It used to be a box of its own, positioned to look flush with the panel by
  // sharing its width and right edge. Inside the conversation it is part of it,
  // which is what lets one control put both away together.
  it("stands in the conversation's own foot, under the messages", () => {
    const html = renderToStaticMarkup(
      <Chat messages={[said("1", "帮我看下")]}>
        <Composer onSend={() => {}} shown onOpen={() => {}} />
      </Chat>,
    );

    const chat = html.indexOf('class="hi-chat"');
    const scroller = html.indexOf('data-slot="message-scroller"');
    const foot = html.indexOf('class="hi-chat-foot"');
    expect(chat).toBeGreaterThanOrEqual(0);
    expect(scroller).toBeGreaterThan(chat);
    expect(foot, "the foot is inside the conversation, after the messages").toBeGreaterThan(
      scroller,
    );
    expect(html).toContain('data-slot="input-group"');
  });

  it("renders nothing where the shell gives it no foot", () => {
    const html = renderToStaticMarkup(<Chat messages={[said("1", "帮我看下")]} />);
    expect(html).not.toContain("hi-chat-foot");
  });
});

describe("readsAsName", () => {
  it("tells a name from a minted cluster id", () => {
    expect(readsAsName("赵力")).toBe(true);
    expect(readsAsName("samantha")).toBe(true);
    expect(readsAsName("7j2wa4r8")).toBe(false);
    expect(readsAsName("2xk04cyd")).toBe(false);
  });
});
