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
//! reported, for `hi_review_view`. Rendering a show the moment it is shown is the
//! same act at the same instant on the same module — so the picture is of what the
//! person is looking at, not of a reconstruction.
//!
//! Three properties make it affordable:
//!
//! **Content-addressed, so each artifact renders once.** The key is the compiled
//! module's own hash. Re-showing `factory/tasks` reuses the shot; recompiling it
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
//! **A named surface's picture is keyed by its ref, and a record's by its artifact.**
//! Content-addressing alone froze the wrong half of this: `factory/tasks` renders once
//! and then shows that morning's board forever, while re-opening it deliberately
//! re-resolves to *today's*. So the two kinds of picture are stored apart —
//! `_shots/<artifact>.png` for an inline view, which is only ever the artifact it
//! compiled to and so is written once; `_shots/ref/<ref>.png` for a named view, which
//! is a standing surface and is re-taken when the person opens it and the last one has
//! gone stale — older than [`REFRESH_AFTER`], or older than the view's own source, which
//! the agent rewrites. The URL carries the file's mtime so the year-long cache the
//! `_shots/` route hands out still expires on a re-take.
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

/// How old a named surface's picture may be before opening it takes a new one. A
/// surface is a live board, so its tile is a claim about what is on it now; an hour-old
/// claim is a lie the person can see through, and a per-open re-render is a browser per
/// click. Fifteen minutes is where a picture stops being about the same working stretch.
const REFRESH_AFTER: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Renders happen one at a time. See the module docs: the alternative is a show
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

/// Where a named surface's picture lives — `_shots/ref/<ref>.png`, mirroring the ref's
/// own path. A ref is validated to names and `/` (see [`crate::mind::views::valid_ref`]),
/// so it is already a safe relative path and a safe URL; anything else has no picture.
fn ref_shot_path(data_dir: &Path, view_ref: &str) -> Option<PathBuf> {
    crate::mind::views::valid_ref(view_ref)
        .then(|| shots_dir(data_dir).join("ref").join(format!("{view_ref}.png")))
}

/// The URL a captured shot is served at, or `None` while none has been taken.
///
/// Called once per history entry when the appearance state is built — a couple of
/// dozen `stat`s on a response that is only produced on a version bump.
pub fn url_for(data_dir: &Path, module_url: &str) -> Option<String> {
    let name = shot_name(module_url);
    shots_dir(data_dir).join(&name).exists().then(|| format!("/views/_shots/{name}"))
}

/// The URL of a named surface's current picture, or `None` while none has been taken.
///
/// **Carries the file's mtime.** The `_shots/` route serves a year-long immutable
/// `Cache-Control`, which is right for a content-addressed artifact and wrong for a
/// path that is re-taken in place: without the stamp the browser would go on showing
/// this morning's board out of its own cache no matter how often the server re-renders
/// it. The stamp changes on every re-take, so each picture is still cached forever —
/// as itself.
pub fn url_for_ref(data_dir: &Path, view_ref: &str) -> Option<String> {
    let path = ref_shot_path(data_dir, view_ref)?;
    let stamp = std::fs::metadata(&path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(format!("/views/_shots/ref/{view_ref}.png?v={stamp}"))
}

/// Capture `module_url` in the background, then call `done` if a new shot landed.
///
/// Returns immediately. `done` is how the picture reaches the people already
/// watching: the appearance state that carried this show was built before the shot
/// existed, so something has to bump the version once it does.
pub fn capture(data_dir: PathBuf, module_url: String, done: impl FnOnce() + Send + 'static) {
    let path = shots_dir(&data_dir).join(shot_name(&module_url));
    spawn_capture(path, module_url, None, done);
}

/// Capture a *named* surface — the picture behind `factory/tasks` rather than behind
/// the artifact it happens to have compiled to.
///
/// Unlike a record shot this one is re-taken once it has gone stale — see [`take_ref`]
/// — because the thing it is a picture of has moved on. Same browser, same lock, same
/// silence on failure.
pub fn capture_ref(
    data_dir: PathBuf,
    view_ref: String,
    module_url: String,
    done: impl FnOnce() + Send + 'static,
) {
    tokio::spawn(async move {
        if take_ref(&data_dir, &view_ref, &module_url).await {
            done();
        }
    });
}

/// The same capture, waited on. For a caller working through a list, which needs to
/// know when one is finished before starting the next — see the band's warm-up in
/// [`super::view::list_views`]. `true` if a new picture landed.
pub async fn take_ref(data_dir: &Path, view_ref: &str, module_url: &str) -> bool {
    let Some(path) = ref_shot_path(data_dir, view_ref) else {
        return false;
    };
    // A picture older than the source it is a picture of is of a build that no longer
    // exists — the agent rewrote the view — and no amount of it being *recent* makes it
    // current. This is the one staleness that cannot wait for the clock.
    let source = data_dir.join("views").join(format!("{view_ref}.jsx"));
    let written = std::fs::metadata(&source).ok().and_then(|m| m.modified().ok());
    match run(&path, module_url, Some(REFRESH_AFTER), written).await {
        Ok(landed) => landed,
        // A thumbnail is decoration on a row that works without it — see `spawn_capture`.
        Err(error) => {
            tracing::debug!(view_ref = %view_ref, %error, "capturing a view thumbnail failed");
            false
        }
    }
}

/// Is `path` a picture we are content to keep?
///
/// `stale_after: None` is write-once — any file there will do, which is what a record of
/// a show wants. `Some(ttl)` also requires it to be younger than `ttl`, and
/// `newer_than` requires it to postdate the source it claims to be a picture of.
fn good_enough(
    path: &Path,
    stale_after: Option<std::time::Duration>,
    newer_than: Option<std::time::SystemTime>,
) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(taken) = meta.modified() else {
        // A filesystem that cannot say when the file was written can still say it is
        // there, which is all a write-once key needs.
        return stale_after.is_none() && newer_than.is_none();
    };
    if newer_than.is_some_and(|written| taken < written) {
        return false;
    }
    match stale_after {
        None => true,
        Some(ttl) => taken.elapsed().is_ok_and(|age| age < ttl),
    }
}

