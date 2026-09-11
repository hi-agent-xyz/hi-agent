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
import { useState, useCallback } from "react";
import { useLive, TEMPO } from "@hi/core";

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
    unreachable: "Your agent did not answer. Nothing was claimed.",
    alsoYours: (names) => `Also yours, not answered to here: ${names.join(", ")}`,
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
    pairWith: "Open this address on the other device and enter this code. It lasts ten minutes.",
    pairAt: "or scan with the Hi Agent app",
    done: "Done",
    // — devices asking to be let in
    waiting: "Waiting to be let in",
    waitingWhy: "Check the code matches the one on that device before you let it in.",
    approve: "Let in",
    deny: "No",
    asked: (when) => `asked ${when}`,
    // Registering a device to a person. The copy has to say what it changes and what
    // it does not, because "whose phone is this" sounds like it should cover the
    // microphone and it deliberately does not.
    whose: "Whose device",
    nobody: "Not said",
    whoseWhy: "What's typed, handed over or opened on it is attributed to them. Not what its microphone hears — that is answered by the voice itself.",
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
    unreachable: "你的 agent 没有回应，名字没有取成。",
    alsoYours: (names) => `也是你的，但这台机器不用：${names.join("、")}`,
    devices: "设备",
    devicesWhy: "能找到它的东西。每一个都是你放进来的，也可以在这里收回。",
    noDevices: "还没放进来过任何设备。",
    neverUsed: "还没用过",
    lastSeen: (when) => `最近 ${when}`,
    added: (when) => `${when} 加入`,
    revoke: "收回",
    revoking: "收回中…",
    addDevice: "加一台设备",
    pairWith: "在另一台设备上打开这个地址，输入这个码。十分钟内有效。",
    pairAt: "或者用 Hi Agent App 扫码",
    done: "好了",
    waiting: "有设备在等着进来",
    waitingWhy: "放行之前，先核对那台设备上显示的码是不是这一个。",
    approve: "放进来",
    deny: "不行",
    asked: (when) => `${when}问的`,
    whose: "谁的设备",
    nobody: "没指定",
    whoseWhy: "在上面打的字、递过来的文件、点开的页面，都算在这个人名下。麦克风听到的不算——那由声音本身回答。",
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
  // Who there is to register a device to, and whether this window is allowed to.
  // Registering is loopback-only, so a phone opening this page must not be shown a
  // control that would only ever be refused.
  const [people, setPeople] = useState([]);
  const [settable, setSettable] = useState(false);
  // Devices that have asked to be let in and are waiting on a person. This page is
  // the only place they appear — the agent is deliberately never told, because the
  // endpoint that files them is unauthenticated.
  const [waiting, setWaiting] = useState([]);

  const loadDevices = useCallback(
    () =>
      fetch("/api/surfaces")
        .then((r) => r.json())
        .then((d) => {
          setDevices(d.surfaces || []);
          setSettable(!!d.subject_settable);
        })
        .catch(() => setDevices((prev) => (prev === null ? [] : prev))),
    [],
  );

  // The people the agent knows, so a device is registered by picking somebody rather
  // than by typing a name that may match nobody.
  const loadPeople = useCallback(
    () =>
      fetch("/api/people")
        .then((r) => r.json())
        .then((d) => setPeople((d.people || []).map((p) => p.subject)))
        .catch(() => {}),
    [],
  );

  const loadRequests = useCallback(
    () =>
      fetch("/api/access/requests")
        .then((r) => r.json())
        .then((d) => setWaiting(d.requests || []))
        // Keep the last answer on a failed read: a request blinking out of a list
        // somebody is about to click is worse than one shown a tick too long.
        .catch(() => {}),
    [],
  );

  const loadHandle = useCallback(
    () =>
      // Always answers: a core with no name is a normal core, and the reason it
      // has none (no account yet, no community reachable) rides in `why`.
      fetch("/api/handle")
        .then((r) => r.json())
        .then(setHandle)
        .catch(() => setHandle((prev) => prev ?? { handles: [] })),
    [],
  );

  // The name changes when they claim one and when the community becomes reachable — the
  // second of those has no click behind it, so `why` can go stale on its own.
  useLive(loadHandle, { period: TEMPO.ledger });

  return (
    <div style={S.page}>
      <div style={S.h1}>{L.title}</div>
      <Name state={handle} onChanged={loadHandle} />
      <Devices list={devices} people={people} settable={settable}
        waiting={waiting} reloadRequests={loadRequests}
        reload={loadDevices} reloadPeople={loadPeople} />
    </div>
  );
}

// ── the name ──────────────────────────────────────────────────────────────────

