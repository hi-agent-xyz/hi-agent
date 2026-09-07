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
// **And now it shows the file, not just its name.** A row used to be a link with
// `target="_blank"`, which answered "is it still there" and nothing beyond it. That
// worked in a browser tab and nowhere else this face is shown: the menu-bar popover is
// a `WKWebView` and the iOS and Android clients are webviews too, none of which have a
// tab to open — and a `.docx` was a download on every one of them, including the
// browser. Holding a contract you cannot read is most of the way to not holding it.
//
// The preview is `@open-file-viewer/core`, resolved through the import map to
// `src/shared/ofv.ts` — see that shim for the two things it changes about the library
// (offline PDF assets, and the stylesheet) and why.
//
// **Watched, and how far.** A PDF, a PNG, a note and an unknown binary were each opened
// in headless Chromium on the host's own `render.html` with the real import map — so the
// bare specifiers, the chunk boundaries, the light/dark read and the same-origin pdf.js
// assets are all observed, not argued. What that run did *not* have is a live core: the
// listing came from a stub serving a `drive/` on disk, so nothing here has been seen
// against a real `/api/drive`, and nothing at all has been seen on a phone — where the
// subpath prefix is the ordinary case and `fileUrl()` above is what has to survive it.
// This is the recall half of journey 19 ("呈现(投成 view)"); it has no 实测 yet.
//
// Colour comes from the host theme tokens (see tasks.jsx for the vocabulary).
import { useEffect, useRef, useState } from "react";
import { url, useLive, TEMPO } from "@hi/core";
// `@open-file-viewer/core` is NOT imported here — see `Preview`. A static import at the
// top of a view module loads when the module loads, which is when the page is opened,
// which would make every look at the list pay for a viewer it may never use.

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
    // The viewer's own chrome (toolbar, error and unsupported states) is localized
    // by the library from this tag, so it has to move with the words around it.
    ofv: "en-US",
    title: "Drive",
    top: { projects: "Projects", notes: "Notes", papers: "Papers" },
    count: (n, size) => `${n === 1 ? "1 file" : `${n} files`} · ${size}`,
    emptyBig: "Nothing kept here yet.",
    emptySub: "Contracts, documents, drafts — anything worth keeping as the original file shows up here once you hand it over.",
    // A shelf that exists and holds nothing is an answer, not a gap in the list.
    shelfEmpty: "Empty.",
    loose: "Loose",
    close: "Close",
    // Position within the shelf being previewed, so the arrows have a scale.
    position: (i, n) => `${i} of ${n}`,
    // Shown only while the viewer chunk is in flight — on a local core that is a
    // flash, over the community relay it is a moment worth accounting for.
    opening: "Opening…",
    openFailed: "Could not load the viewer. The file is still there — use Download.",
  },
  zh: {
    ofv: "zh-CN",
    title: "文件",
    top: { projects: "项目", notes: "笔记", papers: "文稿" },
    count: (n, size) => `${n} 份 · ${size}`,
    emptyBig: "还没存着什么。",
    emptySub: "合同、证件、稿子这类要留原件的东西，传给它以后会在这里。",
    shelfEmpty: "空的。",
    loose: "散着的",
    close: "关闭",
    position: (i, n) => `第 ${i} / ${n}`,
    opening: "正在打开……",
    openFailed: "预览组件没能加载。文件还在，可以直接下载。",
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

/** The bytes endpoint for one drive entry, prefix-correct on a phone. */
function fileUrl(path) {
  return url(`/api/drive/file/${path.split("/").map(encodeURIComponent).join("/")}`);
}

/** The skin this window is actually in — the same read `lib/stageReport.ts` makes. */
function theme() {
  const forced = document.documentElement.getAttribute("data-theme");
  if (forced === "light" || forced === "dark") return forced;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

/**
 * The file itself, over the list.
 *
 * Takes the **whole shelf** and an index rather than one file, so the viewer's own
 * prev/next arrows walk the shelf you opened from. That costs nothing here — the
 * listing is already loaded — and it is how you actually read a drive: three scans of
 * one contract are a thing you page through, not three separate errands back to a list.
 *
 * The viewer is imperative and owns its subtree, so it is built in an effect against a
 * ref and torn down by `destroy()`. `index` is therefore a **starting** position, not a
 * binding: once mounted, the viewer's own arrows move the queue, and the heading follows
 * it via `onLoad` rather than the other way round. That direction is the whole of it —
 * the first version drove the viewer from React state, and pressing "Next file" changed
 * the document on screen while the heading above it went on naming the previous file.
 * One owner per piece of state, and for the queue that owner is the viewer.
 *
 * ── the library arrives here, and not before ─────────────────────────────────
 *
 * The `import()` below is the only reference to it in this file, so the chunk is fetched
 * the first time somebody opens a file — not when this view is opened, which is what a
 * top-of-file import would have meant, and not at boot, which is what the import map
 * already avoids. An import map resolves a dynamic specifier exactly as it does a static
 * one, so this stays a bare name and still lands on the host's chunk.
 *
 * Splitting further — a plugin per file type, chosen from the extension — was measured
 * and dropped. For most of them the case is clear: the viewer core is ~32 KB gzipped and
 * image + text + pdf + audio + video *together* add ~25 KB, so splitting those saves ~20 KB
 * against a floor paid regardless, and costs a chunk graph plus a round trip on any shelf
 * of mixed types. `officePlugin` is the one that could argue back — it statically pulls
 * jszip and docx-preview, and adding it took this chunk from ~113 KB to ~173 KB gzipped,
 * which everyone opening a plain `.txt` now also pays. It stays in anyway: 60 KB, once,
 * cached after, against a `.docx` contract that would otherwise only be downloadable.
 *
 * The split that does pay is already inside the library: pdf.js, heic2any, mermaid and
 * the rest are its own `await import()`s, so a format's real weight arrives only when
 * that format is opened.
 */
function Preview({ files, index, onClose }) {
  const mount = useRef(null);
  const panel = useRef(null);
  const [at, setAt] = useState(index);
  const [ready, setReady] = useState(false);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    panel.current?.focus();
    const onKey = (e) => { if (e.key === "Escape") onClose(); };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    if (!mount.current) return undefined;
    // Closing while the chunk is still in flight is an ordinary thing to do, and the
    // viewer does not exist yet when cleanup runs. `live` is what stops the late
    // resolution from building one into a node React has already taken away.
    let live = true;
    let viewer = null;

    // The `try` covers building the viewer and not only fetching it: a throw from
    // `createViewer` would otherwise leave `ready` false forever, and "Opening…" that
    // never resolves is a worse failure than one that says so.
    (async () => {
      try {
        const ofv = await import("@open-file-viewer/core");
        if (!live) return;

        const {
          createViewer, imagePlugin, textPlugin, pdfPlugin, officePlugin,
          audioPlugin, videoPlugin, fallbackPlugin,
        } = ofv;
        viewer = createViewer({
          container: mount.current,
          // A string source is read as a URL. `fileName` is passed because the extension
          // is what picks the plugin, and a percent-encoded path segment is a poor place
          // to recover it from.
          files: files.map((f) => ({ file: fileUrl(f.path), fileName: f.path.split("/").pop() })),
          initialIndex: index,
          // `imagePlugin` covers scans, `pdfPlugin` and `officePlugin` the documents
          // (a contract arrives as one or the other), `textPlugin` notes and source.
          // `fallbackPlugin` is last and must stay last: it accepts everything, so a
          // plugin after it would never be reached. It is also the honest answer for a
          // format nothing here reads — name, size, type, and a download button —
          // rather than a blank pane.
          plugins: [
            imagePlugin(), pdfPlugin(), officePlugin(), textPlugin(),
            audioPlugin(), videoPlugin(), fallbackPlugin(),
          ],
          locale: L.ofv,
          theme: theme(),
          width: "100%",
          height: "100%",
          // The toolbar is off unless asked for, and without it the shelf queue above has
          // no arrows to walk it — the `‹ 1 / 1 ›` a PDF shows is pdf.js paging one
          // document, not this. Named field by field rather than `true`, because `true`
          // turns on all six and one of them is wrong here: `print` opens the platform
          // print dialog, and this view's main homes are a menu-bar popover and two phone
          // webviews, where that is at best a dead end. `rotate` earns its place for the
          // opposite reason — a scan arrives sideways more often than not.
          toolbar: { zoom: true, rotate: true, download: true, fullscreen: true, search: true, print: false },
          // Fires for every file the queue settles on, including the first — so this is
          // both the initial heading and every move after it. `getCurrentIndex()` rather
          // than matching on the file: two shelves can hold the same name.
          //
          // `viewer?.` because this can in principle fire from inside `createViewer`,
          // before the assignment above has happened. It does not today — the first
          // file's load is asynchronous — and a null here is harmless: `at` is already
          // the initial index.
          onLoad: () => { if (live) setAt((prev) => viewer?.getCurrentIndex() ?? prev); },
        });
        if (live) setReady(true);
      } catch {
        if (live) setFailed(true);
      }
    })();

    return () => { live = false; viewer?.destroy(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [files]);

  const current = files[at];

  return (
    <div style={S.scrim} onClick={onClose}>
      <div ref={panel} style={S.panel} role="dialog" aria-modal="true"
        aria-label={current?.path || L.title} tabIndex={-1}
        onClick={(e) => e.stopPropagation()}>
        <div style={S.panelHead}>
          <div style={S.panelTitle}>
            <span style={S.panelName}>{current?.path.split("/").pop()}</span>
            <span style={S.panelSub}>
              {[current?.path, files.length > 1 ? L.position(at + 1, files.length) : null]
                .filter(Boolean).join(" · ")}
            </span>
          </div>
          <button type="button" style={S.close} aria-label={L.close} onClick={onClose}>×</button>
        </div>
        <div style={S.stageWrap}>
          <div ref={mount} style={S.stage} />
          {!ready && <div style={S.stageNote}>{failed ? L.openFailed : L.opening}</div>}
        </div>
      </div>
    </div>
  );
}

export default function Drive() {
  const [entries, setEntries] = useState(null);
  // Which shelf is open and where in it. Held as the shelf's file array so the
  // viewer's queue survives a poll that reorders nothing — see the `useLive` note.
  const [open, setOpen] = useState(null);

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

  // The frame stays put and the list scrolls inside it, so the preview can be absolutely
  // positioned against the frame. The scroller has to be the child and not the root:
  // anchor the preview to a scrolling box and it slides away with the rows under it.
  return (
    <div style={S.page}>
      <div style={S.scroll}>
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
              {list.map((f, i) => (
                // Still an `<a>` at the bytes, and still the real href: the preview is
                // the primary action, but middle-click, ⌘-click and "save link as" are
                // how people get a file *out*, and a `<div onClick>` would take all
                // three away. The click is intercepted only when it is a plain one.
                <a key={f.path} style={S.row} href={fileUrl(f.path)}
                  onClick={(e) => {
                    if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0) return;
                    e.preventDefault();
                    setOpen({ files: list, index: i });
                  }}>
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

      {open && (
        <Preview
          files={open.files}
          index={open.index}
          onClose={() => setOpen(null)}
        />
      )}
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
  // The positioning context for the preview, and nothing else — see the note at the
  // return. The padding and the scrolling moved to `scroll` below.
  page: { "--v-shadow": "0 1px 2px var(--shadow),0 8px 22px var(--shadow)",
    position: "relative", width: "100%", height: "100%", minHeight: 0,
    color: "var(--fg)", fontFamily: "var(--font-display)" },
  scroll: { width: "100%", height: "100%", minHeight: 0, overflowY: "auto", boxSizing: "border-box",
    padding: "max(20px, var(--hi-safe-top)) clamp(14px,4vw,44px) 128px" },
  head: { display: "flex", alignItems: "baseline", justifyContent: "space-between", marginBottom: 20 },
  h1: { fontSize: "clamp(22px,6vw,30px)", fontWeight: 800, letterSpacing: 0 },
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
  // The row wraps, and the filename claims a floor of 150px before it will shrink.
  // Four fixed-width children on a phone left the name — the only part of the row
  // anyone is reading for — with about 40px and an ellipsis; letting size and date
  // fall to a second line costs a line and keeps the file identifiable.
  row: { display: "flex", alignItems: "center", flexWrap: "wrap", gap: 12, textDecoration: "none", color: "inherit",
    background: "var(--surface-strong)", borderRadius: 13, boxShadow: "var(--v-shadow)",
    padding: "11px 14px", cursor: "pointer" },
  ext: { flex: "none", width: 40, fontSize: 10.5, fontWeight: 800, letterSpacing: ".04em",
    textTransform: "uppercase", color: "var(--accent)", background: "var(--accent-wash)",
    borderRadius: 7, padding: "5px 0", textAlign: "center" },
  name: { flex: "1 1 150px", minWidth: 0, fontSize: 14, fontWeight: 600, letterSpacing: "-.01em",
    overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" },
  size: { flex: "none", fontSize: 12.5, color: "var(--fg-mute)", width: 64, textAlign: "right" },
  when: { flex: "none", fontSize: 12.5, color: "var(--fg-mute)", width: 82, textAlign: "right" },

  // ── the preview layer ───────────────────────────────────────────────────────
  // `absolute`, not `fixed`, and the reason is a rule rather than a preference: a view
  // belongs to its own frame, and `fixed` would escape the content inset and lie over
  // the host's own surfaces — the controls, the conversation. docs/arch/stage.md says an
  // agent surface may never do that, and `workers.jsx` carries the same note.
  //
  // Safe-area padding on all four sides regardless: on a phone the top inset is the
  // notch and the bottom one is the home indicator.
  scrim: { position: "absolute", inset: 0, zIndex: "var(--z-cover)",
    background: "var(--scrim-bg)", backdropFilter: "blur(3px)",
    display: "flex", alignItems: "center", justifyContent: "center",
    padding: "max(12px, var(--hi-safe-top)) max(12px, var(--hi-safe-right)) max(12px, var(--hi-safe-bottom)) max(12px, var(--hi-safe-left))" },
  panel: { display: "flex", flexDirection: "column", width: "min(1100px, 100%)", height: "100%",
    minHeight: 0, background: "var(--surface-strong)", borderRadius: 16, overflow: "hidden",
    boxShadow: "0 1px 2px var(--shadow),0 18px 50px var(--shadow)", outline: "none" },
  panelHead: { flex: "none", display: "flex", alignItems: "center", gap: 12,
    padding: "12px 12px 12px 16px", borderBottom: "1px solid var(--surface-border)" },
  panelTitle: { flex: "1 1 auto", minWidth: 0 },
  panelName: { display: "block", fontSize: 14.5, fontWeight: 700, letterSpacing: "-.01em",
    overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" },
  panelSub: { display: "block", fontSize: 12, color: "var(--fg-mute)", marginTop: 2,
    overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" },
  close: { flex: "none", width: 32, height: 32, borderRadius: 9, border: "none", cursor: "pointer",
    background: "var(--surface)", color: "var(--fg-dim)", fontSize: 19, lineHeight: 1 },
  // The viewer sizes itself to this box and re-reads it on resize, so it must be a
  // real flex child with `minHeight: 0` — without that the pane grows to its content
  // and the panel scrolls instead of the document inside it.
  //
  // This element is also the one the library puts `.ofv-root` on, which is what lets
  // the `--ofv-*` overrides below land: an inline custom property beats the value the
  // class sets. Its own dark palette is a cold navy against this app's warm, matte one
  // — side by side in the same panel that reads as two programs, so the seam runs right
  // under the filename. Only the surfaces, ink and accent are re-pointed; the library
  // keeps its own layout, spacing and shadows.
  stageWrap: { position: "relative", flex: "1 1 auto", minHeight: 0, minWidth: 0, display: "flex" },
  // Sits over the empty mount until the viewer has been built into it. Centred and
  // quiet: this is a wait, not an event.
  stageNote: { position: "absolute", inset: 0, display: "flex", alignItems: "center",
    justifyContent: "center", padding: "0 24px", textAlign: "center",
    fontSize: 13.5, color: "var(--fg-mute)", pointerEvents: "none" },
  stage: { flex: "1 1 auto", minHeight: 0, minWidth: 0,
    "--ofv-bg": "transparent",
    "--ofv-surface": "var(--surface)",
    "--ofv-surface-muted": "var(--surface-strong)",
    "--ofv-toolbar-bg": "transparent",
    "--ofv-text": "var(--fg)",
    "--ofv-text-muted": "var(--fg-mute)",
    "--ofv-border": "var(--surface-border)",
    "--ofv-button-hover": "var(--accent-wash)",
    "--ofv-accent": "var(--accent)",
    "--ofv-accent-soft": "var(--accent-soft)",
    // Not re-pointed: `--ofv-highlight` is the search hit marker, and it has to stay a
    // colour nothing else on the page uses — the accent already means "interactive".
  },
};
