import { useCallback, useRef, useState } from "react";
import { usePresence, useSpeech, useChannels, useSendText, useScene } from "../core";
import { useViews } from "../core/views";
import { floorLayout, CAPTIONS_ID, CAMERA_ID, type Participant } from "../core/layout";
import { useHandoff } from "../hooks/useHandoff";
import { Atmosphere } from "./Atmosphere";
import { Presence } from "./Presence";
import { SpeechText } from "./SpeechText";
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
 *   Atmosphere · Presence (the agent) · SpeechText (its words) · ViewSlot
 *   (agent-authored views) · the channel controls / input line.
 *
 * Placement is one job: every participant — the agent views, the live captions,
 * and the camera self-view — is laid out by a single `floorLayout` pass. But that
 * unifies *placement*, never *lifecycle*: the captions `<div>` and `<CameraPreview>`
 * are mounted ONCE here, above the swappable `ViewSlot`, and the layout only flips
 * their props/classes. They must never move into `ViewSlot` or a participant
 * `.map()` — re-mounting `<CameraPreview>` re-acquires the camera and blacks out
 * the feed.
 */
export function Shell() {
  const scene = useScene();
  const presence = usePresence();
  const sentences = useSpeech();
  const ch = useChannels();
  const sendText = useSendText();
  const { views, meta, clear } = useViews();
  const [pastedInputText, setPastedInputText] = useState<{ id: number; text: string } | null>(null);
  const pasteIdRef = useRef(0);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const pasteIntoTextInput = useCallback((text: string) => {
    pasteIdRef.current += 1;
    setPastedInputText({ id: pasteIdRef.current, text });
  }, []);
  const handoff = useHandoff({
    scene,
    textInputOpen: ch.textInput,
    sendText,
    pasteIntoTextInput,
  });

  // Everything on screen is a participant. Views carry their declared geometry
  // (wire-authoritative; a module-self-declared fallback fills in for inline
  // `source` views with no wire geometry). The captions are always a participant;
  // the camera joins only while its stream is live.
  const participants: Participant[] = [
    ...views.map((v) => ({
      id: v.id,
      kind: "view" as const,
      geometry: v.geometry ?? meta.get(v.id)?.geometry,
    })),
    { id: CAPTIONS_ID, kind: "captions" as const },
    ...(ch.visionStream ? [{ id: CAMERA_ID, kind: "camera" as const }] : []),
  ];
  const { demote, placements } = floorLayout(participants);

  const captions = placements.get(CAPTIONS_ID);
  const camera = placements.get(CAMERA_ID);
  const captionsDocked = captions?.docked ?? false;

  return (
    <div
      className="hi-root"
      data-file-drop={handoff.feedback?.state}
      onDragEnterCapture={handoff.onFileDragEnter}
      onDragOverCapture={handoff.onFileDragOver}
      onDragLeaveCapture={handoff.onFileDragLeave}
      onDropCapture={handoff.onFileDrop}
    >
      <Atmosphere />
      <Presence state={presence.state} demote={demote} />

      {/* PINNED participant — mounted once, here, across every layout. The layout
          only flips `pip` (fullscreen backdrop ↔ corner thumbnail); the same
          <video> stays mounted so the feed never re-attaches and blacks out. */}
      <CameraPreview stream={ch.visionStream} pip={camera?.pip ?? false} />

      {/* PINNED participant — the conversation's words. Docks as caption pills
          when something fills the stage behind them (a view or the camera), else
          sits centered as the lead. Hidden only when the topmost view renders the
          words itself. Stays at this mount site across every layout. */}
      {captions && !captions.hidden && (
        <div
          className={captionsDocked ? "hi-stage hi-stage--captions" : "hi-stage"}
          data-region={captionsDocked ? captions.region : undefined}
          // Tells the dock to pull its left edge past the camera pip (bottom-left)
          // so the bottom bar's three zones — pip · captions · controls — never overlap.
          data-camera={captionsDocked && camera?.pip ? "pip" : undefined}
        >
          <SpeechText items={captionsDocked ? sentences.slice(-1) : sentences} />
        </div>
      )}

      <ViewSlot placements={placements} />

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
      />
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
      <KeyboardFallback
        onSend={sendText}
        open={ch.textInput}
        pastedText={pastedInputText}
        onOpen={() => ch.setTextChannel(true)}
        onClose={() => ch.setTextChannel(false)}
      />
      <HandoffOverlay
        feedback={handoff.feedback}
        onRetry={handoff.retry}
        onDismiss={handoff.dismiss}
      />
    </div>
  );
}
