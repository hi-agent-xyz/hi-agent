// Client for the inbound text channel.
//
// `postInText` sends a typed line to the agent (POST /api/in/text). The server
// folds the accepted line into the shared text appearance; the sending window
// keeps no optimistic private copy.

/**
 * Send a text signal to the agent.
 * Returns when the server has accepted the body (202).
 */
export async function postInText(opts: {
  body: string;
  signal?: AbortSignal;
}): Promise<void> {
  const res = await fetch("/api/in/text", {
    method: "POST",
    headers: {
      "Content-Type": "text/plain; charset=utf-8",
      // `text/plain` is one of the three content types a cross-site *simple*
      // request can carry, so off-box the core will not accept it on a cookie
      // alone. This header cannot be set without a preflight, which is the
      // proof. Harmless on loopback, where nothing is gated.
      "X-HI-Surface": "1",
    },
    body: opts.body,
    signal: opts.signal,
  });

  if (!res.ok) {
    const detail = await res.text().catch(() => "");
    throw new Error(
      `/api/in/text POST failed: ${res.status} ${res.statusText}${detail ? ` — ${detail}` : ""}`,
    );
  }
}
