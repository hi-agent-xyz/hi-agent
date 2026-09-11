//! Watch the facets tree, so a board stops asking whether the ledger changed.
//!
//! **The filesystem is where the change happens**, which is the same reason
//! [`view_watch`](super::view_watch) hangs off it rather than off a tool call. A task record
//! is a `facet.md`, and the agent keeps it with a shell: the long dated accounts under
//! `## 当前接管状态` are appended by whoever is holding the duty, never through
//! [`write_task`](crate::mind::memory::tasks::write_task). There is no call to notice, so
//! noticing means watching the file.
//!
//! One dimension, one counter. `memory/facets/<dimension>/<subject>/facet.md` says which
//! store a write belongs to in its own path, so a projects write does not wake the task board
//! and a task write does not wake the chart's project list.
//!
//! ## Why this is not a recursive watch
//!
//! Because a task's folder is also **where its work happened**. One live store holds 6.6 GB
//! under `facets/` across **13,982 directories** — cloned repos, `node_modules`, scraped
//! pages — and a single task that cloned a repo accounts for 2,011 of them. `notify` takes
//! one inotify watch per directory on Linux, against a `max_user_watches` that is commonly
//! 8,192; a recursive watch here would exhaust it on this store today, and the number grows
//! with whatever a worker clones next.
//!
//! So this watches **only the three levels that can hold a record**, none of them recursively:
//! the root (a new dimension appearing), each dimension (a new subject appearing), and each
//! subject (its `facet.md` being written). That is 1 + 10 + 291 = 302 watches on the same
//! store — bounded by *how many records there are* rather than by what is inside them, which
//! is the property that has to hold. Everything a worker writes beside a record is invisible
//! here, which is also exactly right: it is not a change to the ledger.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{EventKind, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use super::stores::StoreVersions;
use crate::mind::memory::facets::FACET_FILE;
use crate::mind::memory::layout;

/// How long to let a burst of writes settle before bumping. A single record is saved as more
/// than one event — a write and a rename, two or three in a row — and bumping on each would
/// hand every parked reader the same change several times over.
const SETTLE: Duration = Duration::from_millis(300);

/// Past this many watched directories, say so once. Not a cap: refusing to watch a record
/// would make a board quietly stop following it, which is worse than the warning. This is the
/// number to look at when somebody's inotify limit is the thing that broke.
const MANY_WATCHES: usize = 2000;

/// What the watcher callback hands the loop. Two kinds because only the loop owns the
/// `Watcher`, so a directory that has just appeared cannot be subscribed from inside the
/// callback that saw it.
enum Seen {
    /// A record under this dimension was written, moved or removed.
    Record(String),
    /// A path that may be a new dimension or subject directory, and so may need a watch.
    Maybe(PathBuf),
}

/// Start watching `<data_dir>/memory/facets` for record writes.
///
/// Best-effort, like the views watcher. If no watcher can be had, the review reads that park
/// on a version only ever hear about **this process's own writes** — [`patch_task`] bumps
/// directly for exactly that reason — so a board still follows what a person does on it, and
/// stops following what the agent does with a shell. That degradation is logged, loudly,
/// because it is invisible from the screen.
///
/// [`patch_task`]: super::tasks::patch_task
pub fn spawn(versions: Arc<StoreVersions>, data_dir: PathBuf) {
    let root = layout::facets_dir(&data_dir);
    if let Err(error) = std::fs::create_dir_all(&root) {
        tracing::warn!(dir = %root.display(), %error, "cannot watch the facets tree");
        return;
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<Seen>();
    let watch_root = root.clone();
    let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        // A removal counts, unlike in the views watcher: a record that left the tree is a row
        // that has to leave the board, and keeping the last good reading would hold a task on
        // screen that no longer exists.
        if !matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
        ) {
            return;
        }
        for path in event.paths {
            match depth_in(&watch_root, &path) {
                Some(3) if path.file_name() == Some(FACET_FILE.as_ref()) => {
                    if let Some(dimension) = dimension_of(&watch_root, &path) {
                        let _ = tx.send(Seen::Record(dimension));
                    }
                }
                // A dimension or a subject: if it is a directory that just appeared, the loop
                // has to start watching it, or records written under it are never seen.
                Some(1 | 2) => {
                    let _ = tx.send(Seen::Maybe(path));
                }
                _ => {}
            }
        }
    });
    let mut watcher = match watcher {
        Ok(watcher) => watcher,
        Err(error) => {
            tracing::warn!(
                %error,
                "no filesystem watcher: review surfaces will not follow records the agent \
                 writes with a shell, only this process's own writes"
            );
            return;
        }
    };

    let mut watched: HashSet<PathBuf> = HashSet::new();
    if !subscribe(&mut watcher, &mut watched, &root) {
        return;
    }
    for dir in record_dirs(&root) {
        subscribe(&mut watcher, &mut watched, &dir);
    }
    tracing::info!(
        dirs = watched.len(),
        root = %root.display(),
        "watching the facets tree for record writes"
    );
    if watched.len() > MANY_WATCHES {
        tracing::warn!(
            dirs = watched.len(),
            "more watched directories than expected; on Linux check `max_user_watches`"
        );
    }

    tokio::spawn(async move {
        // The watcher stops the moment it is dropped, so it lives as long as the loop.
        let mut watcher = watcher;
        while let Some(first) = rx.recv().await {
            let mut moved: HashSet<String> = HashSet::new();
            take(&mut watcher, &mut watched, &root, first, &mut moved);
            let settle = tokio::time::sleep(SETTLE);
            tokio::pin!(settle);
            loop {
                tokio::select! {
                    () = &mut settle => break,
                    next = rx.recv() => match next {
                        Some(seen) => take(&mut watcher, &mut watched, &root, seen, &mut moved),
                        None => break,
                    },
                }
            }
            for dimension in moved {
                tracing::debug!(%dimension, "a record moved; waking its readers");
                versions.bump(&dimension).await;
            }
        }
        tracing::info!("the facets watcher stopped");
    });
}

