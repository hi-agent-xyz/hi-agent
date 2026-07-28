//! View render — look at a view before shipping it.
//!
//! `docs/arch/foundation.md` is blunt about this: *"the command exited zero" is
//! not "the thing worked"; an artifact is not shipped until it has been looked
//! at.* Views are a product surface, so **building one and reviewing one both
//! have to be things hi-agent can do** — not things the view-builder worker is
//! told to improvise a browser install and a preview harness for, per machine,
//! per install. This capability is the reviewing half, as a first-class,
//! bundled thing.
//!
//! ## The interface
//!
//! Given a compiled view module and a viewport, produce a PNG **and the page's
//! account of what went wrong**. Both halves are load-bearing. The single most
//! common real defect is a view that "renders" as a blank white page because a
//! bare import failed to resolve — pixels alone report that as a success, which
//! is exactly the failure mode the architecture calls out. So [`RenderedView`]
//! carries `problems` and `blank` beside the bytes, and [`RenderedView::verdict`]
//! folds them into pass/fail.
//!
//! ## Why a module URL and not a source string
//!
//! A compiled view deliberately keeps its bare imports (`react`, `@hi/ui`,
//! `@hi/core`, `motion/react`) unresolved, bound at runtime by the host page's
//! import map so host and view share one React instance. There is therefore no
//! way to render a view by handing a browser a file: it must be loaded by a real
//! host page carrying that map. That page is `GET /render/view`, and its map
//! comes from the same embedded artifact `GET /` injects — one map, not a second
//! copy free to drift.
//!
//! So the input is a served module URL (what
//! [`crate::mind::views::ViewCompiler::compile`] returns) plus the base URL of
//! the running server. Turning a stored `<ref>.jsx` into that URL is the
//! compiler's existing job; this capability does not duplicate it.
//!
//! ## Capability vs vendor
//!
//! Per [`super`]: this module is the interface and the adaptation — URL
//! assembly, viewport policy, blank detection, the verdict. One vendor
//! implements it: [`chrome_headless`], a Chromium driven over the DevTools
//! protocol. Like `screencast` and `input`, the vendor is effectively the
//! platform rather than an API key, so there is no `init` and it does not appear
//! in the composition root — the browser is resolved lazily on first use by
//! [`crate::runtime::browser`].
//!
//! Unlike screencast, hotkeys or input synthesis, a *headless* browser needs no
//! window server and no TCC grant, so this capability can be exercised
//! end-to-end over SSH.

use std::time::Duration;

use anyhow::Context;

use crate::foundation::vendors::chrome_headless;

/// The default review viewport: a comfortable desktop stage at retina density,
/// which is the shape a view is actually composed for and the density anything
/// reading the PNG back (a person, or a model) needs to judge type.
pub const DEFAULT_WIDTH: u32 = 1280;
pub const DEFAULT_HEIGHT: u32 = 800;
pub const DEFAULT_SCALE: f64 = 2.0;

/// How long the page gets to mount, settle fonts, and resolve its images.
const DEFAULT_SETTLE: Duration = Duration::from_secs(15);

/// What to render.
#[derive(Debug, Clone)]
pub struct RenderRequest {
    /// Base URL of the running hi-agent server, e.g. `http://127.0.0.1:12358`.
    /// The same value the workers get as `HI_AGENT_BASE_URL`.
    pub base_url: String,
    /// The compiled module's served URL — `/views/_compiled/<hash>.mjs`, as
    /// returned by [`crate::mind::views::ViewCompiler::compile`].
    pub module_url: String,
    /// The placement to render under, matching the view's `.geom.json` sidecar.
    /// `None` renders under the host's floor (a centered `auto` card), which is
    /// what a view with no sidecar gets on the real stage.
    pub region: Option<String>,
    pub size: Option<String>,
    /// Force the light or dark skin; `None` uses the page default (light).
    pub theme: Option<String>,
    pub viewport: Viewport,
}

/// The stage the view is rendered onto.
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    /// Device pixel ratio.
    pub scale: f64,
}

impl Default for Viewport {
    fn default() -> Self {
        Self { width: DEFAULT_WIDTH, height: DEFAULT_HEIGHT, scale: DEFAULT_SCALE }
    }
}

