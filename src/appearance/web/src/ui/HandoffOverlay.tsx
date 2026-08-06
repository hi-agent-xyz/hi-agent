import type { HandoffFeedback } from "../hooks/useHandoff";

interface HandoffOverlayProps {
  feedback: HandoffFeedback | null;
  onRetry: () => void;
  onDismiss: () => void;
}

export function HandoffOverlay({
  feedback,
  onRetry,
  onDismiss,
}: HandoffOverlayProps) {
  if (feedback === null) return null;
  const problem = feedback.state === "partial" || feedback.state === "error";

  return (
    <div
      className="hi-file-drop"
      data-state={feedback.state}
      role={problem ? "alert" : "status"}
      aria-live={problem ? "assertive" : "polite"}
    >
      <div className="hi-file-drop-box">
        <span
          className={`hi-file-drop-icon${feedback.state === "sending" ? " is-spinning" : ""}`}
          aria-hidden
        >
          <HandoffGlyph feedback={feedback} />
        </span>
        <span className="hi-file-drop-text">{feedback.message}</span>
        {problem && (
          <span className="hi-file-drop-actions">
            {feedback.retryable && (
              <button type="button" className="hi-file-drop-retry" onClick={onRetry}>
                Retry
              </button>
            )}
            <button
              type="button"
              className="hi-file-drop-dismiss"
              onClick={onDismiss}
              aria-label="dismiss upload error"
              title="dismiss"
            >
              <CloseGlyph />
            </button>
          </span>
        )}
      </div>
    </div>
  );
}

function HandoffGlyph({ feedback }: { feedback: HandoffFeedback }) {
  if (feedback.state === "sent") {
    return (
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none">
        <path
          d="m5 12 4 4L19 6"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    );
  }
  if (feedback.state === "partial" || feedback.state === "error") {
    return (
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none">
        <path
          d="M12 7v6M12 17h.01"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
        />
      </svg>
    );
  }
  if (feedback.kind === "text") {
    return (
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none">
        <path
          d="M6 5h12M12 5v14M8.5 19h7"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
        />
      </svg>
    );
  }
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none">
      <path
        d="M12 16V5M8 9l4-4 4 4M5 19h14"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function CloseGlyph() {
  return (
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" aria-hidden>
      <path
        d="m7 7 10 10M17 7 7 17"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
      />
    </svg>
  );
}
