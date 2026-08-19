// purpose: 怎么找到它 — the agent's address and the devices that may reach it. Read, plus
// exactly the three verbs those endpoints can honour: claim a name, add a device, revoke
// one.
//
// The app's roster — which agent this app is with — deliberately is *not* here. It is the
// app's state, not the agent's, and a view bundled into the core asking for it would be
// reaching across the one boundary this whole design exists to draw. It also 404s on every
// core with no app in front of it, which the renderer correctly reports as a problem.
//
// The sibling of tasks/skills/memories/tools/drive, and it exists for the same reason:
// all of this was reachable only by curl. A person could not see what their agent was
// called, could not tell which of their devices still had a way in, and had no way to
// take one back.
//
// Colour comes from the host theme tokens (see tasks.jsx for the vocabulary).
import { useState, useEffect, useCallback } from "react";
import { url } from "@hi/core";

// ── words ─────────────────────────────────────────────────────────────────────
// TODO(i18n): en + zh are hand-written. Further languages are meant to be authored at
// runtime rather than shipped here; an unsupported language lands on English.
const T = {
  en: {
    title: "Reach",
    // — the name
    name: "Name",
    nameWhy: "Your agent's address. Permanent once taken, and yours — nothing expires.",
    unnamed: "No name yet. Your agent works, and is reachable from this machine only.",
    claim: "Claim",
    rename: "Rename",
    namePlaceholder: "a name",
    nameRules: "lowercase letters, digits and hyphens",
    // — the devices
    devices: "Devices",
    devicesWhy: "What may reach your agent. Each was let in once and can be taken back here.",
    noDevices: "Nothing has been let in yet.",
    neverUsed: "never used",
    lastSeen: (when) => `last seen ${when}`,
    added: (when) => `added ${when}`,
    revoke: "Revoke",
    revoking: "Revoking…",
    addDevice: "Add a device",
    pairWith: "Open your agent on the other device and enter this code. It lasts ten minutes.",
    pairAt: "or scan with the Hi Agent app",
    done: "Done",
  },
  zh: {
    title: "怎么找到它",
    name: "名字",
    nameWhy: "你的 agent 的地址。取了就是你的，不会过期。",
    unnamed: "还没取名字。它照常工作，只是只能从这台机器上找到。",
    claim: "取名",
    rename: "改名",
    namePlaceholder: "起个名字",
    nameRules: "小写字母、数字和连字符",
    devices: "设备",
    devicesWhy: "能找到它的东西。每一个都是你放进来的，也可以在这里收回。",
    noDevices: "还没放进来过任何设备。",
    neverUsed: "还没用过",
    lastSeen: (when) => `最近 ${when}`,
    added: (when) => `${when} 加入`,
    revoke: "收回",
    revoking: "收回中…",
    addDevice: "加一台设备",
    pairWith: "在另一台设备上打开你的 agent，输入这个码。十分钟内有效。",
    pairAt: "或者用 Hi Agent App 扫码",
    done: "好了",
  },
};

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

// A mutating call from a view: JSON or the header, or the core refuses it off-box as
// something a cross-site form could have sent. See foundation/surfaces.
const WRITE = { "Content-Type": "application/json", "X-HI-Surface": "1" };

export default function Reach() {
  const [handle, setHandle] = useState(null); // {handles, limit, why?}
  const [devices, setDevices] = useState(null);

  const loadDevices = useCallback(() => {
    fetch(url("/api/surfaces"))
      .then((r) => r.json())
      .then((d) => setDevices(d.surfaces || []))
      .catch(() => setDevices([]));
  }, []);

  const loadHandle = useCallback(() => {
    // Always answers: a core with no name is a normal core, and the reason it
    // has none (no account yet, no community reachable) rides in `why`.
    fetch(url("/api/handle"))
      .then((r) => r.json())
      .then(setHandle)
      .catch(() => setHandle({ handles: [] }));
  }, []);

  useEffect(() => {
    loadHandle();
    loadDevices();
  }, [loadHandle, loadDevices]);

  return (
    <div style={S.page}>
      <div style={S.h1}>{L.title}</div>
      <Name state={handle} onChanged={loadHandle} />
      <Devices list={devices} reload={loadDevices} />
    </div>
  );
}

