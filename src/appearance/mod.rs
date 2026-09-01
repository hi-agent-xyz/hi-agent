//! Appearance — the agent's web surface.
//!
//! This module owns everything a browser sees: the embedded Vite SPA, the
//! Open Graph metadata, and the routes that serve them. The SPA itself lives
//! in `src/appearance/web/` and is embedded at compile time via `rust-embed`.
//!
//! ## Routes mounted by `router()`
//!
//! - `GET /`            — index.html with OG tags injected
//! - `GET /assets/*`    — hashed JS/CSS bundles from Vite
//! - `GET /favicon.ico` — favicon if present in dist/
//! - icon set + `GET /site.webmanifest` — brand icons & PWA manifest
//! - `GET /vite.svg`    — Vite's default logo if shipped
//!
//! Step 1's server module is expected to mount this router at `/` after
//! attaching all channel routes; axum matches more specific routes first, so
//! `/api/in/*` and `/api/out/*` and friends keep working.
//!
//! ## Coordination with Step 1
//!
//! Step 1 owns `src/server/mod.rs` and the `AppState` type. To stay
//! independent of that timing, `router()` is generic over `S`. When Step 1
//! lands, `crate::foundation::server::AppState` can be substituted in directly without
//! touching this file.
//!
//! ## Future seam: agent-authored, runtime-swappable skins (NOT built yet)
//!
//! The embedded SPA below is intentionally "skin 0": the default appearance.
//! The design leaves room for the agent to author and evolve its own
//! appearance at runtime without a rebuild. None of this is implemented today;
//! it is recorded here so the seam is cheap to pick up later:
//!
//! - **Storage.** Runtime skins live under `<data_dir>/appearance/skins/<id>/`
//!   (self-contained HTML/JS/CSS), with an `active.json` pointer. The embedded
//!   default is the un-deletable fallback and is served whenever no runtime
//!   skin is active.
//! - **Serving.** New routes (e.g. `GET /appearance/active`,
//!   `GET /appearance/skin/{id}/*path`) serve the active skin; a long-poll
//!   mirroring `GET /view` lets the shell hot-swap when the active skin
//!   changes.
//! - **Bridge.** A skin renders in a sandboxed iframe and talks to the host
//!   over `postMessage` only — the host streams it presence state / sentences /
//!   views and accepts a narrow `sendText`. Mic, credentials and the upstream
//!   proxy stay strictly host-side; a skin never gets same-origin.
//! - **Authoring.** The agent would register and activate skins the same way it
//!   already puts rich content on screen: dedicated tool calls alongside
//!   `show`, driven in the background by the heartbeat.
//! - **Safety.** Activation is gated (preview + approval) and auto-reverts to
//!   the embedded default if a skin fails to load; `GET /?skin=default` is the
//!   escape hatch. The session core (channels, presence state machine, mic) is
//!   skin-independent — today it lives in `web/src/hooks/useAgentSession.ts`,
//!   and look-and-feel is centralized in the `:root` tokens of
//!   `web/src/ui/global.css`, so a token-only re-theme needs no canvas changes.

pub mod embed;
pub mod og;

