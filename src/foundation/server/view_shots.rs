//! Thumbnails for the views band — a picture of what was on the screen.
//!
//! The band's history row used to carry a coloured initial in the box a thumbnail
//! would occupy, on the argument that a view is a live React app and there is no
//! moment at which its pixels are available: capturing at replace time reaches a
//! module that may already be unmounted, and re-mounting a *named* view offscreen
//! renders today rather than the record it stands in.
//!
//! That argument assumed the browser had to be the person's. It doesn't:
//! [`view_render`](crate::body::capabilities::view_render) already drives a headless
//! Chromium over the same `/render/view` host page, at the same frame the window
//! reported, for `hi_review_view`. Rendering a raise the moment it is raised is the
//! same act at the same instant on the same module — so the picture is of what the
//! person is looking at, not of a reconstruction.
//!
//! Three properties make it affordable:
//!
//! **Content-addressed, so each artifact renders once.** The key is the compiled
//! module's own hash. Re-raising `factory/tasks` reuses the shot; recompiling it
//! produces a new module and therefore a new one. `<data>/views/_shots/<hash>.png`
//! sits beside `_compiled/` and is disposable in exactly the same way.
//!
//! **One at a time.** A `show, say, show, say` walk-through would otherwise put
//! three Chromiums on a machine that is also running the agent. Captures queue on
//! one lock and each is bounded by the renderer's own settle timeout.
//!
//! **Best-effort and silent.** A failed, blank or timed-out render writes nothing
//! and the tile falls back to its mark, which is where it started. Nothing on the
//! screen waits for this — [`ViewBus::apply`](super::view_bus::ViewBus::apply) has
//! already returned by the time the browser opens.
//!
//! The one honest limitation: a view that reads live data renders with the data it
//! has a second later, not a frozen copy. At 118×76 that is a distinction without a
//! difference, and it is the same distinction a browser's tab switcher makes.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::body::capabilities::view_render;

/// The thumbnail's long edge, in pixels. The tile is 118×76 CSS px, so this covers
/// a 4× display and any later decision to show the picture bigger, while keeping a
/// shot around 40–80 KB.
const THUMB_WIDTH: u32 = 480;

/// How many shots to keep. The history is bounded at 24 entries, but the *cache* is
/// keyed by artifact and would otherwise grow with every view ever recompiled. This
/// is roughly ten histories' worth — enough that going back to something from this
/// morning still has its picture.
const KEEP: usize = 200;

/// Renders happen one at a time. See the module docs: the alternative is a raise
/// sequence spawning a browser per beat.
static CAPTURING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Where shots live — a tool dir inside the views tree, like `_compiled/`.
fn shots_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("views").join("_shots")
}

