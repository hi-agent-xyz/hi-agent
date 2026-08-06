import {
  combineStatus,
  statusLabel,
  type ActivityState,
  type CombinedState,
} from "./Presence";

interface StatusButtonProps {
  activity: ActivityState;
}

export function StatusButton({ activity }: StatusButtonProps) {
  const state = combineStatus(activity);
  const label = statusLabel(state);

  return (
    <button
      type="button"
      className={`hi-channel hi-status-button hi-status-button--${state}`}
      title={label}
      aria-label={`agent status: ${label}`}
    >
      <StatusGlyph state={state} />
    </button>
  );
}

function StatusGlyph({ state }: { state: CombinedState }) {
  if (state === "rest") {
    return <span className="hi-status-glyph hi-status-glyph--rest" aria-hidden="true" />;
  }

  return (
    <span className={`hi-status-glyph hi-status-glyph--${state}`} aria-hidden="true">
      {state === "speaking" && (
        <span className="hi-status-wave">
          <i />
          <i />
          <i />
        </span>
      )}
    </span>
  );
}
