import { defineConfig, type Plugin, type ProxyOptions } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import basicSsl from "@vitejs/plugin-basic-ssl";
import { fileURLToPath } from "node:url";
import { cpSync, createReadStream, existsSync, statSync } from "node:fs";
import { join } from "node:path";

// Resolve a filesystem path relative to this config file. URL pathnames keep
// percent-encoding (notably spaces), while Vite needs the decoded OS path.
const r = (p: string) => fileURLToPath(new URL(p, import.meta.url));

// The shared-instance contract. Each shim entry (src/shared/*) re-exports a
// dependency that BOTH host chrome and agent-authored view modules must share a
// single instance of. The shadcn entries are the actual generated component
// source files, exposed under their standard import names — no UI barrel or
// wrapper package sits between a view and shadcn.
const SHARED_SPECIFIERS: Record<string, string> = {
  "src/shared/react.ts": "react",
  "src/shared/react-dom.ts": "react-dom",
  "src/shared/jsx-runtime.ts": "react/jsx-runtime",
  "src/shared/motion.ts": "motion/react",
  "src/shared/core.ts": "@hi/core",
};

// Libraries a view may import, which is a different thing from the contract above.
// Nothing here needs to be a single instance — these are pure functions over plain
// objects, and two copies would behave identically. They are published for the one
// reason a view cannot get a library any other way: a view is *transformed*, never
// bundled (`views/mod.rs`), so its bare imports survive into the compiled `.mjs` for
// the browser to resolve, and the user's machine has no node_modules to bundle from.
// Unmapped specifier, unresolvable module, blank view.
//
// Kept apart from `SHARED_SPECIFIERS` so that table keeps meaning what it says. An
// entry there is load-bearing — React must be one instance or hooks break across the
// view boundary — and folding a convenience into it would make the two
// indistinguishable, so the next reader could not tell which entries may be dropped.
//
// The cost is real and is the bar for adding one, though it is not quite what this
// paragraph used to say. An entry is built as its own chunk and ships in the host
// bundle unconditionally — that weight is in the `.dmg`, the `.deb` and the Docker
// image whether or not any view ever imports it. What it does *not* do is load on
// every page: entry chunks named only by the import map are absent from the
// `modulepreload` list `index.html` carries, so the browser fetches one the first
// time something imports it and not before. Measured, on this build.
//
// So the bar is disk, and latency only for whoever actually reaches the feature —
// which is worth knowing before deciding how hard to fight for a smaller entry.
//
// `@open-file-viewer/core` maps to `src/shared/ofv.ts`, not to the package: that
// shim shadows `pdfPlugin` so a view cannot accidentally fetch the PDF worker and
// CMaps from jsDelivr, which is upstream's default. The specifier stays the real
// package name so a view the agent writes from prior knowledge of the library
// resolves — see the shim's header for why shadowing beat renaming.
const LIBRARY_SPECIFIERS: Record<string, string> = {
  "src/shared/d3-hierarchy.ts": "d3-hierarchy",
  "src/shared/d3-flextree.ts": "d3-flextree",
  "src/shared/ofv.ts": "@open-file-viewer/core",
};

const SHADCN_SPECIFIERS: Record<string, string> = Object.fromEntries(
  [
    "accordion",
    "alert",
    "avatar",
    "badge",
    "button",
    "card",
    "checkbox",
    "input",
    "label",
    "progress",
    "scroll-area",
    "select",
    "separator",
    "skeleton",
    "switch",
    "table",
    "tabs",
    "textarea",
    "tooltip",
  ].map((name) => [
    `src/ui/shadcn/${name}.tsx`,
    `@/components/ui/${name}`,
  ]),
);

const IMPORT_SPECIFIERS = {
  ...SHARED_SPECIFIERS,
  ...LIBRARY_SPECIFIERS,
  ...SHADCN_SPECIFIERS,
};

