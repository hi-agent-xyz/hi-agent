// Built-in: 在办的事 — every task the agent is carrying, and the two verbs that let a
// person end one. Tasks are facets under `memory/facets/tasks/<subject>/`, opened freely
// by the agent and (per docs/user-journeys/gaps.md) almost never closed: a throwaway
// question becomes a `watch` that nothing can retire, and because an open task keeps the
// cognition timer with work to do, the thing wakes all night. So this surface leads with
// state and staleness, and puts 完成 / 不做了 on every row. Reads and writes /api/tasks.
//
// Colour comes from the host theme tokens — `--fg` / `--fg-dim` / `--fg-mute`,
// `--surface-strong`, `--line` / `--line-strong`, `--accent` (+ `-soft` / `-wash`),
// `--shadow`, `--font-display`. Don't invent token names: `var(--card,#fff)` looks like a
// token and is really a hardcoded white.
import { useState, useEffect, useCallback } from "react";

export const captionAside = "top";

const J = { "Content-Type": "application/json" };
const api = {
  list: () => fetch("/api/tasks").then((r) => r.json()),
  patch: (subject, patch) =>
    fetch(`/api/tasks/${encodeURIComponent(subject)}`, { method: "PATCH", headers: J, body: JSON.stringify(patch) })
      .then((r) => r.json()),
};

