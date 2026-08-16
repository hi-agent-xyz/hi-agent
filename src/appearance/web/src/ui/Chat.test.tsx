import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { Chat, groupMessages } from "./Chat";
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

describe("readsAsName", () => {
  it("tells a name from a minted cluster id", () => {
    expect(readsAsName("赵力")).toBe(true);
    expect(readsAsName("samantha")).toBe(true);
    expect(readsAsName("7j2wa4r8")).toBe(false);
    expect(readsAsName("2xk04cyd")).toBe(false);
  });
});
