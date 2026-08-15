//! End-to-end test of the headless view renderer.
//!
//! This is the test the capability exists for. It compiles two views — one good,
//! one whose bare import cannot possibly resolve — serves them through the real
//! router, renders both in a real headless browser, and asserts that the second
//! is reported as **FAILED with the error text**, not as a blank success.
//!
//! Three things must already be on the machine for it to run, and each is
//! *skipped* (loudly) rather than faked:
//!
//! - **the built SPA**, because the whole point is that a view is bound by the
//!   host page's import map, and the map only exists in `dist/` (`make build`);
//! - **esbuild**, to compile JSX to ESM;
//! - **a browser**, already installed or already cached — the test never pulls
//!   the ~100 MB managed build behind your back.
//!
//! Skipping is honest here: unlike screencast or input synthesis there is no GUI
//! wall, so on a properly provisioned box this really does run end to end.

use std::path::PathBuf;
use std::time::Duration;

use hi_agent::body::capabilities::view_render::{self, RenderRequest, Verdict};
use hi_agent::mind::memory::Memory;
use hi_agent::mind::views::ViewCompiler;
use hi_agent::foundation::surfaces::{Acceptor, accepted_on};
use hi_agent::foundation::server::{self, ServerSeams};
use tempfile::tempdir;
use tokio::net::TcpListener;

/// A view that renders visible content through the shared `@hi/ui` primitives —
/// i.e. one that only works if the import map bound its bare imports.
const GOOD_VIEW: &str = r#"
import { Card, Stack, Text } from "@hi/ui";
export default function Spending() {
  return (
    <Card>
      <Stack>
        <Text>Groceries crept up; everything else held steady.</Text>
        <Text>Nothing else moved.</Text>
      </Stack>
    </Card>
  );
}
"#;

/// The failure that matters: a bare specifier no import map resolves. esbuild
/// compiles it happily (it does not bundle), the module 404s/fails in the
/// browser, and the page paints nothing — a screenshot alone would call this a
/// pass.
const BROKEN_VIEW: &str = r#"
import { Chart } from "@totally/not-a-real-package";
export default function Broken() {
  return <Chart />;
}
"#;

struct Harness {
    base_url: String,
    compiler: ViewCompiler,
    _dir: tempfile::TempDir,
    _seams: ServerSeams,
}

/// Stand up the real router over a temp data dir, plus a compiler writing into
/// the same tree the `/views/` route serves from.
async fn harness(esbuild: PathBuf) -> Harness {
    let dir = tempdir().expect("tempdir");
    let memory = Memory::open(dir.path()).await.expect("memory");
    let observatory = hi_agent::foundation::observatory::Observatory::new(None);
    let (router, seams) = server::build(
        memory,
        dir.path().to_path_buf(),
        observatory,
        hi_agent::foundation::codex::WireTap::new(),
        hi_agent::body::reaction::ToolRegistry::new(),
        hi_agent::body::reaction::Floor::new(),
        hi_agent::body::attachments::Attachments::new(),
        None,
    );
    // A test is a local caller, and says so: without an acceptor the gate
    // fails closed and every request here would be a 401.
    let router = accepted_on(router, Acceptor::Loopback);

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let compiler = ViewCompiler::new(esbuild, dir.path());
    Harness { base_url: format!("http://{addr}"), compiler, _dir: dir, _seams: seams }
}

