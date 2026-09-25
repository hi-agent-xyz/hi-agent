interface PanelButtonProps {
  /** Whether the panel is away. The button is the way in, so it is only drawn
   * while there is somewhere to come in from. */
  shown: boolean;
  /** Something was said while the panel was away that the person has not opened
   * the panel to read. */
  unread: boolean;
  /** The mic is open. Said here because with the panel away this button is the
   * only thing on screen that belongs to the person. */
  listening: boolean;
  onOpen: () => void;
}

/**
 * The way into the panel: one round button in the room's bottom-right corner.
 *
 * **It replaced an edge.** The room used to keep no controls at all, and the way in
 * was a twenty-point strip on the window's right-hand side. On Android both of the
 * screen's side edges are the system's Back gesture, so a strip there fought the
 * platform for every touch; on a desktop it was a hairline nobody found without
 * already being at the edge; and on every shape a first-time person had nothing on
 * screen telling them there was anything to open (`docs/arch/stage.md` § *The room
 * keeps one button*).
 *
 * **One press and the panel is open** — no menu of smaller buttons between the
 * press and the thing, because the panel is already where every control is.
 *
 * **It carries state, not only an entrance.** A dot says something was said while
 * the panel was away; a ring says the mic is open. The second is the reason a
 * zero-button room was hard to defend: a person looking at a view had no way to
 * see whether they were being listened to.
 *
 * It stands in the strip the bottom chrome has always held (`--hi-chrome-bottom`),
 * so a view that keeps its content clear of that token keeps it clear of this.
 */
export function PanelButton({ shown, unread, listening, onOpen }: PanelButtonProps) {
  return (
    <button
      type="button"
      className="hi-panel-button"
      data-shown={shown ? "true" : "false"}
      data-listening={listening ? "true" : undefined}
      onClick={onOpen}
      // Gone rather than merely transparent while the panel is out, so it can be
      // neither tabbed to nor read out behind the panel it opened.
      aria-hidden={shown ? undefined : true}
      tabIndex={shown ? undefined : -1}
      aria-label={unread ? "open the conversation — new message" : "open the conversation"}
      title="open the conversation"
    >
      <ChatGlyph />
      {unread && <span className="hi-panel-button-dot" aria-hidden="true" />}
    </button>
  );
}

function ChatGlyph() {
  return (
    <svg viewBox="0 0 24 24" width="22" height="22" fill="none" aria-hidden="true">
      <path
        d="M5 5.5h14a1.5 1.5 0 0 1 1.5 1.5v8.5A1.5 1.5 0 0 1 19 17h-8l-4.5 3.5V17H5a1.5 1.5 0 0 1-1.5-1.5V7A1.5 1.5 0 0 1 5 5.5z"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinejoin="round"
      />
    </svg>
  );
}