// After the bundle is built, write dist/importmap.json mapping each shared bare
// specifier to its emitted, content-hashed chunk URL. The Rust `index()` handler
// injects this map into the served HTML (Stage 2).
function emitImportMap(): Plugin {
  return {
    name: "hi-emit-importmap",
    generateBundle(_options, bundle) {
      const imports: Record<string, string> = {};
      for (const chunk of Object.values(bundle)) {
        if (chunk.type !== "chunk" || !chunk.isEntry || !chunk.facadeModuleId) continue;
        const facade = chunk.facadeModuleId.replace(/\\/g, "/");
        for (const [suffix, spec] of Object.entries(IMPORT_SPECIFIERS)) {
          if (facade.endsWith(suffix)) imports[spec] = "/" + chunk.fileName;
        }
      }
      this.emitFile({
        type: "asset",
        fileName: "importmap.json",
        source: JSON.stringify({ imports }, null, 2),
      });
    },
  };
}

// Dev mirror of the import map. In prod the Rust `index()` handler injects a map
// pointing each specifier at its built entry chunk; in dev there is no build, so
// we point at the source module Vite serves. An agent view fetched raw from the
// backend then resolves React, @hi/core and direct shadcn imports to the same
// modules the host loaded. Only `apply: "serve"`: the build path emits its map.
//
// Why this only affects views: Vite pre-resolves the host's own bare imports at
// transform time, so they never consult the import map — only the backend-served
// view modules carry live bare specifiers for the browser to resolve.
function devImportMap(): Plugin {
  const imports = Object.fromEntries(
    Object.entries(IMPORT_SPECIFIERS).map(([file, spec]) => [spec, "/" + file]),
  );
  return {
    name: "hi-dev-importmap",
    apply: "serve",
    transformIndexHtml() {
      return [
        {
          tag: "script",
          attrs: { type: "importmap" },
          children: JSON.stringify({ imports }, null, 2),
          injectTo: "head-prepend",
        },
      ];
    },
  };
}

// The two pdfjs asset trees that are not JavaScript, served same-origin under
// `/pdfjs/`. `src/shared/ofv.ts` points pdfjs at them so that opening a PDF the
// core is already holding needs no network — upstream's default is a jsDelivr URL.
//
// CMaps are what make a CJK PDF render at all when it references a predefined
// character collection instead of embedding one, so on this product they are not
// the optional half: ~1.7 MB for 169 files, against a scanned Chinese contract
// showing blank glyphs. `standard_fonts` (~800 KB) covers PDFs that name a base-14
// font without embedding it. Both are only ever fetched once a PDF is opened.
//
// Not `public/`: that would vendor 185 binaries into git to be re-synced by hand
// on every pdfjs bump. Copying from node_modules at build time keeps the version
// in package.json, where the worker import already reads it.
function pdfjsAssets(): Plugin {
  const trees = ["cmaps", "standard_fonts"];
  const from = (tree: string) => r(`./node_modules/pdfjs-dist/${tree}`);
  return {
    name: "hi-pdfjs-assets",
    // Prod: land them in dist/ so RustEmbed picks them up with everything else.
    writeBundle(options) {
      const outDir = options.dir ?? r("./dist");
      for (const tree of trees) {
        cpSync(from(tree), join(outDir, "pdfjs", tree), { recursive: true });
      }
    },
    // Dev: Vite only serves `public/`, so mirror the prod path by hand or every
    // PDF opened in dev silently falls back to... nothing, since the shim removed
    // the CDN default. Same seam the import map needs mirroring for, same reason.
    configureServer(server) {
      server.middlewares.use("/pdfjs", (req, res, next) => {
        const rel = decodeURIComponent((req.url ?? "").split("?")[0] ?? "").replace(/^\/+/, "");
        const tree = trees.find((t) => rel === t || rel.startsWith(t + "/"));
        // No traversal out of the two trees, and no path that is not one of them.
        if (!tree || rel.includes("..")) return next();
        const file = join(from(tree), rel.slice(tree.length + 1));
        if (!existsSync(file) || !statSync(file).isFile()) return next();
        res.setHeader("content-type", "application/octet-stream");
        createReadStream(file).pipe(res);
      });
    },
  };
}