fn spawn_capture(
    path: PathBuf,
    module_url: String,
    stale_after: Option<std::time::Duration>,
    done: impl FnOnce() + Send + 'static,
) {
    tokio::spawn(async move {
        match run(&path, &module_url, stale_after, None).await {
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
async fn run(
    path: &Path,
    module_url: &str,
    stale_after: Option<std::time::Duration>,
    newer_than: Option<std::time::SystemTime>,
) -> anyhow::Result<bool> {
    if good_enough(path, stale_after, newer_than) {
        return Ok(false);
    }
    // Published at startup; absent in a unit test and on a process that never stood
    // the view path up. Nothing to report — there is simply no renderer.
    let Some(ctx) = crate::mind::views::render_context() else {
        return Ok(false);
    };

    let _one_at_a_time = CAPTURING.lock().await;
    // Another capture of the same key may have finished while we queued.
    if good_enough(path, stale_after, newer_than) {
        return Ok(false);
    }

    let mut req = view_render::RenderRequest::new(&ctx.base_url, module_url);
    // The frame the window reported, at 1× — the shot is about to be scaled down to
    // a tile, so rendering it at retina density would only cost time and memory.
    req.viewport.scale = 1.0;
    // The skin the person is actually in. A light picture of a view they saw dark is
    // a wrong record, and the window reports its theme for exactly this.
    req.theme = view_render::stage_theme();
    // And the language they picked, for the same reason: the system views carry both
    // copies and choose per render, so without this every tile of them is a picture of
    // a screen in English that the person has never seen.
    req.lang = crate::appearance::language();

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
    let dir = path.parent().unwrap_or(path);
    tokio::fs::create_dir_all(dir).await?;
    tokio::fs::write(path, &thumb).await?;
    tracing::debug!(module_url = %module_url, bytes = thumb.len(), "captured a view thumbnail");
    prune(dir).await;
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

/// Drop the oldest shots past [`KEEP`]. Bites the artifact cache, which grows with
/// every recompile; `ref/` is bounded by the number of named views in the tree and so
/// never reaches it. Best-effort: a directory that cannot be read
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
    fn a_named_surface_is_keyed_by_its_ref_and_stamped_with_its_age() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(url_for_ref(dir.path(), "factory/tasks"), None, "nothing taken yet");

        let path = ref_shot_path(dir.path(), "factory/tasks").unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"x").unwrap();

        let url = url_for_ref(dir.path(), "factory/tasks").unwrap();
        let (file, stamp) = url.split_once("?v=").unwrap();
        assert_eq!(file, "/views/_shots/ref/factory/tasks.png");
        assert!(stamp.parse::<u64>().unwrap() > 0, "carries the mtime: {url}");
    }

    /// A ref reaches this as text from the wire. Everything that is not a ref — a
    /// traversal above all — has no picture rather than a path.
    #[test]
    fn only_a_valid_ref_names_a_picture() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["../../etc/passwd", "factory/../../x", "has.dot", ""] {
            assert!(ref_shot_path(dir.path(), bad).is_none(), "{bad}");
        }
    }

    /// A record shot is written once; a surface shot is written again once it is old.
    #[test]
    fn a_surface_picture_goes_stale_and_a_record_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        assert!(!good_enough(&path, None, None), "nothing there yet");
        std::fs::write(&path, b"x").unwrap();

        assert!(good_enough(&path, None, None), "a record is any file at the key");
        assert!(good_enough(&path, Some(REFRESH_AFTER), None), "a fresh surface stands");
        assert!(
            !good_enough(&path, Some(std::time::Duration::ZERO), None),
            "an aged-out surface is re-taken",
        );
    }

    /// The agent rewrites views, and a picture of the build before the rewrite is wrong
    /// however recently it was taken.
    #[test]
    fn a_picture_older_than_the_view_it_shows_is_not_good_enough() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        std::fs::write(&path, b"x").unwrap();
        let taken = std::fs::metadata(&path).unwrap().modified().unwrap();

        let before = taken - std::time::Duration::from_secs(60);
        let after = taken + std::time::Duration::from_secs(60);
        assert!(good_enough(&path, Some(REFRESH_AFTER), Some(before)), "source is older");
        assert!(!good_enough(&path, Some(REFRESH_AFTER), Some(after)), "source is newer");
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
