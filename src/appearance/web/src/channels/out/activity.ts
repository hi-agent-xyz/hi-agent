export interface AgentActivity {
  reaction_ready: boolean;
  reaction_busy: boolean;
  delegated_busy_count: number;
}

/**
 * Subscribe to authoritative backend activity. The first event is an immediate
 * snapshot; later events arrive only when a live session changes.
 */
export function subscribeActivity(
  onActivity: (activity: AgentActivity) => void,
  onStatus?: (live: boolean) => void,
): () => void {
  const events = new EventSource("/api/activity");
  events.addEventListener("open", () => onStatus?.(true));
  events.addEventListener("error", () => onStatus?.(false));
  events.addEventListener("activity", (event) => {
    try {
      onActivity(JSON.parse((event as MessageEvent).data) as AgentActivity);
    } catch {
      /* ignore malformed frames */
    }
  });
  return () => events.close();
}
