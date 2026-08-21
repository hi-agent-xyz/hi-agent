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
//! A compiled view deliberately keeps its bare imports (`react`,
//! `@/components/ui/card`, `@hi/core`, `motion/react`) unresolved, bound at
//! runtime by the host page's import map so host and view share one React
//! instance. There is therefore no way to render a view by handing a browser a
//! file: it must be loaded by a real host page carrying that map. That page is
//! `GET /render/view`, and its map comes from the same embedded artifact `GET /`
//! injects — one map, not a second copy free to drift.
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

/// The fallback review viewport, used only when no face has reported its frame:
/// a comfortable desktop stage at retina density. Nothing composes *for* this —
/// it is what a review falls back to when the app is headless (Docker, a test,
/// a tool call made before any window opened), so it is a plausible desktop
/// shape rather than a chosen canvas.
pub const DEFAULT_WIDTH: u32 = 1280;
pub const DEFAULT_HEIGHT: u32 = 800;
pub const DEFAULT_SCALE: f64 = 2.0;

/// Bounds on a reported frame. A browser can report a window a few pixels tall
/// mid-drag, and a bogus report is worse than no report: it silently moves the
/// frame every view is built and judged against. Anything outside this is
/// ignored, keeping the last good frame.
const MIN_STAGE: u32 = 320;
const MAX_STAGE: u32 = 16_384;

/// The frame the desktop window is showing **right now**, as the face last
/// reported it.
///
/// This is the whole answer to "what size does a view get built for". A view is
/// composed edge to edge for one landscape frame, so a builder composing against
/// a constant that matches no real window — and a reviewer signing off on that
/// same constant — is the defect: the person's cards collide at a size neither
/// of them ever saw. The face knows the number without any OS call, so it
/// reports it and this holds the latest.
///
/// `RwLock` rather than a `OnceLock`, because unlike the compiler and base URL
/// published beside it in `mind::views`, this one legitimately changes: the
/// person drags the window, and the next review should use the new size.
static STAGE: std::sync::RwLock<Option<Viewport>> = std::sync::RwLock::new(None);

/// The skin the desktop window is currently in, as the face last reported it.
///
/// Sits beside [`STAGE`] and is reported by the same lane for the same reason: the
/// page already knows the answer (`data-theme`, else `prefers-color-scheme`) with no
/// OS call, and a picture of a view rendered in the other skin is a wrong record of
/// what the person was looking at.
///
/// Only the *thumbnail* path reads it. `hi_review_view` deliberately renders both
/// skins, because a review exists to catch the colour that resolves in one and not
/// the other — pinning it to the window's would blind the review to half of that.
static STAGE_THEME: std::sync::RwLock<Option<&'static str>> = std::sync::RwLock::new(None);

/// Record the skin the desktop window is in. Anything that is not a skin we can
/// render is ignored, keeping the last good answer.
pub fn set_stage_theme(theme: &str) -> bool {
    let known = match theme {
        "light" => "light",
        "dark" => "dark",
        _ => return false,
    };
    if let Ok(mut slot) = STAGE_THEME.write() {
        *slot = Some(known);
    }
    true
}

/// The skin to render into, or `None` when no window has reported one — which the
/// render page reads as its own default (light).
pub fn stage_theme() -> Option<String> {
    STAGE_THEME.read().ok().and_then(|s| *s).map(str::to_owned)
}

/// Record the frame the desktop window is showing. Out-of-range reports are
/// dropped rather than stored (see [`MIN_STAGE`]).
///
/// Deliberately fed by the **desktop window only**. The same page also runs in
/// the menu-bar popover (380×540, portrait) and in a plain browser tab, and a
/// review rendered at the popover's frame would be a review of something nobody
/// is composing for. The client gates on the same `chrome=titlebar` flag the
/// window already sets to claim its titlebar strip.
pub fn set_stage_frame(width: u32, height: u32, scale: f64) -> bool {
    if !(MIN_STAGE..=MAX_STAGE).contains(&width) || !(MIN_STAGE..=MAX_STAGE).contains(&height) {
        return false;
    }
    // A device pixel ratio outside this is not a display we can render for; clamp
    // rather than reject, since the frame itself is still good.
    let scale = if scale.is_finite() { scale.clamp(1.0, 4.0) } else { DEFAULT_SCALE };
    if let Ok(mut slot) = STAGE.write() {
        *slot = Some(Viewport { width, height, scale });
    }
    true
}

/// The frame a review should render into: what the window last reported, or the
/// fallback when nothing has.
pub fn stage_frame() -> Viewport {
    STAGE.read().ok().and_then(|s| *s).unwrap_or_default()
}

/// How long the page gets to mount, settle fonts, and resolve its images.
const DEFAULT_SETTLE: Duration = Duration::from_secs(15);

