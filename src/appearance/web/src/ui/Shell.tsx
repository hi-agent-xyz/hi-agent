import { useCallback, useEffect, useRef, useState } from "react";
import { usePresence, useMessages, useChannels, useSendText } from "../core";
import { useViews } from "../core/views";
import { stage as composeStage } from "../core/layout";
import { useHandoff } from "../hooks/useHandoff";
import { onHostKey } from "../lib/keyboard";
import { Atmosphere } from "./Atmosphere";
import { Presence } from "./Presence";
import { Chat } from "./Chat";
import { SpeechText, type SpeechItem } from "./SpeechText";
import { useCaption } from "./caption";
import { ViewSlot } from "./ViewSlot";
import { Composer } from "./Composer";
import { ChannelControls } from "./ChannelControls";
import { CameraPreview } from "./CameraPreview";
import { HandoffOverlay } from "./HandoffOverlay";
import { ViewsBand } from "./ViewsBand";

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
 *            self-view, the conversation, the input line, the controls, alerts
 *
 * The order carries a meaning, not just a value: **the agent's plane is below the
 * person's.** Nothing the agent shows can rise above the record of what was said
 * or the controls to answer it. Each plane is a stacking context, so ordering
 * inside one is a local question — a view writing `z-index: 9999` climbs to the
 * top of `view` and no further.
 *
 * Placement is one job, and `composeStage` is the whole of it: it decides
 * *geometry* — panel, pill or stood down; fill or pip — and never who covers
 * whom, which is static. Its answer for the conversation is one box: the panel
 * does not change measure or corner when the agent puts something up. It also decides placement, **never lifecycle**: `<Chat>` and
 * `<CameraPreview>` are mounted ONCE here, above the swappable `ViewSlot`, and
 * the pass only flips their props and classes. They must never move into
 * `ViewSlot` or a conditional branch — re-mounting `<CameraPreview>` re-acquires
 * the camera and blacks out the feed, and re-mounting `<Chat>` throws away the
 * scroll position and every page of scrollback already fetched.
 */