impl RenderRequest {
    /// A request for `module_url` at the default review viewport.
    pub fn new(base_url: impl Into<String>, module_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            module_url: module_url.into(),
            region: None,
            size: None,
            theme: None,
            viewport: Viewport::default(),
        }
    }

    /// Render under a declared placement (the view's `.geom.json`).
    pub fn with_geometry(
        mut self,
        region: Option<String>,
        size: Option<String>,
    ) -> Self {
        self.region = region;
        self.size = size;
        self
    }

    /// The `/render/view` URL this request loads.
    pub fn page_url(&self) -> String {
        let mut url = format!(
            "{}/render/view?module={}",
            self.base_url.trim_end_matches('/'),
            urlencode(&self.module_url)
        );
        for (key, value) in [
            ("region", self.region.as_deref()),
            ("size", self.size.as_deref()),
            ("theme", self.theme.as_deref()),
        ] {
            if let Some(v) = value {
                url.push_str(&format!("&{key}={}", urlencode(v)));
            }
        }
        url
    }
}

/// The outcome of one render.
#[derive(Debug, Clone)]
pub struct RenderedView {
    /// The screenshot, PNG bytes.
    pub png: Vec<u8>,
    /// Everything the page reported going wrong: uncaught exceptions, console
    /// errors, failed loads, unresolved imports. Empty is the good case.
    pub problems: Vec<String>,
    /// The screenshot is one flat colour — nothing was drawn. On its own this is
    /// not proof of a defect (a view *could* legitimately paint one colour), but
    /// combined with a silent page it is the classic "resolved to nothing"
    /// failure, so it is reported rather than hidden.
    pub blank: bool,
    /// The page itself said it failed (module didn't load, no default export, the
    /// component threw) or never reported at all.
    pub failed: bool,
    /// The page never settled inside the timeout.
    pub timed_out: bool,
}

/// Pass or fail, and why. The reason is written for whoever has to fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Rendered,
    Failed(String),
}

impl RenderedView {
    /// Fold the evidence into one answer. A view is only "rendered" if the page
    /// mounted it, reported no problems, settled in time, **and** actually drew
    /// something — a blank PNG with a clean console is still not a view.
    pub fn verdict(&self) -> Verdict {
        if self.timed_out {
            return Verdict::Failed(format!(
                "the view never finished rendering{}",
                detail(&self.problems)
            ));
        }
        if self.failed {
            return Verdict::Failed(format!("the view failed to render{}", detail(&self.problems)));
        }
        if !self.problems.is_empty() {
            return Verdict::Failed(format!("the view reported errors{}", detail(&self.problems)));
        }
        if self.blank {
            return Verdict::Failed(
                "the view rendered nothing — the screenshot is one flat colour".to_string(),
            );
        }
        Verdict::Rendered
    }

    /// True when [`Verdict::Rendered`].
    pub fn ok(&self) -> bool {
        self.verdict() == Verdict::Rendered
    }
}

fn detail(problems: &[String]) -> String {
    if problems.is_empty() {
        return String::new();
    }
    format!(":\n  {}", problems.join("\n  "))
}

/// Render one view and report both the pixels and the problems.
///
/// Resolves a headless browser on first use (system Chrome/Chromium/Edge if the
/// machine has one, else the pinned managed build — see
/// [`crate::runtime::browser`]), loads the standalone host page, waits for it to
/// settle, and captures.
pub async fn render(req: &RenderRequest) -> anyhow::Result<RenderedView> {
    let browser = crate::runtime::browser::ensure()
        .await
        .context("resolving a headless browser for the view renderer")?;
    tracing::debug!(
        browser = %browser.bin.display(),
        origin = browser.origin,
        url = %req.page_url(),
        "rendering a view",
    );

    let capture = chrome_headless::capture(
        &browser,
        &chrome_headless::PageRequest {
            url: req.page_url(),
            width: req.viewport.width,
            height: req.viewport.height,
            scale: req.viewport.scale,
            settle_timeout: DEFAULT_SETTLE,
        },
    )
    .await
    .context("rendering the view in the headless browser")?;

    let blank = is_blank_png(&capture.png);
    Ok(RenderedView {
        png: capture.png,
        problems: capture.problems,
        blank,
        failed: capture.page_failed,
        timed_out: capture.timed_out,
    })
}

/// True when every pixel of `png` is (near enough) the same colour — a screenshot
/// of nothing. Deliberately strict: the point is to catch the white page a failed
/// import leaves behind, not to judge minimalism, so a single drawn card is
/// enough to make this false.
///
/// Undecodable bytes count as blank: we could not see a view, so we must not
/// claim we did.
pub fn is_blank_png(png: &[u8]) -> bool {
    // A small per-channel tolerance absorbs the antialiasing/colour-management
    // noise a real screenshot carries even on a flat background.
    const TOLERANCE: i16 = 4;

    let Ok(img) = image::load_from_memory(png) else {
        return true;
    };
    let rgb = img.to_rgb8();
    let mut pixels = rgb.pixels();
    let Some(first) = pixels.next() else {
        return true;
    };
    let [r0, g0, b0] = first.0;
    for p in pixels {
        let [r, g, b] = p.0;
        if (r as i16 - r0 as i16).abs() > TOLERANCE
            || (g as i16 - g0 as i16).abs() > TOLERANCE
            || (b as i16 - b0 as i16).abs() > TOLERANCE
        {
            return false;
        }
    }
    true
}

