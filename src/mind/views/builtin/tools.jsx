// Built-in: 会用的工具 — the whole tool surface, laid out by which rung gets which tools.
//
// The odd one out in this family: everything else here reviews accumulated state that a
// person may need to correct, and this reviews compile-time code. There is nothing to fix
// from the screen, and that is deliberate — it is here because until now there was no way
// to see the registry whole. `tools/list` over MCP answers with the *calling* role's
// surface only (keyed off `X-HI-Role`), so the honest question "what can it do, and which
// part of it can do that" had no answer anywhere.
//
// Colour comes from the host theme tokens (see tasks.jsx for the vocabulary).
import { useState, useEffect } from "react";

export const captionAside = "top";

// ── words ─────────────────────────────────────────────────────────────────────
// English is the default and the fallback. `Tools` is this system's own vocabulary —
// like Skills and Memory — so it stays the English word in both tables. The role
// labels are plain descriptions of a rung rather than jargon, so those do translate;
// the raw role id shown beside each one never does.
//
// TODO(i18n): en + zh are hand-written. Further languages are meant to be authored at
// runtime — the agent reads the surface and writes the variant — rather than shipped
// here. Until that exists, an unsupported language lands on English.
const T = {
  en: {
    title: "Tools",
    count: (n, layers) => `${n} tools · split across ${layers} layers`,
    noTools: "No tools.",
    // What each rung is *for*, in one line — the role name alone doesn't say why its
    // tool list is shaped the way it is.
    role: {
      reactor: ["the speaking layer", "speaks up when someone is there, puts things on the screen; does no work"],
      cognition: ["the thinking layer", "doesn't do the work itself — hands it out, then watches for the answer"],
      deliberation: ["the pondering layer", "can only hand a thought back"],
      reflection: ["the memory-keeping layer", "looks back, writes into memory"],
      other: ["other", "deliberately empty — a fake realtime capability used to hang here"],
    },
  },
  zh: {
    title: "Tools",
    count: (n, layers) => `${n} 个 · 分给 ${layers} 层`,
    noTools: "没有工具。",
    role: {
      reactor: ["说话的那层", "在场时开口、把东西放到屏幕上；不干活"],
      cognition: ["想事情的那层", "自己不动手，派活出去、盯着回音"],
      deliberation: ["琢磨的那层", "只能把想法递回去"],
      reflection: ["整理记忆的那层", "回头看，写进记忆"],
      other: ["其他", "刻意空着 —— 曾经这里挂着一份假的实时能力"],
    },
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

const ROLE = L.role;

export default function Tools() {
  const [roles, setRoles] = useState(null);

  useEffect(() => {
    let alive = true;
    fetch("/api/tools").then((r) => r.json())
      .then((d) => alive && setRoles(d.roles || []))
      .catch(() => alive && setRoles([]));
    return () => { alive = false; };
  }, []);

  if (roles === null) return <div style={S.page}><div style={S.h1}>{L.title}</div></div>;

  const total = roles.reduce((n, r) => n + (r.tools?.length || 0), 0);

  return (
    <div style={S.page}>
      <div style={S.head}>
        <div style={S.h1}>{L.title}</div>
        <span style={S.count}>{L.count(total, roles.filter((r) => r.tools?.length).length)}</span>
      </div>

      {roles.map((r) => {
        const [label, why] = ROLE[r.role] || [r.role, ""];
        const tools = r.tools || [];
        return (
          <div key={r.role} style={S.group}>
            <div style={S.roleHead}>
              <span style={S.roleName}>{label}</span>
              <span style={S.roleRaw}>{r.role}</span>
            </div>
            {why && <div style={S.why}>{why}</div>}
            {tools.length === 0 ? (
              <div style={S.emptyRole}>{L.noTools}</div>
            ) : (
              <div style={S.list}>
                {tools.map((t) => (
                  <div key={t.name} style={S.row}>
                    <span style={S.name}>{t.name}</span>
                    <span style={S.desc}>{firstSentence(t.description)}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

// Tool descriptions are written for the model and run long. The first sentence is the
// part a person scanning the list actually wants.
function firstSentence(s) {
  if (!s) return "";
  const flat = s.replace(/\s+/g, " ").trim();
  const cut = flat.search(/[.。]\s|[.。]$/);
  const one = cut > 0 ? flat.slice(0, cut + 1) : flat;
  return one.length > 150 ? `${one.slice(0, 150)}…` : one;
}

const S = {
  page: { "--v-shadow": "0 1px 2px var(--shadow),0 8px 22px var(--shadow)",
    padding: "8px 4px 40px", color: "var(--fg)", fontFamily: "var(--font-display)" },
  head: { display: "flex", alignItems: "baseline", justifyContent: "space-between", marginBottom: 22 },
  h1: { fontSize: 30, fontWeight: 800, letterSpacing: "-.03em" },
  count: { fontSize: 13, color: "var(--fg-mute)", fontWeight: 600 },

  group: { marginBottom: 24 },
  roleHead: { display: "flex", alignItems: "baseline", gap: 9 },
  roleName: { fontSize: 16, fontWeight: 750, letterSpacing: "-.02em" },
  roleRaw: { fontSize: 11.5, color: "var(--fg-mute)" },
  why: { fontSize: 12.5, color: "var(--fg-mute)", marginTop: 3, marginBottom: 10, lineHeight: 1.5 },
  emptyRole: { fontSize: 13, color: "var(--fg-mute)", padding: "4px 0 2px" },

  list: { display: "flex", flexDirection: "column", gap: 6 },
  row: { display: "flex", gap: 12, alignItems: "baseline", background: "var(--surface-strong)",
    borderRadius: 12, boxShadow: "var(--v-shadow)", padding: "10px 14px" },
  name: { flex: "0 0 148px", fontSize: 13, fontWeight: 700, letterSpacing: "-.01em",
    color: "var(--accent)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" },
  desc: { flex: 1, minWidth: 0, fontSize: 12.5, lineHeight: 1.5, color: "var(--fg-dim)" },
};