use axum::{
    Router,
    body::Body,
    extract::Path,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use tower_http::compression::{CompressionLayer, CompressionLevel};

/// Build the appearance router.
///
/// `S` is the server's shared state type. We don't depend on its concrete
/// shape here — none of the appearance handlers need state today. When the
/// OG layer becomes state-aware, this signature stays the same and the
/// handler bodies start using `State<S>`.
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/", get(index))
        // The inspect console is a client-routed SPA section. Serve the same SPA
        // shell for `/inspect` and any nested path so a deep link or refresh on
        // e.g. `/inspect/sessions` boots the app, which then renders the right view.
        .route("/inspect", get(index))
        .route("/inspect/{*path}", get(index))
        // The Settings page is a client-routed SPA section too — same shell so a
        // direct load or refresh of `/settings` boots the app rather than 404ing.
        .route("/settings", get(index))
        .route("/settings/{*path}", get(index))
        // The headless render page — one agent view, mounted standalone, for the
        // renderer to screenshot. It lives here rather than in the server router
        // because everything it needs is embedded (the built page + the build's
        // import map), and because the import-map injection it depends on is
        // *this* module's job — see `render_view`.
        .route("/render/view", get(render_view))
        .route("/favicon.ico", get(favicon))
        // Brand icon set + PWA manifest. The router only serves paths it names,
        // so each root-level asset Vite copies from `public/` needs an explicit
        // route or it 404s. All are embedded from dist/ and served verbatim.
        .route("/icon.svg", get(|| async { serve_embedded("icon.svg") }))
        .route("/favicon-16x16.png", get(|| async { serve_embedded("favicon-16x16.png") }))
        .route("/favicon-32x32.png", get(|| async { serve_embedded("favicon-32x32.png") }))
        .route("/apple-touch-icon.png", get(|| async { serve_embedded("apple-touch-icon.png") }))
        .route(
            "/apple-touch-icon-precomposed.png",
            get(|| async { serve_embedded("apple-touch-icon.png") }),
        )
        .route(
            "/android-chrome-192x192.png",
            get(|| async { serve_embedded("android-chrome-192x192.png") }),
        )
        .route(
            "/android-chrome-512x512.png",
            get(|| async { serve_embedded("android-chrome-512x512.png") }),
        )
        .route("/site.webmanifest", get(|| async { serve_embedded("site.webmanifest") }))
        .route("/vite.svg", get(vite_svg))
        .route("/assets/{*path}", get(asset))
        // Compress the SPA on the way out. Everything above is a whole buffered
        // body — HTML, JS, CSS, icons — so there is nothing here to stall, which
        // is exactly why the layer lives on *this* router and not on the server's:
        // `/api/out/text` and the observe streams next door are long-poll bodies
        // that end an utterance by closing, and a compressor between them and the
        // socket would hold those bytes back.
        //
        // The bytes are already built and content-hashed, so this is the whole
        // saving available: the SPA's first load is ~420 kB raw against ~125 kB
        // encoded. Vite's many small entry chunks are a deliberate part of the
        // import-map contract (see `web/vite.config.ts`) and cost ~5% against one
        // combined blob — a rounding error next to sending it uncompressed.
        //
        // `DefaultPredicate` skips what must not be touched: bodies under 32 B,
        // anything already `content-encoding`d, `text/event-stream`, and
        // already-compressed image types — so the icon set below passes through.
        // Quality 6, not the default. tower-http defaults to brotli quality 4, and
        // browsers prefer `br` over `gzip` — so the default actually ships *more*
        // bytes than gzip would (56,844 vs 56,549 on the 180 kB `global` chunk).
        // Measured on that chunk: q4 56,844 (6 ms) · q6 53,307 (2 ms) · q11 49,036
        // (149 ms). q6 beats gzip and stays cheap; q11 buys 8% more for 70x the CPU,
        // on a body that is recompressed per request because nothing caches it here.
        // `Precise(6)` leaves gzip at its own level 6 — already its default.
        .layer(CompressionLayer::new().quality(CompressionLevel::Precise(6)))
}

/// The language the person picked, captured once at startup.
///
/// It reaches the page as the `lang` attribute on `<html>`, which is where a bundled
/// view reads it from — the review surfaces ship English and Chinese copy and select
/// between them per render. It is published here rather than read per request because
/// [`router`] is generic over its state (`Router<S>`), so `index` has no `AppState` to
/// pull a `data_dir` out of; and because the setting already "applies on restart" (see
/// [`crate::foundation::config::KEY_LANGUAGE`]), a value captured at boot is not stale.
///
/// Stored verbatim — `system`, `en`, `zh-Hans`. Resolving `system` against the actual
/// machine is the browser's job (`navigator.language`), not ours.
static LANGUAGE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Publish the language for [`index`] to stamp onto the page. Called once from startup;
/// later calls are ignored.
pub fn set_language(value: impl Into<String>) {
    let _ = LANGUAGE.set(value.into());
}

