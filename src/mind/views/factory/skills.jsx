// purpose: 学会的本事 — review the workshop notes under `skills/` for staleness, and delete one that has gone off.
// A workshop note is what a working session leaves behind so the next one starts ahead. They are plain .md under `skills/`, with the factory seeds in
// `skills/factory/` rewritten every boot.
//
// The review this surface exists for is staleness, not tidiness. journey 24 is built on a
// skill having a perishable half — which tool is best, this year's prices, an API that
// moved — and a stale note silently poisons every later run of that job. So the age is
// shown on every row, loudly past a month, and the one correction verb is deleting a note
// that has gone bad. Built-ins can't be deleted: they are reseeded on the next boot, so
// the button would be a lie.
//
// Colour comes from the host theme tokens (see tasks.jsx for the vocabulary).
import { useState, useEffect, useCallback } from "react";

const api = {
  list: () => fetch("/api/skills").then((r) => r.json()),
  read: (path) => fetch(`/api/skills/${path.split("/").map(encodeURIComponent).join("/")}`).then((r) => r.json()),
  remove: (path) =>
    fetch(`/api/skills/${path.split("/").map(encodeURIComponent).join("/")}`, { method: "DELETE" }).then((r) => r.json()),
};

// ── words ─────────────────────────────────────────────────────────────────────
// English is the default and the fallback. `Skills` is this system's own vocabulary —
// like Tools and Memory — so it stays the English word in both tables; everything
// around it is ordinary language and gets translated.
//
// TODO(i18n): en + zh are hand-written. Further languages are meant to be authored at
// runtime — the agent reads the surface and writes the variant — rather than shipped
// here. Until that exists, an unsupported language lands on English.
const T = {
  en: {
    title: "Skills",
    builtinSect: "Factory-issued · rewritten at every boot",
    emptyBig: "Nothing saved up yet.",
    emptySub: "After it works through something hard that will come around again, it leaves a note here.",
    del: "Delete",
    confirmQ: "Out of date?", cancel: "Never mind", confirmDel: "Delete",
    loading: "Reading…", readErr: "Can't read it.",
    today: "written today", daysAgo: (n) => `${n}d ago`,
    monthsAgo: (n) => `${n}mo ago`, yearsAgo: (n) => `${n}y ago`,
  },
  zh: {
    title: "Skills",
    builtinSect: "出厂自带 · 每次启动重写",
    emptyBig: "还没自己攒下什么。",
    emptySub: "干过一件难的、以后还会再遇上的活之后，它会在这里留个笔记。",
    del: "删掉",
    confirmQ: "过时了？", cancel: "算了", confirmDel: "删掉",
    loading: "读取中…", readErr: "读不出来。",
    today: "今天写的", daysAgo: (n) => `${n} 天前`,
    monthsAgo: (n) => `${n} 个月前`, yearsAgo: (n) => `${n} 年前`,
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

export default function Skills() {
  const [skills, setSkills] = useState(null);
  const [openPath, setOpenPath] = useState(null);
  const [content, setContent] = useState(null);
  const [confirming, setConfirming] = useState(null);

  const reload = useCallback(async () => {
    const d = await api.list().catch(() => ({ skills: [] }));
    setSkills(d.skills || []);
  }, []);
  useEffect(() => { reload(); }, [reload]);

  useEffect(() => {
    if (!openPath) { setContent(null); return; }
    let alive = true;
    setContent({ loading: true });
    api.read(openPath).then((d) => alive && setContent(d)).catch(() => alive && setContent({ error: true }));
    return () => { alive = false; };
  }, [openPath]);

  const drop = async (path) => {
    await api.remove(path).catch(() => {});
    setConfirming(null);
    if (openPath === path) setOpenPath(null);
    reload();
  };

  if (skills === null) return <div style={S.page}><div style={S.h1}>{L.title}</div></div>;

  const learned = skills.filter((s) => !s.builtin);
  const builtin = skills.filter((s) => s.builtin);

  return (
    <div style={S.page}>
      <div style={S.h1}>{L.title}</div>

      {learned.length === 0 ? (
        <div style={S.empty}>
          <div style={S.emptyBig}>{L.emptyBig}</div>
          <div style={S.emptySub}>{L.emptySub}</div>
        </div>
      ) : (
        <div style={S.list}>
          {learned.map((s) => (
            <Row key={s.path} skill={s} open={openPath === s.path} content={content}
              onToggle={() => setOpenPath(openPath === s.path ? null : s.path)}
              confirming={confirming === s.path}
              onAskDelete={() => setConfirming(s.path)}
              onCancelDelete={() => setConfirming(null)}
              onDelete={() => drop(s.path)} />
          ))}
        </div>
      )}

      {builtin.length > 0 && (
        <>
          <div style={S.sect}>{L.builtinSect}</div>
          <div style={S.list}>
            {builtin.map((s) => (
              <Row key={s.path} skill={s} open={openPath === s.path} content={content}
                onToggle={() => setOpenPath(openPath === s.path ? null : s.path)} />
            ))}
          </div>
        </>
      )}
    </div>
  );
}

function Row({ skill, open, content, onToggle, confirming, onAskDelete, onCancelDelete, onDelete }) {
  const age = ageOf(skill.modified);
  return (
    <div style={{ ...S.row, ...(skill.builtin ? S.rowBuiltin : {}) }}>
      <button type="button" style={{ ...S.reset, ...S.rowTop }} aria-expanded={open} onClick={onToggle}>
        <span style={S.name}>{skill.name}</span>
        {age && <span style={{ ...S.age, ...(age.old ? S.ageOld : {}) }}>{age.text}</span>}
      </button>
      {skill.excerpt && !open && <div style={S.excerpt}>{skill.excerpt}</div>}
      <div style={S.metaRow}>
        <span style={S.path}>{skill.path}</span>
        {!skill.builtin && onAskDelete && (
          confirming ? (
            <span style={S.confirm}>
              <span style={S.confirmQ}>{L.confirmQ}</span>
              <button type="button" style={{ ...S.reset, ...S.link }} onClick={onCancelDelete}>{L.cancel}</button>
              <button type="button" style={{ ...S.reset, ...S.link, ...S.linkDanger }} onClick={onDelete}>{L.confirmDel}</button>
            </span>
          ) : (
            <button type="button" style={{ ...S.reset, ...S.del }} onClick={onAskDelete}>{L.del}</button>
          )
        )}
      </div>
      {open && (
        <div style={S.body}>
          {content?.loading ? <div style={S.none}>{L.loading}</div>
            : content?.error ? <div style={S.none}>{L.readErr}</div>
            : <div style={S.prose}>{content?.content || ""}</div>}
        </div>
      )}
    </div>
  );
}

// A skill's age is the only cheap proxy for "the fast-moving half of this may have
// rotted". Past a month it is said loudly; that is a prompt to re-check, not a verdict.
function ageOf(modified) {
  if (!modified) return null;
  const t = new Date(modified).getTime();
  if (Number.isNaN(t)) return null;
  const days = Math.floor((Date.now() - t) / 86400000);
  if (days < 1) return { text: L.today, old: false };
  if (days < 30) return { text: L.daysAgo(days), old: false };
  if (days < 365) return { text: L.monthsAgo(Math.floor(days / 30)), old: true };
  return { text: L.yearsAgo(Math.floor(days / 365)), old: true };
}

const S = {
  page: { "--v-shadow": "0 1px 2px var(--shadow),0 8px 22px var(--shadow)",
    width: "100%", height: "100%", minHeight: 0, overflowY: "auto", boxSizing: "border-box",
    padding: "max(28px, var(--hi-safe-top)) clamp(20px,3vw,44px) 128px",
    color: "var(--fg)", fontFamily: "var(--font-display)" },
  h1: { fontSize: 30, fontWeight: 800, letterSpacing: 0, marginBottom: 22 },
  // Everything that responds to a click is a real <button>, so it is reachable by tab
  // and by Enter/Space for free. This strips the UA chrome back to the div it replaced.
  reset: { appearance: "none", border: "none", background: "none", font: "inherit",
    color: "inherit", textAlign: "left", cursor: "pointer", padding: 0 },
  sect: { fontSize: 11.5, fontWeight: 700, letterSpacing: ".05em", color: "var(--fg-mute)",
    margin: "26px 0 11px" },

  empty: { padding: "46px 8px", textAlign: "center" },
  emptyBig: { fontSize: 17, fontWeight: 600, color: "var(--fg-dim)" },
  emptySub: { fontSize: 13.5, color: "var(--fg-mute)", marginTop: 7, lineHeight: 1.55 },

  list: { display: "flex", flexDirection: "column", gap: 10 },
  row: { background: "var(--surface-strong)", borderRadius: 16, boxShadow: "var(--v-shadow)",
    padding: "14px 16px 12px" },
  rowBuiltin: { background: "transparent", boxShadow: "none", border: "1px solid var(--line)" },
  rowTop: { display: "flex", width: "100%", alignItems: "baseline", gap: 10, cursor: "pointer" },
  name: { fontSize: 15.5, fontWeight: 650, letterSpacing: "-.01em", flex: 1, minWidth: 0 },
  age: { fontSize: 12.5, color: "var(--fg-mute)", fontWeight: 600, flex: "none" },
  ageOld: { color: "var(--accent)" },

  excerpt: { marginTop: 7, fontSize: 13, lineHeight: 1.55, color: "var(--fg-dim)",
    display: "-webkit-box", WebkitLineClamp: 2, WebkitBoxOrient: "vertical", overflow: "hidden" },
  metaRow: { display: "flex", alignItems: "center", justifyContent: "space-between", marginTop: 10, gap: 10 },
  path: { fontSize: 11.5, color: "var(--fg-mute)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" },
  del: { fontSize: 12, fontWeight: 600, color: "var(--fg-mute)", cursor: "pointer", flex: "none" },
  confirm: { display: "flex", alignItems: "center", gap: 11, flex: "none" },
  confirmQ: { fontSize: 12, color: "var(--fg-dim)" },
  link: { fontSize: 12, fontWeight: 700, color: "var(--fg-dim)", cursor: "pointer" },
  linkDanger: { color: "var(--accent)" },

  body: { marginTop: 12, paddingTop: 12, borderTop: "1px solid var(--line)" },
  prose: { fontSize: 13, lineHeight: 1.68, color: "var(--fg-dim)", whiteSpace: "pre-wrap",
    maxHeight: 340, overflowY: "auto" },
  none: { fontSize: 13, color: "var(--fg-mute)" },
};
