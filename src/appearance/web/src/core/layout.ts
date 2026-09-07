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

/** How the conversation is presented. One surface, two states — not two surfaces.
 *
 * - `panel`   — the conversation as the panel's Messages tab, carrying the full
 *               scrollback and, in its foot, the line being written. **The only
 *               box the conversation ever stands in**, whether or not the agent
 *               has something up: it used to fill the frame while the stage was
 *               empty and jump to the corner the moment a view appeared, so the
 *               agent showing a slide moved and resized the thing being read,
 *               mid-read. One box, one measure, always.
 * - `pill`    — collapsed to the newest line, floating over whatever is behind it.
 *               A caption and not a shelf: it shows while the line is worth
 *               reading and then fades, because what it holds is a copy of
 *               something the list behind it keeps. The dwell is the shell's
 *               (`ui/caption.ts`); this pass says where the pill goes, never how
 *               long it stays.
 *
 * Putting it away puts *all* of it away, the line included, because the line is
 * inside it (`ui/Composer.tsx`). That is what lets one control own the surface —
 * see `docs/arch/stage.md`.
 *
 * The panel replaced a corner popover, which had replaced a rail. The rail was a
 * column the view had to inset past for as long as the conversation was open, and
 * it lost on the word *permanently*: a third of the window whether or not anyone
 * was reading. The panel's middle stop is that same column, charged only while a
 * person is holding it open and reflowing only because they pulled it there. See
 * `docs/arch/stage.md` § *The panel, and the axis it runs on*.
 */
export type Conversation = "panel" | "pill";

/** The self-view fills the frame when nothing else is on it, else a corner pip. */
export type Camera = "fill" | "pip";

export interface StageInput {
  /** The agent has a view up. */
  content: boolean;
  /** The panel is at the room — off the right-hand side of the window, so the
   * conversation is not on screen and the newest line is a caption over whatever
   * is. A window's own position on the axis (`lib/panel.ts`), never server state,
   * so a phone cannot put a desktop's panel away. */
  away: boolean;
}

export interface Stage {
  conversation: Conversation;
  /** Where the self-view goes — the backdrop, or a corner pip once a view leads.
   * Whether the camera is *on* is not an input: nothing here depends on it, and
   * `CameraPreview` draws nothing without a stream either way. */
  camera: Camera;
  /** How far to fade the presence while something leads. */
  demote: number;
}

/**
 * Arrange the stage.
 *
 * The conversation yields the *frame* to whatever the agent puts up, and never the
 * *screen*: it stands in its panel over the view rather than degrading, because it
 * is the one surface that keeps — see `docs/arch/text-transcript.md`. It is at the
 * pill until the person opens it and back at the pill when they put it away, and
 * nothing else moves it: not the agent, not a view.
 *
 * **What is on the stage no longer moves it.** The pass used to answer `stage` on
 * an empty room and `popover` once a view or the camera was up — two boxes, two
 * measures, and the switch between them happened *while someone was reading*: the
 * agent putting a slide up pulled the conversation out from under the eye and
 * re-laid it in a corner a third the size. Now `content` and `camera` decide only
 * what is *behind* the panel, never where the panel is. The only inputs left that
 * move the conversation are the person's own put-away and a view's claim on the
 * words.
 *
 * **Asking works with nothing on the stage too**, and that is a reversal: the pass
 * used to ignore the put-away unless something else was up, on the reasoning that
 * hiding the only thing on screen is not a thing to offer. Two changes make it
 * one. The pill is timed now, so what is left behind is the room and a line that
 * fades, not a shelf. And the same control that puts the conversation away is the
 * text channel's — refusing here would leave the one channel whose button does
 * nothing in the state where it is the whole face. Any printable key brings it
 * back (`ui/Composer.tsx`).
 *
 * There is no width threshold *here* any more. The one that survives is in
 * `lib/panel.ts`, where it belongs: a phone has no middle stop, because below
 * ~640px a screen cannot be split into two usable columns. This pass reads no
 * width at all, and a narrow window gets the whole scrollback rather than only
 * the newest line.
 */
export function stage(input: StageInput): Stage {
  // The host always draws the conversation. A view used to be able to stand it down by
  // declaring `owns_conversation`, and the one view that ever did was the outage notice,
  // which renders a fixed message rather than the words — so the claim took away the
  // record and the line at the moment the person most needed both. See `docs/arch/stage.md`.
  const conversation: Conversation = input.away ? "pill" : "panel";

  return {
    conversation,
    camera: input.content ? "pip" : "fill",
    demote: input.content ? 0.72 : 0,
  };
}
