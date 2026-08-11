import { useCallback, useRef, useState } from "react";
import { usePresence, useMessages, useChannels, useSendText } from "../core";
import { useViews } from "../core/views";
import { stage as composeStage } from "../core/layout";
import { useHandoff } from "../hooks/useHandoff";
import { useViewportWidth } from "../hooks/useViewport";
import { Atmosphere } from "./Atmosphere";
import { Presence } from "./Presence";
import { Chat } from "./Chat";
import { SpeechText, type SpeechItem } from "./SpeechText";
import { ViewSlot } from "./ViewSlot";
import { KeyboardFallback } from "./KeyboardFallback";
import { ChannelControls } from "./ChannelControls";
import { CameraPreview } from "./CameraPreview";
import { HandoffOverlay } from "./HandoffOverlay";

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
 * *geometry* — rail or pill, fill or pip — and never who covers whom, which is
 * static. It also decides placement, **never lifecycle**: `<Chat>` and
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
  const sendText = useSendText();
  const { views, clear } = useViews();
  const width = useViewportWidth();
  // The person's own collapse. A window preference, deliberately not server state:
  // it says how THIS window draws the conversation, not what the agent expressed,
  // and a phone with no room for a rail must not collapse a desktop's.
  const [railCollapsed, setRailCollapsed] = useState(false);
  const [pastedInputText, setPastedInputText] = useState<{ id: number; text: string } | null>(null);
  const pasteIdRef = useRef(0);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const pasteIntoTextInput = useCallback((text: string) => {
    pasteIdRef.current += 1;
    setPastedInputText({ id: pasteIdRef.current, text });
  }, []);
  const handoff = useHandoff({
    textInputOpen: ch.textInput,
    sendText,
    pasteIntoTextInput,
  });

  const top = views[views.length - 1];
  const layout = composeStage({
    content: views.length > 0,
    camera: !!ch.visionStream,
    ownsConversation: top?.traits?.owns_conversation ?? false,
    width,
    collapsed: railCollapsed,
  });

  // The conversation is shown as itself in two of its four states; the pill is a
  // different rendering of the same list, and `hidden` is a view having taken the
  // words over. `<Chat>` stays mounted through all of them.
  const chatShown = layout.conversation === "stage" || layout.conversation === "rail";

  // The pill shows the newest thing said, or the line currently being recognized
  // if one is in flight — the same tail the chat ends on.
  const newest = messages[messages.length - 1];
  const lastSpoken: SpeechItem[] = interim
    ? [{ id: -1, text: interim, speaker: "user", pending: true }]
    : newest
      ? [{ id: 0, text: newest.text, speaker: newest.role === "user" ? "user" : "agent" }]
      : [];

  return (
    <div
      className="hi-root"
      data-rail={layout.rail ? "open" : undefined}
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

        {/* PINNED — the conversation. One list, mounted once, drawn three ways:
            the whole frame when nothing else is up, a rail beside a view, and
            hidden behind the pill when collapsed. `data-shown` is a visibility
            flip and not a branch, so the scroller keeps its position and its
            already-fetched scrollback across every transition. */}
        <div
          className={layout.rail ? "hi-stage hi-stage--rail" : "hi-stage"}
          data-shown={chatShown ? "true" : "false"}
          aria-hidden={chatShown ? undefined : true}
        >
          <Chat messages={messages} interim={interim} onLoadOlder={loadOlder} />
        </div>

        {layout.conversation === "pill" && (
          <div
            className="hi-stage hi-stage--captions"
            // Tells the dock to pull its left edge past the camera pip
            // (bottom-left) so the bottom bar's three zones — pip · captions ·
            // controls — never overlap.
            data-camera={layout.camera === "pip" ? "pip" : undefined}
          >
            <SpeechText items={lastSpoken} />
          </div>
        )}

        {/* The lower cluster starts with the activity status, then keeps every
            channel available. Managed energy is represented only by the
            gate-owned full-screen view. */}
        <ChannelControls
          activity={presence.state}
          audioOn={ch.audioInput}
          onToggleAudio={ch.toggleAudio}
          audioError={ch.audioError}
          videoOn={ch.videoInput}
          onToggleVideo={ch.toggleVideo}
          videoError={ch.videoError}
          textOn={ch.textInput}
          onToggleText={() => ch.setTextChannel(!ch.textInput)}
          voiceOn={ch.audioOutput}
          onToggleVoice={ch.toggleAudioOutput}
          onPickFiles={() => fileInputRef.current?.click()}
          fileSending={handoff.isSending}
          onCloseViews={clear}
          conversation={layout.conversation}
          onToggleConversation={() => setRailCollapsed((collapsed) => !collapsed)}
        />

        <KeyboardFallback
          onSend={sendText}
          open={ch.textInput}
          pastedText={pastedInputText}
          onOpen={() => ch.setTextChannel(true)}
          onClose={() => ch.setTextChannel(false)}
          anchor={layout.input}
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
