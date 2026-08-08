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

export interface SubscribeOpts {
  /** Abort signal so the caller can cancel cleanly on unmount. */
  signal: AbortSignal;
  /**
   * Id of the last utterance received in full, or null to start at the oldest
   * the server still holds — which is what makes a reply produced before this
   * client ever connected still arrive.
   */
  after?: number | null;
  /** Called with the id of the utterance this subscription carries. */
  onUtterance?: (id: number) => void;
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
  const qs = opts.after == null ? "" : `?after=${opts.after}`;
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
  if (Number.isFinite(delivered)) opts.onUtterance?.(delivered);

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
