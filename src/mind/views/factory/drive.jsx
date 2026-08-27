// purpose: 存着的文件 — browse `drive/`, the artifacts the agent keeps as bytes (contracts, scans, papers). Read-only.
// The verbatim, precious half of the data dir (`projects/`, `notes/`, `papers/`).
// Handed artifacts live here: a contract, a passport scan, a paper — files the agent
// keeps as bytes rather than as memory.
//
// Read-only, and honestly so. docs/data-dir-layout.md says the tree exists but its
// contract does not: graduation into `drive/projects/` is not built, and neither is the
// notebook. So this is a window, not a file manager — it answers "what did I actually
// hand it, and is it still there", which today nothing can.
//
// Colour comes from the host theme tokens (see tasks.jsx for the vocabulary).
import { useState } from "react";
import { url, useLive, TEMPO } from "@hi/core";

// ── words ─────────────────────────────────────────────────────────────────────
// English is the default and the fallback. `Drive` is an ordinary word for the shelf of
// files, so it reads as 文件 in Chinese — unlike Memory, which is this system's own
// vocabulary and stays in English in both.
//
// TODO(i18n): en + zh are hand-written. Further languages are meant to be authored at
// runtime — the agent reads the surface and writes the variant — rather than shipped
// here. Until that exists, an unsupported language lands on English.
const T = {
  en: {
    title: "Drive",
    top: { projects: "Projects", notes: "Notes", papers: "Papers" },
    count: (n, size) => `${n === 1 ? "1 file" : `${n} files`} · ${size}`,
    emptyBig: "Nothing kept here yet.",
    emptySub: "Contracts, documents, drafts — anything worth keeping as the original file shows up here once you hand it over.",
    // A shelf that exists and holds nothing is an answer, not a gap in the list.
    shelfEmpty: "Empty.",
    loose: "Loose",
  },
  zh: {
    title: "文件",
    top: { projects: "项目", notes: "笔记", papers: "文稿" },
    count: (n, size) => `${n} 份 · ${size}`,
    emptyBig: "还没存着什么。",
    emptySub: "合同、证件、稿子这类要留原件的东西，传给它以后会在这里。",
    shelfEmpty: "空的。",
    loose: "散着的",
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

// Not a folder name: the bucket for files handed over without a shelf. A real drive
// directory can never collide with it — `/` cannot appear in one path segment.
const LOOSE = "/loose";

export default function Drive() {
  const [entries, setEntries] = useState(null);

  // It files things here itself — a carrier hands over an artifact and the shelf grows
  // while this is on screen. A read that does not come back leaves the last good listing
  // standing; only the very first one is allowed to settle on empty, because before it
  // there is nothing to keep and the skeleton has to end somewhere.
  useLive(
    () =>
      fetch("/api/drive")
        .then((r) => r.json())
        .then((d) => setEntries(d.entries || []))
        .catch(() => setEntries((prev) => (prev === null ? [] : prev))),
    { period: TEMPO.ledger },
  );

  if (entries === null) return <div style={S.page}><div style={S.h1}>{L.title}</div></div>;

  const files = entries.filter((e) => !e.dir);

  // With no file anywhere, the shelves are beside the point: "what have I handed it"
  // is answered better by the sentence than by three empty rows.
  if (files.length === 0) {
    return (
      <div style={S.page}>
        <div style={S.h1}>{L.title}</div>
        <div style={S.empty}>
          <div style={S.emptyBig}>{L.emptyBig}</div>
          <div style={S.emptySub}>{L.emptySub}</div>
        </div>
      </div>
    );
  }

  // Group by the first path segment so the top folders read as the shelves they are,
  // rather than as an undifferentiated file list. The endpoint sends directory entries
  // too, on purpose — an empty `drive/papers/` is a real answer to "what am I holding",
  // so a shelf with nothing on it gets a row of its own rather than vanishing. Files
  // sitting at the root of the drive have no shelf; they collect under `loose`.
  const groups = new Map();
  for (const e of entries) {
    if (e.dir && !e.path.includes("/")) {
      if (!groups.has(e.path)) groups.set(e.path, []);
      continue;
    }
    if (e.dir) continue;
    const top = e.path.includes("/") ? e.path.split("/")[0] : LOOSE;
    if (!groups.has(top)) groups.set(top, []);
    groups.get(top).push(e);
  }

  // The three named shelves lead, in the order docs/data-dir-layout.md names them;
  // anything else the agent made follows, alphabetically, and loose files land last.
  const rank = (top) => {
    const known = ["projects", "notes", "papers"].indexOf(top);
    if (known >= 0) return [0, known, ""];
    return top === LOOSE ? [2, 0, ""] : [1, 0, top];
  };
  const shelves = [...groups.entries()].sort(([a], [b]) => {
    const [ga, ka, na] = rank(a);
    const [gb, kb, nb] = rank(b);
    return ga - gb || ka - kb || na.localeCompare(nb);
  });

  return (
    <div style={S.page}>
      <div style={S.head}>
        <div style={S.h1}>{L.title}</div>
        <span style={S.count}>{L.count(files.length, bytes(files.reduce((n, f) => n + (f.bytes || 0), 0)))}</span>
      </div>

      {shelves.map(([top, list]) => (
        <div key={top} style={S.group}>
          <div style={S.sect}>
            {top === LOOSE ? L.loose : L.top[top] || top}
            <span style={S.sectN}>{list.length}</span>
          </div>
          {list.length === 0 ? (
            <div style={S.shelfEmpty}>{L.shelfEmpty}</div>
          ) : (
            <div style={S.list}>
              {list.map((f) => (
                <a key={f.path} style={S.row} href={url(`/api/drive/file/${f.path.split("/").map(encodeURIComponent).join("/")}`)}
                  target="_blank" rel="noreferrer">
                  <span style={S.ext}>{(f.ext || "·").slice(0, 4)}</span>
                  <span style={S.name}>{top === LOOSE ? f.path : f.path.split("/").slice(1).join("/")}</span>
                  <span style={S.size}>{bytes(f.bytes)}</span>
                  <span style={S.when}>{short(f.modified)}</span>
                </a>
              ))}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

function bytes(n) {
  if (!n) return "0 B";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / 1048576).toFixed(1)} MB`;
}
function short(iso) {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return `${d.getFullYear()}/${d.getMonth() + 1}/${d.getDate()}`;
}

const S = {
  page: { "--v-shadow": "0 1px 2px var(--shadow),0 8px 22px var(--shadow)",
    width: "100%", height: "100%", minHeight: 0, overflowY: "auto", boxSizing: "border-box",
    padding: "max(28px, var(--hi-safe-top)) clamp(20px,3vw,44px) 128px",
    color: "var(--fg)", fontFamily: "var(--font-display)" },
  head: { display: "flex", alignItems: "baseline", justifyContent: "space-between", marginBottom: 20 },
  h1: { fontSize: 30, fontWeight: 800, letterSpacing: 0 },
  count: { fontSize: 13, color: "var(--fg-mute)", fontWeight: 600 },

  empty: { padding: "46px 8px", textAlign: "center" },
  emptyBig: { fontSize: 17, fontWeight: 600, color: "var(--fg-dim)" },
  emptySub: { fontSize: 13.5, color: "var(--fg-mute)", marginTop: 7, lineHeight: 1.55 },

  group: { marginBottom: 22 },
  sect: { display: "flex", alignItems: "baseline", gap: 8, fontSize: 11.5, fontWeight: 700,
    letterSpacing: ".05em", color: "var(--fg-mute)", marginBottom: 9 },
  sectN: { fontWeight: 600 },
  shelfEmpty: { fontSize: 13, color: "var(--fg-mute)", padding: "2px 2px 4px" },

  list: { display: "flex", flexDirection: "column", gap: 6 },
  row: { display: "flex", alignItems: "center", gap: 12, textDecoration: "none", color: "inherit",
    background: "var(--surface-strong)", borderRadius: 13, boxShadow: "var(--v-shadow)",
    padding: "11px 14px", cursor: "pointer" },
  ext: { flex: "none", width: 40, fontSize: 10.5, fontWeight: 800, letterSpacing: ".04em",
    textTransform: "uppercase", color: "var(--accent)", background: "var(--accent-wash)",
    borderRadius: 7, padding: "5px 0", textAlign: "center" },
  name: { flex: 1, minWidth: 0, fontSize: 14, fontWeight: 600, letterSpacing: "-.01em",
    overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" },
  size: { flex: "none", fontSize: 12.5, color: "var(--fg-mute)", width: 64, textAlign: "right" },
  when: { flex: "none", fontSize: 12.5, color: "var(--fg-mute)", width: 82, textAlign: "right" },
};