/// Locate an esbuild native binary already provisioned on this host — the
/// standalone view-tool install or a managed runtime's `node_modules`. Same probe
/// the view-compiler unit tests use.
fn esbuild_probe() -> Option<PathBuf> {
    let (os, arch) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => ("darwin", "arm64"),
        ("macos", "x86_64") => ("darwin", "x64"),
        ("linux", "aarch64") => ("linux", "arm64"),
        ("linux", "x86_64") => ("linux", "x64"),
        ("windows", "x86_64") => ("win32", "x64"),
        _ => return None,
    };
    let platform = format!("{os}-{arch}");
    let bin_rel = if cfg!(target_os = "windows") {
        PathBuf::from("@esbuild").join(&platform).join("esbuild.exe")
    } else {
        PathBuf::from("@esbuild").join(&platform).join("bin/esbuild")
    };
    let cache = directories::ProjectDirs::from("dev", "human-interface", "hi-agent")?
        .cache_dir()
        .to_path_buf();

    // Where the runtime puts it today: one `esbuild` beside the codex binary.
    // The two `node_modules` layouts below are older shapes — the view-tool
    // install and the node adapter's — and the adapter is gone. This probe kept
    // looking only there after the codex swap moved it, so every test in this
    // file skipped and reported `ok`, which is the exact failure the file was
    // written to catch, wearing a different hat.
    if let Ok(entries) = std::fs::read_dir(cache.join("runtime")) {
        for entry in entries.flatten() {
            let bin = entry.path().join("esbuild/bin/esbuild");
            if bin.exists() {
                return Some(bin);
            }
        }
    }

    let view_tool = cache.join("view-tool");
    if let Ok(entries) = std::fs::read_dir(&view_tool) {
        for entry in entries.flatten() {
            let bin = entry.path().join("node_modules").join(&bin_rel);
            if bin.exists() {
                return Some(bin);
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(cache.join("runtime")) {
        for entry in entries.flatten() {
            let bin = entry.path().join("adapter/node_modules").join(&bin_rel);
            if bin.exists() {
                return Some(bin);
            }
        }
    }
    None
}

/// Everything the e2e render needs, or `None` with the reason printed.
async fn ready() -> Option<Harness> {
    if hi_agent::appearance::embed::get("render.html").is_none() {
        eprintln!(
            "skipping: the web bundle is not built, so there is no render page and \
             no import map (run `npm run build` in src/appearance/web/, then rebuild)"
        );
        return None;
    }
    let Some(esbuild) = esbuild_probe() else {
        eprintln!("skipping: esbuild is not provisioned on this host");
        return None;
    };
    let Some(browser) = hi_agent::runtime::browser::available() else {
        eprintln!(
            "skipping: no headless browser is installed or cached (the test will not \
             download one; run the app once, or install Chrome/Chromium)"
        );
        return None;
    };
    eprintln!("rendering with {} ({})", browser.bin.display(), browser.origin);
    Some(harness(esbuild).await)
}

#[tokio::test]
async fn a_good_view_renders_to_a_non_blank_png() {
    let Some(h) = ready().await else { return };

    let module_url = h.compiler.compile(GOOD_VIEW).await.expect("compiles");
    let out = view_render::render(&RenderRequest::new(h.base_url.as_str(), module_url.as_str()))
        .await
        .expect("render succeeds");

    assert_eq!(
        out.verdict(),
        Verdict::Rendered,
        "a good view should render cleanly; problems: {:?}",
        out.problems
    );
    assert!(!out.png.is_empty(), "a PNG was produced");
    assert!(
        !view_render::is_blank_png(&out.png),
        "the screenshot must not be a blank page"
    );
    // The PNG really is a PNG, at (at least) the requested viewport — the device
    // scale factor makes it larger, which is the point of capturing at retina
    // density for something a person or a model will read back.
    let img = image::load_from_memory(&out.png).expect("decodes as an image");
    assert!(
        img.width() >= view_render::DEFAULT_WIDTH && img.height() >= view_render::DEFAULT_HEIGHT,
        "the viewport should be honored, got {}x{}",
        img.width(),
        img.height()
    );
}

#[tokio::test]
async fn a_view_with_a_broken_import_is_reported_as_failed() {
    let Some(h) = ready().await else { return };

    let module_url = h.compiler.compile(BROKEN_VIEW).await.expect("compiles");
    let out = view_render::render(&RenderRequest::new(h.base_url.as_str(), module_url.as_str()))
        .await
        .expect("render completes even though the view is broken");

    match out.verdict() {
        Verdict::Failed(why) => {
            assert!(
                why.contains("not-a-real-package") || why.contains("resolve module specifier"),
                "the failure must name the unresolved import, got: {why}"
            );
        }
        Verdict::Rendered => panic!(
            "a view whose import cannot resolve must NOT be reported as rendered \
             (this is the blank-white-screenshot failure the capability exists to catch)"
        ),
    }
    assert!(!out.problems.is_empty(), "the page's errors must be reported back");
}

#[tokio::test]
async fn the_render_page_is_served_with_the_host_import_map() {
    // Cheap and independent of esbuild/browser: the page the renderer loads must
    // carry the same import map the real face gets, with the map ahead of the
    // module script (the browser rejects one added later).
    if hi_agent::appearance::embed::get("render.html").is_none() {
        eprintln!("skipping: the web bundle is not built");
        return;
    }
    let dir = tempdir().expect("tempdir");
    let memory = Memory::open(dir.path()).await.expect("memory");
    let observatory = hi_agent::foundation::observatory::Observatory::new(None);
    let (router, _seams) = server::build(
        memory,
        dir.path().to_path_buf(),
        observatory,
        hi_agent::foundation::codex::WireTap::new(),
        hi_agent::body::reaction::ToolRegistry::new(),
        hi_agent::body::reaction::Floor::new(),
        hi_agent::body::attachments::Attachments::new(),
        None,
    );
    // A test is a local caller, and says so: without an acceptor the gate
    // fails closed and every request here would be a 401.
    let router = accepted_on(router, Acceptor::Loopback);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let html = reqwest::get(format!("http://{addr}/render/view?module=/x.mjs"))
        .await
        .expect("request")
        .text()
        .await
        .expect("body");

    let map = html.find("type=\"importmap\"").expect("the import map is injected");
    let script = html.find("type=\"module\"").expect("the page has a module script");
    assert!(map < script, "the import map must precede the module script");
    for spec in ["\"react\"", "\"@hi/ui\"", "\"@hi/core\"", "\"motion/react\""] {
        assert!(html.contains(spec), "the map must bind {spec}");
    }
}

/// The `reach` surface renders, and that is not a formality: it is the one
/// bundled view whose module scope touches `@hi/core`, and a bad import there
/// fails at *runtime* — the view compiles, the page loads, and the panel is
/// blank with the reason only in a console nobody reads.
///
/// It is rendered against a core with no name, no devices and no app, which is
/// exactly a first run: every section has to hold its shape while every fetch
/// behind it answers empty or 404.
#[tokio::test]
#[ignore = "launches a real browser; run with --ignored"]
async fn the_reach_surface_renders_on_a_core_that_has_nothing_yet() {
    let Some(h) = ready().await else { return };

    let source = include_str!("../src/mind/views/factory/reach.jsx");
    let module_url = h.compiler.compile(source).await.expect("reach compiles");
    let out = view_render::render(&RenderRequest::new(h.base_url.as_str(), module_url.as_str()))
        .await
        .expect("render succeeds");

    assert_eq!(
        out.verdict(),
        Verdict::Rendered,
        "reach should render cleanly; problems: {:?}",
        out.problems
    );
    assert!(
        !view_render::is_blank_png(&out.png),
        "reach rendered blank — its module scope probably threw"
    );
}
