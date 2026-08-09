import { useEffect, useMemo, useState, type ReactNode } from "react";
import { selectedUnder, usePath } from "./router";
import { subscribeWireFrames, type WireDir, type RawFrame } from "./api";

const BASE = "/inspect/sessions";
const MAX_FRAMES = 5000;

// 24-hour wall-clock with milliseconds — frames arrive faster than a second, so
// the ms part is what tells two adjacent frames apart.
function time(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const p = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`;
}

// Direction from the *session's* point of view (this is the sessions page): a
// frame hi-agent sends is `in`coming to the session; one the subprocess emits is
// `out`going from it; `err` is what it wrote to stderr.
function dirLabel(dir: WireDir): string {
  return dir === "send" ? "in" : dir === "recv" ? "out" : "err";
}

// A one-line, whitespace-collapsed preview of the raw frame — shown as the
// collapsed summary so the log scans without expanding every payload.
function preview(raw: string): string {
  const line = raw.replace(/\s+/g, " ").trim();
  return line.length > 160 ? `${line.slice(0, 160)}…` : line;
}

// A readable view for the Frame column. Some payloads carry content worth
// reading at a glance: streamed message and reasoning deltas become their text, a
// completed item names what it was, and `turn/start` renders each input part as a
// stacked text block. Everything else falls back to a one-line `preview`.
function frameSummary(raw: string): ReactNode {
  try {
    const msg = JSON.parse(raw);
    const params = msg?.params;

    if (msg?.method === "item/agentMessage/delta" && typeof params?.delta === "string") {
      return `agentMessage: "${params.delta}"`;
    }
    if (msg?.method === "item/reasoning/summaryTextDelta" && typeof params?.delta === "string") {
      return `reasoning: "${params.delta}"`;
    }
    if (msg?.method === "item/started" || msg?.method === "item/completed") {
      const item = params?.item;
      if (item?.type === "mcpToolCall") {
        return `${item.type}: ${item.server}/${item.tool}, ${item.status}`;
      }
      if (item?.type === "commandExecution") {
        return `${item.type}: ${item.command}, ${item.status}`;
      }
      if (item?.type) return `${item.type}${item.status ? `: ${item.status}` : ""}`;
    }
    if (msg?.method === "turn/completed") {
      const turn = params?.turn;
      const err = turn?.error?.message;
      return `turn ${turn?.status}${err ? `: ${err}` : ""}`;
    }
    if (msg?.method === "thread/tokenUsage/updated") {
      const u = params?.usage ?? params;
      if (typeof u?.inputTokens === "number" && typeof u?.outputTokens === "number") {
        return `tokenUsage: ${u.inputTokens} in / ${u.outputTokens} out`;
      }
    }
    if (msg?.method === "turn/start" && Array.isArray(params?.input)) {
      return (
        <div className="frprompt">
          {params.input.map((p: { text?: string }, i: number) => (
            <div key={i} className="frprompt-block">
              {typeof p?.text === "string" ? p.text : JSON.stringify(p)}
            </div>
          ))}
        </div>
      );
    }
    // The system prompt now arrives as `baseInstructions` on thread/start, where ACP
    // had to smuggle it into the first prompt — worth showing in full, same as one.
    if (msg?.method === "thread/start" && typeof params?.baseInstructions === "string") {
      return (
        <div className="frprompt">
          <div className="frprompt-block">{params.baseInstructions}</div>
        </div>
      );
    }
  } catch {
    // fall through to the generic preview
  }
  return preview(raw);
}

// Pretty-print the verbatim line as JSON; fall back to the raw string when it
// isn't JSON (rare — stderr spew). This is a debug surface, so nothing is
// summarized or truncated.
function pretty(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}

// A short human label for a frame: its JSON-RPC method, or — for responses,
// which carry no method — whether it resolved or errored.
function frameLabel(f: RawFrame): string {
  if (f.method) return f.method;
  if (/"error"\s*:/.test(f.raw)) return "↩ error";
  if (/"result"\s*:/.test(f.raw)) return "↩ result";
  return "—";
}

// One group of frames the inspector renders as a "session": all the frames of a
// single subprocess connection. One subprocess hosts one session, so a group's
// `initialize`/`thread/start` frames (which precede, and so lack, a threadId)
// belong to the same session as the rest — there is no separate handshake bucket.
interface Group {
  key: string; // URL key + dedup key
  conn: number;
  role: string; // which rung this subprocess hosts — stamped on every frame
  agentSession: number | null; // hi-agent's own id, present from the first frame
  sessionId: string | null; // adopted from the thread/start response; null until then
  frames: RawFrame[];
}

// The five rungs, plus a fallback. A session's identity here is its *rung* — the
// connection number says which subprocess, which is not a question anyone has.
const ROLES = ["reaction", "deliberation", "cognition", "reflection", "worker"];
function roleClass(role: string): string {
  return ROLES.includes(role) ? role : "unknown";
}

// Fold the flat frame stream into per-connection groups, preserving first-seen
// order. Every frame of one subprocess shares its `conn`, so the handshake frames
// group with their session; the sessionId is adopted as soon as a frame reveals
// it. Knows nothing about the reaction — pure wire.
function group(frames: RawFrame[]): Group[] {
  const map = new Map<number, Group>();
  for (const f of frames) {
    let g = map.get(f.conn);
    if (!g) {
      g = {
        key: `c::${f.conn}`,
        conn: f.conn,
        role: f.role,
        agentSession: f.agent_session,
        sessionId: f.thread_id,
        frames: [],
      };
      map.set(f.conn, g);
    }
    if (!g.sessionId && f.thread_id) g.sessionId = f.thread_id;
    if (g.agentSession == null && f.agent_session != null) g.agentSession = f.agent_session;
    g.frames.push(f);
  }
  return [...map.values()];
}

export function SessionsView() {
  const { path, navigate } = usePath();
  const selected = selectedUnder(path, BASE);
  const [frames, setFrames] = useState<RawFrame[]>([]);
  const [live, setLive] = useState(false);

  // One SSE connection feeds the entire view — every raw wire frame, replayed on
  // connect then live. The session list and each detail pane are derived from
  // this single stream; no polling, no per-session endpoint.
  useEffect(() => {
    return subscribeWireFrames(
      (f) =>
        setFrames((prev) => {
          const next = prev.length >= MAX_FRAMES ? prev.slice(prev.length - MAX_FRAMES + 1) : prev;
          return [...next, f];
        }),
      setLive,
    );
  }, []);

  const groups = useMemo(() => group(frames), [frames]);
  const current = groups.find((g) => g.key === selected) ?? null;

  return (
    <div className="wire">
      <aside className="wire-list">
        <div className="wire-list-head">
          <span>Agent sessions</span>
          <span className={`live-dot ${live ? "on" : ""}`} title={live ? "frame stream live" : "reconnecting"} />
        </div>
        {groups.length === 0 ? (
          <div className="muted pad">No wire frames yet. They appear on the first contact with an agent session.</div>
        ) : (
          <ul>
            {groups.map((g) => (
              <li
                key={g.key}
                className={g.key === selected ? "sel" : ""}
                onClick={() => navigate(`${BASE}/${encodeURIComponent(g.key)}`)}
              >
                <span className={`skind ${roleClass(g.role)}`}>{g.role || "?"}</span>
                <span className="nm">
                  {g.agentSession != null ? `#${g.agentSession}` : "opening…"}
                  {g.sessionId ? <span className="muted"> · {g.sessionId.slice(0, 12)}</span> : null}
                </span>
                <span className="badges">
                  <span className="mini">{g.frames.length}</span>
                </span>
              </li>
            ))}
          </ul>
        )}
      </aside>

      <section className="wire-detail">
        {!current ? (
          <div className="muted pad">Select a session to see its raw wire frames.</div>
        ) : (
          <FrameLog group={current} />
        )}
      </section>
    </div>
  );
}

function FrameLog({ group: g }: { group: Group }) {
  return (
    <div className="detail-head">
      <div className="dh-title">
        <b>{g.role || "session"}</b>
        <span className="muted">
          {g.agentSession != null ? <> · session <code>#{g.agentSession}</code></> : <> · opening…</>}
          {g.sessionId ? <> · thread <code>{g.sessionId}</code></> : null} · {g.frames.length} frames
        </span>
      </div>

      <div className="wire-events">
        <table className="evtable frtable">
          <thead>
            <tr>
              <th>Time</th>
              <th>Dir</th>
              <th>Method</th>
              <th>#</th>
              <th>Frame</th>
              <th>JSON</th>
            </tr>
          </thead>
          <tbody>
            {g.frames.map((f) => (
              <tr key={f.seq}>
                <td className="ts">{time(f.ts)}</td>
                <td className={`frdir ${f.dir}`}>{dirLabel(f.dir)}</td>
                <td className="evname">{frameLabel(f)}</td>
                <td className="evseq">{f.seq}</td>
                <td className="evframe">{frameSummary(f.raw)}</td>
                <td className="evraw">
                  <details>
                    <summary>{preview(f.raw)}</summary>
                    <pre>{pretty(f.raw)}</pre>
                  </details>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
