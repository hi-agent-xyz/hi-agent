// Parse one long-lived newline-delimited JSON response.

export async function* readNdjson<T>(
  res: Response,
  signal: AbortSignal,
): AsyncGenerator<T, void, void> {
  if (!res.body) return;
  const reader = res.body.getReader();
  const decoder = new TextDecoder("utf-8");
  let buf = "";
  try {
    while (!signal.aborted) {
      const { value, done } = await reader.read();
      if (done) return;
      buf += decoder.decode(value, { stream: true });
      let nl: number;
      while ((nl = buf.indexOf("\n")) >= 0) {
        const line = buf.slice(0, nl).trim();
        buf = buf.slice(nl + 1);
        if (line.length === 0) continue;
        try {
          yield JSON.parse(line) as T;
        } catch {
          /* skip a malformed line rather than tearing down the stream */
        }
      }
    }
  } finally {
    try {
      reader.releaseLock();
    } catch {
      /* ignore */
    }
  }
}