/// The language the person picked, or `None` when they left it on `system` — which is
/// the browser's question to answer (`navigator.language`), not ours. Read by the
/// thumbnail capture, which drives a headless page that has no browser preference to
/// fall back on and would otherwise picture every bundled view in English.
pub fn language() -> Option<String> {
    LANGUAGE
        .get()
        .map(String::as_str)
        .filter(|v| !v.is_empty() && *v != "system")
        .map(str::to_owned)
}

/// Rewrite `<html lang="…">` to the person's setting. A no-op when nothing was
/// published or the document has no `lang` attribute to replace — in which case the
/// built `index.html` keeps its `lang="en"`, which is the right default anyway.
fn inject_lang(html: String) -> String {
    let Some(lang) = LANGUAGE.get() else { return html };
    if lang.is_empty() || !html.contains("<html lang=\"") {
        return html;
    }
    // Only the first occurrence, and only the opening tag Vite emits.
    html.replacen("<html lang=\"en\"", &format!("<html lang=\"{lang}\""), 1)
}

/// The path this core is served under, from `X-Forwarded-Prefix`.
///
/// Empty for every ordinary shape — a core at its own root, or reached through an
/// app's proxy. `"/ana"` when the community routes it by subpath, which is the
/// one case where the absolute paths this page emits do not start where the page
/// does.
///
/// Read per request rather than stored, because the same core answers on
/// loopback *and* through the community, and the answer differs.
///
/// One parse for the whole core — [`crate::foundation::surfaces::base_path`] —
/// because the page, the cookie and the QR must agree on where here is.
fn forwarded_prefix(headers: &axum::http::HeaderMap) -> String {
    crate::foundation::surfaces::base_path(headers)
}

/// Rewrite the root-absolute URLs a built page emits so they start at `prefix`.
///
/// Targeted at `src="/…"`, `href="/…"` and the import map's `": "/…"` rather
/// than every `"/` in the document: a blunt replace would also hit strings
/// inside inline scripts, and the failure would be silent and weird.
///
/// `//` is left alone — that is a protocol-relative URL to somewhere else, not a
/// path on this origin.
fn reroot(html: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        return html.to_string();
    }
    let mut out = html.to_string();
    for (pattern, kept) in [("src=\"/", "src=\""), ("href=\"/", "href=\""), ("\": \"/", "\": \"")] {
        out = out.replace(pattern, &format!("{kept}{prefix}/"));
    }
    // A protocol-relative URL (`//somewhere/x`) is another origin, not a path
    // here, and must not be given ours.
    out.replace(&format!("{prefix}//"), "//")
}

/// Tell the page where it is, so its own fetches can start there too. Read by
/// `lib/base.ts`; absent (and therefore empty) in every unprefixed shape.
fn inject_base(html: String, prefix: &str) -> String {
    if prefix.is_empty() {
        return html;
    }
    let needle = "<script type=\"importmap\">";
    let tag = format!("<script>window.__HI_BASE__ = \"{prefix}\";</script>\n    ");
    match html.find(needle) {
        Some(idx) => {
            let mut out = String::with_capacity(html.len() + tag.len());
            out.push_str(&html[..idx]);
            out.push_str(&tag);
            out.push_str(&html[idx..]);
            out
        }
        None => html,
    }
}

