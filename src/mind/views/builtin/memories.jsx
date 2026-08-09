// purpose: 记得的事 — read the facets memory holds about people, projects and topics, and correct one in place.
// docs/arch/data.md says a facet is "revisable, correctable by one sentence from the
// person", and that "a correction that does not stick is now a memory bug to fix". Until
// this view there was nothing that could show a facet at all, let alone fix one — and
// gaps.md records facets accumulating false "not yet delivered" claims that reflection
// never reconciles. So the edit box is the point; the browsing is just how you reach it.
//
// The `tasks` dimension is listed but marked, because tasks have their own surface where
// the state verbs live — editing the raw prose here would skip them.
//
// Colour comes from the host theme tokens (see tasks.jsx for the vocabulary).
import { useState, useEffect, useCallback } from "react";

const J = { "Content-Type": "application/json" };
const enc = (s) => encodeURIComponent(s);
const api = {
  index: () => fetch("/api/facets").then((r) => r.json()),
  read: (d, s) => fetch(`/api/facets/${enc(d)}/${enc(s)}`).then((r) => r.json()),
  write: (d, s, content) =>
    fetch(`/api/facets/${enc(d)}/${enc(s)}`, { method: "PUT", headers: J, body: JSON.stringify({ content }) })
      .then((r) => r.json()),
  episodes: () => fetch("/api/episodes?limit=40").then((r) => r.json()),
};

