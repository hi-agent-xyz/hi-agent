import {
  statusLabel,
  type ActivityState,
} from "./Presence";

interface StatusButtonProps {
  activity: ActivityState;
}

export function StatusButton({ activity }: StatusButtonProps) {
  const label = statusLabel(activity);

  return (
    <button
      type="button"
      className={`hi-channel hi-status-button hi-status-button--${activity}`}
      title={label}
      aria-label={`agent status: ${label}`}
    >
      <StatusGlyph state={activity} />
    </button>
  );
}

function StatusGlyph({ state }: { state: ActivityState }) {
  if (state === "idle") {
    return <span className="hi-status-glyph hi-status-glyph--idle" aria-hidden="true" />;
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