/// `GET /` — serve index.html with OG tags injected before `</head>`.
///
/// If the embedded `index.html` is missing (debug builds before the SPA is
/// built), respond with a small dev placeholder pointing at Vite on :12359.
async fn index(headers: axum::http::HeaderMap) -> Response {
    let prefix = forwarded_prefix(&headers);
    let tags = og::OgTags::default_for_agent();

    match embed::get("index.html") {
        Some(file) => {
            // Inject OG tags just before </head>. Fall back to appending to
            // the document if no </head> is found (shouldn't happen with
            // Vite output, but never panic on user-driven input).
            let html = String::from_utf8_lossy(file.data.as_ref()).into_owned();
            let injection = og::render(&tags);
            let injected = match html.find("</head>") {
                Some(idx) => {
                    let mut out = String::with_capacity(html.len() + injection.len());
                    out.push_str(&html[..idx]);
                    out.push_str(&injection);
                    out.push_str(&html[idx..]);
                    out
                }
                None => format!("{html}{injection}"),
            };

            // An import map must precede the first module script — the browser
            // rejects one added after a module has begun loading — so this
            // injects ahead of Vite's `<script type="module">`, not before
            // </head> like the OG tags. It lets a runtime-imported agent view
            // module resolve `react` / `@hi/core` / `motion/react` to the same
            // shared chunks the host loaded (see web/vite.config.ts).
            let injected = inject_importmap(injected);
            // Then the subpath, if the community put us under one: every
            // absolute path this page emits has to start there, and the page
            // has to be told so its own fetches do too. A no-op otherwise.
            let injected = inject_base(injected, &prefix);
            let injected = reroot(&injected, &prefix);
            // Last, so the `lang` swap sees the final opening tag.
            let injected = inject_lang(injected);

            html_response(injected, StatusCode::OK)
        }
        None => {
            // Debug builds without a built SPA: friendly placeholder.
            html_response(dev_placeholder(), StatusCode::OK)
        }
    }
}

/// `GET /assets/*path` — serve a hashed asset from the embedded dist.
async fn asset(Path(path): Path<String>) -> Response {
    serve_embedded(&format!("assets/{path}"))
}

/// `GET /render/view?module=…&region=…&size=…&theme=…&chrome=…` — a host page carrying
/// exactly one agent view, for the headless renderer to load and screenshot.
///
/// **Why a route at all.** A compiled view keeps its bare imports (`react`,
/// `@/components/ui/card`, `@hi/core`, `motion/react`) unresolved by design, so
/// the host and the view share one React instance. There is therefore no such
/// thing as rendering a view by pointing a browser at its `.mjs`: it has to be
/// loaded by a page that carries the import map. Backend routes are the one
/// thing the agent cannot hot-load, so this ships as a bundled seed (see
/// `docs/arch/foundation.md#hot-loading`).
///
/// **One map, not two.** The map comes from the same embedded `importmap.json`
/// the SPA's `index()` injects, through the same [`inject_importmap`] — a second,
/// hand-maintained copy would drift the moment a shim is added, and views would
/// fail to resolve only in review.
///
/// The query string is read client-side (`src/render/main.tsx`), so the HTML is
/// identical for every render and needs no state here.
async fn render_view() -> Response {
    match embed::get("render.html") {
        Some(file) => {
            let html = String::from_utf8_lossy(file.data.as_ref()).into_owned();
            html_response(inject_importmap(html), StatusCode::OK)
        }
        // Debug builds before `npm run build`: there is no built page and no
        // import map, so a render could only produce a misleading blank. Say so.
        None => html_response(
            "<!doctype html><title>hi-agent view render</title>\
             <p>The web bundle has not been built, so there is no render page and \
             no import map. Run `npm run build` in src/appearance/web/ (or \
             `make build`) and rebuild.</p>"
                .to_string(),
            StatusCode::SERVICE_UNAVAILABLE,
        ),
    }
}

/// Inject the build's `importmap.json` (if embedded) as a `<script type=
/// "importmap">` ahead of the first module script. A no-op when no map is
/// embedded (debug builds before the SPA is built).
fn inject_importmap(html: String) -> String {
    match embed::get("importmap.json") {
        Some(file) => {
            let map = String::from_utf8_lossy(file.data.as_ref());
            splice_importmap(&html, map.trim())
        }
        None => html,
    }
}

/// Splice an import map script — and the preloads for what a view will import
/// through it — before the first `<script type="module"` in `html`. Pure (no
/// embed access) so the ordering invariant is unit-testable. Returns `html`
/// unchanged if it has no module script.
fn splice_importmap(html: &str, map_json: &str) -> String {
    let needle = "<script type=\"module\"";
    let Some(idx) = html.find(needle) else {
        return html.to_string();
    };
    let tag = format!(
        "<script type=\"importmap\">\n{map_json}\n    </script>\n    {}",
        view_preload_links(map_json)
    );
    let mut out = String::with_capacity(html.len() + tag.len());
    out.push_str(&html[..idx]);
    out.push_str(&tag);
    out.push_str(&html[idx..]);
    out
}

