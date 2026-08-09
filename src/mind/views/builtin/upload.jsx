// purpose: hand the agent a file — drag-drop or pick on this device, or scan a QR to send one from a phone.
// Shown when the user wants to hand over a file (a contract, a passport scan).
// A handed file is an artifact, not something
// the agent looks at — both doors post to the `file` channel, which wakes the
// agent. Seeded at `_builtin/upload`; the agent may adapt it like any view.
import { useState, useEffect, useRef } from "react";

// ── words ─────────────────────────────────────────────────────────────────────
// English is the default and the fallback. This surface used to be Chinese-only; the
// strings are the same handful, now selected off the person's language setting.
//
// TODO(i18n): en + zh are hand-written. Further languages are meant to be authored at
// runtime — the agent reads the surface and writes the variant — rather than shipped
// here. Until that exists, an unsupported language lands on English.
const T = {
  en: {
    title: "Send me a file",
    dropHere: "Drop a file here",
    pick: "or click to pick · contracts, ID photos, PDFs all work",
    qrAlt: "Scan to upload", qrHint: "Scan with your phone", qrWait: "Making the code…",
    qrFailed: "Couldn't make the code.", qrRetry: "Try again",
    sent: "sent", failed: "failed", sending: "sending",
  },
  zh: {
    title: "传文件给我",
    dropHere: "把文件拖到这里",
    pick: "或点击选择 · 合同、证件照、PDF 都行",
    qrAlt: "扫码上传", qrHint: "手机扫码传", qrWait: "二维码准备中…",
    qrFailed: "二维码没做出来。", qrRetry: "再试一次",
    sent: "已发送", failed: "失败", sending: "发送中",
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

export default function Upload() {
  const [url, setUrl] = useState(null);
  const [qrFailed, setQrFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [items, setItems] = useState([]); // { key, name, state: sending|done|error }
  const [drag, setDrag] = useState(false);
  const inputRef = useRef(null);

  // Mint an upload link for the phone QR. A failure has to say so and offer another go:
  // the door that stays on "preparing…" for ever reads as a slow one, and the person
  // waits for a code that is never coming instead of using the drop zone beside it.
  useEffect(() => {
    let alive = true;
    setQrFailed(false);
    fetch("/api/handoff", { method: "POST" })
      .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
      .then((d) => {
        if (!alive) return;
        if (d?.url) setUrl(d.url);
        else setQrFailed(true);
      })
      .catch(() => alive && setQrFailed(true));
    return () => {
      alive = false;
    };
  }, [attempt]);

  async function send(files) {
    for (const file of files) {
      const key = file.name + ":" + file.size + ":" + file.lastModified;
      setItems((xs) => [...xs.filter((x) => x.key !== key), { key, name: file.name, state: "sending" }]);
      try {
        const fd = new FormData();
        fd.append("file", file, file.name);
        const r = await fetch("/api/in/file", { method: "POST", body: fd });
        setItems((xs) => xs.map((x) => (x.key === key ? { ...x, state: r.ok ? "done" : "error" } : x)));
      } catch {
        setItems((xs) => xs.map((x) => (x.key === key ? { ...x, state: "error" } : x)));
      }
    }
  }

  function onDrop(e) {
    e.preventDefault();
    setDrag(false);
    const files = e.dataTransfer?.files;
    if (files?.length) send([...files]);
  }

  return (
    <div style={S.root}>
      <div style={S.title}>{L.title}</div>
      <div style={S.row}>
        <div
          onDragOver={(e) => {
            e.preventDefault();
            setDrag(true);
          }}
          onDragLeave={() => setDrag(false)}
          onDrop={onDrop}
          onClick={() => inputRef.current?.click()}
          style={{ ...S.drop, ...(drag ? S.dropActive : null) }}
        >
          <div style={{ fontSize: 40, marginBottom: 8 }}>⬆</div>
          <div style={{ fontWeight: 600, fontSize: 16 }}>{L.dropHere}</div>
          <div style={S.hint}>{L.pick}</div>
          <input
            ref={inputRef}
            type="file"
            multiple
            hidden
            onChange={(e) => {
              if (e.target.files?.length) send([...e.target.files]);
              e.target.value = "";
            }}
          />
        </div>

        <div style={S.qrCol}>
          {url ? (
            <>
              <img alt={L.qrAlt} width={148} height={148} style={S.qrImg} src={"/api/qr?data=" + encodeURIComponent(url)} />
              <div style={S.hint}>{L.qrHint}</div>
            </>
          ) : qrFailed ? (
            <>
              <div style={S.hint}>{L.qrFailed}</div>
              <button type="button" style={S.retry} onClick={() => setAttempt((n) => n + 1)}>{L.qrRetry}</button>
            </>
          ) : (
            <div style={S.hint}>{L.qrWait}</div>
          )}
        </div>
      </div>

      {items.length > 0 && (
        <div style={S.list}>
          {items.map((x) => (
            <div key={x.key} style={S.listRow}>
              <span style={{ width: 18 }}>{x.state === "done" ? "✓" : x.state === "error" ? "⚠" : "…"}</span>
              <span style={S.name}>{x.name}</span>
              <span style={S.hint}>{x.state === "done" ? L.sent : x.state === "error" ? L.failed : L.sending}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// Full-canvas utility surface. The view owns its background and scrolling while
// keeping the handoff controls at a readable working width.
const S = {
  root: { width: "100%", height: "100%", minHeight: 0, overflowY: "auto", boxSizing: "border-box",
    display: "flex", flexDirection: "column", gap: 18,
    padding: "36px clamp(20px,5vw,72px) 128px", background: "var(--bg-0)",
    fontFamily: "var(--font-display)", color: "var(--fg)" },
  title: { width: "min(980px,100%)", margin: "0 auto", fontWeight: 800, fontSize: 28,
    lineHeight: 1.2, letterSpacing: 0, textAlign: "left" },
  row: { width: "min(980px,100%)", margin: "0 auto", display: "flex", flexWrap: "wrap",
    gap: 16, alignItems: "stretch" },
  drop: { flex: "1 1 240px", minHeight: 184, border: "2px dashed var(--line-strong)", borderRadius: 16, display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", textAlign: "center", padding: 20, cursor: "pointer", transition: "border-color .15s, background .15s" },
  dropActive: { borderColor: "var(--accent-line)", background: "var(--accent-wash)" },
  qrCol: { flex: "0 0 auto", display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", gap: 8, minWidth: 168 },
  // The white plate stays literal in both themes: a QR needs its light quiet zone to scan.
  qrImg: { borderRadius: 12, background: "#fff", padding: 8 },
  hint: { color: "var(--fg-mute)", fontSize: 13 },
  retry: { appearance: "none", font: "inherit", fontSize: 13, fontWeight: 700, cursor: "pointer",
    color: "var(--accent)", background: "none", border: "none", padding: "2px 4px" },
  list: { width: "min(980px,100%)", margin: "0 auto", display: "flex", flexDirection: "column", gap: 6 },
  listRow: { display: "flex", gap: 8, alignItems: "center", fontSize: 14 },
  name: { flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" },
};
