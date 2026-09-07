// Open File Viewer — inline preview for a file the agent is holding.
//
// A drive file used to be an `<a href target="_blank">`: fine for a PNG in a
// browser tab, useless in the popover `WKWebView` and on the phone shells, where
// there is no tab to open, and useless for `.docx`/`.xlsx` anywhere, which the
// browser only downloads. This renders into a container the view owns instead.
//
// Like `d3-hierarchy.ts` and unlike its other neighbours here, this is NOT about
// a shared instance — OFV holds no state the host needs to see. It is here for
// the reason `views/mod.rs` gives: a view is *transformed*, never bundled, so its
// bare imports survive into the compiled `.mjs`, and the user's machine has no
// node_modules to resolve them from. The host has to ship it and name it in the
// map or the import is unresolvable and the view renders blank.
//
// ── why this file is not `export * from "@open-file-viewer/core"` ──────────────
//
// Upstream's `pdfPlugin()` defaults its worker, its CMaps and its standard fonts
// to `https://cdn.jsdelivr.net/npm/pdfjs-dist@<version>/...` (see `configurePdfWorker`
// and the `cMapUrl`/`standardFontDataUrl` defaults in its `plugins/pdf.ts`). Both
// halves of that are wrong here:
//
//   - **Offline.** A core on a laptop with no network would fail to open a PDF it
//     is holding on disk. Nothing else in this app needs the internet to show you
//     something it already has.
//   - **Privacy.** `docs/arch/privacy.md` puts the model behind a loopback proxy so
//     that reaching outward is a decided act. A CDN learning the moment you opened
//     a contract — and, via the CMap requested, roughly what script it is in — is
//     the same class of leak arriving through a side door.
//
// So `pdfPlugin` below **shadows** the upstream export under the same name: same
// signature, same options, offline defaults, and an explicit option still wins.
// Shadowing rather than renaming is deliberate. Views are written by the agent at
// runtime, largely from prior knowledge of this library's public API, and a view
// that reasonably writes `pdfPlugin({})` must not silently reach the CDN. There is
// no review pass in which a forgotten option gets caught.
//
// The assets themselves are served same-origin by the host — see `pdfjsAssets()`
// in `vite.config.ts` for how they get into `dist/` and how dev mirrors it.
//
// ── two pins, on purpose ──────────────────────────────────────────────────────
//
// Both versions in `package.json` are exact, not `^`. The library is at 0.1.x, three
// months old, and ~85% of its commits are one author's — at 0.x a caret still lets
// every patch in, and this file depends on internals a patch may move: that
// `pdfPlugin` takes `workerSrc`/`cMapUrl`/`standardFontDataUrl`, and that
// `style.css` is a separate export. `pdfjs-dist` is pinned because the worker's
// path (`build/pdf.worker.mjs`) is version-shaped and the CMap and font trees are
// copied out of the package by `vite.config.ts` at build time; those three have to
// agree, and a float is how they stop agreeing.
//
// Known and accepted at this pin: `@aiden0z/pptx-renderer`, a transitive dependency
// of the library, asks for `pdfjs-dist >=5`, so `npm ls` reports it unmet. It is
// inert here — that renderer is reached only through `officePlugin`, which this
// build does not register — and satisfying it would mean moving pdfjs off the
// version the library itself develops against.
//
// ── which plugins are re-exported ─────────────────────────────────────────────
//
// Everything `drive/` is specified to hold — `projects/`, `notes/`, `papers/`:
// contracts, scans, drafts, notes. `officePlugin` is in that list because a contract
// is a `.docx` far more often than it is anything else, and a contract you cannot
// read is most of the reason this view exists.
//
// They travel in one chunk rather than one per format, which is measured rather than
// assumed: the viewer core alone is ~32 KB gzipped and image + text + pdf + audio +
// video *together* add ~25 KB on top, so splitting would save ~20 KB in the best
// case against a floor paid regardless — and cost a chunk graph plus a round trip on
// any shelf of mixed types. The split that does pay is already the library's own:
// pdf.js, heic2any, mermaid and the rest are `await import()`ed inside each plugin,
// so a format's real weight arrives only when that format is actually opened.
//
// Nothing is excluded on size grounds. An earlier version of this file dropped
// `officePlugin` (jszip + docx-preview, statically imported, the heaviest entry
// here) and aliased `mermaid` and `hls.js` to a throwing stub, to keep the embedded
// SPA down by a few MB. That was the wrong trade and is reverted: the `.dmg` this
// ships inside is a few hundred MB, so single-digit MB is noise, while a format that
// silently fails to preview is not.
export {
  imagePlugin,
  textPlugin,
  officePlugin,
  audioPlugin,
  videoPlugin,
  fallbackPlugin,
  isPreviewSupported,
} from "@open-file-viewer/core";

