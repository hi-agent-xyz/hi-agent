import { useCallback, useEffect, useRef, useState } from "react";
import { usePresence, useMessages, useChannels, useSendText } from "../core";
// Straight from the session module, not through `@hi/core`: the upstream's state is
// the host's to draw, and the condition notice is deliberately not in the inventory a
// view authors against (`docs/arch/stage.md`).
import { useCondition } from "../core/session";
import { useViews } from "../core/views";
import { stage as composeStage } from "../core/layout";
import { useHandoff } from "../hooks/useHandoff";
import { onHostKey } from "../lib/keyboard";
import { useShape } from "../lib/shape";
import { advance, depth, initial, opened, retreat, type Stop } from "../lib/panel";
import { focusInViewPlane, leaveViewPlane } from "../lib/spatial";
import { onShellBack, reportBackDepth } from "../lib/tvBack";
import { Atmosphere } from "./Atmosphere";
import { Presence } from "./Presence";
import { Chat } from "./Chat";
import { SpeechText, type SpeechItem } from "./SpeechText";
import { useCaption } from "./caption";
import { ViewSlot } from "./ViewSlot";
import { Composer } from "./Composer";
import { CameraPreview } from "./CameraPreview";
import { Panel, type Tab } from "./Panel";
import { PanelGesture } from "./PanelGesture";
import { PanelButton } from "./PanelButton";

/**
 * The host chrome — a calm, breathing room — reading the session through
 * `@hi/core` hooks rather than owning it. The session lives in the providers
 * above this component, so the swappable `ViewSlot` below never tears down the
 * mic / audio / channel loops when the agent swaps a view.
 *
 * **Three planes, and everything on screen is on exactly one of them**
 * (`docs/arch/stage.md`):
 *
 *   ground — the paper and the grain
 *   view   — everything the agent put up: its content view, the host's condition
 *            notice over it, ordered by the wire and by nothing else
 *   cover  — everything the host owns and the agent can never occlude: the camera
 *            self-view, the caption, the panel, alerts
 *
 * The order carries a meaning, not just a value: **the agent's plane is below the
 * person's.** Nothing the agent shows can rise above the record of what was said
 * or the controls to answer it. Each plane is a stacking context, so ordering
 * inside one is a local question — a view writing `z-index: 9999` climbs to the
 * top of `view` and no further.
 *
 * **The person's side of the screen is one box on one axis.** The conversation,
 * the views navigator and the channel controls were three surfaces with two
 * drawings apiece, chosen by a media query. They are the panel now, and where it
 * is, is a stop: `room`, `panel`, `full` (`lib/panel.ts`, `docs/arch/stage.md` §
 * *The panel, and the axis it runs on*). This component owns that number and
 * nothing else does.
 *
 * **The room keeps one button.** With the panel away, what is on screen is what
 * the agent put there, edge to edge, with the caption, the camera pip and
 * `<PanelButton>` over it. The button is the way in on every shape but the
 * television, whose remote has `→`; typing, `Escape` and a trackpad's swipe are
 * still ways along the axis, and are listed in `<PanelGesture>` and the key
 * ladders below. The panel's way out is drawn too — a close button in its head, and
 * on Android the system's Back, which the ladder below is reported to.
 *
 * Placement is one job, and `composeStage` is the whole of it: it decides
 * *geometry* — the conversation as itself or as a caption; the camera filling or
 * a pip — and never who covers whom, which is static. It also decides placement,
 * **never lifecycle**: `<Chat>` and `<CameraPreview>` are mounted ONCE here, above
 * the swappable `ViewSlot`, and the pass only flips their props and classes. They
 * must never move into `ViewSlot` or a conditional branch — re-mounting
 * `<CameraPreview>` re-acquires the camera and blacks out the feed, and re-mounting
 * `<Chat>` throws away the scroll position and every page of scrollback already
 * fetched. `<Panel>` is mounted at every stop for the same reason: `room` is that
 * box off the right-hand side of the window, not a state in which it is gone.
 */
