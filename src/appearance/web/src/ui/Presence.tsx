export type ActivityState =
  | "starting"
  | "listening"
  | "speaking"
  | "typing"
  | "working"
  | "idle";

export interface ActivitySignals {
  ready: boolean;
  listening: boolean;
  speaking: boolean;
  reactionBusy: boolean;
  delegatedBusy: boolean;
}

/** Project concurrent facts into the one status shown on the face. */
export function projectActivityState(signals: ActivitySignals): ActivityState {
  if (!signals.ready) return "starting";
  if (signals.listening) return "listening";
  if (signals.speaking) return "speaking";
  if (signals.reactionBusy) return "typing";
  if (signals.delegatedBusy) return "working";
  return "idle";
}

export function statusLabel(state: ActivityState): string {
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
