import type { ReactNode } from "react";
import { ChannelControls } from "./ChannelControls";
import { Views } from "./Views";
import type { Stop } from "../lib/panel";

/** Which of the panel's bodies is showing. */
export type Tab = "messages" | "views";

interface PanelProps {
  /** Where the panel is on the axis. Drives its geometry, and nothing else here
   * reads a width. */
  stop: Stop;
  tab: Tab;
  onTab: (tab: Tab) => void;
  /** The messages body. Passed in rather than rendered here because `<Chat>` is
   * mounted exactly once by the shell and must stay that way — see `ui/Shell.tsx`. */
  children: ReactNode;
  /** Fired when a view is chosen, so the shell can get the panel out of the way
   * if it is covering the thing that was just chosen. */
  onChose: () => void;
  audioOn: boolean;
  onToggleAudio: () => void;
  audioError?: string | null;
  videoOn: boolean;
  onToggleVideo: () => void;
  videoError?: string | null;
  voiceOn: boolean;
  onToggleVoice: () => void;
  onCloseViews: () => void;
}

/**
 * The panel — the person's side of the screen, all of it, in one box.
 *
 * It was three surfaces with two drawings apiece: a conversation that was a corner
 * popover on a window and a pushed page on a phone, a views navigator that was a
 * short band or a second pushed page, and a row of channel controls that was a
 * corner cluster or a page's head bar. Six drawings, three dismissal rules, two
 * page types on a stack, and a `bar` prop whose whole job was to say which of two
 * positions a row of buttons was standing in. They are one box now, and where it
 * is, is a stop on one axis (`lib/panel.ts`, `docs/arch/stage.md` § *The panel,
 * and the axis it runs on*).
 *
 * **It is always mounted, at every stop.** `room` is this box off the right-hand
 * side of the window, not a state in which it does not exist — which is what lets
 * `<Chat>` keep its scroll position and every page of already-fetched scrollback
 * across every open and close, the invariant this face has held since the stage
 * was three planes. The stop moves it; nothing about the stop branches on whether
 * it is here.
 *
 * **Two boxes, because the panel has two measures and only one of them is where
 * it is.** The `aside` is the window — it starts at the panel's left edge and runs
 * to the right-hand side of the screen, and a drag moves that edge pixel by pixel.
 * The box inside it is laid out at the measure of the stop and is pinned to the
 * right, so a drag *reveals* the panel rather than re-laying it: a conversation
 * being pulled in does not re-wrap every line on the way (`lib/panel.ts`
 * § `measureOf`). At rest the two are exactly the same size, so this split is
 * invisible except under a finger.
 *
 * **A head that does not change and a body that does.** The head is the channel
 * row and the tabs, and it is the same at every stop and on every tab — that is
 * the rule left standing where *every channel is one press away wherever you are*
 * used to be: **no channel is behind a mode**. The body is one tab at a time.
 *
 * **The messages body is handed in; the views body is rendered here.** Not a
 * stylistic split: `<Chat>` must never be unmounted, so it is mounted once by the
 * shell above everything that can change, and switching tabs only flips its
 * visibility. `<Views>` has no such constraint and polls the inventory while it is
 * up, so leaving the tab should genuinely stop it — mounting it with the tab is
 * the honest way to say that.
 */
export function Panel({ stop, tab, onTab, children, onChose, ...channels }: PanelProps) {
  return (
    <aside
      className="hi-panel"
      data-stop={stop}
      aria-label="panel"
      // The room is the panel gone, so nothing in it is reachable by the focus, by
      // a screen reader, or by spatial navigation looking for something to the
      // right. Hidden rather than unmounted, for the scrollback.
      aria-hidden={stop === "room" ? true : undefined}
      inert={stop === "room"}
    >
      <div className="hi-panel-measure">
        <div className="hi-panel-head">
          <div className="hi-panel-tabs" role="tablist" aria-label="panel">
            <button
              type="button"
              role="tab"
              className={`hi-panel-tab${tab === "messages" ? " is-on" : ""}`}
              aria-selected={tab === "messages"}
              onClick={() => onTab("messages")}
            >
              Messages
            </button>
            <button
              type="button"
              role="tab"
              className={`hi-panel-tab${tab === "views" ? " is-on" : ""}`}
              aria-selected={tab === "views"}
              onClick={() => onTab("views")}
            >
              Views
            </button>
          </div>
          <ChannelControls {...channels} />
        </div>

        <div className="hi-panel-body">
          {/* Visibility, never a branch: the messages body holds the scroller. */}
          <div className="hi-panel-pane" data-shown={tab === "messages" ? "true" : "false"}>
            {children}
          </div>
          {tab === "views" && (
            <div className="hi-panel-pane" data-shown="true">
              <Views stacked={stop === "full"} onChose={onChose} />
            </div>
          )}
        </div>
      </div>
    </aside>
  );
}
