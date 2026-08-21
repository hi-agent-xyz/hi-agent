import type { ActivityState } from "./Presence";
import { StatusButton } from "./StatusButton";

interface ChannelControlsProps {
  /** The agent's current short-lived activity. */
  activity: ActivityState;
  /** Whether the mic (audio input) channel is live. */
  audioOn: boolean;
  /** Flip the audio channel on/off. */
  onToggleAudio: () => void;
  /** Surfaced if the last attempt to turn audio on failed. */
  audioError?: string | null;
  /** Whether the camera (vision input) channel is live. */
  videoOn: boolean;
  /** Flip the vision channel on/off. */
  onToggleVideo: () => void;
  /** Surfaced if the last attempt to turn vision on failed. */
  videoError?: string | null;
  /** Whether the text channel is on — the conversation and its line, together. */
  textOn: boolean;
  /** Show the conversation, or put it away. */
  onToggleText: () => void;
  /** Whether the agent's voice (audio output) is on. */
  voiceOn: boolean;
  /** Mute/unmute the agent's voice. */
  onToggleVoice: () => void;
  /** Open the system file picker as the keyboard/touch alternative to dropping. */
  onPickFiles: () => void;
  /** A file batch is currently being uploaded. */
  fileSending: boolean;
  /** Close the agent's view — the conversation takes the screen back. */
  onCloseViews: () => void;
  /** Whether the views band is open. */
  viewsOpen: boolean;
  /** Open/close the views band. */
  onToggleViews: () => void;
}

/**
 * The channel controls — a quiet cluster in the corner. The input channels (mic,
 * camera, text) and the output channel (voice) are all independent: each can be
 * on or off at any time, and they don't conflict. Every control is always
 * present (no state-gated chrome) so a user who can't (or won't) use a given
 * channel still has a clear way in or out; the trailing reset closes the agent's
 * view, which gives the conversation the screen back.
 * Order: status · mic · speaker · text · attach · camera · views · reset, and every one
 * of them is always there — the cluster has no item that comes and goes.
 *
 * **One control per channel, and the text one owns the whole of its channel.**
 * There used to be two: a keyboard button that showed the input line and a
 * separate conversation button that opened the popover over a view — a control
 * that had to appear and disappear, because there is nothing to open a popover
 * over unless something else is on the stage. Now that the line is written inside
 * the conversation rather than beside it (`ui/Composer.tsx`), the two were toggling
 * halves of one surface, and one press moves all of it: the record, the scrollback
 * and the line. What is left is what the cluster was always for — mic, speaker,
 * text, camera, one apiece.
 *
 * **Nothing here signals a raise, because a raise is not something to be signalled about.**
 * There was a return-to-live button, and then a dot on the views control in its place, both
 * standing in for a window that had gone back and would not follow the agent onto the
 * screen. A raise takes the window with it now (`docs/arch/stage.md`), so there is nothing
 * left to stand in for: what the agent put up is what is up, and the band is where going
 * back lives.
 */
