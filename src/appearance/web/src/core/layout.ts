// The compositor: one pure pass that divides the stage between the agent's view
// and the host's own surfaces — the conversation and the camera self-view.
// Deterministic (no solver, no async coordinator).
//
// It decides *geometry*, and only geometry. Who covers whom is not computed here
// and never was a per-state question: every surface belongs to one of three
// planes — ground, view, cover — for its whole life, declared once in the
// stylesheet. See `docs/arch/stage.md`. Before that split, `floorLayout` decided
// docked/pip/hidden while the stylesheet decided covering, and the two disagreed:
// the pass called the conversation a participant while `.hi-stage { z-index: 2 }`
// parked it under the view plane at 50, which is why showing anything collapsed
// the conversation to a caption.
//
// It also decides *placement, never lifecycle*: the chat and the camera <video>
// are mounted once by the shell, above the swappable view slot, and this pass only
// flips their props and classes. Re-mounting the camera re-acquires the device and
// blacks out the feed; re-mounting the chat throws away the scroll position and
// every page of scrollback already fetched.

/** How the conversation is presented. One surface, four states — not four surfaces.
 *
 * - `stage`   — it fills the frame: the default face.
 * - `popover` — a panel over the agent's view, in the corner the controls hold,
 *               carrying the full scrollback and, in its foot, the line being
 *               written.
 * - `pill`    — collapsed to the newest line, floating over the view. A caption
 *               and not a shelf: it shows while the line is worth reading and
 *               then fades, because what it holds is a copy of something the
 *               list behind it keeps. The dwell is the shell's (`ui/caption.ts`);
 *               this pass says where the pill goes, never how long it stays.
 * - `hidden`  — a view is rendering the words itself.
 *
 * Putting it away puts *all* of it away, the line included, because the line is
 * inside it (`ui/Composer.tsx`). That is what lets one control own the surface —
 * see `docs/arch/stage.md`.
 *
 * The popover replaced a rail: a column the view had to inset past for as long as
 * the conversation was open. The view is what is being read while it is up, and a
 * permanent third of the window is a steep price for a surface that is idle most
 * of that time — overlaying on demand costs the view nothing while it is closed.
 * See `docs/arch/stage.md`.
 */
export type Conversation = "stage" | "popover" | "pill" | "hidden";

/** The self-view fills the frame when nothing else is on it, else a corner pip. */
export type Camera = "fill" | "pip";

export interface StageInput {
  /** The agent has a view up. */
  content: boolean;
  /** The camera channel is live. */
  camera: boolean;
  /** The top-most view renders the conversation itself. */
  ownsConversation: boolean;
  /** The person put the conversation away — the text channel's own on/off. A
   * window preference, never server state, so a phone cannot collapse a
   * desktop's. */
  collapsed: boolean;
}

export interface Stage {
  conversation: Conversation;
  camera: Camera;
  /** How far to fade the presence while something leads. */
  demote: number;
}

/**
 * Arrange the stage.
 *
 * The conversation yields the *frame* to whatever the agent puts up, and never the
 * *screen*: it becomes a panel over the view rather than degrading, because it is
 * the one surface that keeps — see `docs/arch/text-transcript.md`. It collapses to
 * the pill only when the person asks, and disappears only when a view has taken
 * over rendering the words.
 *
 * **Asking works with nothing on the stage too**, and that is a reversal: the pass
 * used to ignore `collapsed` unless something else was up, on the reasoning that
 * hiding the only thing on screen is not a thing to offer. Two changes make it
 * one. The pill is timed now, so what is left behind is the room and a line that
 * fades, not a shelf. And the same control that puts the conversation away is the
 * text channel's — refusing here would leave the one channel whose button does
 * nothing in the state where it is the whole face. Any printable key brings it
 * back (`ui/Composer.tsx`).
 *
 * There is no width threshold any more. The rail needed one — below ~760px a
 * window cannot be split into two usable columns — but a popover splits nothing,
 * so the same gesture works at every size and a narrow window gets the whole
 * scrollback rather than only the newest line.
 *
 * The camera counts as occupying the stage for the same reason a view does: the
 * self-view is full-bleed, so leaving the conversation on the stage over it would
 * cover the thing the person turned on to see.
 */
export function stage(input: StageInput): Stage {
  const occupied = input.content || input.camera;

  const conversation: Conversation =
    input.content && input.ownsConversation
      ? "hidden"
      : input.collapsed
        ? "pill"
        : occupied
          ? "popover"
          : "stage";

  return {
    conversation,
    camera: input.content ? "pip" : "fill",
    demote: input.content ? 0.72 : 0,
  };
}