export function Shell() {
  const presence = usePresence();
  const { messages, interim, loadOlder } = useMessages();
  const ch = useChannels();
  // Pulled out because `useChannels` hands back a fresh object every render, and
  // the dismissal effect below would resubscribe its window listeners on each one.
  const { setTextChannel } = ch;
  const sendText = useSendText();
  const { views, clear } = useViews();
  // Whether the views band is open. A window preference like the text channel's
  // own on/off, and never server state for the same reason: it says what this
  // window is showing the person, not what the agent expressed.
  const [bandOpen, setBandOpen] = useState(false);
  const [pastedInputText, setPastedInputText] = useState<{ id: number; text: string } | null>(null);
  const pasteIdRef = useRef(0);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const popoverRef = useRef<HTMLDivElement | null>(null);
  // Stable, because the composer's start-typing-to-open listener depends on it.
  const openConversation = useCallback(() => setTextChannel(true), [setTextChannel]);
  const pasteIntoTextInput = useCallback((text: string) => {
    pasteIdRef.current += 1;
    setPastedInputText({ id: pasteIdRef.current, text });
  }, []);

  const layout = composeStage({
    content: views.length > 0,
    collapsed: !ch.text,
  });

  // The conversation is shown as itself in one of its two states; the pill is a
  // different rendering of the same list. `<Chat>` stays mounted through both.
  const chatShown = layout.conversation === "popover";

  const handoff = useHandoff({
    // Whether there is a line on screen to paste into. Since the line lives in
    // the conversation, that is the same question as whether the conversation is
    // drawn as itself: put away, or stood down by a view that renders the words
    // itself, a paste is sent rather than dropped into a box nobody can see.
    textInputOpen: chatShown,
    sendText,
    pasteIntoTextInput,
  });

  // Escape puts the conversation away, in every state it is up in. It defers to
  // whoever already handled it, so clearing a half-typed line closes the line and
  // leaves the conversation up.
  //
  // Through `onHostKey` rather than a `window` listener, because a key pressed in
  // the panel is chrome's and stops at the document — one node short of the window
  // (`lib/keyboard.ts`). It also means a view holding the focus keeps its own
  // Escape.
  useEffect(() => {
    if (!chatShown) return;
    return onHostKey((event) => {
      if (event.key === "Escape" && !event.defaultPrevented) setTextChannel(false);
    });
  }, [chatShown, setTextChannel]);

  // The other dismissal: a press on what is behind the panel. Armed only while
  // there IS something behind it — a view. With nothing on the stage the press
  // lands on bare paper, and the room is not a thing anyone reaches past the
  // conversation for; a click to focus the window would put the record away.
  // (Before the panel became the conversation's only box, this was the same
  // condition by accident: the box existed only while a view did.)
  //
  // Host chrome is never "behind": the controls cluster, whose toggle would
  // otherwise close on the press and reopen on the click, and the views band that
  // cluster opens — picking a view there is how a view gets on the stage, and
  // putting one up must not take the conversation down with it. The line being
  // written needs no exclusion of its own: it is inside the panel, so `contains`
  // already covers it.
  useEffect(() => {
    if (!chatShown || views.length === 0) return;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Element | null;
      if (popoverRef.current?.contains(target as Node)) return;
      if (target?.closest?.(".hi-channels, .hi-views-band")) return;
      setTextChannel(false);
    };
    window.addEventListener("pointerdown", onPointerDown, true);
    return () => window.removeEventListener("pointerdown", onPointerDown, true);
  }, [chatShown, views.length, setTextChannel]);

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

  return (
    <div
      className="hi-root"
      data-file-drop={handoff.feedback?.state}
      onDragEnterCapture={handoff.onFileDragEnter}
      onDragOverCapture={handoff.onFileDragOver}
      onDragLeaveCapture={handoff.onFileDragLeave}
      onDropCapture={handoff.onFileDrop}
    >
      <div className="hi-plane hi-plane--ground">
        <Presence state={presence.state} demote={layout.demote} />
        <Atmosphere />
      </div>

      {/* The agent's plane. Its internal order is the wire's array order — content
          first, the condition notice over it — so it needs no z-index at all. */}
      <div className="hi-plane hi-plane--view">
        <ViewSlot />
      </div>

      {/* The person's plane. Transparent to the pointer as a whole; each surface
          on it takes its own events back, so the gaps between them stay clickable
          down to the view underneath. */}
      <div className="hi-plane hi-plane--cover">
        <CameraPreview stream={ch.visionStream} pip={layout.camera === "pip"} />

        {/* PINNED — the conversation. One list, mounted once, standing in ONE box:
            the same panel in the same corner at the same measure whether or not
            the agent has a view up, so nothing it shows moves the thing being
            read. `data-shown` is a visibility flip and not a branch, so the
            scroller keeps its position and its already-fetched scrollback across
            every transition. */}
        <div
          ref={popoverRef}
          className="hi-stage"
          data-shown={chatShown ? "true" : "false"}
          aria-hidden={chatShown ? undefined : true}
        >
          {/* The one activity the person is shown, and it is shown inside the
              conversation because that is what it is about — see `ui/Chat.tsx`. */}
          <Chat
            messages={messages}
            interim={interim}
            typing={presence.state === "typing"}
            onLoadOlder={loadOlder}
          >
            <Composer
              onSend={sendText}
              shown={chatShown}
              pastedText={pastedInputText}
              onOpen={openConversation}
            />
          </Chat>
        </div>

        {layout.conversation === "pill" && (
          // The dock steps past the camera pip (bottom-left) so the bottom
          // bar's three zones — pip · captions · controls — never overlap, and
          // that step is keyed in the stylesheet on the pip *being on screen*
          // (`:has(.hi-selfview--pip)`), the same way the input line does it.
          // It used to be this element's `data-camera`, read off the layout
          // pass — which says `pip` whenever a view leads, camera or no camera.
          // With the camera off the words then stepped past nothing and sat
          // ~400px right of centre.
          <div
            className="hi-captions"
            data-shown={captionShown ? "true" : "false"}
            aria-hidden={captionShown ? undefined : true}
          >
            <SpeechText items={lastSpoken} />
          </div>
        )}

        {/* The views band, directly above the controls that open it. Short by
            design: it is opened to compare what is up with something that was, and a
            tall sheet would cover the thing being compared. */}
        {bandOpen && <ViewsBand onDismiss={() => setBandOpen(false)} />}

        {/* The lower cluster is controls and only controls: every channel, always
            available, nothing that merely reports. Managed energy is represented
            only by the gate-owned full-screen view. */}
        <ChannelControls
          audioOn={ch.audioInput}
          onToggleAudio={ch.toggleAudio}
          audioError={ch.audioError}
          videoOn={ch.videoInput}
          onToggleVideo={ch.toggleVideo}
          videoError={ch.videoError}
          textOn={ch.text}
          onToggleText={() => setTextChannel(!ch.text)}
          voiceOn={ch.audioOutput}
          onToggleVoice={ch.toggleAudioOutput}
          onPickFiles={() => fileInputRef.current?.click()}
          fileSending={handoff.isSending}
          onCloseViews={clear}
          viewsOpen={bandOpen}
          onToggleViews={() => setBandOpen((open) => !open)}
        />

        <HandoffOverlay
          feedback={handoff.feedback}
          onRetry={handoff.retry}
          onDismiss={handoff.dismiss}
        />
      </div>

      <input
        ref={fileInputRef}
        type="file"
        multiple
        hidden
        onChange={(event) => {
          const files = event.target.files;
          if (files?.length) void handoff.sendFiles(Array.from(files));
          event.target.value = "";
        }}
      />
    </div>
  );
}