/// The specifiers **every** compiled view resolves through the map, whatever
/// else it does: `react/jsx-runtime` because the compiler runs esbuild with
/// `--jsx=automatic`, `react` because a view with state imports hooks, and
/// `@hi/core` because a review surface re-reads itself with `useLive`.
///
/// Deliberately not the whole map. `motion/react` is 183 kB and two bundled
/// views use it; the shadcn entries are a component library nobody loads all of.
/// Preloading either would trade a round trip for bytes on a connection where
/// bytes are the scarcer thing.
const VIEW_PRELOAD_SPECIFIERS: [&str; 3] = ["react/jsx-runtime", "react", "@hi/core"];

/// `<link rel="modulepreload">` for [`VIEW_PRELOAD_SPECIFIERS`].
///
/// **What this buys is a round trip, not bytes.** The three shims are ~1.7 kB
/// between them and their own dependencies are already in the page's preload
/// list. The cost they carry is *when they are discovered*: nothing in the
/// document mentions them, so the browser learns they exist only after it has
/// fetched and parsed a compiled view — which does not happen until the agent
/// shows one. Relayed, every request on that chain is a full trip out through
/// the community and back down the core's tunnel, and the face's cold path
/// already runs six of them end to end. This deletes one of the last two,
/// after which a shown view's module is the only thing still outstanding.
///
/// Emitted as root-absolute paths so `reroot` moves them under the community's
/// subpath along with everything else the page emits.
fn view_preload_links(map_json: &str) -> String {
    let Ok(map) = serde_json::from_str::<serde_json::Value>(map_json) else {
        return String::new();
    };
    let Some(imports) = map.get("imports").and_then(|imports| imports.as_object()) else {
        return String::new();
    };
    let mut out = String::new();
    for specifier in VIEW_PRELOAD_SPECIFIERS {
        let Some(url) = imports.get(specifier).and_then(|url| url.as_str()) else { continue };
        out.push_str(&format!("<link rel=\"modulepreload\" crossorigin href=\"{url}\">\n    "));
    }
    out
}

async fn favicon() -> Response {
    // Some Vite setups inline a data: URI for favicon and skip the file.
    // In that case fall through to 404; the browser silently moves on.
    serve_embedded("favicon.ico")
}

async fn vite_svg() -> Response {
    serve_embedded("vite.svg")
}

fn serve_embedded(path: &str) -> Response {
    match embed::get(path) {
        Some(file) => {
            let mime = embed::content_type_for(path);
            let body = Body::from(file.data.into_owned());
            let mut resp = Response::new(body);
            if let Ok(value) = HeaderValue::from_str(mime) {
                resp.headers_mut().insert(header::CONTENT_TYPE, value);
            }
            // Vite emits content-hashed filenames under /assets, so they are
            // safe to cache forever. Everything else gets a conservative
            // short cache — index.html is served by `index()` directly so
            // it isn't routed through here.
            //
            // **`private`, not `public`, and the hash is not what decides that.**
            // A content hash says the bytes will never change; it says nothing
            // about who may have them. Every path through here is behind the gate
            // (`surfaces::open_path` opens only `/healthz`, `POST /api/session`
            // and the upload pair), so `public` invites a shared cache to store a
            // response that was served *because* a credential was checked and then
            // hand it to someone with none. That is not hypothetical: relayed, the
            // core sits behind a CDN, and an authorized fetch of an asset made an
            // unauthenticated fetch of the same asset answer `200` from the edge
            // instead of the core's `401`. `private` keeps the browser cache —
            // which is all this was ever for — and forbids the shared one.
            let cache = if path.starts_with("assets/") {
                "private, max-age=31536000, immutable"
            } else {
                "private, max-age=300"
            };
            if let Ok(value) = HeaderValue::from_str(cache) {
                resp.headers_mut().insert(header::CACHE_CONTROL, value);
            }
            resp
        }
        None => not_found(),
    }
}

