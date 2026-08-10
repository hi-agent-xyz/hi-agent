// Subscriber for the backend-owned current text appearance.
//
// GET /api/out/text is one long-lived NDJSON response. Its first line is the
// current state; every later line replaces that state wholesale. There are no
// message ids, client ids, cursors, acknowledgements or historical replay.

import { readNdjson } from "../ndjson";

export interface AgentTextState {
  text: string;
  /** False while the reply is growing; true at the latest utterance boundary. */
  final: boolean;
}

/** Everything textual the appearance is showing now. */
export interface TextAppearanceState {
  /** Latest settled human line in the current exchange. */
  user?: string;
  /** Agent reply accumulated for the current exchange. */
  agent?: AgentTextState;
  /** Cumulative rolling STT partial, overlaid until it settles or expires. */
  interim?: string;
}

export interface SubscribeOpts {
  signal: AbortSignal;
}

/** Validate one decoded state snapshot before it reaches the UI. */
export function parseTextAppearanceState(value: unknown): TextAppearanceState | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return null;
  const raw = value as Record<string, unknown>;

  if (raw.user !== undefined && typeof raw.user !== "string") return null;
  if (raw.interim !== undefined && typeof raw.interim !== "string") return null;

  let agent: AgentTextState | undefined;
  if (raw.agent !== undefined) {
    if (typeof raw.agent !== "object" || raw.agent === null || Array.isArray(raw.agent)) {
      return null;
    }
    const candidate = raw.agent as Record<string, unknown>;
    if (typeof candidate.text !== "string" || typeof candidate.final !== "boolean") {
      return null;
    }
    agent = { text: candidate.text, final: candidate.final };
  }

  return {
    ...(typeof raw.user === "string" ? { user: raw.user } : {}),
    ...(agent ? { agent } : {}),
    ...(typeof raw.interim === "string" ? { interim: raw.interim } : {}),
  };
}

/**
 * Observe the current text appearance. The server sends one snapshot
 * immediately and keeps this response open for future replacements.
 */
export async function* subscribeOutText(
  opts: SubscribeOpts,
): AsyncGenerator<TextAppearanceState, void, void> {
  const res = await fetch("/api/out/text", {
    method: "GET",
    headers: { Accept: "application/x-ndjson" },
    signal: opts.signal,
    cache: "no-store",
  });

  if (!res.ok) {
    throw new Error(`/api/out/text subscribe failed: ${res.status} ${res.statusText}`);
  }

  for await (const value of readNdjson<unknown>(res, opts.signal)) {
    const state = parseTextAppearanceState(value);
    if (state) yield state;
  }
}
