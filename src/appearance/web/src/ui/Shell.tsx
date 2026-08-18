import { useCallback, useEffect, useRef, useState } from "react";
import { usePresence, useMessages, useChannels, useSendText } from "../core";
import { useViews } from "../core/views";
import { stage as composeStage } from "../core/layout";
import { useHandoff } from "../hooks/useHandoff";
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
 * *geometry* — panel or pill, fill or pip — and never who covers whom, which is
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
  // The person's own collapse. A window preference, deliberately not server state:
  // it says how THIS window draws the conversation, not what the agent expressed,
  // and a second device must not put away the conversation on this one.
  const [collapsed, setCollapsed] = useState(false);
  const [pastedInputText, setPastedInputText] = useState<{ id: number; text: string } | null>(null);
  const pasteIdRef = useRef(0);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const popoverRef = useRef<HTMLDivElement | null>(null);
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
    collapsed,
  });

  // The conversation is shown as itself in two of its four states; the pill is a
  // different rendering of the same list, and `hidden` is a view having taken the
  // words over. `<Chat>` stays mounted through all of them.
  const chatShown = layout.conversation === "stage" || layout.conversation === "popover";
  const popover = layout.conversation === "popover";

  // A popover is dismissed by reaching past it: Escape, or a press on anything
  // behind it. Two exclusions, and both are the surface's own body rather than
  // "behind" it — the controls cluster (whose toggle would otherwise close on the
  // press and reopen on the click) and the input line, which is this panel's foot
  // standing in its own box. Escape defers to whoever already handled it, so
  // clearing a half-typed line closes the line and leaves the conversation up.
  useEffect(() => {
    if (!popover) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) setCollapsed(true);
    };
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Element | null;
      if (popoverRef.current?.contains(target as Node)) return;
      if (target?.closest?.(".hi-channels, .hi-kbd")) return;
      setCollapsed(true);
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("pointerdown", onPointerDown, true);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("pointerdown", onPointerDown, true);
    };
  }, [popover]);

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
            the whole frame when nothing else is up, a popover over a view, and
            hidden behind the pill when put away. `data-shown` is a visibility
            flip and not a branch, so the scroller keeps its position and its
            already-fetched scrollback across every transition. */}
        <div
          ref={popoverRef}
          className={popover ? "hi-stage hi-stage--popover" : "hi-stage"}
          data-shown={chatShown ? "true" : "false"}
          aria-hidden={chatShown ? undefined : true}
        >
          <Chat messages={messages} interim={interim} onLoadOlder={loadOlder} />
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
          <div className="hi-stage hi-stage--captions">
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
          onToggleConversation={() => setCollapsed((away) => !away)}
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
