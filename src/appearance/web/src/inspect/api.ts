import { url } from "../lib/base";
// Inspect data layer — typed views over the endpoints the Rust backend exposes:
//   GET  /api/sessions                    → live snapshot of the voice (JSON)
//   GET  /api/sessions/events             → SSE of every lifecycle event ("session")
//   GET  /api/channels                     → SSE of channel activity ("channel")
//
// These mirror the serde shapes in `src/observatory/mod.rs` and
// `src/server/channels.rs`. Kept deliberately thin: the inspect views poll the
// snapshot and subscribe to the event streams.

export type SessionKind = "reaction" | "worker" | "summarizer";
export type WorkerState = "running" | "done" | "failed";

export interface SessionView {
  id: string;
  kind: SessionKind;
  opened_at: string;
  in_flight: boolean;
  turns: number;
}

export interface TurnView {
  turn: number;
  started_at: string;
  finished_at: string | null;
  stop_reason: string | null;
  reply_chars: number | null;
}

// Voice-shaped state only. Worker lifecycle is NOT here: a working session belongs to
// whoever created it, so it is not the state of the mouth. Read workers off the event
// stream instead (`worker_spawned` / `worker_finished`), which is keyed by session.
export interface AgentView {
  reaction_session: SessionView | null;
  budget_chars: number;
  swap_after_chars: number;
  swap_count: number;
  last_swap_at: string | null;
  last_turn: TurnView | null;
  turns_total: number;
}

// One lifecycle event. `event` is the discriminant; the rest of the fields
// depend on it (see EventKind in the backend). We keep them loosely typed and
// read defensively in the renderer.
export interface SessionEvent {
  seq: number;
  ts: string;
  event: string;
  [k: string]: unknown;
}

/** Fetch the live snapshot of the voice. Throws on network/HTTP error. */
export async function fetchSessions(signal?: AbortSignal): Promise<AgentView> {
  const res = await fetch(url("/api/sessions"), { signal });
  if (!res.ok) throw new Error(`GET /api/sessions → ${res.status}`);
  return (await res.json()) as AgentView;
}

export type Channel = "text" | "vision" | "audio" | "touch" | "smell" | "taste";
export type Direction = "in" | "out";

// One unit of channel activity — a recognized/spoken line on the
// text channel, or a metadata summary for binary/structured channels. Mirrors
// `ChannelSignal` in `src/server/channels.rs`.
export interface ChannelSignal {
  ts: string;
  channel: Channel;
  direction: Direction;
  body: string;
  final: boolean;
}

/**
 * Subscribe to the merged channel-activity stream. Returns an
 * unsubscribe fn. This is live presence only — the backend replays nothing, so a
 * fresh subscriber sees activity from the moment it connects. EventSource
 * auto-reconnects.
 */
export function subscribeChannels(
  onSignal: (sig: ChannelSignal) => void,
  onStatus?: (live: boolean) => void,
): () => void {
  const es = new EventSource("/api/channels");
  es.addEventListener("open", () => onStatus?.(true));
  es.addEventListener("error", () => onStatus?.(false));
  es.addEventListener("channel", (e) => {
    try {
      onSignal(JSON.parse((e as MessageEvent).data) as ChannelSignal);
    } catch {
      /* ignore malformed frame */
    }
  });
  return () => es.close();
}

// One raw JSON-RPC line on the agent wire, business-logic agnostic. Mirrors
// `RawFrame` in `src/foundation/codex/tap.rs`. `raw` is the verbatim line; the rest is
// the little we parse out for grouping (the inspector keys sessions off `conn`, and
// labels them with `thread_id` once codex has minted one).
export type WireDir = "send" | "recv" | "stderr";

export interface RawFrame {
  seq: number;
  ts: string;
  conn: number;
  // Which rung this subprocess hosts: reaction, deliberation, cognition,
  // reflection, worker. The tap has always stamped it and this interface used to
  // drop it, so the inspector could only call a session by its connection number.
  role: string;
  // hi-agent's own session id, minted before the subprocess starts — so it names
  // the handshake frames too, which `thread_id` cannot. A slug: `cognition`, or
  // `view-builder-kyoto-trip`. Frame logs from runs before ids were slugs carry a
  // decimal string here instead.
  agent_session: string | null;
  dir: WireDir;
  thread_id: string | null;
  method: string | null;
  id: unknown;
  raw: string;
}

/**
 * Subscribe to the raw frame feed (`GET /api/wire/frames/events`). Returns an
 * unsubscribe fn. One SSE connection carries one frame type, `frame`: the
 * buffered ring replays on connect, then live frames stream. EventSource
 * auto-reconnects.
 */
export function subscribeWireFrames(
  onFrame: (frame: RawFrame) => void,
  onStatus?: (live: boolean) => void,
): () => void {
  const es = new EventSource("/api/wire/frames/events");
  es.addEventListener("open", () => onStatus?.(true));
  es.addEventListener("error", () => onStatus?.(false));
  es.addEventListener("frame", (e) => {
    try {
      onFrame(JSON.parse((e as MessageEvent).data) as RawFrame);
    } catch {
      /* ignore malformed frame */
    }
  });
  return () => es.close();
}

export interface EventStreamHandlers {
  /** A lifecycle event arrived — append it to the event log. */
  onEvent?: (ev: SessionEvent) => void;
  /** A fresh full snapshot arrived — replace prior state. */
  onSnapshot?: (conversations: AgentView[]) => void;
  /** Connection liveness toggled (open → true, error/reconnecting → false). */
  onStatus?: (live: boolean) => void;
}

/**
 * Subscribe to the lifecycle stream. Returns an unsubscribe fn. One SSE
 * connection carries two frame types: `session` lifecycle events (buffered
 * history replayed on connect, then live) and periodic `snapshot` frames (the
 * full live mirror). Reading conversation state from the snapshot frames here
 * means the dashboard polls nothing — it holds a single connection rather than
 * leaking a `/api/sessions` request per tick into a starved HTTP/1.1 pool.
 * EventSource auto-reconnects.
 */
export function subscribeEvents(handlers: EventStreamHandlers): () => void {
  const es = new EventSource("/api/sessions/events");
  es.addEventListener("open", () => handlers.onStatus?.(true));
  es.addEventListener("error", () => handlers.onStatus?.(false));
  es.addEventListener("session", (e) => {
    try {
      handlers.onEvent?.(JSON.parse((e as MessageEvent).data) as SessionEvent);
    } catch {
      /* ignore malformed frame */
    }
  });
  es.addEventListener("snapshot", (e) => {
    try {
      handlers.onSnapshot?.(JSON.parse((e as MessageEvent).data) as AgentView[]);
    } catch {
      /* ignore malformed frame */
    }
  });
  return () => es.close();
}
