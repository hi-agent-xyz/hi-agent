//! Watch the views tree, so a view rewritten while someone is looking at it follows.
//!
//! A compiled module is content-addressed over the source *as it was when the view
//! went up*, and the screen holds that URL: [`ViewBus::apply`](super::view_bus::ViewBus::apply)
//! pins what the agent showed, [`go_to`](super::view_bus::ViewBus::go_to) pins what the
//! person opened. Both are right about the moment they run and neither hears the next
//! write of the file. So a builder that rewrites `knq/editing-live-commentary` while the
//! person is parked on it changes nothing on screen: the same board stays up, compiled
//! from a source that no longer exists, until someone leaves the view and comes back —
//! which re-resolves the ref and is the workaround this module deletes.
//!
//! [`refresh_sources`](super::view_bus::ViewBus::refresh_sources) already states the
//! rule for the one moment it could observe: the screen shows the view as it *is*, not
//! as it compiled. This is that rule at every other moment. The source of the change is
//! the filesystem because that is where the change happens — a view is saved by writing
//! the file (`src/identity/workers/view-builder.md`: "no special tool, just write the
//! file"), so there is no tool call to hang this off and nothing else that knows.
//!
//! It is an event, not a tick: the watcher costs nothing until a file is written, and a
//! write to a view nobody is looking at costs a string compare. Nothing here wakes a
//! rung — the screen changes, and the next turn reads what is on it the way it always
//! does.
//!
//! **What it deliberately does not do is re-take the tile.** A show and an open both
//! capture, because both happen at conversational cadence; a save happens at whatever
//! cadence a builder types, and a Chromium per keystroke-batch is not worth a thumbnail.
//! The picture is re-taken the next time anyone opens the view, which is the design's
//! own answer for when a surface's picture is worth having.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{EventKind, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use super::view_bus::ViewBus;
use crate::mind::views::ViewCompiler;

/// How long to let a burst of writes settle before recompiling. An editor saves a file
/// two or three times (truncate, write, rename) and a builder writing a 45 KB view lands
/// several events; recompiling on each would spawn an esbuild per event and hand the
/// screen a half-written module in between.
const SETTLE: Duration = Duration::from_millis(400);

/// Start watching `<data_dir>/views` for source rewrites. Best-effort: a platform that
/// will not give us a watcher logs and leaves the screen exactly as it was before this
/// existed — pinned until the view is re-opened.
pub fn spawn(bus: ViewBus, data_dir: PathBuf, compiler: ViewCompiler) {
    let root = data_dir.join("views");
    if let Err(error) = std::fs::create_dir_all(&root) {
        tracing::warn!(dir = %root.display(), %error, "cannot watch the views tree");
        return;
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let watch_root = root.clone();
    let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        // A rename lands as Create on the new path, which is how most editors and
        // `apply_patch` write. Removes are not interesting: the module is still on disk
        // and a stale view beats an empty room, the same call `refresh_sources` makes.
        if !matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
            return;
        }
        for path in event.paths {
            if let Some(view_ref) = ref_of(&watch_root, &path) {
                let _ = tx.send(view_ref);
            }
        }
    });
    let mut watcher = match watcher {
        Ok(watcher) => watcher,
        Err(error) => {
            tracing::warn!(%error, "no filesystem watcher; views will not follow their source");
            return;
        }
    };
    if let Err(error) = watcher.watch(&root, RecursiveMode::Recursive) {
        tracing::warn!(dir = %root.display(), %error, "cannot watch the views tree");
        return;
    }

    tokio::spawn(async move {
        // The watcher stops the moment it is dropped, so it lives as long as the loop.
        let _watcher = watcher;
        while let Some(first) = rx.recv().await {
            let mut refs = HashSet::from([first]);
            let settle = tokio::time::sleep(SETTLE);
            tokio::pin!(settle);
            loop {
                tokio::select! {
                    () = &mut settle => break,
                    next = rx.recv() => match next {
                        Some(view_ref) => { refs.insert(view_ref); }
                        None => break,
                    },
                }
            }
            for view_ref in refs {
                follow(&bus, &data_dir, &compiler, &view_ref).await;
            }
        }
        tracing::info!("the views watcher stopped");
    });
}

/// The ref a written path names, or `None` for anything that is not a view's source.
///
/// The compiled modules, the thumbnails and the workshop's other scratch dirs all sit
/// *inside* the views tree, and the compiler writes to `_compiled` constantly — so
/// everything under a leading `_` is skipped, or a recompile would trigger the watch
/// that triggered it.
fn ref_of(root: &Path, path: &Path) -> Option<String> {
    if path.extension()? != "jsx" {
        return None;
    }
    let rel = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for part in rel.components() {
        let part = part.as_os_str().to_str()?;
        if part.starts_with('_') {
            return None;
        }
        parts.push(part);
    }
    let joined = parts.join("/");
    Some(joined.strip_suffix(".jsx")?.to_string())
}

/// Recompile `view_ref` and swap it under the layers showing it, if any are.
///
/// The screen is asked first so a builder saving its way through a workshop of views
/// nobody is looking at never spawns a compiler. A source that does not compile — which
/// is every intermediate save of a file being written — leaves the screen alone.
async fn follow(bus: &ViewBus, data_dir: &Path, compiler: &ViewCompiler, view_ref: &str) {
    if !bus.shows_ref(view_ref).await {
        return;
    }
    let source = match crate::mind::views::resolve_ref(data_dir, view_ref).await {
        Ok(source) => source,
        Err(error) => {
            tracing::debug!(view_ref, %error, "a view on screen was written and cannot be read");
            return;
        }
    };
    let module_url = match compiler.compile(&source).await {
        Ok(module_url) => module_url,
        Err(error) => {
            tracing::debug!(view_ref, %error, "a view on screen was written and does not compile");
            return;
        }
    };
    if bus.follow_source(view_ref, &module_url).await {
        tracing::info!(view_ref, module_url, "a view on screen was rewritten; the screen followed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_file_names_its_ref() {
        let root = Path::new("/data/views");
        assert_eq!(
            ref_of(root, Path::new("/data/views/knq/live-commentary.jsx")).as_deref(),
            Some("knq/live-commentary"),
        );
        assert_eq!(
            ref_of(root, Path::new("/data/views/factory/tasks.jsx")).as_deref(),
            Some("factory/tasks"),
        );
    }

    #[test]
    fn the_tool_dirs_and_everything_else_are_not_views() {
        let root = Path::new("/data/views");
        // The compiler's own output, or a recompile would trigger the watch that
        // triggered it.
        assert_eq!(ref_of(root, Path::new("/data/views/_compiled/ab12.mjs")), None);
        // A `.jsx` under a tool dir is still not a ref.
        assert_eq!(ref_of(root, Path::new("/data/views/_preview/draft.jsx")), None);
        assert_eq!(ref_of(root, Path::new("/data/views/_shots/ref/factory/tasks.png")), None);
        // Data a view reads is not the view.
        assert_eq!(ref_of(root, Path::new("/data/views/knq/notes.json")), None);
        // Outside the tree entirely.
        assert_eq!(ref_of(root, Path::new("/data/memory/raw/x.jsx")), None);
    }
}