/// What to render.
///
/// A view declares nothing about itself — it is full-bleed, one at a time, and the host
/// owns the conversation over it — so there is nothing here but where to get the module
/// and what frame to put it in.
#[derive(Debug, Clone)]
pub struct RenderRequest {
    /// Base URL of the running hi-agent server, e.g. `http://127.0.0.1:12358`.
    /// The same value the workers get as `HI_AGENT_BASE_URL`.
    pub base_url: String,
    /// The compiled module's served URL — `/views/_compiled/<hash>.mjs`, as
    /// returned by [`crate::mind::views::ViewCompiler::compile`].
    pub module_url: String,
    /// Force the light or dark skin; `None` uses the page default (light).
    pub theme: Option<String>,
    /// Force the language a bundled view selects its copy with (`en`, `zh-Hans`, …),
    /// stamped onto `<html lang>` by the render page. `None` leaves the page default,
    /// which is English. Only the system views ship more than one language; an
    /// agent-authored view is written in whatever language it was asked for, so this is
    /// deliberately opt-in rather than swept like the theme.
    pub lang: Option<String>,
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
    /// A request for `module_url` at the frame the desktop window is currently
    /// showing — [`stage_frame`], which falls back to [`Viewport::default`] when
    /// no window has reported one.
    ///
    /// This is what makes the review honest. Both worker prompts tell their
    /// session that the screenshot *is* what the person sees; that is only true
    /// if the frame it renders into is the frame the window is actually showing.
    pub fn new(base_url: impl Into<String>, module_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            module_url: module_url.into(),
            theme: None,
            lang: None,
            viewport: stage_frame(),
        }
    }

    /// The `/render/view` URL this request loads.
    ///
    /// It always carries `chrome=titlebar`: the review stands in for the desktop
    /// window, whose content spans a native titlebar, so the page has to reserve
    /// that strip here too. A review rendered without it is a review that can't
    /// catch a header sitting under the traffic lights — which is precisely the
    /// kind of fault only a screenshot finds.
    pub fn page_url(&self) -> String {
        let mut url = format!(
            "{}/render/view?module={}&chrome=titlebar",
            self.base_url.trim_end_matches('/'),
            urlencode(&self.module_url)
        );
        for (key, value) in [("theme", self.theme.as_deref()), ("lang", self.lang.as_deref())] {
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

    /// The reported frame is what a review renders into — the point of the whole
    /// lane. Asserted through `RenderRequest::new` rather than against the store
    /// directly, because a store nothing reads would pass its own test and still
    /// leave every review at the fallback.
    ///
    /// One test, not four: `STAGE` is a process-global and `cargo test` runs the
    /// cases in this binary concurrently, so separate tests would race each other
    /// through it.
    #[test]
    fn the_reported_frame_is_what_a_review_renders_into() {
        let frame = || {
            let v = RenderRequest::new("http://h:1", "/m.mjs").viewport;
            (v.width, v.height)
        };

        assert!(set_stage_frame(1920, 1080, 2.0));
        assert_eq!(frame(), (1920, 1080));

        // A resize is just the next report; the newest frame wins.
        assert!(set_stage_frame(1000, 720, 2.0));
        assert_eq!(frame(), (1000, 720));

        // A window mid-drag can report a sliver, and a stored sliver would
        // silently become the frame every view is built and judged against.
        assert!(!set_stage_frame(4, 3, 2.0));
        assert!(!set_stage_frame(99_999, 720, 2.0));
        assert_eq!(frame(), (1000, 720), "a rejected report keeps the last good frame");

        // A nonsense DPR still carries a usable frame, so clamp the scale rather
        // than throw the report away.
        assert!(set_stage_frame(1000, 720, f64::NAN));
        assert_eq!(stage_frame().scale, DEFAULT_SCALE);
        assert!(set_stage_frame(1000, 720, 99.0));
        assert_eq!(stage_frame().scale, 4.0);
    }

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
    fn page_url_carries_the_module_and_the_frame() {
        let req = RenderRequest::new("http://127.0.0.1:12358/", "/views/_compiled/ab12.mjs");
        let url = req.page_url();
        assert!(url.starts_with("http://127.0.0.1:12358/render/view?"), "{url}");
        assert!(url.contains("module=%2Fviews%2F_compiled%2Fab12.mjs"), "{url}");
        assert!(url.contains("&chrome=titlebar"), "the review reserves the titlebar: {url}");
        assert!(!url.contains("theme="), "an unset theme is not sent: {url}");
        // A view declares nothing: no placement, and no claim on the conversation.
        assert!(!url.contains("region="), "{url}");
        assert!(!url.contains("size="), "{url}");
        assert!(!url.contains("owns_conversation"), "{url}");
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
            vec!["TypeError: Failed to resolve module specifier \"@/components/ui/card\""],
            false,
        );
        match v.verdict() {
            Verdict::Failed(why) => {
                assert!(
                    why.contains("@/components/ui/card"),
                    "the error text must survive: {why}"
                );
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