export function Shell() {
  const presence = usePresence();
  const { messages, interim, loadOlder } = useMessages();
  const condition = useCondition();
  const ch = useChannels();
  const sendText = useSendText();
  const { views, clear } = useViews();
  // Which arrangement this is. Read once here and handed down, rather than each
  // surface asking: what it decides is which stops exist at all, and two
  // components disagreeing about that would put a middle stop on a screen too
  // narrow to hold one.
  const shape = useShape();

  // Where the panel is. A window preference like the put-away it replaces, and
  // never server state for the same reason: it says what this window is showing
  // the person, not what the agent expressed. Seeded per shape and not persisted
  // — neither being open nor being away outlives the page.
  const [stop, setStop] = useState<Stop>(() => initial(shape));
  const [tab, setTab] = useState<Tab>("messages");
  const [pastedInputText, setPastedInputText] = useState<{ id: number; text: string } | null>(null);
  const pasteIdRef = useRef(0);
  // The face's root box, handed to `<PanelGesture>`. It carries the stop, and the
  // panel and the strip that moves it are found under it as the edge's riders.
  const rootRef = useRef<HTMLDivElement | null>(null);

  // Rotating a phone into landscape makes it `wide`, which has a stop the phone
  // does not — and being at `full` on a shape that no longer offers it would leave
  // the panel covering a window it should be sitting beside. Re-seat rather than
  // re-seed: the person's position on the axis is kept as closely as the new shape
  // allows, so a panel that was open stays open.
  const shapeRef = useRef(shape);
  useEffect(() => {
    if (shapeRef.current === shape) return;
    shapeRef.current = shape;
    setStop((at) => (at === "room" ? "room" : opened(shape)));
  }, [shape]);

  // Stable, because the composer's start-typing-to-open listener depends on it.
  const openConversation = useCallback(() => {
    setTab("messages");
    setStop((at) => (at === "room" ? opened(shape) : at));
  }, [shape]);
  const pasteIntoTextInput = useCallback((text: string) => {
    pasteIdRef.current += 1;
    setPastedInputText({ id: pasteIdRef.current, text });
  }, []);

  const layout = composeStage({
    content: views.length > 0,
    away: stop === "room",
  });

  // The conversation is shown as itself in one of its two states; the pill is a
  // different rendering of the same list. `<Chat>` stays mounted through both.
  const chatShown = layout.conversation === "panel" && tab === "messages";

  const handoff = useHandoff({ openConversation, pasteIntoTextInput });

  // Escape retreats a stop, in every stop the panel is out in. It defers to
  // whoever already handled it, so clearing a half-typed line closes the line and
  // leaves the panel where it is.
  //
  // **The arrows are not the host's on a keyboard.** They used to step this axis
  // as well, and that was the host taking a key agent views have every reason to
  // want: a deck pages with them, a board moves its selection with them, a player
  // scrubs with them. A view that binds them on the window hears them only while
  // nothing inside it holds the focus (`lib/keyboard.ts`) — which is precisely the
  // case the host was claiming out from under it, and the case a view laid out as
  // a canvas rather than as controls is always in.
  //
  // The television keeps them, and that is not an exception to the rule so much as
  // the rule arriving somewhere else: there the four arrows are the only
  // instrument the room has, `installSpatialNav` already owns them, and it claims
  // a press only when it actually moved the focus — so a right press that reaches
  // here is one that found nothing to move to and has nowhere else to go
  // (`lib/spatial.ts`). Every other shape opens the panel by typing into it, by
  // its edge, by two fingers, or by the strip.
  //
  // Through `onHostKey` rather than a `window` listener, because a key pressed in
  // the panel is chrome's and stops at the document — one node short of the window
  // (`lib/keyboard.ts`). It also means a view holding the focus keeps its own
  // arrows and its own Escape.
  useEffect(() => {
    return onHostKey((event) => {
      if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey) return;

      // Escape is not excluded while a line is being written: the composer clears
      // its own draft and marks the key handled, and only an empty line lets it
      // through to this.
      if (event.key === "Escape") {
        setStop((at) => retreat(shape, at));
        return;
      }
      if (shape !== "tv") return;

      // Inside a line being written the arrows move the caret.
      const active = document.activeElement as HTMLElement | null;
      const typing =
        active?.tagName === "INPUT" || active?.tagName === "TEXTAREA" || active?.isContentEditable;
      if (typing) return;

      if (event.key === "ArrowRight") {
        setStop((at) => advance(shape, at));
        event.preventDefault();
      } else if (event.key === "ArrowLeft") {
        setStop((at) => retreat(shape, at));
        event.preventDefault();
      }
    });
  }, [shape]);

  // The television's Escape, which is the remote's Back button.
  //
  // Same ladder and in the same order, with one rung the keyboard does not need: a
  // view holding the focus. On a desktop the pointer leaves a view by clicking
  // somewhere else, and there is no pointer here — while a view has the focus the
  // arrows are its own (`lib/keyboard.ts`), so Back is the only way out of it.
  //
  // Reported as a depth rather than handled by the shell, because *what* closes is
  // this component's business and *whether Back is ours at all* is the shell's.
  // When the count is zero the shell keeps the press and shows its own chrome, one
  // step from leaving the app. The panel is worth one rung per stop behind it, so
  // Back walks the axis a stop at a time rather than dropping the whole thing.
  //
  // Where the focus is has to be watched rather than read: it moves without React,
  // so a depth computed during render would be the depth as of whatever last
  // happened to re-render. `focusout` is listened for alongside `focusin` because
  // focus leaving for nothing at all — a view unmounting under it — is a rung
  // coming off the ladder and fires only the former.
  const [focusInView, setFocusInView] = useState(false);
  useEffect(() => {
    if (shape !== "tv") return;
    const update = () => setFocusInView(focusInViewPlane());
    update();
    document.addEventListener("focusin", update);
    document.addEventListener("focusout", update);
    return () => {
      document.removeEventListener("focusin", update);
      document.removeEventListener("focusout", update);
    };
  }, [shape]);

  // Reported on every shape, not only the television's: an Android phone's Back is
  // the same button delivered the same way, and with the panel's way in moved off
  // the screen's edges it is the panel's natural way out. Where no shell listens
  // — a browser tab, iOS, the desktops — both ends are inert.
  const backDepth = (focusInView ? 1 : 0) + depth(shape, stop);
  useEffect(() => {
    reportBackDepth(backDepth);
  }, [backDepth]);

  useEffect(() => {
    return onShellBack(() => {
      if (shape === "tv" && leaveViewPlane()) return;
      setStop((at) => retreat(shape, at));
    });
  }, [shape]);

  // Something said while the panel was away. The caption shows it for a moment and
  // then fades; this is what is left once it has, on the one thing the room keeps.
  // Cleared by opening the panel, which is where it can be read.
  //
  // Keyed on the newest message's id rather than on a count, so a page of older
  // scrollback arriving at the top is not news. The first history to arrive is the
  // seed and is not news either — it was all said before this page existed.
  const [unread, setUnread] = useState(false);
  const latest = messages[messages.length - 1];
  const newestId = latest?.id;
  const newestFromAgent = latest?.role !== "user";
  const seenRef = useRef<string | undefined>(undefined);
  useEffect(() => {
    const seen = seenRef.current;
    seenRef.current = newestId;
    if (stop !== "room") {
      setUnread(false);
      return;
    }
    if (seen !== undefined && newestId !== seen && newestFromAgent) setUnread(true);
  }, [newestId, newestFromAgent, stop]);

  // The pill shows the newest thing said, or the line currently being recognized
  // if one is in flight — the same tail the chat ends on. It is a caption, so it
  // shows for as long as that line is worth reading and then fades: what it holds
  // is a copy, and the list behind it keeps the original. `lastSpoken` is not
  // cleared when the dwell runs out — the line stays mounted and the dock fades,
  // so a fresh line has something to cross-fade with rather than appearing into a
  // collapsed box.
  const newest = messages[messages.length - 1];
  const captionShown = useCaption({ interim, line: newest });
  const lastSpoken: SpeechItem[] = interim
    ? [{ id: -1, text: interim, speaker: "user", pending: true }]
    : newest
      ? [{ id: 0, text: newest.text, speaker: newest.role === "user" ? "user" : "agent" }]
      : [];

  const channels = {
    audioOn: ch.audioInput,
    onToggleAudio: ch.toggleAudio,
    audioError: ch.audioError,
    videoOn: ch.videoInput,
    onToggleVideo: ch.toggleVideo,
    videoError: ch.videoError,
    voiceOn: ch.audioOutput,
    onToggleVoice: ch.toggleAudioOutput,
    onCloseViews: clear,
  };

  return (
    <div
      ref={rootRef}
      className="hi-root"
      data-stop={stop}
      onDragEnterCapture={handoff.onFileDragOver}
      onDragOverCapture={handoff.onFileDragOver}
      onDropCapture={handoff.onFileDrop}
    >
      <div className="hi-plane hi-plane--ground">
        <Presence state={presence.state} demote={layout.demote} />
        <Atmosphere />
      </div>

      {/* The agent's plane. Its internal order is the wire's array order — content
          first, the condition notice over it — so it needs no z-index at all. It
          insets past the panel at the middle stop and only there, which is the
          reflow the person fires by pulling the panel in. */}
      <div className="hi-plane hi-plane--view">
        <ViewSlot />
      </div>

      {/* The person's plane. Transparent to the pointer as a whole; each surface
          on it takes its own events back, so the gaps between them stay clickable
          down to the view underneath. */}
      <div className="hi-plane hi-plane--cover">
        <CameraPreview stream={ch.visionStream} pip={layout.camera === "pip"} />

        {layout.conversation === "pill" && (
          // The dock steps past the camera pip (bottom-left) so the bottom bar's
          // two zones — pip · captions — never overlap, and that step is keyed in
          // the stylesheet on the pip *being on screen* (`:has(.hi-selfview--pip)`).
          <div
            className="hi-captions"
            data-shown={captionShown ? "true" : "false"}
            aria-hidden={captionShown ? undefined : true}
          >
            <SpeechText items={lastSpoken} />
          </div>
        )}

        {/* PINNED — the panel, and everything the person owns inside it. Mounted
            once at every stop, so the scroller keeps its position and its
            already-fetched scrollback across every open and close. */}
        <Panel
          stop={stop}
          tab={tab}
          onTab={setTab}
          // Choosing a view moves the screen. If the panel is covering that screen,
          // step it back so the person can see what they picked; if it is sitting
          // beside it, they already can.
          onChose={() => setStop((at) => (at === "full" ? retreat(shape, at) : at))}
          onClose={shape === "tv" ? undefined : () => setStop("room")}
          {...channels}
        >
          <Chat
            messages={messages}
            interim={interim}
            typing={presence.state === "typing"}
            condition={condition}
            onLoadOlder={loadOlder}
          >
            <Composer
              onSend={sendText}
              shown={chatShown}
              pastedText={pastedInputText}
              onOpen={openConversation}
              onPickFiles={(files) => void handoff.sendFiles(files)}
              filesSending={handoff.sending}
            />
          </Chat>
        </Panel>

        {/* The way in. Not on the television: its remote has `→`, and a round button
            in a corner is not something a D-pad should have to travel to. */}
        <PanelButton
          shown={stop === "room" && shape !== "tv"}
          unread={unread}
          listening={ch.audioInput}
          onOpen={openConversation}
        />

        {/* Last, so the strip is over everything it may have to claim a touch from
            — including the panel it moves. */}
        <PanelGesture shape={shape} stop={stop} onStop={setStop} root={rootRef} />
      </div>
    </div>
  );
}
