//! The gate a view passes before it is handed to somebody who is not the owner.
//!
//! This is the check's end-to-end test, and it exists because nothing ran the check
//! against a real browser: its scope list is computed from the ref, the render is a real
//! Chrome on the real host page, and a request **the page** makes on every load —
//! `/favicon.ico`, when a document declares no icon — was read as the view's own appetite
//! and refused. Every view in an install refused to publish, which no test that skips the
//! browser could have seen. See `docs/arch/sharing.md`.
//!
//! It skips, loudly, when the web bundle or a browser is missing — the same discipline as
//! its neighbours in `view_render.rs`.

use std::path::PathBuf;
use std::time::Duration;

use hi_agent::foundation::server;
use hi_agent::foundation::surfaces::{Acceptor, accepted_on};
use hi_agent::mind::memory::Memory;
use tempfile::tempdir;
use tokio::net::TcpListener;

/// A view written as a module rather than compiled: the check is handed a module URL and
/// never compiles anything, so this keeps the test off esbuild. It mounts through the same
/// host page and resolves through the same import map a compiled view does.
const PROBE: &str = r#"
import { jsx } from "react/jsx-runtime";
export default function Probe() {
  return jsx("p", { children: "share probe" });
}
"#;

/// The failure the check exists for: a view that reads a file which is not its own.
const REACHING: &str = r#"
import { jsx } from "react/jsx-runtime";
export default function Reaching() {
  return jsx("img", { src: "/views/another-project/notes.png", alt: "" });
}
"#;

#[tokio::test]
async fn a_share_check_passes_a_view_and_still_refuses_one_that_reads_outside_it() {
    if hi_agent::appearance::embed::get("render.html").is_none() {
        eprintln!("skipping: the web bundle is not built");
        return;
    }
    let Some(browser) = hi_agent::runtime::browser::available() else {
        eprintln!("skipping: no headless browser is installed or cached");
        return;
    };
    eprintln!("checking with {} ({})", browser.bin.display(), browser.origin);

    let dir = tempdir().expect("tempdir");
    let memory = Memory::open(dir.path()).await.expect("memory");
    let observatory = hi_agent::foundation::observatory::Observatory::new(None);
    let (router, _seams) = server::build(
        memory,
        dir.path().to_path_buf(),
        observatory,
        hi_agent::foundation::codex::WireTap::new(),
        hi_agent::foundation::privacy::PrivacyBoundary::open(dir.path()).unwrap(),
        hi_agent::body::reaction::ToolRegistry::new(),
        hi_agent::body::reaction::Floor::new(),
        hi_agent::body::attachments::Attachments::new(),
        None,
    );
    // A test is a local caller, and says so: without an acceptor the gate fails closed
    // and every request here would be a 401.
    let router = accepted_on(router, Acceptor::Loopback);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let compiled = dir.path().join("views").join("_compiled");
    std::fs::create_dir_all(&compiled).expect("views/_compiled");
    std::fs::write(compiled.join("probe.mjs"), PROBE).expect("probe");
    std::fs::write(compiled.join("reaching.mjs"), REACHING).expect("reaching");
    tokio::time::sleep(Duration::from_millis(20)).await;

    // The check reads the render context rather than being handed it, so that what it
    // checks is the thing it will publish — and the context carries a compiler beside the
    // origin. `compile()` is the only method that touches that binary and the check never
    // calls it, because a module URL is what it is given; the path below deliberately does
    // not exist, so the day the check starts compiling this test says so out loud.
    hi_agent::mind::views::set_render_context(
        hi_agent::mind::views::ViewCompiler::new(PathBuf::from("/nonexistent/esbuild"), dir.path()),
        format!("http://{addr}"),
    );

    let checked = server::view_share::check("probe-view", "/views/_compiled/probe.mjs")
        .await
        .expect("the check runs");
    assert!(checked.ok, "the check refused a view it should pass: {:?}", checked.refusals);
    let html = checked.html.as_deref().expect("the check reads the settled DOM back");
    assert!(
        html.contains("rel=\"icon\""),
        "the page a share is published as must declare its own icon, or every browser asks \
         for /favicon.ico — a request this scope refuses and no view made"
    );

    let refused = server::view_share::check("probe-view", "/views/_compiled/reaching.mjs")
        .await
        .expect("the check runs");
    assert!(!refused.ok, "a view reading another view's file must be refused");
    assert!(
        refused.refusals.iter().any(|r| r.contains("/views/another-project/notes.png")
            && r.contains("outside this view's own files")),
        "the refusal must name the path it read: {:?}",
        refused.refusals
    );
}