// ── words ─────────────────────────────────────────────────────────────────────
// English is the default and the fallback. `Task` is an ordinary word, so it reads as
// 任务 in Chinese — unlike Tools / Skills / Memory, which are this system's own
// vocabulary and stay in English in both.
//
// TODO(i18n): en + zh are hand-written. Further languages are meant to be authored at
// runtime — the agent reads the surface and writes the variant — rather than shipped
// here. Until that exists, an unsupported language lands on English.
const T = {
  en: {
    title: "Tasks",
    closedN: (n) => `${n} closed`, hideClosed: "Hide closed",
    emptyBig: "Nothing on your plate.", emptySub: "Whatever it picks up next shows up here.",
    kind: { wip: "doing", serving: "standing", watch: "watching", deadline: "due", staged: "queued" },
    done: "Done", drop: "Drop it", wasDone: "done", wasDropped: "dropped",
    malformed: "This one won't parse — it was probably written wrong to begin with. You can just end it.",
    noBody: "(no notes)", liveness: "What counts as still alive",
    verify: "Check", restart: "If it stops", owner: "Owned by",
    overdue: (d) => `${d}d overdue`, dueToday: "due today", dueIn: (d) => `${d}d left`,
    idle: (d) => `untouched ${d}d`, checkedAgo: (d) => `checked ${d}d ago`,
    checkedNow: "just checked", never: "never checked",
  },
  zh: {
    title: "任务",
    closedN: (n) => `已了结的 ${n} 件`, hideClosed: "收起已了结的",
    emptyBig: "手上没有在办的事。", emptySub: "它接下来接到的活会出现在这里。",
    kind: { wip: "在做", serving: "长期在跑", watch: "盯着", deadline: "有期限", staged: "排着" },
    done: "完成", drop: "不做了", wasDone: "已完成", wasDropped: "没做",
    malformed: "这条读不出完整格式 —— 多半是当初写歪了。可以直接了结它。",
    noBody: "（没有正文）", liveness: "怎么算还活着",
    verify: "验", restart: "断了怎么起", owner: "归谁",
    overdue: (d) => `过期 ${d} 天`, dueToday: "今天到期", dueIn: (d) => `还有 ${d} 天`,
    idle: (d) => `${d} 天没动了`, checkedAgo: (d) => `${d} 天前看过`,
    checkedNow: "刚看过", never: "还没看过",
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

export default function Tasks() {
  const [tasks, setTasks] = useState(null);
  const [showClosed, setShowClosed] = useState(false);
  const [busy, setBusy] = useState(null);

  const reload = useCallback(async () => {
    const d = await api.list().catch(() => ({ tasks: [] }));
    setTasks(d.tasks || []);
  }, []);
  useEffect(() => { reload(); }, [reload]);

  const settle = async (subject, state) => {
    setBusy(subject);
    await api.patch(subject, { state }).catch(() => {});
    setBusy(null);
    reload();
  };

  if (tasks === null) return <div style={S.page}><div style={S.h1}>{L.title}</div></div>;

  const open = tasks.filter((x) => x.state === "open");
  const closed = tasks.filter((x) => x.state !== "open");

  return (
    <div style={S.page}>
      <div style={S.head}>
        <div style={S.h1}>{L.title}</div>
        {closed.length > 0 && (
          <span style={S.toggle} onClick={() => setShowClosed((v) => !v)}>
            {showClosed ? L.hideClosed : L.closedN(closed.length)}
          </span>
        )}
      </div>

      {open.length === 0 ? (
        <div style={S.empty}>
          <div style={S.emptyBig}>{L.emptyBig}</div>
          <div style={S.emptySub}>{L.emptySub}</div>
        </div>
      ) : (
        <div style={S.list}>
          {open.map((x) => (
            <Row key={x.subject} task={x} busy={busy === x.subject} onSettle={settle} />
          ))}
        </div>
      )}

      {showClosed && (
        <div style={{ ...S.list, marginTop: 18, opacity: 0.62 }}>
          {closed.map((x) => (
            <Row key={x.subject} task={x} closed busy={busy === x.subject} onSettle={settle} />
          ))}
        </div>
      )}
    </div>
  );
}

function Row({ task, closed, busy, onSettle }) {
  const [open, setOpen] = useState(false);
  const stale = staleness(task);

  return (
    <div style={{ ...S.row, ...(task.malformed ? S.rowBad : {}) }}>
      <div style={S.rowTop} onClick={() => setOpen((v) => !v)}>
        <span style={S.kind}>{L.kind[task.kind] || task.kind}</span>
        <span style={S.title}>{task.title || task.subject}</span>
        {task.reportTo && <span style={S.scene}>→ {task.reportTo}</span>}
        {stale && <span style={{ ...S.stale, ...(stale.warn ? S.staleWarn : {}) }}>{stale.text}</span>}
        {closed && <span style={S.closedTag}>{task.state === "done" ? L.wasDone : L.wasDropped}</span>}
      </div>

      {task.malformed && (
        <div style={S.bad}>{L.malformed}</div>
      )}

      {open && (
        <div style={S.body}>
          {task.body ? <div style={S.prose}>{task.body}</div> : <div style={S.none}>{L.noBody}</div>}
          {task.liveness && (
            <div style={S.liveness}>
              <div style={S.livenessTtl}>{L.liveness}</div>
              {task.liveness.verify && <div><b>{L.verify}:</b> {task.liveness.verify}</div>}
              {task.liveness.restart && <div><b>{L.restart}:</b> {task.liveness.restart}</div>}
              {task.liveness.owner && <div><b>{L.owner}:</b> {task.liveness.owner}</div>}
            </div>
          )}
          <div style={S.meta}>{task.subject}</div>
        </div>
      )}

      {!closed && (
        <div style={S.acts}>
          <button style={S.btnGhost} disabled={busy} onClick={() => onSettle(task.subject, "dropped")}>{L.drop}</button>
          <button style={S.btnPrimary} disabled={busy} onClick={() => onSettle(task.subject, "done")}>{L.done}</button>
        </div>
      )}
    </div>
  );
}

// Two different clocks, and the gap between them is the interesting part: `due` is a
// promise to the person, `checked` is when the agent last actually looked. A watch that
// has not been checked in days is the failure mode the journeys keep recording.
function staleness(task) {
  const now = Date.now();
  if (task.due) {
    const d = new Date(task.due).getTime();
    if (!Number.isNaN(d)) {
      const days = Math.round((d - now) / 86400000);
      if (days < 0) return { text: L.overdue(-days), warn: true };
      if (days === 0) return { text: L.dueToday, warn: true };
      if (days <= 7) return { text: L.dueIn(days), warn: false };
    }
  }
  if (task.checked) {
    const c = new Date(task.checked).getTime();
    if (!Number.isNaN(c)) {
      const days = Math.floor((now - c) / 86400000);
      if (days >= 3) return { text: L.idle(days), warn: true };
      if (days >= 1) return { text: L.checkedAgo(days), warn: false };
      return { text: L.checkedNow, warn: false };
    }
  }
  return { text: L.never, warn: true };
}

const S = {
  page: { "--v-shadow": "0 1px 2px var(--shadow),0 8px 22px var(--shadow)",
    padding: "8px 4px 40px", color: "var(--fg)", fontFamily: "var(--font-display)" },
  head: { display: "flex", alignItems: "baseline", justifyContent: "space-between", marginBottom: 22 },
  h1: { fontSize: 30, fontWeight: 800, letterSpacing: "-.03em" },
  toggle: { fontSize: 13, fontWeight: 600, color: "var(--fg-mute)", cursor: "pointer" },

  empty: { padding: "46px 8px", textAlign: "center" },
  emptyBig: { fontSize: 17, fontWeight: 600, color: "var(--fg-dim)" },
  emptySub: { fontSize: 13.5, color: "var(--fg-mute)", marginTop: 7 },

  list: { display: "flex", flexDirection: "column", gap: 10 },
  row: { background: "var(--surface-strong)", borderRadius: 16, boxShadow: "var(--v-shadow)",
    padding: "14px 16px 12px" },
  rowBad: { outline: "1px solid var(--accent-line)" },
  rowTop: { display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap", cursor: "pointer" },
  kind: { fontSize: 11.5, fontWeight: 700, letterSpacing: ".02em", color: "var(--accent)",
    background: "var(--accent-wash)", padding: "3px 9px", borderRadius: 999, flex: "none" },
  title: { fontSize: 15.5, fontWeight: 650, letterSpacing: "-.01em", flex: 1, minWidth: 0 },
  scene: { fontSize: 12.5, color: "var(--fg-mute)" },
  stale: { fontSize: 12.5, color: "var(--fg-mute)", fontWeight: 600 },
  staleWarn: { color: "var(--accent)" },
  closedTag: { fontSize: 11.5, fontWeight: 700, color: "var(--fg-mute)" },

  bad: { marginTop: 9, fontSize: 13, color: "var(--accent)", lineHeight: 1.5 },
  body: { marginTop: 12, paddingTop: 12, borderTop: "1px solid var(--line)" },
  prose: { fontSize: 13.5, lineHeight: 1.62, color: "var(--fg-dim)", whiteSpace: "pre-wrap" },
  none: { fontSize: 13.5, color: "var(--fg-mute)" },
  liveness: { marginTop: 12, fontSize: 13, lineHeight: 1.6, color: "var(--fg-dim)",
    background: "var(--accent-wash)", borderRadius: 12, padding: "11px 13px" },
  livenessTtl: { fontSize: 11.5, fontWeight: 700, color: "var(--fg-mute)", marginBottom: 5,
    letterSpacing: ".04em" },
  meta: { marginTop: 11, fontSize: 11.5, color: "var(--fg-mute)" },

  acts: { display: "flex", gap: 9, justifyContent: "flex-end", marginTop: 12 },
  btnGhost: { padding: "8px 15px", borderRadius: 11, fontWeight: 700, fontSize: 13, cursor: "pointer",
    border: "1px solid var(--line-strong)", background: "transparent", color: "var(--fg-dim)",
    fontFamily: "inherit" },
  btnPrimary: { padding: "8px 15px", borderRadius: 11, fontWeight: 700, fontSize: 13, cursor: "pointer",
    border: "none", background: "var(--accent)", color: "var(--bg-0)", fontFamily: "inherit" },
};
