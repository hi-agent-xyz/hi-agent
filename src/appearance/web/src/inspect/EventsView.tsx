import { useEffect, useState, type ReactNode } from "react";
import { subscribeEvents, type SessionEvent } from "./api";

const MAX_EVENTS = 5000;

// 24-hour wall-clock with milliseconds. Two agents can exchange several messages
// inside one second, and the order is the whole point of reading this.
function time(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const p = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`;
}

function clip(s: string, n = 160): string {
  const line = s.replace(/\s+/g, " ").trim();
  return line.length > n ? `${line.slice(0, n)}…` : line;
}

// Everything except the envelope — the envelope is already its own columns, and a
// debug surface should not print the same fact twice.
function payload(e: SessionEvent): Record<string, unknown> {
  const { seq: _s, ts: _t, event: _e, ...rest } = e;
  return rest;
}

// A readable one-line summary per event kind, with the full payload still one click
// away in the JSON column. Same division `SessionsView` uses for wire frames: read
// the shape at a glance, expand for ground truth.
function summary(e: SessionEvent): ReactNode {
  const s = (k: string): string => (typeof e[k] === "string" ? (e[k] as string) : "");
  const n = (k: string): number | null => (typeof e[k] === "number" ? (e[k] as number) : null);

  switch (e.event) {
    case "message_sent": {
      // The edge. `from: null` is the host putting something in a mailbox on nobody's
      // behalf — a report handed up, a follow-up merged — which is a real crossing and
      // reads differently from one agent choosing to speak to another.
      // Both ends are session ids now — an address *is* a session id, so there is
      // nothing left to resolve after the fact.
      const from = n("from");
      const to = n("to");
      return (
        <span className={`edge ${s("delivery")}`}>
          <b>{from === null ? "host" : `#${from}`}</b> → <b>{`#${to}`}</b>
          <span className="muted"> {s("delivery")}</span>
          {" "}
          <span className="edge-msg">{clip(s("message"), 120)}</span>
        </span>
      );
    }
    case "session_opened":
    case "session_closed":
      return `${s("kind")} ${s("id")}`;
    case "turn_started":
      return `turn ${n("turn")}: ${clip(s("input"), 120)}`;
    case "turn_finished":
      return `turn ${n("turn")}: ${clip(s("reply"), 120)} (${n("reply_chars")} chars, ${s("stop_reason") || "—"})`;
    case "hot_swap":
      return `${s("old_id")} → ${s("new_id")}, ${n("briefing_chars")} chars briefed`;
    case "worker_spawned":
    case "worker_resumed":
      return `#${n("id")}: ${clip(s("task"), 120)}`;
    case "worker_finished":
      return `#${n("id")}: ${s("state")}, ${n("summary_chars")} chars`;
    default:
      return clip(JSON.stringify(payload(e)));
  }
}

/**
 * The lifecycle event log — the observatory's history, verbatim.
 *
 * This tab exists because the stream had no reader. `subscribeEvents` has carried an
 * `onEvent` callback since it was written and nothing subscribed to it, so every
 * event the host recorded was reachable only by curl. In particular no surface showed
 * **agent-to-agent messages**, which is the one thing you cannot infer from the other
 * two tabs: `Conversation` shows the live channels and `Sessions` shows raw wire traffic,
 * and an edge between two agents is neither.
 *
 * Deliberately unaggregated. Per `docs/arch/foundation.md#debug-surfaces` a debug
 * view shows ground truth — one row per recorded event, in recorded order, with the
 * raw payload expandable. Nothing here re-derives state the backend didn't send.
 */
export function EventsView() {
  const [events, setEvents] = useState<SessionEvent[]>([]);
  const [live, setLive] = useState(false);
  const [only, setOnly] = useState<string>("");

  useEffect(() => {
    return subscribeEvents({
      onEvent: (e) =>
        setEvents((prev) => {
          const next = prev.length >= MAX_EVENTS ? prev.slice(prev.length - MAX_EVENTS + 1) : prev;
          return [...next, e];
        }),
      onStatus: setLive,
    });
  }, []);

  const kinds = [...new Set(events.map((e) => e.event))].sort();
  const shown = only ? events.filter((e) => e.event === only) : events;

  return (
    <div className="wire-detail">
      <div className="detail-head">
        <div className="dh-title">
          <b>lifecycle events</b>
          <span className="muted">
            {events.length} recorded
            {only ? ` · showing ${shown.length} ${only}` : ""}
          </span>
          <span className={`live-dot ${live ? "on" : ""}`} title={live ? "event stream live" : "reconnecting"} />
        </div>

        <div className="evfilter">
          <button className={only === "" ? "sel" : ""} onClick={() => setOnly("")}>
            all
          </button>
          {kinds.map((k) => (
            <button key={k} className={only === k ? "sel" : ""} onClick={() => setOnly(k)}>
              {k}
            </button>
          ))}
        </div>

        {shown.length === 0 ? (
          <div className="muted pad">
            No events yet. They appear as soon as a session opens, a turn runs, or one
            agent messages another.
          </div>
        ) : (
          <div className="wire-events">
            <table className="evtable frtable">
              <thead>
                <tr>
                  <th>Time</th>
                  <th>Event</th>
                  <th>#</th>
                  <th>Detail</th>
                  <th>JSON</th>
                </tr>
              </thead>
              <tbody>
                {shown.map((e) => (
                  <tr key={e.seq}>
                    <td className="ts">{time(e.ts)}</td>
                    <td className={`evname ${e.event}`}>{e.event}</td>
                    <td className="evseq">{e.seq}</td>
                    <td className="evframe">{summary(e)}</td>
                    <td className="evraw">
                      <details>
                        <summary>{clip(JSON.stringify(payload(e)))}</summary>
                        <pre>{JSON.stringify(e, null, 2)}</pre>
                      </details>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}
