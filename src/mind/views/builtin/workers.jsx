// Built-in: 手头在跑的 — the working sessions alive right now. Read-only on purpose:
// the registry has no stop verb, and a button that pretended to kill a worker would be
// worse than no button.
//
// The empty state is the point of this view as much as the list is. gaps.md records the
// agent reporting a watch as healthy while `GET /api/sessions` showed zero workers, and a
// restart eating an in-flight report (`worker report dropped; scene loop gone`). "Nothing
// is running" is a real, load-bearing answer here — so it is stated plainly rather than
// rendered as an empty box that reads like a loading state.
//
// Colour comes from the host theme tokens (see tasks.jsx for the vocabulary). Reads
// /api/workers; polls, because the whole value is that it is current.
import { useState, useEffect, useCallback } from "react";

export const captionAside = "top";

const api = {
  list: () => fetch("/api/workers").then((r) => r.json()),
  one: (id) => fetch(`/api/workers/${encodeURIComponent(id)}`).then((r) => r.json()),
};

// ── words ─────────────────────────────────────────────────────────────────────
// English is the default and the fallback. `Worker` is this system's own vocabulary —
// the rung a session runs at — so the title stays the English word in both. The rung
// labels under `role` are descriptions of what each one does, so those do get said in
// the reader's language.
//
// The registry stores the *rung*, not the worker type — `create_worker` consumes the
// type to pick a prompt and never records it — so "打杂 / 做界面" is not knowable here.
// These are the five rungs the registry actually has.
//
// TODO(i18n): en + zh are hand-written. Further languages are meant to be authored at
// runtime — the agent reads the surface and writes the variant — rather than shipped
// here. Until that exists, an unsupported language lands on English.
const T = {
  en: {
    title: "Workers",
    runningN: (n) => `${n} running`,
    emptyBig: "Nothing is running right now.",
    emptySub: "That's the whole answer — not a page still loading.",
    // Short on purpose: these render inside a small pill beside the task, and the
    // Chinese equivalents are two or three characters. A clause does not fit.
    role: {
      worker: "working",
      cognition: "thinking",
      deliberation: "mulling",
      reflection: "filing",
      reaction: "speaking",
    },
    noTask: "(no note on what it's doing)",
    busy: "running", idle: "idle",
    turns: (n) => `${n} turns`, queued: "mail waiting", ownedBy: (o) => `owned by ${o}`,
    noLog: "Nothing said yet.",
    secs: (n) => `${n}s`, mins: (n) => `${n}m`, hours: (n) => `${n}h`, days: (n) => `${n}d`,
  },
  zh: {
    title: "Workers",
    runningN: (n) => `${n} 个在跑`,
    emptyBig: "现在没有活在跑。",
    emptySub: "这就是全部 —— 不是还没加载出来。",
    role: {
      worker: "干活的",
      cognition: "想事情的",
      deliberation: "琢磨的",
      reflection: "整理记忆的",
      reaction: "说话的",
    },
    noTask: "（没写在做什么）",
    busy: "在跑", idle: "闲着",
    turns: (n) => `${n} 轮`, queued: "还有没读的", ownedBy: (o) => `归 ${o}`,
    noLog: "还没说过话。",
    secs: (n) => `${n} 秒`, mins: (n) => `${n} 分钟`, hours: (n) => `${n} 小时`, days: (n) => `${n} 天`,
  },
};

// App setting first — the host puts it on `<html lang>` — then the system locale when
// that setting says to follow the person, then English.
function words() {
  const app = document.documentElement.lang || "";
  const chain = !app || /^system$/i.test(app) ? [navigator.language] : [app, navigator.language];
  for (const tag of chain) {
    if (/^zh\b/i.test(tag || "")) return T.zh;
    if (/^en\b/i.test(tag || "")) return T.en;
  }
  return T.en;
}
const L = words();