/// Fold one event into the batch, subscribing to a directory that has just appeared.
///
/// **A new subject directory counts as a change to its dimension**, and not only because it
/// usually is one. The watch on it is added *after* it appears, so a record written into it
/// in the same breath — which is how a task is created — can land before anything is
/// listening. Bumping here is what closes that gap: the reader re-reads the tree and finds
/// whatever is in it, rather than waiting for a second write that may never come.
fn take(
    watcher: &mut impl Watcher,
    watched: &mut HashSet<PathBuf>,
    root: &Path,
    seen: Seen,
    moved: &mut HashSet<String>,
) {
    match seen {
        Seen::Record(dimension) => {
            moved.insert(dimension);
        }
        Seen::Maybe(path) => {
            if !path.is_dir() || watched.contains(&path) {
                return;
            }
            subscribe(watcher, watched, &path);
            if let Some(dimension) = dimension_under(root, &path) {
                moved.insert(dimension);
            }
        }
    }
}

fn subscribe(watcher: &mut impl Watcher, watched: &mut HashSet<PathBuf>, dir: &Path) -> bool {
    if let Err(error) = watcher.watch(dir, RecursiveMode::NonRecursive) {
        tracing::warn!(dir = %dir.display(), %error, "cannot watch this part of the facets tree");
        return false;
    }
    watched.insert(dir.to_path_buf());
    true
}

/// Every `<dimension>` and `<dimension>/<subject>` directory under `root`, and nothing
/// deeper. Reads two levels of the tree rather than walking it: the third level is the 6.6 GB.
fn record_dirs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(dimensions) = std::fs::read_dir(root) else {
        return out;
    };
    for dimension in dimensions.flatten() {
        let dimension = dimension.path();
        if !dimension.is_dir() {
            continue;
        }
        if let Ok(subjects) = std::fs::read_dir(&dimension) {
            for subject in subjects.flatten() {
                let subject = subject.path();
                if subject.is_dir() {
                    out.push(subject);
                }
            }
        }
        out.push(dimension);
    }
    out
}

/// How many components below `root` a path sits, or `None` if it is not under it at all.
fn depth_in(root: &Path, path: &Path) -> Option<usize> {
    Some(path.strip_prefix(root).ok()?.components().count())
}

/// The dimension a `<dimension>/<subject>/facet.md` belongs to.
fn dimension_of(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    if rel.components().count() != 3 || path.file_name() != Some(FACET_FILE.as_ref()) {
        return None;
    }
    Some(rel.components().next()?.as_os_str().to_str()?.to_owned())
}

/// The dimension a directory is or is under — `tasks` for both `tasks` and `tasks/ship-it`.
fn dimension_under(root: &Path, dir: &Path) -> Option<String> {
    let rel = dir.strip_prefix(root).ok()?;
    if !matches!(rel.components().count(), 1 | 2) {
        return None;
    }
    Some(rel.components().next()?.as_os_str().to_str()?.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_names_its_own_store() {
        let root = Path::new("/d/memory/facets");
        assert_eq!(
            dimension_of(root, Path::new("/d/memory/facets/tasks/ship-it/facet.md")).as_deref(),
            Some("tasks")
        );
        assert_eq!(
            dimension_of(root, Path::new("/d/memory/facets/projects/knq/facet.md")).as_deref(),
            Some("projects")
        );
    }

    /// The 6.6 GB question: a worker writing into a task's folder is not a ledger change, and
    /// is not even seen — nothing under a subject is watched except the record itself.
    #[test]
    fn the_work_beside_a_record_is_not_a_change_to_it() {
        let root = Path::new("/d/memory/facets");
        for path in [
            "/d/memory/facets/tasks/ship-it/evidence-run.md",
            "/d/memory/facets/tasks/ship-it/repo/src/main.rs",
            "/d/memory/facets/facet.md",
            "/d/memory/facets/tasks/facet.md",
        ] {
            assert_eq!(dimension_of(root, Path::new(path)), None, "{path}");
        }
        assert_eq!(depth_in(root, Path::new("/d/memory/facets/tasks/x/repo/a.rs")), Some(4));
    }

    /// Only the two levels that can hold a record, so the watch count follows how many
    /// records there are and not what a worker cloned into one of them.
    #[test]
    fn only_the_levels_that_can_hold_a_record_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("tasks/ship-it/repo/src/deep")).unwrap();
        std::fs::create_dir_all(root.join("tasks/other-one")).unwrap();
        std::fs::create_dir_all(root.join("projects/knq")).unwrap();
        std::fs::write(root.join("tasks/ship-it/facet.md"), "---\n---\n").unwrap();

        let mut found: Vec<String> = record_dirs(root)
            .iter()
            .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        found.sort();
        assert_eq!(
            found,
            vec!["projects", "projects/knq", "tasks", "tasks/other-one", "tasks/ship-it"],
            "the repo under ship-it is not watched"
        );
    }

    #[test]
    fn a_new_subject_directory_is_a_change_to_its_dimension() {
        let root = Path::new("/d/memory/facets");
        assert_eq!(dimension_under(root, Path::new("/d/memory/facets/tasks")).as_deref(), Some("tasks"));
        assert_eq!(
            dimension_under(root, Path::new("/d/memory/facets/tasks/ship-it")).as_deref(),
            Some("tasks")
        );
        assert_eq!(dimension_under(root, Path::new("/d/memory/facets/tasks/x/repo")), None);
    }
}