// ── words ─────────────────────────────────────────────────────────────────────
// English is the default and the fallback. `Memory` is this system's own vocabulary, so
// it stays the English word in Chinese too — unlike the dimensions (people / projects /
// topics / tasks), which are ordinary words and read as 人 / 项目 / 话题 / 任务.
//
// TODO(i18n): en + zh are hand-written. Further languages are meant to be authored at
// runtime — the agent reads the surface and writes the variant — rather than shipped
// here. Until that exists, an unsupported language lands on English.
const MON = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const T = {
  en: {
    title: "Memory",
    dim: { people: "People", projects: "Projects", topics: "Topics", tasks: "Tasks" },
    emptyBig: "Nothing remembered yet.",
    emptySub: "After a few conversations and a few things done, the people, projects and topics grow in here.",
    tasksWarn: "Tasks have their own surface, and Done / Drop it live there. Here you only see the raw text.",
    pick: "Pick one on the left.", unreadable: "Can't read this one.",
    dimEmpty: "Nothing in this one yet.",
    save: "Save", savedTag: "Saved", dirtyTag: "Edited, not saved",
    // The write can fail (a renamed route, a read-only disk). Saying so is the point:
    // a correction that silently didn't land is the bug this whole surface exists for.
    failedTag: "Couldn't save — nothing was written.",
    unsavedMark: "edited, not saved",
    modified: (at) => `last updated ${at}`,
    recent: "Recently",
    stamp: (d, hm) => `${MON[d.getMonth()]} ${d.getDate()}, ${hm}`,
  },
  zh: {
    title: "Memory",
    dim: { people: "人", projects: "项目", topics: "话题", tasks: "任务" },
    emptyBig: "还什么都没记住。",
    emptySub: "聊过几次、做过几件事之后，人、项目和话题会在这里长出来。",
    tasksWarn: "在办的事有自己的界面，完成 / 不做了 在那边。这里只看得到原文。",
    pick: "左边挑一个。", unreadable: "读不出来。",
    dimEmpty: "这一类还是空的。",
    save: "记下来", savedTag: "已记下", dirtyTag: "改了还没存",
    failedTag: "没存上 —— 什么都没写进去。",
    unsavedMark: "改了还没存",
    modified: (at) => `上次更新 ${at}`,
    recent: "最近发生的",
    stamp: (d, hm) => `${d.getMonth() + 1}月${d.getDate()}日 ${hm}`,
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

export default function Memories() {
  const [dims, setDims] = useState(null);
  const [dim, setDim] = useState(null);
  const [subject, setSubject] = useState(null);
  const [facet, setFacet] = useState(null);
  // Unsaved edits, keyed `dimension/subject`, kept across a switch of subject: this is
  // the surface for correcting a memory, so the correction must survive a click on the
  // rail. A key is present only while the text differs from what was read, so "has a
  // draft" and "is dirty" are the same question — the rail marks off it too.
  const [drafts, setDrafts] = useState({});
  const [saved, setSaved] = useState(false);
  const [failed, setFailed] = useState(false);
  const [episodes, setEpisodes] = useState(null);

  useEffect(() => {
    api.index().then((d) => {
      const list = d.dimensions || [];
      setDims(list);
      const first = list.find((x) => x.dimension !== "tasks") || list[0];
      if (first) { setDim(first.dimension); setSubject(first.subjects?.[0] ?? null); }
    }).catch(() => setDims([]));
    api.episodes().then((d) => setEpisodes(d.episodes || [])).catch(() => setEpisodes([]));
  }, []);

  useEffect(() => {
    if (!dim || !subject) { setFacet(null); return; }
    let alive = true;
    setSaved(false); setFailed(false);
    api.read(dim, subject).then((d) => {
      if (!alive) return;
      setFacet(d);
    }).catch(() => alive && setFacet({ error: true }));
    return () => { alive = false; };
  }, [dim, subject]);

  const key = dim && subject ? `${dim}/${subject}` : null;
  const stored = facet && !facet.error ? facet.content || "" : "";
  const draft = key !== null && key in drafts ? drafts[key] : stored;
  const dirty = key !== null && key in drafts;

  // One rule for the whole surface: a draft exists only while it differs from what was
  // read, so typing the original text back is not an edit.
  const edit = (value) => {
    if (key === null) return;
    setSaved(false); setFailed(false);
    setDrafts((all) => {
      const next = { ...all };
      if (value === stored) delete next[key];
      else next[key] = value;
      return next;
    });
  };

  // The PUT answers with the file as it now stands (`{ ok, bytes, modified }`) — read it
  // rather than assuming. A write that did not land must not report "Saved".
  const save = useCallback(async () => {
    if (!dim || !subject || key === null) return;
    const result = await api.write(dim, subject, draft).catch(() => null);
    if (!result?.ok) { setFailed(true); return; }
    setFacet((f) => ({ ...(f || {}), content: draft, modified: result.modified }));
    setDrafts((all) => { const next = { ...all }; delete next[key]; return next; });
    setFailed(false);
    setSaved(true);
  }, [dim, subject, key, draft]);

  if (dims === null) return <div style={S.page}><div style={S.h1}>{L.title}</div></div>;

  const current = dims.find((d) => d.dimension === dim);

  if (dims.length === 0) {
    return (
      <div style={S.page}>
        <div style={S.h1}>{L.title}</div>
        <div style={S.empty}>
          <div style={S.emptyBig}>{L.emptyBig}</div>
          <div style={S.emptySub}>{L.emptySub}</div>
        </div>
        {episodes?.length > 0 && <Episodes list={episodes} />}
      </div>
    );
  }

  return (
    <div style={S.page}>
      <div style={S.h1}>{L.title}</div>

      <div style={S.dims}>
        {dims.map((d) => (
          <button key={d.dimension} type="button" aria-pressed={d.dimension === dim}
            style={{ ...S.reset, ...S.dimChip, ...(d.dimension === dim ? S.dimOn : {}) }}
            onClick={() => { setDim(d.dimension); setSubject(d.subjects?.[0] ?? null); }}>
            {L.dim[d.dimension] || d.dimension}
            <span style={S.dimN}>{d.count}</span>
          </button>
        ))}
      </div>

      <div style={S.split}>
        <div style={S.side}>
          {(current?.subjects || []).map((s) => (
            <button key={s} type="button" aria-pressed={s === subject}
              style={{ ...S.reset, ...S.subj, ...(s === subject ? S.subjOn : {}) }}
              onClick={() => setSubject(s)}>
              <span style={S.subjName}>{s}</span>
              {`${dim}/${s}` in drafts && <span style={S.unsaved} title={L.unsavedMark} aria-label={L.unsavedMark} />}
            </button>
          ))}
          {(current?.subjects || []).length === 0 && <div style={S.none}>{L.dimEmpty}</div>}
        </div>

        <div style={S.pane}>
          {dim === "tasks" && (
            <div style={S.warn}>{L.tasksWarn}</div>
          )}
          {!subject ? <div style={S.none}>{L.pick}</div>
            : facet?.error ? <div style={S.none}>{L.unreadable}</div>
            : (
              <>
                <textarea style={S.editor} value={draft} spellCheck={false}
                  onChange={(e) => edit(e.target.value)} />
                <div style={S.actions}>
                  <span style={{ ...S.stamp, ...(failed ? S.stampFailed : {}) }} role={failed ? "alert" : undefined}>
                    {failed ? L.failedTag
                      : saved ? L.savedTag
                      : dirty ? L.dirtyTag
                      : facet?.modified ? L.modified(short(facet.modified)) : ""}
                  </span>
                  <button style={{ ...S.btnPrimary, ...(dirty ? {} : S.btnOff) }} disabled={!dirty} onClick={save}>
                    {L.save}
                  </button>
                </div>
              </>
            )}
        </div>
      </div>

      {episodes?.length > 0 && <Episodes list={episodes} />}
    </div>
  );
}

function Episodes({ list }) {
  return (
    <>
      <div style={S.sect}>{L.recent}</div>
      <div style={S.eps}>
        {list.map((e, i) => (
          <div key={i} style={S.ep}>
            <span style={S.epAt}>{short(e.at)}</span>
            <span style={S.epGist}>{e.gist}</span>
          </div>
        ))}
      </div>
    </>
  );
}

function short(iso) {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const hm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  return L.stamp(d, hm);
}

const S = {
  page: { "--v-shadow": "0 1px 2px var(--shadow),0 8px 22px var(--shadow)",
    width: "100%", height: "100%", minHeight: 0, overflowY: "auto", boxSizing: "border-box",
    padding: "28px clamp(20px,3vw,44px) 128px", background: "var(--bg-0)",
    color: "var(--fg)", fontFamily: "var(--font-display)" },
  // Everything that responds to a click is a real <button>, so it is reachable by tab
  // and by Enter/Space for free. This strips the UA chrome back to the div it replaced.
  reset: { appearance: "none", border: "none", background: "none", font: "inherit",
    color: "inherit", textAlign: "left", cursor: "pointer" },
  h1: { fontSize: 30, fontWeight: 800, letterSpacing: 0, marginBottom: 20 },
  sect: { fontSize: 11.5, fontWeight: 700, letterSpacing: ".05em", color: "var(--fg-mute)",
    margin: "28px 0 11px" },

  empty: { padding: "46px 8px", textAlign: "center" },
  emptyBig: { fontSize: 17, fontWeight: 600, color: "var(--fg-dim)" },
  emptySub: { fontSize: 13.5, color: "var(--fg-mute)", marginTop: 7, lineHeight: 1.55 },

  dims: { display: "flex", gap: 8, flexWrap: "wrap", marginBottom: 16 },
  dimChip: { display: "flex", alignItems: "center", gap: 6, fontSize: 13, fontWeight: 650,
    color: "var(--fg-dim)", background: "transparent", border: "1px solid var(--line-strong)",
    padding: "6px 13px", borderRadius: 999, cursor: "pointer" },
  dimOn: { color: "var(--accent)", background: "var(--accent-wash)", borderColor: "var(--accent-line)" },
  dimN: { fontSize: 11.5, color: "var(--fg-mute)", fontWeight: 600 },

  split: { display: "flex", gap: 16, alignItems: "stretch", flexWrap: "wrap" },
  side: { flex: "0 0 190px", display: "flex", flexDirection: "column", gap: 2, maxHeight: 380, overflowY: "auto" },
  subj: { display: "flex", alignItems: "center", gap: 7, width: "100%", fontSize: 13.5, fontWeight: 600,
    padding: "8px 11px", borderRadius: 10, color: "var(--fg-dim)" },
  subjName: { flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" },
  subjOn: { background: "var(--surface-strong)", color: "var(--fg)", boxShadow: "var(--v-shadow)" },
  unsaved: { flex: "none", width: 6, height: 6, borderRadius: "50%", background: "var(--accent)" },

  pane: { flex: "1 1 320px", minWidth: 0, display: "flex", flexDirection: "column", gap: 10 },
  warn: { fontSize: 12.5, color: "var(--accent)", background: "var(--accent-wash)",
    borderRadius: 10, padding: "9px 12px", lineHeight: 1.5 },
  editor: { width: "100%", minHeight: 300, resize: "vertical", boxSizing: "border-box",
    background: "var(--surface-strong)", color: "var(--fg)", border: "1px solid var(--line)",
    borderRadius: 14, padding: "14px 16px", fontFamily: "inherit", fontSize: 13.5, lineHeight: 1.68,
    outline: "none" },
  actions: { display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 },
  stamp: { fontSize: 12.5, color: "var(--fg-mute)" },
  stampFailed: { color: "var(--danger)", fontWeight: 600 },
  btnPrimary: { padding: "9px 17px", borderRadius: 11, fontWeight: 700, fontSize: 13, cursor: "pointer",
    border: "none", background: "var(--accent)", color: "var(--bg-0)", fontFamily: "inherit" },
  btnOff: { opacity: 0.4, cursor: "default" },
  none: { fontSize: 13, color: "var(--fg-mute)", padding: "8px 2px" },

  eps: { display: "flex", flexDirection: "column", gap: 1 },
  ep: { display: "flex", gap: 12, alignItems: "baseline", padding: "8px 2px",
    borderBottom: "1px solid var(--line)" },
  epAt: { fontSize: 11.5, color: "var(--fg-mute)", flex: "0 0 92px", fontWeight: 600 },
  epGist: { fontSize: 13, color: "var(--fg-dim)", flex: 1, minWidth: 0, lineHeight: 1.5 },
};