export function ChannelControls({
  activity,
  audioOn,
  onToggleAudio,
  audioError,
  videoOn,
  onToggleVideo,
  videoError,
  textOn,
  onToggleText,
  voiceOn,
  onToggleVoice,
  onPickFiles,
  fileSending,
  onCloseViews,
  viewsOpen,
  onToggleViews,
}: ChannelControlsProps) {
  return (
    <div className="hi-channels" role="group" aria-label="agent status and channels">
      <StatusButton activity={activity} />
      <span className="hi-channel-separator" aria-hidden="true" />

      <button
        type="button"
        className={`hi-channel${audioOn ? " is-on" : ""}`}
        onClick={onToggleAudio}
        title={audioError ?? (audioOn ? "mic on — tap to mute" : "mic off — tap to listen")}
        aria-pressed={audioOn}
        aria-label={audioOn ? "turn microphone off" : "turn microphone on"}
      >
        <MicGlyph muted={!audioOn} />
      </button>

      <button
        type="button"
        className={`hi-channel${voiceOn ? " is-on" : ""}`}
        onClick={onToggleVoice}
        title={voiceOn ? "voice on — tap to mute" : "voice muted — tap to unmute"}
        aria-pressed={voiceOn}
        aria-label={voiceOn ? "mute the agent's voice" : "unmute the agent's voice"}
      >
        <SpeakerGlyph muted={!voiceOn} />
      </button>

      <button
        type="button"
        className={`hi-channel${textOn ? " is-on" : ""}`}
        onClick={onToggleText}
        title={textOn ? "conversation — tap to put away" : "conversation — tap to open"}
        aria-pressed={textOn}
        aria-expanded={textOn}
        aria-label={textOn ? "put the conversation away" : "open the conversation"}
      >
        <KeyboardGlyph />
      </button>

      <button
        type="button"
        className="hi-channel"
        onClick={onPickFiles}
        disabled={fileSending}
        title={fileSending ? "sending files" : "attach files"}
        aria-label={fileSending ? "files are uploading" : "attach files"}
      >
        <AttachGlyph />
      </button>

      <button
        type="button"
        className={`hi-channel${videoOn ? " is-on" : ""}`}
        onClick={onToggleVideo}
        title={videoError ?? (videoOn ? "camera on — tap to turn off" : "camera off — tap to turn on")}
        aria-pressed={videoOn}
        aria-label={videoOn ? "turn camera off" : "turn camera on"}
      >
        <CamGlyph off={!videoOn} />
      </button>

      <button
        type="button"
        className={`hi-channel${viewsOpen ? " is-on" : ""}`}
        onClick={onToggleViews}
        title="views — what has been shown, and where you can go"
        aria-pressed={viewsOpen}
        aria-expanded={viewsOpen}
        aria-label="views"
      >
        <ViewsGlyph />
      </button>

      <button
        type="button"
        className="hi-channel"
        onClick={onCloseViews}
        title="close the view — the conversation takes the screen back"
        aria-label="close the view"
      >
        <ResetGlyph />
      </button>
    </div>
  );
}

/** Tiles: the band's contents, not any one view. */
function ViewsGlyph() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" aria-hidden="true">
      <rect x="3" y="4.5" width="7.5" height="6" rx="1.5" stroke="currentColor" strokeWidth="1.6" />
      <rect x="13.5" y="4.5" width="7.5" height="6" rx="1.5" stroke="currentColor" strokeWidth="1.6" />
      <rect x="3" y="13.5" width="7.5" height="6" rx="1.5" stroke="currentColor" strokeWidth="1.6" />
      <rect x="13.5" y="13.5" width="7.5" height="6" rx="1.5" stroke="currentColor" strokeWidth="1.6" />
    </svg>
  );
}

function MicGlyph({ muted }: { muted: boolean }) {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" aria-hidden="true">
      <rect x="9" y="3" width="6" height="11" rx="3" stroke="currentColor" strokeWidth="1.6" />
      <path
        d="M6 11a6 6 0 0 0 12 0M12 17v3"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
      />
      {muted && (
        <line x1="4" y1="4" x2="20" y2="20" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
      )}
    </svg>
  );
}

function CamGlyph({ off }: { off: boolean }) {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" aria-hidden="true">
      <rect x="3" y="6" width="13" height="12" rx="2.5" stroke="currentColor" strokeWidth="1.6" />
      <path d="M16 10l5-3v10l-5-3" stroke="currentColor" strokeWidth="1.6" strokeLinejoin="round" />
      {off && (
        <line x1="4" y1="4" x2="20" y2="20" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
      )}
    </svg>
  );
}

function SpeakerGlyph({ muted }: { muted: boolean }) {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" aria-hidden="true">
      <path
        d="M4 9v6h3l5 4V5L7 9H4z"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinejoin="round"
      />
      {muted ? (
        <path d="M16 9l5 6M21 9l-5 6" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
      ) : (
        <path
          d="M16 9a4 4 0 0 1 0 6M18.5 6.5a7.5 7.5 0 0 1 0 11"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinecap="round"
        />
      )}
    </svg>
  );
}

function KeyboardGlyph() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" aria-hidden="true">
      <rect x="3" y="6" width="18" height="12" rx="2" stroke="currentColor" strokeWidth="1.6" />
      <path
        d="M7 10h.01M11 10h.01M15 10h.01M8 14h8"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
      />
    </svg>
  );
}

function AttachGlyph() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" aria-hidden="true">
      <path
        d="m9 12 5.8-5.8a3 3 0 1 1 4.2 4.2l-7.5 7.5a5 5 0 0 1-7.1-7.1L12 3.2"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function ResetGlyph() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" aria-hidden="true">
      <rect x="4" y="5" width="16" height="14" rx="2.5" stroke="currentColor" strokeWidth="1.6" />
      <path d="M9 10l6 4M15 10l-6 4" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </svg>
  );
}
