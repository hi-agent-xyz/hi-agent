interface ChannelControlsProps {
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
  /** Whether the agent's voice (audio output) is on. */
  voiceOn: boolean;
  /** Mute/unmute the agent's voice. */
  onToggleVoice: () => void;
  /** Close the agent's view — the room takes the screen back. */
  onCloseViews: () => void;
}

/**
 * The channel controls — one row in the panel's head.
 *
 * The input channels (mic, camera) and the output channel (voice) are all
 * independent: each can be on or off at any time, and they don't conflict. The
 * trailing reset closes the agent's view, which gives the room the screen back.
 * Order: mic · speaker · camera · reset.
 *
 * **They live in exactly one place, and it is the panel's head.** There was a
 * corner cluster floating over the room and a `bar` variant of the same row
 * standing in a pushed page's head, chosen by shape — two placements, one of
 * which drew the text control as a back chevron so that the row could keep
 * claiming it had no item that came and went. Both are gone with the room's
 * chrome (`docs/arch/stage.md` § *The panel, and the axis it runs on*): the room
 * is now whatever the agent put in it and nothing else, and this row is visible
 * at every stop and on every tab of the panel that replaced the corner.
 *
 * **What that costs, named rather than hidden: turning the mic on is two acts.**
 * The older rule was that every channel is always present and one press away
 * wherever you are, and on a phone the mic is the main way in — *"go back to the
 * room to unmute"* was written down as a tax not worth paying, and it is the tax
 * now being paid. It is paid because the rule protects against a channel becoming
 * *unreachable*, and one gesture is not unreachable. The narrower rule that
 * replaces it is the one this row still keeps: **no channel is behind a mode.**
 * Every control is here, in one row, at every stop and on every tab — not on a
 * settings tab, not behind a disclosure.
 *
 * **There is no text control any more.** It opened and closed the conversation,
 * and the panel is what does that now — from inside the panel it would have been
 * a button meaning "close the thing you are looking at", which is the edge's job.
 * There is no views control either: the views navigator is a tab beside the
 * messages rather than a band this row opened.
 *
 * **Handing over a file is not a channel, so it is not in the row.** There was an
 * attach button that opened the system file picker, sitting between the text
 * control and the camera as though files were a fifth channel to turn on. They are
 * not: a file is a handed artifact, and the window already takes one dropped or
 * pasted anywhere on it (`hooks/useHandoff`), so the button was a second door onto
 * something that works everywhere — the same reason `factory/upload` was deleted.
 * The cost that removal named — a touch device has neither gesture, so on a phone
 * there was no way to hand over a file at all — is paid at the line being written
 * (`ui/Composer.tsx`), where a file is an artifact arriving in a conversation
 * rather than a channel to turn on. `/api/handoff` and `/up/<token>` are still
 * standing and still have no caller.
 *
 * **Every control here does something, and none of them reports.** The row used to
 * open with a read-only status disc that drew whichever of six activities the agent
 * was in — a button that could not be pressed, sitting in a row of buttons. Five of
 * those six states are the agent going about its own business and are nobody's cue
 * to act, so they are drawn nowhere now; the sixth, a reply being composed, is a
 * thing said in a conversation and is drawn in the conversation (`ui/Chat.tsx`).
 *
 * **Nothing here signals a show, because a show is not something to be signalled
 * about.** There was a return-to-live button, and then a dot on the views control in
 * its place, both standing in for a window that had gone back and would not follow
 * the agent onto the screen. A show takes the window with it now
 * (`docs/arch/stage.md`), so there is nothing left to stand in for.
 */
export function ChannelControls({
  audioOn,
  onToggleAudio,
  audioError,
  videoOn,
  onToggleVideo,
  videoError,
  voiceOn,
  onToggleVoice,
  onCloseViews,
}: ChannelControlsProps) {
  // A channel that refused to open has to say so where it can be read. `title`
  // is the desktop half of that and nothing at all on a phone, where a tap that
  // leaves the button exactly as it was is the entire report — which is how a
  // mic that never opened read as a button that ignored presses.
  const note = audioError ?? videoError ?? null;

  return (
    <div className="hi-channels" role="group" aria-label="channels">
      {note && (
        <p className="hi-channel-note" role="status">
          {note}
        </p>
      )}

      <button
        type="button"
        className={`hi-channel${audioOn ? " is-on" : ""}${audioError ? " is-error" : ""}`}
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
        className={`hi-channel${videoOn ? " is-on" : ""}${videoError ? " is-error" : ""}`}
        onClick={onToggleVideo}
        title={videoError ?? (videoOn ? "camera on — tap to turn off" : "camera off — tap to turn on")}
        aria-pressed={videoOn}
        aria-label={videoOn ? "turn camera off" : "turn camera on"}
      >
        <CamGlyph off={!videoOn} />
      </button>

      <button
        type="button"
        className="hi-channel"
        onClick={onCloseViews}
        title="close the view — the room takes the screen back"
        aria-label="close the view"
      >
        <ResetGlyph />
      </button>
    </div>
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



function ResetGlyph() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" aria-hidden="true">
      <rect x="4" y="5" width="16" height="14" rx="2.5" stroke="currentColor" strokeWidth="1.6" />
      <path d="M9 10l6 4M15 10l-6 4" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </svg>
  );
}
