import {
  combineStatus,
  statusLabel,
  type ActivityState,
  type AvailabilityState,
  type CombinedState,
} from "./Presence";

interface StatusButtonProps {
  activity: ActivityState;
  availability: AvailabilityState;
}

export function StatusButton({ activity, availability }: StatusButtonProps) {
  const state = combineStatus(activity, availability);
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
  if (state === "out_of_energy") {
    return (
      <span className="hi-status-glyph hi-status-glyph--energy" aria-hidden="true">
        !
      </span>
    );
  }

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