export type {
  FileViewer,
  PdfPluginOptions,
  PreviewOptions,
  PreviewSource,
  PreviewItem,
  PreviewPlugin,
  PreviewLocale,
  PreviewMessages,
  PreviewTheme,
} from "@open-file-viewer/core";

import {
  createViewer as upstreamCreateViewer,
  pdfPlugin as upstreamPdfPlugin,
  type PdfPluginOptions,
  type PreviewOptions,
} from "@open-file-viewer/core";
import { url } from "../lib/base";

// The stylesheet, as a string, injected on first use rather than linked from the
// page. A view is transformed and never bundled, so it cannot `import` a CSS file
// itself — and linking it from `index.html` would make every surface pay ~20 KB
// gzipped on every load for a viewer most of them never open. Carried inside this
// chunk it arrives when the chunk does, which `drive.jsx` defers until somebody
// actually opens a file.
import ofvStyles from "@open-file-viewer/core/style.css?inline";

// Vite emits the worker as its own hashed asset and hands back the URL. Importing
// it (rather than naming `/pdfjs/pdf.worker.mjs` by hand) keeps it content-hashed
// and keeps the pdfjs version in one place — package.json.
import pdfWorkerUrl from "pdfjs-dist/build/pdf.worker.mjs?url";

// `url()` and not a bare path: on a phone the core is served under a subpath
// (`https://hi-agent.xyz/ana`), so `/assets/…` lands on the community's routes
// instead. `installBase()` patches `fetch`, which covers the CMap and font reads,
// but **not** the worker — that is `new Worker(src)` inside pdfjs, which no seam
// intercepts. Prefixing all three here is uniform and `url()` is idempotent.
//
// The trailing slashes matter: pdfjs concatenates, it does not join.
const WORKER_SRC = () => url(pdfWorkerUrl);
const CMAP_URL = () => url("/pdfjs/cmaps/");
const STANDARD_FONT_URL = () => url("/pdfjs/standard_fonts/");

/**
 * `pdfPlugin` with every remote default replaced by a same-origin one.
 *
 * Identical to upstream otherwise; pass `workerSrc`, `cMapUrl` or
 * `standardFontDataUrl` explicitly and yours wins.
 */
export function pdfPlugin(options: PdfPluginOptions = {}) {
  return upstreamPdfPlugin({
    workerSrc: WORKER_SRC(),
    cMapUrl: CMAP_URL(),
    standardFontDataUrl: STANDARD_FONT_URL(),
    ...options,
  });
}

const STYLE_ID = "ofv-styles";

/**
 * `createViewer`, with the library's stylesheet guaranteed to be present.
 *
 * Idempotent and cheap after the first call: one `<style>` in `<head>`, keyed by
 * id, shared by every viewer on the page. Injecting here rather than asking each
 * caller to do it is the same reasoning as the `pdfPlugin` shadow above — a view
 * that forgets would render the viewer completely unstyled, and there is no
 * review pass on a view the agent wrote at runtime.
 */
export function createViewer(options: PreviewOptions) {
  if (typeof document !== "undefined" && !document.getElementById(STYLE_ID)) {
    const style = document.createElement("style");
    style.id = STYLE_ID;
    style.textContent = ofvStyles;
    document.head.append(style);
  }
  return upstreamCreateViewer(options);
}