// ── the name ──────────────────────────────────────────────────────────────────

function Name({ state, onChanged }) {
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState("");

  if (state === null) return <Section title={L.name} why={L.nameWhy} />;

  const held = state.handles?.[0];

  async function claim() {
    const name = draft.trim().toLowerCase();
    if (!name || busy) return;
    setBusy(true);
    setRefused("");
    try {
      const r = await fetch(url("/api/handle"), {
        method: "POST",
        headers: WRITE,
        body: JSON.stringify({ handle: name }),
      });
      // The registry's own words: "that handle is in use", "sign in first…".
      // A person choosing a name is owed the reason, not a status code.
      if (!r.ok) setRefused((await r.text()).trim());
      else {
        setDraft("");
        onChanged();
      }
    } catch {
      setRefused("");
    } finally {
      setBusy(false);
    }
  }

  return (
    <Section title={L.name} why={L.nameWhy}>
      {held ? (
        <div style={S.address}>{stripScheme(held.base_url)}</div>
      ) : (
        <div style={S.note}>{state.why || L.unnamed}</div>
      )}
      <div style={S.row}>
        <input
          style={S.input}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && claim()}
          placeholder={L.namePlaceholder}
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
        />
        <button style={S.button} onClick={claim} disabled={busy || !draft.trim()}>
          {held ? L.rename : L.claim}
        </button>
      </div>
      <div style={S.hint}>{refused || L.nameRules}</div>
    </Section>
  );
}