export default function Workers() {
  const [workers, setWorkers] = useState(null);
  const [openId, setOpenId] = useState(null);
  const [messages, setMessages] = useState([]);

  const reload = useCallback(async () => {
    const d = await api.list().catch(() => ({ workers: [] }));
    setWorkers(d.workers || []);
  }, []);

  // Poll: a roster that is stale is worse than none, because it reads as authoritative.
  useEffect(() => {
    reload();
    const t = setInterval(reload, 4000);
    return () => clearInterval(t);
  }, [reload]);

  useEffect(() => {
    if (!openId) { setMessages([]); return; }
    let alive = true;
    api.one(openId).then((d) => alive && setMessages(d.messages || [])).catch(() => {});
    return () => { alive = false; };
  }, [openId, workers]);

  if (workers === null) return <div style={S.page}><div style={S.h1}>{L.title}</div></div>;

  return (
    <div style={S.page}>
      <div style={S.head}>
        <div style={S.h1}>{L.title}</div>
        <span style={S.count}>{workers.length ? L.runningN(workers.length) : ""}</span>
      </div>

      {workers.length === 0 ? (
        <div style={S.empty}>
          <div style={S.emptyBig}>{L.emptyBig}</div>
          <div style={S.emptySub}>{L.emptySub}</div>
        </div>
      ) : (
        <div style={S.list}>
          {workers.map((w) => (
            <div key={w.id} style={S.row}>
              <div style={S.rowTop} onClick={() => setOpenId(openId === w.id ? null : w.id)}>
                <span style={{ ...S.dot, ...(w.busy ? S.dotBusy : {}) }} aria-hidden />
                <span style={S.role}>{L.role[w.role] || w.role}</span>
                <span style={S.task}>{w.task || L.noTask}</span>
                {w.scene && <span style={S.scene}>{w.scene}</span>}
                <span style={S.elapsed}>{elapsed(w.started)}</span>
              </div>

              <div style={S.stats}>
                <span>{w.busy ? L.busy : L.idle}</span>
                {typeof w.turns === "number" && <span>{L.turns(w.turns)}</span>}
                {/* `queued` is a bool: the registry merges a burst into one prompt, so it
                    knows there is mail waiting but never how much. */}
                {w.queued && <span style={S.queued}>{L.queued}</span>}
                {w.owner && <span>{L.ownedBy(w.owner)}</span>}
              </div>

              {w.tail && openId !== w.id && <div style={S.tail}>{w.tail}</div>}

              {openId === w.id && (
                <div style={S.log}>
                  {messages.length === 0
                    ? <div style={S.none}>{L.noLog}</div>
                    : messages.map((m, i) => <div key={i} style={S.logLine}>{m}</div>)}
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function elapsed(started) {
  if (!started) return "";
  const t = new Date(started).getTime();
  if (Number.isNaN(t)) return "";
  const s = Math.max(0, Math.floor((Date.now() - t) / 1000));
  if (s < 60) return L.secs(s);
  if (s < 3600) return L.mins(Math.floor(s / 60));
  if (s < 86400) return L.hours(Math.floor(s / 3600));
  return L.days(Math.floor(s / 86400));
}

const S = {
  page: { "--v-shadow": "0 1px 2px var(--shadow),0 8px 22px var(--shadow)",
    width: "100%", height: "100%", minHeight: 0, overflowY: "auto", boxSizing: "border-box",
    padding: "28px clamp(20px,3vw,44px) 128px", background: "var(--bg-0)",
    color: "var(--fg)", fontFamily: "var(--font-display)" },
  head: { display: "flex", alignItems: "baseline", justifyContent: "space-between", marginBottom: 22 },
  h1: { fontSize: 30, fontWeight: 800, letterSpacing: 0 },
  count: { fontSize: 13, color: "var(--fg-mute)", fontWeight: 600 },

  empty: { padding: "46px 8px", textAlign: "center" },
  emptyBig: { fontSize: 17, fontWeight: 600, color: "var(--fg-dim)" },
  emptySub: { fontSize: 13.5, color: "var(--fg-mute)", marginTop: 7 },

  list: { display: "flex", flexDirection: "column", gap: 10 },
  row: { background: "var(--surface-strong)", borderRadius: 16, boxShadow: "var(--v-shadow)",
    padding: "14px 16px" },
  rowTop: { display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap", cursor: "pointer" },
  dot: { width: 8, height: 8, borderRadius: "50%", background: "var(--line-strong)", flex: "none" },
  dotBusy: { background: "var(--accent)" },
  role: { fontSize: 11.5, fontWeight: 700, color: "var(--accent)", background: "var(--accent-wash)",
    padding: "3px 9px", borderRadius: 999, flex: "none" },
  task: { fontSize: 15, fontWeight: 620, letterSpacing: "-.01em", flex: 1, minWidth: 0 },
  scene: { fontSize: 12.5, color: "var(--fg-mute)" },
  elapsed: { fontSize: 12.5, color: "var(--fg-mute)", fontWeight: 600 },

  stats: { display: "flex", gap: 14, marginTop: 9, fontSize: 12.5, color: "var(--fg-mute)" },
  queued: { color: "var(--accent)", fontWeight: 600 },

  tail: { marginTop: 10, fontSize: 12.5, color: "var(--fg-dim)", lineHeight: 1.5,
    overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" },
  log: { marginTop: 12, paddingTop: 12, borderTop: "1px solid var(--line)",
    display: "flex", flexDirection: "column", gap: 7, maxHeight: 260, overflowY: "auto" },
  logLine: { fontSize: 12.5, lineHeight: 1.55, color: "var(--fg-dim)", whiteSpace: "pre-wrap" },
  none: { fontSize: 13, color: "var(--fg-mute)" },
};