// During dev, the browser only talks to Vite (:12359). Vite proxies every
// human-interface channel route — all under `/api/*` — to the Rust server on
// :12358.
//
// TLS is on (basic-ssl, a self-signed localhost cert) for one reason: HTTP/2.
// Browsers only negotiate h2 over TLS, and h2 multiplexes every request over a
// single connection — which matters here because the face holds ~6 long-lived
// streams at once (the channel long-polls + the mic socket + the inspect SSE)
// and HTTP/1.1's ~6-connections-per-origin cap would otherwise starve any
// further request (worklet fetch, a second tab, the inspect snapshot). Vite 7.2+
// keeps h2 even with `server.proxy` set (it moved off the h2-incapable
// `http-proxy` to `http-proxy3`); the upstream hop to :12358 stays HTTP/1.1,
// which is fine — the connection ceiling only bites browser-side.
//
// The proxy MUST NOT buffer: /api/out/text (and the /api/in/* observe streams)
// are long-poll/streaming endpoints where the body trickles in and body-close
// ends the utterance. http-proxy streams by default (selfHandleResponse stays
// false). We disable timeouts so a quiet long-poll is not killed mid-flight.
const proxy: Record<string, ProxyOptions> = Object.fromEntries(
  // `/api/*` — the human-interface channels. `/views/*` — compiled agent view
  // modules and images the Rust server serves from disk. `/auth/*` — the opt-in
  // OIDC sign-in endpoints (login redirect + IdP callback + logout; gates nothing),
  // so the browser completes the sign-in round-trip against the backend in dev exactly
  // as it does same-origin in prod. The browser fetches these by URL, so dev
  // must reach the backend the same way prod (same-origin embed) does, or every
  // `show` 404s and sign-in dead-ends.
  ["/api", "/views", "/auth"].map((path) => [
    path,
    {
      target: "http://127.0.0.1:12358",
      changeOrigin: false,
      // /api/in/audio/stream is a WebSocket (continuous mic → STT). Without
      // ws:true the proxy leaves the Upgrade handshake hanging and mic audio
      // never reaches the backend. Regular HTTP proxying is unaffected by this.
      ws: true,
      // Streaming-friendly: do not buffer, do not give up.
      proxyTimeout: 0,
      timeout: 0,
      configure: (proxy) => {
        // Best-effort: surface upstream errors instead of swallowing them.
        // http-proxy3's ProxyServer is a typed EventEmitter; the installed
        // @types/node doesn't surface its generic `.on`, so reach the listener
        // through a minimal structural shape. On an HTTP error `res` is a
        // ServerResponse; on a WS-upgrade error it's a raw Socket (no writeHead)
        // — narrow structurally before trying to reply.
        const emitter = proxy as unknown as {
          on(event: "error", handler: (err: Error, req: unknown, res: unknown) => void): void;
        };
        emitter.on("error", (err, _req, res) => {
          // eslint-disable-next-line no-console
          console.error("[vite proxy] upstream error:", err.message);
          const http = res as {
            headersSent?: boolean;
            writeHead?: (status: number, headers: Record<string, string>) => void;
            end?: (body?: string) => void;
          };
          if (http && !http.headersSent && http.writeHead && http.end) {
            try {
              http.writeHead(502, { "content-type": "text/plain" });
              http.end("upstream unreachable");
            } catch {
              // ignore
            }
          }
        });
      },
    } satisfies ProxyOptions,
  ]),
);

