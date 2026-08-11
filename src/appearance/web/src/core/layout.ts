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
 * - `stage` — it fills the frame: the default face.
 * - `rail`  — a column beside the agent's view, with its full scrollback and input.
 * - `pill`  — collapsed to the newest line, floating over the view.
 * - `hidden` — a view is rendering the words itself.
 */
export type Conversation = "stage" | "rail" | "pill" | "hidden";

/** The self-view fills the frame when nothing else is on it, else a corner pip. */
export type Camera = "fill" | "pip";

/** The input line follows the conversation — it is where the messages are. */
export type Input = "center" | "rail";

/** Below this window width there is no rail: a window this narrow cannot show two
 * things at once, and splitting it gives you two unusable ones. The conversation
 * is then the whole stage or the pill, and the person swaps between them. */
export const RAIL_MIN_WIDTH = 760;

export interface StageInput {
  /** The agent has a view up. */
  content: boolean;
  /** The camera channel is live. */
  camera: boolean;
  /** The top-most view renders the conversation itself. */
  ownsConversation: boolean;
  /** Window width, for the rail threshold. */
  width: number;
  /** The person collapsed the rail. A window preference — never server state, so
   * a phone with no room for a rail cannot collapse a desktop's. */
  collapsed: boolean;
}

export interface Stage {
  conversation: Conversation;
  camera: Camera;
  input: Input;
  /** How far to fade the presence while something leads. */
  demote: number;
  /** Convenience for the shell: the view frame must inset past the rail. */
  rail: boolean;
}

/**
 * Arrange the stage.
 *
 * The conversation yields the *frame* to whatever the agent puts up, and never the
 * *screen*: it narrows to a rail rather than collapsing, because it is the one
 * surface that keeps — see `docs/arch/text-transcript.md`. It collapses to the pill
 * only when the person asks or the window has no room, and disappears only when a
 * view has taken over rendering the words.
 *
 * The camera counts as occupying the stage for the same reason a view does: the
 * self-view is full-bleed, so leaving the conversation on the stage over it would
 * cover the thing the person turned on to see.
 */
export function stage(input: StageInput): Stage {
  const occupied = input.content || input.camera;
  const tooNarrow = input.width < RAIL_MIN_WIDTH;

  const conversation: Conversation =
    input.content && input.ownsConversation
      ? "hidden"
      : !occupied
        ? "stage"
        : input.collapsed || tooNarrow
          ? "pill"
          : "rail";

  return {
    conversation,
    camera: input.content ? "pip" : "fill",
    input: conversation === "rail" ? "rail" : "center",
    demote: input.content ? 0.72 : 0,
    rail: conversation === "rail",
  };
}