fn html_response(body: String, status: StatusCode) -> Response {
    let mut resp = Response::new(Body::from(body));
    *resp.status_mut() = status;
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    // index.html should never be cached — OG tags depend on runtime state.
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, must-revalidate"),
    );
    resp
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}

fn dev_placeholder() -> String {
    // Minimal, themed to match the SPA. Mentions :12359 so a new contributor
    // knows where to look.
    r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta name="color-scheme" content="light dark" />
    <title>hi-agent (dev)</title>
    <style>
      :root { color-scheme: light dark; }
      body {
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
          Helvetica, Arial, sans-serif;
        margin: 0; padding: 48px 24px; line-height: 1.5;
        display: flex; justify-content: center;
      }
      main { max-width: 560px; }
      code { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
      .hint { opacity: 0.7; font-size: 14px; margin-top: 16px; }
    </style>
  </head>
  <body>
    <main>
      <h1>hi-agent</h1>
      <p>The embedded web bundle has not been built yet.</p>
      <p>
        For development, run the Vite dev server on
        <code>http://127.0.0.1:12359</code> — it proxies the channel routes
        back to this Rust server on <code>:12358</code>.
      </p>
      <pre><code>cd src/appearance/web &amp;&amp; pnpm install &amp;&amp; pnpm dev</code></pre>
      <p class="hint">
        For a release build, <code>pnpm build</code> writes to
        <code>src/appearance/web/dist/</code> and the next
        <code>cargo build --release</code> embeds it.
      </p>
    </main>
  </body>
</html>
"#
    .to_string()
}

#[cfg(test)]
mod prefix_tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn a_prefix_is_a_path_or_it_is_nothing() {
        let mut h = HeaderMap::new();
        assert_eq!(forwarded_prefix(&h), "");
        h.insert("x-forwarded-prefix", HeaderValue::from_static("/ana"));
        assert_eq!(forwarded_prefix(&h), "/ana");
        h.insert("x-forwarded-prefix", HeaderValue::from_static("/ana/"));
        assert_eq!(forwarded_prefix(&h), "/ana");
        // Not a path, or a path that climbs, is not a prefix — it is about to be
        // spliced into a page.
        for bad in ["ana", "/../admin", "/a\"onload=x"] {
            h.insert("x-forwarded-prefix", HeaderValue::from_str(bad).unwrap());
            assert_eq!(forwarded_prefix(&h), "", "{bad}");
        }
    }

    #[test]
    fn the_pages_own_paths_start_where_the_page_does() {
        let html = r#"<link href="/favicon.ico"><script type="module" src="/assets/i.js"></script>"#;
        let out = reroot(html, "/ana");
        assert!(out.contains(r#"href="/ana/favicon.ico""#), "{out}");
        assert!(out.contains(r#"src="/ana/assets/i.js""#), "{out}");
        // And unprefixed is byte-for-byte what it was.
        assert_eq!(reroot(html, ""), html);
    }

    #[test]
    fn the_import_map_is_rerooted_too_and_other_origins_are_not() {
        let html = r#"<script type="importmap">{"imports":{"react": "/assets/r.js"}}</script>
<script src="//cdn.example/x.js"></script>"#;
        let out = reroot(html, "/ana");
        assert!(out.contains(r#""react": "/ana/assets/r.js""#), "{out}");
        assert!(out.contains(r#"src="//cdn.example/x.js""#), "a protocol-relative URL was claimed: {out}");
    }

    #[test]
    fn the_page_is_told_where_it_is_only_when_it_is_somewhere() {
        let html = r#"<head><script type="importmap">{}</script></head>"#;
        let out = inject_base(html.to_string(), "/ana");
        assert!(out.contains(r#"window.__HI_BASE__ = "/ana";"#), "{out}");
        assert!(
            out.find("__HI_BASE__").unwrap() < out.find("importmap").unwrap(),
            "the page must know before it loads anything"
        );
        assert_eq!(inject_base(html.to_string(), ""), html);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_placeholder_is_self_contained_html() {
        let html = dev_placeholder();
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("hi-agent"));
        assert!(html.contains(":12359"));
    }

    #[test]
    fn content_type_for_assets() {
        assert!(embed::content_type_for("x.js").starts_with("application/javascript"));
        assert!(embed::content_type_for("x.css").starts_with("text/css"));
        assert!(embed::content_type_for("x.unknown").starts_with("application/octet-stream"));
    }

    #[test]
    fn importmap_spliced_before_first_module_script() {
        let html = r#"<head><script type="module" crossorigin src="/x.js"></script></head>"#;
        let out = splice_importmap(html, r#"{"imports":{"react":"/assets/r.js"}}"#);
        let map_pos = out.find("type=\"importmap\"").expect("import map present");
        let mod_pos = out.find("type=\"module\"").expect("module script present");
        assert!(map_pos < mod_pos, "import map must precede the module script");
        assert!(out.contains(r#""react":"/assets/r.js""#));
    }

    #[test]
    fn splice_importmap_noops_without_module_script() {
        let html = "<head></head>";
        assert_eq!(splice_importmap(html, "{}"), "<head></head>");
    }

    /// **What a view will import is discovered from the document, not from the
    /// view.** Nothing in the page names the shims a compiled view resolves
    /// through the map, so without these links the browser cannot know they
    /// exist until it has parsed a module that only arrives when the agent shows
    /// something — an extra round trip on the relayed path, taken at the one
    /// moment somebody is waiting to see a view.
    ///
    /// The absences are the other half of the rule: this is the three shims a
    /// view always needs, not the map. `motion/react` is 183 kB.
    #[test]
    fn the_shims_every_view_needs_are_preloaded_and_the_rest_of_the_map_is_not() {
        let map = r#"{"imports":{"@hi/core":"/assets/share-core.js",
            "react":"/assets/share-react.js","react/jsx-runtime":"/assets/share-jsx.js",
            "motion/react":"/assets/share-motion.js","@/components/ui/card":"/assets/ui-card.js"}}"#;
        let html = r#"<head><script type="module" crossorigin src="/x.js"></script></head>"#;
        let out = splice_importmap(html, map);

        for preloaded in ["/assets/share-core.js", "/assets/share-react.js", "/assets/share-jsx.js"] {
            assert!(
                out.contains(&format!(r#"<link rel="modulepreload" crossorigin href="{preloaded}">"#)),
                "{preloaded} is imported by every compiled view and must be preloaded: {out}"
            );
        }
        assert!(!out.contains(r#"modulepreload" crossorigin href="/assets/share-motion.js""#));
        assert!(!out.contains(r#"modulepreload" crossorigin href="/assets/ui-card.js""#));

        // Still ahead of the module script, and after the map it belongs to.
        let map_pos = out.find("type=\"importmap\"").expect("import map present");
        let link_pos = out.find("modulepreload").expect("preload present");
        let mod_pos = out.find("type=\"module\"").expect("module script present");
        assert!(map_pos < link_pos && link_pos < mod_pos, "{out}");
    }

    /// The preloads are root-absolute like everything else the page emits, so
    /// the community's subpath moves them with the rest. A link left at `/assets`
    /// under `/ana` asks the *community* for the shim and preloads a 404 — which
    /// is silent, and costs exactly the round trip the link was added to save.
    #[test]
    fn a_preload_follows_the_page_under_a_subpath() {
        let map = r#"{"imports":{"react":"/assets/share-react.js"}}"#;
        let html = r#"<head><script type="module" src="/x.js"></script></head>"#;
        let out = reroot(&splice_importmap(html, map), "/ana");
        assert!(
            out.contains(r#"<link rel="modulepreload" crossorigin href="/ana/assets/share-react.js">"#),
            "{out}"
        );
    }

    /// A debug build's map, or a malformed one, must cost the page nothing more
    /// than the preloads it cannot name.
    #[test]
    fn an_unreadable_map_yields_no_preloads() {
        assert_eq!(view_preload_links("not json"), "");
        assert_eq!(view_preload_links("{}"), "");
        assert_eq!(view_preload_links(r#"{"imports":{"lodash":"/assets/l.js"}}"#), "");
    }
}
