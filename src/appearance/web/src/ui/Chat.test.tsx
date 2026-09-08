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

describe("the conversation", () => {
  // The shape borrowed from shadcn's own chat was a title line, the messages, the
  // line being written. The title line is gone: the panel's tabs name this surface
  // in the row directly above it (`ui/Panel.tsx`), so a header here was the same
  // word twice, one row apart — and where the box is at all is the compositor's
  // business, not this component's.
  it("names nothing, because the tab above it does", () => {
    const html = renderToStaticMarkup(<Chat messages={[said("1", "帮我看下")]} />);
    expect(html).not.toContain("hi-chat-head");
    expect(html).not.toContain("Conversation");
  });

  it("opens on the messages", () => {
    const html = renderToStaticMarkup(<Chat messages={[said("1", "帮我看下")]} />);
    expect(html.indexOf('data-slot="message-scroller"')).toBeGreaterThanOrEqual(0);
  });

  // It named the other side too — the app's mark and "Hi Agent" — which is a
  // messenger's habit: there is one agent, its face is the window's own title,
  // and a badge on a panel that is always the same panel is decoration.
  it("carries no mark", () => {
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
        <Composer
          onSend={() => Promise.resolve()}
          shown
          onOpen={() => {}}
          onPickFiles={() => {}}
          filesSending={false}
        />
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

  // A drop and a paste are both gestures a phone does not have, so the picker is
  // the only way a touch device hands over a file. It stands at the head of the
  // line — the words the file arrives with — and not in the channel row, which is
  // channels to turn on (`ui/ChannelControls.tsx`).
  it("carries a file picker at its head", () => {
    const html = renderToStaticMarkup(
      <Chat messages={[said("1", "帮我看下")]}>
        <Composer
          onSend={() => Promise.resolve()}
          shown
          onOpen={() => {}}
          onPickFiles={() => {}}
          filesSending={false}
        />
      </Chat>,
    );

    expect(html).toContain('type="file"');
    const pick = html.indexOf('aria-label="hand over a file"');
    const line = html.indexOf('aria-label="message the agent"');
    const send = html.indexOf('aria-label="send"');
    expect(pick, "the picker is drawn").toBeGreaterThanOrEqual(0);
    expect(pick, "before the words, where the send button is after them").toBeLessThan(
      line,
    );
    expect(send).toBeGreaterThan(line);
  });

  // The handoff takes one batch at a time and drops a second in silence, so the
  // door is shut while one is on the wire rather than opening onto nothing.
  it("shuts the picker while a batch is being sent", () => {
    const line = (filesSending: boolean) =>
      renderToStaticMarkup(
        <Chat messages={[said("1", "帮我看下")]}>
          <Composer
            onSend={() => Promise.resolve()}
            shown
            onOpen={() => {}}
            onPickFiles={() => {}}
            filesSending={filesSending}
          />
        </Chat>,
      );
    const shut = (html: string) => html.split('disabled=""').length - 1;

    // One either way is the send button, which an empty line always shuts.
    expect(shut(line(false))).toBe(1);
    expect(shut(line(true))).toBe(2);
  });
});

describe("the dots", () => {
  // The one activity anything is drawn for. It used to be a read-only disc in the
  // controls cluster that drew all six states; the other five are the agent's own
  // business and are drawn nowhere now.
  it("stands at the foot, on the agent's side, only while a reply is coming", () => {
    const quiet = renderToStaticMarkup(<Chat messages={[said("1", "帮我看下")]} />);
    expect(quiet).not.toContain("hi-typing");

    const html = renderToStaticMarkup(<Chat messages={[said("1", "帮我看下")]} typing />);
    const message = html.indexOf("帮我看下");
    const dots = html.indexOf("hi-typing");
    expect(dots, "under everything already said").toBeGreaterThan(message);
    // Drawn as a message and not an ornament: the agent's face beside it, in the
    // agent's bubble, on the agent's side — so the reply lands in the space this
    // was already holding.
    expect(html).toContain("icon.svg");
    expect(html.slice(message, dots)).toContain('data-align="start"');
  });

  // Nothing has been said yet, so there is nothing to stamp — and a time on the
  // dots would leave a stray timestamp behind when they went away.
  it("carries no time", () => {
    const html = renderToStaticMarkup(<Chat messages={[]} typing />);
    expect(html).not.toContain("<time");
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