/// Percent-encode a query-parameter value. Tiny by design — the values here are
/// served paths and enum-ish words, not arbitrary user text — so it escapes
/// everything outside the unreserved set rather than trying to be a URL library.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(problems: Vec<&str>, blank: bool) -> RenderedView {
        RenderedView {
            png: Vec::new(),
            problems: problems.into_iter().map(str::to_owned).collect(),
            blank,
            failed: false,
            timed_out: false,
        }
    }

    #[test]
    fn page_url_carries_the_module_and_geometry() {
        let req = RenderRequest::new("http://127.0.0.1:12358/", "/views/_compiled/ab12.mjs")
            .with_geometry(Some("fill".into()), Some("wide".into()));
        let url = req.page_url();
        assert!(url.starts_with("http://127.0.0.1:12358/render/view?"), "{url}");
        assert!(url.contains("module=%2Fviews%2F_compiled%2Fab12.mjs"), "{url}");
        assert!(url.contains("&region=fill"), "{url}");
        assert!(url.contains("&size=wide"), "{url}");
        assert!(!url.contains("theme="), "an unset theme is not sent: {url}");
    }

    #[test]
    fn page_url_does_not_double_the_base_slash() {
        let a = RenderRequest::new("http://h:1", "/m.mjs").page_url();
        let b = RenderRequest::new("http://h:1/", "/m.mjs").page_url();
        assert_eq!(a, b);
        assert!(!a.contains("//render"), "{a}");
    }

    #[test]
    fn urlencode_escapes_path_separators() {
        assert_eq!(urlencode("/a b/c.mjs"), "%2Fa%20b%2Fc.mjs");
        assert_eq!(urlencode("center"), "center");
    }

    #[test]
    fn a_clean_non_blank_render_passes() {
        assert_eq!(rendered(vec![], false).verdict(), Verdict::Rendered);
        assert!(rendered(vec![], false).ok());
    }

    /// The whole point of the capability: a screenshot alone would call this a
    /// pass.
    #[test]
    fn a_blank_render_is_a_failure_even_with_a_silent_console() {
        let v = rendered(vec![], true);
        assert!(!v.ok());
        match v.verdict() {
            Verdict::Failed(why) => assert!(why.contains("rendered nothing"), "{why}"),
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn reported_problems_fail_the_render_and_are_quoted_verbatim() {
        let v = rendered(
            vec!["TypeError: Failed to resolve module specifier \"@hi/ui\""],
            false,
        );
        match v.verdict() {
            Verdict::Failed(why) => {
                assert!(why.contains("@hi/ui"), "the error text must survive: {why}");
            }
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn a_timeout_outranks_everything_else() {
        let mut v = rendered(vec!["slow"], false);
        v.timed_out = true;
        match v.verdict() {
            Verdict::Failed(why) => assert!(why.contains("never finished"), "{why}"),
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn blank_detection_reads_actual_pixels() {
        // One flat colour → blank.
        let flat = image::RgbImage::from_pixel(24, 24, image::Rgb([255, 255, 255]));
        assert!(is_blank_png(&encode(&flat)));

        // One drawn card is enough to be non-blank.
        let mut drawn = image::RgbImage::from_pixel(24, 24, image::Rgb([255, 255, 255]));
        for x in 6..18 {
            for y in 6..18 {
                drawn.put_pixel(x, y, image::Rgb([58, 53, 44]));
            }
        }
        assert!(!is_blank_png(&encode(&drawn)));
    }

    #[test]
    fn near_flat_noise_still_counts_as_blank() {
        // Screenshots of a flat background are never bit-identical.
        let mut noisy = image::RgbImage::from_pixel(16, 16, image::Rgb([255, 255, 255]));
        noisy.put_pixel(3, 3, image::Rgb([253, 254, 255]));
        assert!(is_blank_png(&encode(&noisy)));
    }

    #[test]
    fn undecodable_bytes_are_not_claimed_as_a_render() {
        assert!(is_blank_png(b"not a png"));
        assert!(is_blank_png(&[]));
    }

    fn encode(img: &image::RgbImage) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img.clone())
            .write_to(&mut out, image::ImageFormat::Png)
            .expect("png encodes");
        out.into_inner()
    }
}