function stripScheme(u) {
  return (u || "").replace(/^https?:\/\//, "");
}

// ── the devices ───────────────────────────────────────────────────────────────

function Devices({ list, reload }) {
  const [pairing, setPairing] = useState(null); // {code, url, app_url}
  const [busy, setBusy] = useState("");

  async function addDevice() {
    setBusy("pair");
    try {
      const r = await fetch(url("/api/pair"), { method: "POST", headers: WRITE });
      setPairing(r.ok ? await r.json() : null);
    } catch {
      setPairing(null);
    } finally {
      setBusy("");
    }
  }

  async function revoke(id) {
    setBusy(id);
    try {
      await fetch(url(`/api/surfaces/${id}`), { method: "DELETE", headers: WRITE });
      reload();
    } finally {
      setBusy("");
    }
  }

  return (
    <Section title={L.devices} why={L.devicesWhy}>
      {list === null ? null : list.length === 0 ? (
        <div style={S.note}>{L.noDevices}</div>
      ) : (
        <div style={S.list}>
          {list.map((s) => (
            <div key={s.id} style={S.card}>
              <div style={S.cardMain}>
                <div style={S.cardName}>{s.label}</div>
                <div style={S.cardWhen}>
                  {s.last_seen_at ? L.lastSeen(ago(s.last_seen_at)) : L.neverUsed}
                  {" · "}
                  {L.added(ago(s.created_at))}
                </div>
              </div>
              <button
                style={S.danger}
                onClick={() => revoke(s.id)}
                disabled={busy === s.id}
              >
                {busy === s.id ? L.revoking : L.revoke}
              </button>
            </div>
          ))}
        </div>
      )}

      {pairing ? (
        <div style={S.pair}>
          <div style={S.code}>{pairing.code}</div>
          <div style={S.hint}>{L.pairWith}</div>
          {pairing.url && (
            <>
              <img
                style={S.qr}
                alt=""
                src={url(`/api/qr?data=${encodeURIComponent(pairing.app_url || pairing.url)}`)}
              />
              <div style={S.hint}>{L.pairAt}</div>
            </>
          )}
          <button style={S.button} onClick={() => setPairing(null)}>{L.done}</button>
        </div>
      ) : (
        <button style={S.button} onClick={addDevice} disabled={busy === "pair"}>
          {L.addDevice}
        </button>
      )}
    </Section>
  );
}

// ── shared ────────────────────────────────────────────────────────────────────

function Section({ title, why, children }) {
  return (
    <div style={S.section}>
      <div style={S.h2}>{title}</div>
      <div style={S.why}>{why}</div>
      {children}
    </div>
  );
}

/// A coarse "when", because the exact second of a pairing is never the question.
function ago(iso) {
  const then = Date.parse(iso || "");
  if (!Number.isFinite(then)) return "";
  const mins = Math.max(0, Math.round((Date.now() - then) / 60000));
  if (mins < 1) return L === T.zh ? "刚刚" : "just now";
  if (mins < 60) return L === T.zh ? `${mins} 分钟前` : `${mins}m ago`;
  const hours = Math.round(mins / 60);
  if (hours < 24) return L === T.zh ? `${hours} 小时前` : `${hours}h ago`;
  const days = Math.round(hours / 24);
  return L === T.zh ? `${days} 天前` : `${days}d ago`;
}

const S = {
  page: { "--v-shadow": "0 1px 2px var(--shadow),0 8px 22px var(--shadow)",
    width: "100%", height: "100%", minHeight: 0, overflowY: "auto", boxSizing: "border-box",
    padding: "28px clamp(20px,3vw,44px) 128px",
    color: "var(--fg)", fontFamily: "var(--font-display)" },
  h1: { fontSize: 30, fontWeight: 800, letterSpacing: 0, marginBottom: 26 },

  section: { marginBottom: 30, maxWidth: 620 },
  h2: { fontSize: 16, fontWeight: 750, letterSpacing: "-.02em" },
  why: { fontSize: 12.5, color: "var(--fg-mute)", marginTop: 3, marginBottom: 12, lineHeight: 1.5 },

  address: { fontFamily: "var(--font-mono)", fontSize: 15, fontWeight: 600,
    color: "var(--accent)", marginBottom: 10, wordBreak: "break-all" },
  note: { fontSize: 13, color: "var(--fg-dim)", lineHeight: 1.5, marginBottom: 10 },
  hint: { fontSize: 12, color: "var(--fg-mute)", marginTop: 7, lineHeight: 1.5 },

  row: { display: "flex", gap: 8, alignItems: "center" },
  input: { flex: 1, minWidth: 0, font: "inherit", fontSize: 14, padding: "9px 12px",
    borderRadius: 11, border: "1px solid var(--surface-border)", background: "var(--surface)",
    color: "var(--fg)", outline: "none" },
  button: { font: "inherit", fontSize: 13, fontWeight: 650, padding: "9px 15px",
    borderRadius: 11, border: "1px solid var(--accent-line)", background: "var(--accent-soft)",
    color: "var(--accent)", cursor: "pointer" },
  danger: { font: "inherit", fontSize: 12.5, fontWeight: 650, padding: "7px 12px",
    borderRadius: 10, border: "1px solid var(--danger-line)", background: "var(--danger-wash)",
    color: "var(--danger)", cursor: "pointer" },

  list: { display: "flex", flexDirection: "column", gap: 8, marginBottom: 12 },
  card: { display: "flex", gap: 12, alignItems: "center", justifyContent: "space-between",
    background: "var(--surface-strong)", borderRadius: 12, boxShadow: "var(--v-shadow)",
    padding: "11px 14px" },
  cardMain: { minWidth: 0 },
  cardName: { fontSize: 13.5, fontWeight: 700, letterSpacing: "-.01em",
    overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" },
  cardWhen: { fontSize: 11.5, color: "var(--fg-mute)", marginTop: 2 },

  pair: { background: "var(--surface-strong)", borderRadius: 14, boxShadow: "var(--v-shadow)",
    padding: "16px 18px", display: "flex", flexDirection: "column", alignItems: "flex-start",
    gap: 4, marginTop: 4 },
  code: { fontFamily: "var(--font-mono)", fontSize: 15, fontWeight: 700, letterSpacing: ".02em",
    color: "var(--accent)", wordBreak: "break-all" },
  qr: { width: 168, height: 168, marginTop: 10, borderRadius: 10, background: "#fff", padding: 8 },
};