function Name({ state, onChanged }) {
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState("");

  if (state === null) return <Section title={L.name} why={L.nameWhy} />;

  // The name this core answers to, which is not the same as the first name the
  // account ever took — an account may hold three. `serving` is the core's own
  // answer; the fallback is for an older core that does not send it yet.
  const all = state.handles || [];
  const held = all.find((h) => h.handle === state.serving) || all[0];
  const others = all.filter((h) => h.handle !== held?.handle).map((h) => h.handle);

  async function claim() {
    const name = draft.trim().toLowerCase();
    if (!name || busy) return;
    setBusy(true);
    setRefused("");
    try {
      const r = await fetch("/api/handle", {
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
      // The core did not answer at all. Clearing the message here left the hint
      // line reading the naming rules, which is what a *successful* claim looks
      // like — a failure that reads as nothing having gone wrong.
      setRefused(L.unreachable);
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
      {/* A name is permanent, so renaming does not give the old one back — it is
          still theirs, and saying so is the difference between a rename that
          looks like it lost something and one that reads as what it is. */}
      {others.length > 0 && <div style={S.note}>{L.alsoYours(others)}</div>}
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

function Devices({ list, people, settable, waiting, reloadRequests, reload, reloadPeople }) {
  const [pairing, setPairing] = useState(null); // {code, url, app_url}
  const [busy, setBusy] = useState("");

  // The pairing itself finishes on the *phone*. Nothing on this page hears about it, so
  // without a clock the one moment this section exists for is the one it cannot show: they
  // scan the code, the phone says paired, and the list still reads "no devices".
  //
  // Two speeds, because those are two different things to be doing. While the code is up
  // they are watching for an arrival and it should land as it happens; the rest of the time
  // this is a ledger of what already has a way in, and `last_seen_at` on each card drifts
  // whether or not anyone clicks. Held while a revoke is in flight, so a device cannot
  // reappear between the DELETE and its reload.
  useLive(reload, {
    period: pairing ? TEMPO.watching : TEMPO.ledger,
    hold: () => busy !== "",
  });
  // The roster grows on its own — the agent mints a cluster every time it hears or
  // sees somebody it cannot place — so the picker is refreshed on the same clock as
  // the list it sits in.
  useLive(reloadPeople, { period: TEMPO.ledger, hold: () => busy !== "" });
  // Whoever is asking is standing there holding the device, so this one is always
  // the watching clock: on the ledger's eight seconds they stare at a screen that
  // says nothing happened, which is what "is this broken" feels like.
  useLive(reloadRequests, { period: TEMPO.watching, hold: () => busy !== "" });

  async function register(id, subject) {
    setBusy(id);
    try {
      await fetch(`/api/surfaces/${id}/subject`, {
        method: "POST",
        headers: WRITE,
        body: JSON.stringify({ subject }),
      });
      reload();
    } finally {
      setBusy("");
    }
  }

  async function addDevice() {
    setBusy("pair");
    try {
      const r = await fetch("/api/pair", { method: "POST", headers: WRITE });
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
      await fetch(`/api/surfaces/${id}`, { method: "DELETE", headers: WRITE });
      reload();
    } finally {
      setBusy("");
    }
  }

  // Approving mints a credential the waiting device then claims, so both lists
  // change: the request leaves this one and a device appears in the other.
  async function answer(id, letIn) {
    setBusy(id);
    try {
      await fetch(letIn ? `/api/access/requests/${id}/approve` : `/api/access/requests/${id}`, {
        method: letIn ? "POST" : "DELETE",
        headers: WRITE,
      });
    } finally {
      setBusy("");
    }
    reloadRequests();
    if (letIn) reload();
  }

  return (
    <Section title={L.devices} why={settable ? `${L.devicesWhy} ${L.whoseWhy}` : L.devicesWhy}>
      {waiting.length > 0 && (
        <div style={S.waiting}>
          <div style={S.waitingHead}>{L.waiting}</div>
          <div style={S.hint}>{L.waitingWhy}</div>
          {waiting.map((r) => (
            <div key={r.id} style={S.askCard}>
              {/* The code and whose device it is are one reading — "216948, the TV" —
                  so they travel together and the buttons are the thing that wraps.
                  Split across the card's full width the label lands at the far margin
                  and stops looking like a caption for the number beside it. */}
              <div style={S.askWho}>
                {/* Big enough to read across a desk: the whole point of the code is
                    that two people compare it from where they are standing. */}
                <div style={S.askCode}>{r.code}</div>
                <div style={S.cardMain}>
                  <div style={S.cardName}>{r.label}</div>
                  <div style={S.cardWhen}>{L.asked(ago(r.asked_at))}</div>
                </div>
              </div>
              <div style={S.row}>
                <button
                  style={S.button}
                  onClick={() => answer(r.id, true)}
                  disabled={busy === r.id}
                >
                  {L.approve}
                </button>
                <button
                  style={S.danger}
                  onClick={() => answer(r.id, false)}
                  disabled={busy === r.id}
                >
                  {L.deny}
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

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
                {settable ? (
                  <label style={S.whoseRow}>
                    <span style={S.whoseLabel}>{L.whose}</span>
                    <select
                      style={S.whosePick}
                      value={s.subject || ""}
                      disabled={busy === s.id}
                      onChange={(e) => register(s.id, e.target.value)}
                    >
                      <option value="">{L.nobody}</option>
                      {(people.includes(s.subject) || !s.subject
                        ? people
                        : [s.subject, ...people]
                      ).map((who) => (
                        <option key={who} value={who}>
                          {who}
                        </option>
                      ))}
                    </select>
                  </label>
                ) : (
                  s.subject && (
                    <div style={S.cardWhen}>
                      {L.whose}: {s.subject}
                    </div>
                  )
                )}
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
          {/* The address as well as the code, because the other device needs both
              and the QR is the only other place it appears — unreadably. `url` is
              what the core says it is reachable at (`public_base_url`), so showing
              it here means a wrong one is visible instead of encoded in a QR that
              silently fails to pair. */}
          {pairing.url && <div style={S.pairAddress}>{pairing.url}</div>}
          <div style={S.code}>{pairing.code}</div>
          <div style={S.hint}>{L.pairWith}</div>
          {pairing.url && (
            <>
              <img
                style={S.qr}
                alt=""
                src={`/api/qr?data=${encodeURIComponent(pairing.app_url || pairing.url)}`}
              />
              <div style={S.hint}>{L.pairAt}</div>
            </>
          )}
          <button
            style={S.button}
            onClick={() => {
              setPairing(null);
              // Read once on the way out as well as on the clock: closing the panel is a
              // person saying they are finished pairing, and the tick they would otherwise
              // wait for is the slow one, since the panel is gone by then.
              reload();
            }}
          >
            {L.done}
          </button>
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
    padding: "max(20px, var(--hi-safe-top)) clamp(14px,4vw,44px) 128px",
    color: "var(--fg)", fontFamily: "var(--font-display)" },
  h1: { fontSize: "clamp(22px,6vw,30px)", fontWeight: 800, letterSpacing: 0, marginBottom: 26 },

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
  whoseRow: { display: "flex", alignItems: "center", gap: 8, marginTop: 7 },
  whoseLabel: { fontSize: 11.5, color: "var(--fg-mute)" },
  whosePick: { font: "inherit", fontSize: 12, padding: "3px 6px", borderRadius: 7,
    border: "1px solid var(--line-strong)", background: "transparent", color: "var(--fg)" },

  waiting: { display: "flex", flexDirection: "column", gap: 8, marginBottom: 16,
    padding: "12px 14px", borderRadius: 14, background: "var(--accent-soft)",
    border: "1px solid var(--accent-line)" },
  waitingHead: { fontSize: 13.5, fontWeight: 750, letterSpacing: "-.01em", color: "var(--accent)" },
  askCard: { display: "flex", gap: 12, alignItems: "center", justifyContent: "space-between",
    background: "var(--surface-strong)", borderRadius: 12, boxShadow: "var(--v-shadow)",
    padding: "11px 14px", flexWrap: "wrap" },
  askWho: { display: "flex", gap: 12, alignItems: "center", minWidth: 0, flex: "1 1 auto" },
  askCode: { fontFamily: "var(--font-mono)", fontSize: 26, fontWeight: 800, letterSpacing: ".1em",
    color: "var(--fg)", flex: "0 0 auto" },

  pair: { background: "var(--surface-strong)", borderRadius: 14, boxShadow: "var(--v-shadow)",
    padding: "16px 18px", display: "flex", flexDirection: "column", alignItems: "flex-start",
    gap: 4, marginTop: 4 },
  pairAddress: { fontFamily: "var(--font-mono)", fontSize: 13.5, fontWeight: 650,
    color: "var(--fg)", wordBreak: "break-all", marginBottom: 2 },
  code: { fontFamily: "var(--font-mono)", fontSize: 15, fontWeight: 700, letterSpacing: ".02em",
    color: "var(--accent)", wordBreak: "break-all" },
  qr: { width: 168, height: 168, marginTop: 10, borderRadius: 10, background: "#fff", padding: 8 },
};
