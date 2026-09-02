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
  /** Whether the text channel is on — the conversation and its line, together. */
  textOn: boolean;
  /** Show the conversation, or put it away. */
  onToggleText: () => void;
  /** Whether the agent's voice (audio output) is on. */
  voiceOn: boolean;
  /** Mute/unmute the agent's voice. */
  onToggleVoice: () => void;
  /** Close the agent's view — the conversation takes the screen back. */
  onCloseViews: () => void;
  /** Whether the views band is open. */
  viewsOpen: boolean;
  /** Open/close the views band. */
  onToggleViews: () => void;
  /** Draw the cluster as the head bar of a pushed page rather than as a corner
   * cluster floating over the room — the phone shape, where the conversation is
   * a page and this row is its nav bar. Placement only: the same six controls,
   * re-laid. See the note on the text control below for the one that changes
   * shape with it. */
  bar?: boolean;
}

/**
 * The channel controls — a quiet cluster in the corner. The input channels (mic,
 * camera, text) and the output channel (voice) are all independent: each can be
 * on or off at any time, and they don't conflict. Every control is always
 * present (no state-gated chrome) so a user who can't (or won't) use a given
 * channel still has a clear way in or out; the trailing reset closes the agent's
 * view, which gives the conversation the screen back.
 * Order: mic · speaker · text · camera · views · reset, and every one of them is
 * always there — the cluster has no item that comes and goes.
 *
 * **Handing over a file is not a channel, so it is not in the cluster.** There was
 * an attach button that opened the system file picker, sitting between the text
 * control and the camera as though files were a fifth channel to turn on. They are
 * not: a file is a handed artifact, and the window already takes one dropped or
 * pasted anywhere on it (`hooks/useHandoff`), so the button was a second door onto
 * something that works everywhere — the same reason `factory/upload` was deleted.
 * The cost is named rather than hidden: a touch device has no drop and no paste, so
 * on a phone there is now no way to hand over a file at all. `/api/handoff` and
 * `/up/<token>` are still standing and still have no caller.
 *
 * **Every control here does something, and none of them reports.** The cluster
 * used to open with a read-only status disc that drew whichever of six activities
 * the agent was in — a button that could not be pressed, sitting in a row of
 * buttons. Five of those six states are the agent going about its own business
 * and are nobody's cue to act, so they are drawn nowhere now; the sixth, a reply
 * being composed, is a thing said in a conversation and is drawn in the
 * conversation (`ui/Chat.tsx`).
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
 * **On the phone the same cluster is the page's bar, and the text control is the
 * back chevron.** The conversation there is a page pushed onto the stack rather
 * than a panel in a corner (`docs/arch/stage.md`), so the row rides in its head
 * instead of floating over the room — and the control that owns the text channel
 * is, in that position, the control that takes the page back off the stack. It is
 * drawn as a chevron because that is what it does from there; it is the same
 * button, doing the same one thing it has always done, which is why the cluster
 * still has no item that comes and goes. A second dismissal button beside a
 * keyboard glyph that meant "close this" would have been two doors onto one act.
 *
 * **Nothing here signals a show, because a show is not something to be signalled about.**
 * There was a return-to-live button, and then a dot on the views control in its place, both
 * standing in for a window that had gone back and would not follow the agent onto the
 * screen. A show takes the window with it now (`docs/arch/stage.md`), so there is nothing
 * left to stand in for: what the agent put up is what is up, and the band is where going
 * back lives.
 */
export function ChannelControls({
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
  onCloseViews,
  viewsOpen,
  onToggleViews,
  bar = false,
}: ChannelControlsProps) {
  // A channel that refused to open has to say so where it can be read. `title`
  // is the desktop half of that and nothing at all on a phone, where a tap that
  // leaves the button exactly as it was is the entire report — which is how a
  // mic that never opened read as a button that ignored presses.
  const note = audioError ?? videoError ?? null;

  return (
    <div className={`hi-channels${bar ? " hi-channels--bar" : ""}`} role="group" aria-label="channels">
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
        className={`hi-channel${textOn && !bar ? " is-on" : ""}${bar ? " hi-channel--back" : ""}`}
        onClick={onToggleText}
        title={textOn ? "conversation — tap to put away" : "conversation — tap to open"}
        aria-pressed={bar ? undefined : textOn}
        aria-expanded={textOn}
        aria-label={textOn ? "put the conversation away" : "open the conversation"}
      >
        {/* In the page's bar this button is the way back, so it is drawn as the
            way back — and it drops the on-state with the glyph. The tint means
            "this channel is open"; on a chevron in the bar of the very page that
            channel opened, it says nothing and reads as a back button that has
            somehow been selected. `aria-pressed` goes for the same reason: a
            chevron announced as a pressed toggle is a lie about what pressing it
            does, and the label already says what it does. */}
        {bar ? <BackGlyph /> : <KeyboardGlyph />}
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

/** The way back off the stack — the text control's glyph while the cluster is a
 * pushed page's bar, and the views page's own back button (`ui/ViewsBand.tsx`).
 * A bare chevron, at the weight the platform draws one. Exported so there is one
 * chevron in the application rather than one per page that has a way back. */
export function BackGlyph() {
  return (
    <svg viewBox="0 0 24 24" width="20" height="20" fill="none" aria-hidden="true">
      <path
        d="M15 5l-7 7 7 7"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
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

function ResetGlyph() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" aria-hidden="true">
      <rect x="4" y="5" width="16" height="14" rx="2.5" stroke="currentColor" strokeWidth="1.6" />
      <path d="M9 10l6 4M15 10l-6 4" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </svg>
  );
}