export default defineConfig({
  plugins: [react(), tailwindcss(), basicSsl(), emitImportMap(), devImportMap(), pdfjsAssets()],
  // @hi/core is the live-session surface. UI imports use the standard shadcn
  // paths mapped above directly to generated component source files.
  resolve: {
    alias: {
      "@hi/core": r("./src/core/index.ts"),
      // What the generated shadcn components import internally at build time.
      "@": r("./src"),
    },
  },
  server: {
    port: 12359,
    strictPort: true,
    proxy,
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: true,
    // The AudioWorklet module (imported via `?url`) must be a real, statically
    // served same-origin file: `AudioWorklet.addModule()` cannot load a `data:`
    // URL. Vite inlines assets under `assetsInlineLimit` (default 4096 B) as
    // base64 data URLs — and the worklet is small enough to be inlined, which
    // silently breaks mic capture. Force it to be emitted as a hashed file.
    assetsInlineLimit: (filePath) => (filePath.endsWith("pcmWorklet.js") ? false : undefined),
    // The default 500 kB fires on every build here, and on nothing that is a
    // problem. Each chunk over it is a file-format renderer the OFV plugins reach
    // through `await import()` — heic2any (1.4 MB), the pptx renderer, xlsx,
    // hls.js, pdf.js and its worker — plus mermaid's diagram bundles. None of them
    // load until someone opens that format; the eagerly-loaded entry graph
    // (`index` + `global`) is ~260 kB, and `src/shared/ofv.ts` records why size is
    // deliberately not the axis these are selected on. The limit is set just above
    // the largest of them so a chunk that *does* grow past the pack still warns.
    chunkSizeWarningLimit: 1500,
    rollupOptions: {
      // Keep each shim entry's full export surface (don't tree-shake an entry's
      // re-exports just because nothing in this build imports it).
      preserveEntrySignatures: "exports-only",
      // `rolldown:vite-resolve` warns the moment any module imports a Node
      // builtin, which is before tree-shaking gets a chance to drop it. All four
      // it prints on this build come from node-only branches inside
      // `@open-file-viewer/core`'s dependency tree: `require('util').inspect` in
      // ag-psd's unknown-brush-classId error path, and safe-buffer/safer-buffer
      // reached through jszip's readable-stream and msgreader's iconv-lite. None
      // of them survive into `dist/` — no sourcemap there mentions safer-buffer
      // or `Unknown brush classId` — so the warning names code the browser never
      // loads.
      //
      // Only warnings whose importer is under `node_modules` are dropped. Our own
      // source reaching for a builtin is a real bug and still prints, and a
      // third-party one that ever did survive tree-shaking would throw Vite's
      // named stub error on first property access, so this is not the only guard.
      onLog(level, log, handler) {
        const externalized =
          log.plugin === "rolldown:vite-resolve" &&
          log.message?.includes("has been externalized for browser compatibility") &&
          log.message.includes("/node_modules/");
        if (level === "warn" && externalized) return;
        handler(level, log);
      },
      input: {
        index: r("index.html"),
        // The headless render page (served by Rust at `/render/view`). A second
        // HTML entry in the SAME build on purpose: Rollup then emits the shared
        // modules once, so this page's React/@hi/core are the very chunks the
        // import map names — the page and the view it mounts share one instance,
        // exactly as the real face does.
        render: r("render.html"),
        "share-react": r("src/shared/react.ts"),
        "share-react-dom": r("src/shared/react-dom.ts"),
        "share-jsx-runtime": r("src/shared/jsx-runtime.ts"),
        "share-motion": r("src/shared/motion.ts"),
        "share-core": r("src/shared/core.ts"),
        "lib-d3-hierarchy": r("src/shared/d3-hierarchy.ts"),
        "lib-d3-flextree": r("src/shared/d3-flextree.ts"),
        "lib-ofv": r("src/shared/ofv.ts"),
        ...Object.fromEntries(
          Object.keys(SHADCN_SPECIFIERS).map((file) => {
            const name = file.split("/").at(-1)!.replace(/\.tsx$/, "");
            return [`ui-${name}`, r(file)];
          }),
        ),
      },
    },
  },
});
