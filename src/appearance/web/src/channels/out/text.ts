import { url } from "../../lib/base";
// Subscriber for the backend-owned conversation.
//
// GET /api/out/text is one long-lived NDJSON response. Its first line is the
// current window whole; every later line appends one message or updates the single
// rolling recognition interim. Nothing is ever rewritten or removed.
//
// There is no client id, no cursor, no acknowledgement and no read receipt. The
// `before` on the scrollback fetch is not a cursor either — the backend remembers
// nothing about it; it is where one request starts, sent by a window that already
// holds the newer messages.

import { readNdjson } from "../ndjson";

export type Role = "user" | "agent";

/** A file that came with a message. `ref` fetches from `/api/media/<ref>`. */
export interface Attachment {
  ref: string;
  mime: string;
}

export interface Message {
  /** The journal's uuidv7 — time-sortable, and the same key the backend logged. */
  id: string;
  ts: string;
  role: Role;
  text: string;
  attachment?: Attachment;
}

/** What the conversation looks like right now. */
export interface Conversation {
  messages: Message[];
  /** The live recognition partial — a preview of a message, not a message. */
  interim?: string;
}

export type Frame =
  | { kind: "reset"; conversation: Conversation }
  | { kind: "append"; message: Message }
  | { kind: "interim"; text?: string };

export interface SubscribeOpts {
  signal: AbortSignal;
}

function parseAttachment(value: unknown): Attachment | undefined {
  if (typeof value !== "object" || value === null) return undefined;
  const raw = value as Record<string, unknown>;
  if (typeof raw.ref !== "string" || typeof raw.mime !== "string") return undefined;
  return { ref: raw.ref, mime: raw.mime };
}

export function parseMessage(value: unknown): Message | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return null;
  const raw = value as Record<string, unknown>;
  if (typeof raw.id !== "string" || typeof raw.ts !== "string") return null;
  if (raw.role !== "user" && raw.role !== "agent") return null;
  if (typeof raw.text !== "string") return null;
  const attachment = parseAttachment(raw.attachment);
  return {
    id: raw.id,
    ts: raw.ts,
    role: raw.role,
    text: raw.text,
    ...(attachment ? { attachment } : {}),
  };
}

/** Validate one decoded frame before it reaches the UI. */
export function parseFrame(value: unknown): Frame | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return null;
  const raw = value as Record<string, unknown>;

  if ("reset" in raw) {
    const body = raw.reset;
    if (typeof body !== "object" || body === null || Array.isArray(body)) return null;
    const { messages, interim } = body as Record<string, unknown>;
    if (!Array.isArray(messages)) return null;
    const parsed: Message[] = [];
    for (const m of messages) {
      const message = parseMessage(m);
      if (message) parsed.push(message);
    }
    return {
      kind: "reset",
      conversation: {
        messages: parsed,
        ...(typeof interim === "string" ? { interim } : {}),
      },
    };
  }

  if ("append" in raw) {
    const message = parseMessage(raw.append);
    return message ? { kind: "append", message } : null;
  }

  if ("interim" in raw) {
    return {
      kind: "interim",
      ...(typeof raw.interim === "string" ? { text: raw.interim } : {}),
    };
  }

  return null;
}

/**
 * Observe the conversation. The server sends the current window immediately and
 * keeps this response open for everything said after it.
 */
export async function* subscribeOutText(opts: SubscribeOpts): AsyncGenerator<Frame, void, void> {
  const res = await fetch(url("/api/out/text"), {
    method: "GET",
    headers: { Accept: "application/x-ndjson" },
    signal: opts.signal,
    cache: "no-store",
  });

  if (!res.ok) {
    throw new Error(`/api/out/text subscribe failed: ${res.status} ${res.statusText}`);
  }

  for await (const value of readNdjson<unknown>(res, opts.signal)) {
    const frame = parseFrame(value);
    if (frame) yield frame;
  }
}

/**
 * Older messages, for scrolling back past the live window. Returns them oldest
 * first — the same order the window holds — so a caller prepends the whole array.
 */
export async function fetchOlderMessages(
  before: string,
  opts?: { limit?: number; signal?: AbortSignal },
): Promise<Message[]> {
  const params = new URLSearchParams({ before });
  if (opts?.limit) params.set("limit", String(opts.limit));
  const res = await fetch(url(`/api/messages?${params}`), {
    method: "GET",
    headers: { Accept: "application/json" },
    ...(opts?.signal ? { signal: opts.signal } : {}),
    cache: "no-store",
  });
  if (!res.ok) throw new Error(`/api/messages failed: ${res.status} ${res.statusText}`);
  const body: unknown = await res.json();
  if (!Array.isArray(body)) return [];
  const messages: Message[] = [];
  for (const m of body) {
    const message = parseMessage(m);
    if (message) messages.push(message);
  }
  return messages;
}
