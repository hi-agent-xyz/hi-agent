//! The render path with a **real browser** — the one part of the view-review stack
//! that unit tests structurally cannot reach.
//!
//! `chrome_headless`'s own tests parse CDP payloads and `view_render`'s check pixel
//! math; both are pure. Everything between them — resolving a browser, launching it,
//! finding the ephemeral DevTools port, attaching over the websocket, driving the
//! render, decoding the screenshot — had **never been executed** by anything when
//! this file was written. That is precisely the stretch where "it compiles" says
//! least: a missing shared library, a sandbox refusal or a changed flag all fail
//! here and nowhere else.
//!
//! `#[ignore]` because it needs a browser and ~2s, and CI boxes may have neither.
//! Run it deliberately:
//!
//! ```sh
//! cargo test --test render_smoke -- --ignored --nocapture
//! ```
//!
//! It does not need a running hi-agent: it drives an inline `data:` page, so what
//! it proves is the *browser* half. The page contract (`window.__hiRender`) is
//! honoured by the fixture exactly as `/render/view` honours it, so a settle that
//! resolves here is the same settle the real page gets.

use std::time::Duration;

use hi_agent::foundation::vendors::chrome_headless::{self, PageRequest};
use hi_agent::runtime::browser;

/// A page that declares itself settled the way the real render page does, and
/// paints one solid rectangle so the capture is provably not blank.
const READY_PAGE: &str = "data:text/html,\
<body style='margin:0;background:%23101418'>\
<div style='width:200px;height:120px;background:%233ecf8e;margin:40px'></div>\
<script>window.__hiRender={ready:true,failed:false,errors:[]}</script>";

/// A page that never defines `__hiRender` — the shape of a render page whose own
/// script failed to load. The driver must time out and *say so* rather than hand
/// back a clean-looking screenshot of nothing.
const SILENT_PAGE: &str = "data:text/html,<body style='background:white'></body>";

async fn resolve() -> browser::ResolvedBrowser {
    browser::ensure().await.expect(
        "no headless browser could be resolved — install Chrome/Chromium or let the \
         managed download run",
    )
}

#[tokio::test]
#[ignore = "launches a real browser"]
async fn a_real_browser_launches_attaches_and_screenshots() {
    let browser = resolve().await;
    eprintln!("browser: {} ({})", browser.bin.display(), browser.origin);

    let cap = chrome_headless::capture(
        &browser,
        &PageRequest {
            url: READY_PAGE.to_string(),
            width: 800,
            height: 600,
            scale: 2.0,
            settle_timeout: Duration::from_secs(10),
        },
    )
    .await
    .expect("the browser half of the render path");

    assert!(!cap.timed_out, "the page declared itself ready; the driver missed it");
    assert!(!cap.page_failed, "clean page reported failed: {:?}", cap.problems);
    assert!(cap.problems.is_empty(), "clean page reported problems: {:?}", cap.problems);

    // Real pixels, at the density we asked for.
    assert!(cap.png.len() > 1_000, "screenshot suspiciously small: {} bytes", cap.png.len());
    let img = image::load_from_memory(&cap.png).expect("the capture decodes as an image");
    assert_eq!(
        (img.width(), img.height()),
        (1600, 1200),
        "deviceScaleFactor did not reach the capture"
    );
    assert!(
        !hi_agent::body::capabilities::view_render::is_blank_png(&cap.png),
        "a page with a drawn rectangle read as blank — blank detection or the capture is wrong"
    );
    eprintln!("captured {} bytes, {}x{}", cap.png.len(), img.width(), img.height());
}

/// The failure that matters most: a screenshot that *looks* fine is not a pass.
/// A page that never reports must come back timed-out **and** blank, so the
/// reviewer above it can never mistake white for empty-by-design.
#[tokio::test]
#[ignore = "launches a real browser"]
async fn a_page_that_never_reports_times_out_and_reads_blank() {
    let browser = resolve().await;

    let cap = chrome_headless::capture(
        &browser,
        &PageRequest {
            url: SILENT_PAGE.to_string(),
            width: 400,
            height: 300,
            scale: 1.0,
            settle_timeout: Duration::from_secs(2),
        },
    )
    .await
    .expect("a silent page still captures");

    assert!(cap.timed_out, "a page that never reports must time out");
    assert!(cap.page_failed, "a timeout is a failure, not a pass");
    assert!(
        hi_agent::body::capabilities::view_render::is_blank_png(&cap.png),
        "an empty white page must read as blank"
    );
}
