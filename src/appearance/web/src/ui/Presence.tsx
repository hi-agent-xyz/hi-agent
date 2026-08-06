export type ActivityState =
  | "waking"
  | "idle"
  | "listening"
  | "thinking"
  | "typing"
  | "speaking";

export type CombinedState = Exclude<ActivityState, "idle"> | "rest";

export function combineStatus(activity: ActivityState): CombinedState {
  if (activity.endsWith("ing")) return activity as Exclude<ActivityState, "idle">;
  return "rest";
}

export function statusLabel(state: CombinedState): string {
  return state.charAt(0).toUpperCase() + state.slice(1);
}

export type PresenceState = ActivityState;

interface PresenceProps {
  state: PresenceState;
  /** Retained for API compatibility; the static field no longer recedes. */
  demote?: number;
}

/**
 * The agent's presence — a calm, matte, fully static skin.
 *
 * The background is the theme's warm paper (a soft gradient) and nothing else.
 * The breathing state-glow ("watercolour") was removed: as the only always-on
 * full-screen animation (a `will-change: opacity` layer), it kept the WebKit GPU
 * process busy even at rest. The `state` is still reflected as `data-state` for
 * any future styling hook, but no longer paints a colour field.
 *
 * All CSS — colours come from the `--bg-*` theme tokens (see global.css), so the
 * light/dark swap is a token change alone. No canvas, no audio, no per-frame loop.
 */
export function Presence({ state }: PresenceProps) {
  return <div aria-hidden className="hi-presence" data-state={state} />;
}