/// The file name a module's shot is stored under.
///
/// A compiled module URL is already `/views/_compiled/<hash>.mjs`, and that hash is
/// over the source — exactly the identity a picture of it should have. Anything else
/// (a module served from somewhere this doesn't recognise) is hashed by URL so the
/// function is still total.
fn shot_name(module_url: &str) -> String {
    let stem = module_url
        .rsplit('/')
        .next()
        .and_then(|f| f.strip_suffix(".mjs"))
        .unwrap_or_default();
    if !stem.is_empty() && stem.chars().all(|c| c.is_ascii_alphanumeric()) {
        return format!("{stem}.png");
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    module_url.hash(&mut hasher);
    format!("u{:016x}.png", hasher.finish())
}

/// The URL a captured shot is served at, or `None` while none has been taken.
///
/// Called once per history entry when the appearance state is built — a couple of
/// dozen `stat`s on a response that is only produced on a version bump.
pub fn url_for(data_dir: &Path, module_url: &str) -> Option<String> {
    let name = shot_name(module_url);
    shots_dir(data_dir).join(&name).exists().then(|| format!("/views/_shots/{name}"))
}

/// Capture `module_url` in the background, then call `done` if a new shot landed.
///
/// Returns immediately. `done` is how the picture reaches the people already
/// watching: the appearance state that carried this raise was built before the shot
/// existed, so something has to bump the version once it does.
pub fn capture(data_dir: PathBuf, module_url: String, done: impl FnOnce() + Send + 'static) {
    tokio::spawn(async move {
        match run(&data_dir, &module_url).await {
            Ok(true) => done(),
            Ok(false) => {}
            // A thumbnail is decoration on a record that is complete without it, so a
            // failure is logged at debug and never surfaces. `hi_review_view` is where
            // a view's render problems are meant to be read.
            Err(error) => {
                tracing::debug!(module_url = %module_url, %error, "capturing a view thumbnail failed")
            }
        }
    });
}

/// Render, downscale, write. `Ok(false)` means there was nothing to do or nothing
/// worth keeping.
async fn run(data_dir: &Path, module_url: &str) -> anyhow::Result<bool> {
    let dir = shots_dir(data_dir);
    let path = dir.join(shot_name(module_url));
    if path.exists() {
        return Ok(false);
    }
    // Published at startup; absent in a unit test and on a process that never stood
    // the view path up. Nothing to report — there is simply no renderer.
    let Some(ctx) = crate::mind::views::render_context() else {
        return Ok(false);
    };

    let _one_at_a_time = CAPTURING.lock().await;
    // Another capture of the same artifact may have finished while we queued.
    if path.exists() {
        return Ok(false);
    }

    let mut req = view_render::RenderRequest::new(&ctx.base_url, module_url);
    // The frame the window reported, at 1× — the shot is about to be scaled down to
    // a tile, so rendering it at retina density would only cost time and memory.
    req.viewport.scale = 1.0;
    // The skin the person is actually in. A light picture of a view they saw dark is
    // a wrong record, and the window reports its theme for exactly this.
    req.theme = view_render::stage_theme();

    let rendered = view_render::render(&req).await?;
    // A view that failed to mount, threw, or painted one flat colour has no picture
    // worth keeping — and writing one would pin that emptiness for the artifact's
    // whole life, since the cache never re-renders a key it already has.
    if !rendered.ok() {
        tracing::debug!(
            module_url = %module_url,
            verdict = ?rendered.verdict(),
            "no thumbnail: the view did not render cleanly",
        );
        return Ok(false);
    }

    let thumb = tokio::task::spawn_blocking(move || downscale(&rendered.png)).await??;
    tokio::fs::create_dir_all(&dir).await?;
    tokio::fs::write(&path, &thumb).await?;
    tracing::debug!(module_url = %module_url, bytes = thumb.len(), "captured a view thumbnail");
    prune(&dir).await;
    Ok(true)
}

/// Scale a full-frame screenshot down to a tile, keeping its aspect. A shot already
/// narrower than the target is re-encoded as it is rather than blown up.
fn downscale(png: &[u8]) -> anyhow::Result<Vec<u8>> {
    let img = image::load_from_memory(png)?;
    let scaled = if img.width() > THUMB_WIDTH {
        let height = (img.height() as u64 * THUMB_WIDTH as u64 / img.width().max(1) as u64).max(1);
        img.resize(THUMB_WIDTH, height as u32, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let mut out = Vec::new();
    scaled.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(out)
}

/// Drop the oldest shots past [`KEEP`]. Best-effort: a directory that cannot be read
/// or a file that cannot be removed leaves the cache larger than intended, which is
/// not a condition worth reporting.
async fn prune(dir: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    let mut shots: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        let at = entry
            .metadata()
            .await
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        shots.push((at, path));
    }
    if shots.len() <= KEEP {
        return;
    }
    shots.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in shots.into_iter().skip(KEEP) {
        let _ = tokio::fs::remove_file(path).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compiled_module_is_keyed_by_its_own_hash() {
        assert_eq!(shot_name("/views/_compiled/ab12cd34.mjs"), "ab12cd34.png");
    }

    #[test]
    fn anything_else_is_keyed_by_a_hash_of_the_url() {
        let name = shot_name("/views/hand-written.js");
        assert!(name.starts_with('u') && name.ends_with(".png"), "{name}");
        assert_eq!(name, shot_name("/views/hand-written.js"), "and it is stable");
        assert_ne!(name, shot_name("/views/other.js"));
    }

    #[test]
    fn a_shot_url_is_only_reported_once_the_file_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let module = "/views/_compiled/feed01.mjs";
        assert_eq!(url_for(dir.path(), module), None);
        std::fs::create_dir_all(shots_dir(dir.path())).unwrap();
        std::fs::write(shots_dir(dir.path()).join("feed01.png"), b"x").unwrap();
        assert_eq!(url_for(dir.path(), module).as_deref(), Some("/views/_shots/feed01.png"));
    }

    #[test]
    fn a_wide_screenshot_comes_back_as_a_tile() {
        let wide = image::DynamicImage::new_rgb8(1280, 800);
        let mut png = Vec::new();
        wide.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();

        let thumb = image::load_from_memory(&downscale(&png).unwrap()).unwrap();
        assert_eq!(thumb.width(), THUMB_WIDTH);
        assert_eq!(thumb.height(), 300, "the frame's aspect is kept");
    }
}
