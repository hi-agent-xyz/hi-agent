// Subscriber for the outbound text channel — the agent's worded reply.
//
// Spec rules we obey here:
//   * GET /api/out/text is a long-poll. The server holds the response open and
//     streams body bytes as the agent emits. Body-close ends the utterance.
//   * After body-close we re-subscribe. Each subscription is one utterance.
//   * Reading does not consume: the server retains utterances so every attached
//     surface sees each one. We say where we are with `?after=<id>` and it
//     answers with the id it just delivered on `X-HI-Utterance`, which becomes
//     the next request's cursor. Without that a re-subscribe would loop on the
//     same line; with it, several windows can watch one conversation.
//
// The function is an async generator: each yielded string is a UTF-8 chunk
// of one in-flight utterance. The generator returns when the body closes.

export interface TextChunk {
  /** The chunk of text the server just emitted. */
  text: string;
}

export interface TextCursor {
  /** The server process whose utterance id this belongs to. */
  epoch: string;
  /** The utterance id this reader will pass back as its next `after`. */
  id: number;
}

export interface SubscribeOpts {
  /** Abort signal so the caller can cancel cleanly on unmount. */
  signal: AbortSignal;
  /**
   * Committed cursor, or null to start at the oldest the server still holds.
   * A caller should commit only after the response body closes.
   */
  after?: TextCursor | null;
  /** Called when response headers arrive, before the utterance body is complete. */
  onUtterance?: (cursor: TextCursor) => void;
}

/** Parse the persisted cursor, resetting old numeric or malformed values. */
export function parseTextCursor(raw: string | null): TextCursor | null {
  if (!raw) return null;
  try {
    const value: unknown = JSON.parse(raw);
    if (
      typeof value === "object" &&
      value !== null &&
      typeof (value as { epoch?: unknown }).epoch === "string" &&
      (value as { epoch: string }).epoch.length > 0 &&
      Number.isSafeInteger((value as { id?: unknown }).id) &&
      (value as { id: number }).id >= 0
    ) {
      return { epoch: (value as { epoch: string }).epoch, id: (value as { id: number }).id };
    }
  } catch {
    // A malformed sessionStorage value is equivalent to no cursor.
  }
  return null;
}

export function serializeTextCursor(cursor: TextCursor): string {
  return JSON.stringify(cursor);
}

/**
 * Open one long-poll against /api/out/text. Yields each chunk of text as it
 * arrives. Resolves (returns) when the server closes the body — i.e. the
 * utterance ended. Throws if the request fails or is aborted; callers should
 * treat AbortError as a normal shutdown.
 */
export async function* subscribeOutText(
  opts: SubscribeOpts,
): AsyncGenerator<TextChunk, void, void> {
  const params = new URLSearchParams();
  if (opts.after != null) {
    params.set("epoch", opts.after.epoch);
    params.set("after", String(opts.after.id));
  }
  const qs = params.toString() ? `?${params.toString()}` : "";
  const res = await fetch(`/api/out/text${qs}`, {
    method: "GET",
    headers: {
      Accept: "text/plain, application/octet-stream",
    },
    signal: opts.signal,
    // Streaming responses must not be cached.
    cache: "no-store",
  });

  if (!res.ok) {
    throw new Error(`/api/out/text subscribe failed: ${res.status} ${res.statusText}`);
  }

  const delivered = Number(res.headers.get("X-HI-Utterance"));
  const epoch = res.headers.get("X-HI-Text-Epoch");
  if (Number.isSafeInteger(delivered) && delivered >= 0 && epoch) {
    opts.onUtterance?.({ epoch, id: delivered });
  }

  // Some servers (or proxies) may return a non-streaming body. fall through:
  if (!res.body) {
    const text = await res.text();
    if (text.length > 0) yield { text };
    return;
  }

  const reader = res.body.getReader();
  const decoder = new TextDecoder("utf-8");

  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) return;
      if (!value || value.byteLength === 0) continue;
      const text = decoder.decode(value, { stream: true });
      if (text.length > 0) yield { text };
    }
  } finally {
    try {
      reader.releaseLock();
    } catch {
      // ignore
    }
  }
}
