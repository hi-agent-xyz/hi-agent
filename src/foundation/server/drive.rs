//! Drive endpoints — a read-only window onto `<data_dir>/drive/`.
//!
//! `drive/` is the agent's Documents: files kept by a deliberate act, stored verbatim,
//! never digested by reflection (`docs/data-dir-layout.md`).
//! Because a drive entry is addressed *from memory* — a facet claim carries the path —
//! nothing today lists the tree for a human, so a file the agent filed is invisible
//! until someone greps the disk. These two routes are that listing.
//!
//! - `GET /api/drive` — the whole tree, dirs included, sorted by path.
//! - `GET /api/drive/file/{*path}` — one file's raw bytes.
//!
//! **Read-only, deliberately.** `docs/data-dir-layout.md` records that the `drive/`
//! tree exists but its *contract* does not: graduation of a kept view into
//! `drive/projects/` is not built, and `drive/notes`/`papers` are conventions the agent
//! is expected to grow, not code paths. Adding write verbs here would invent that
//! contract from the view side. So this surface only shows what is already there;
//! [`crate::run`] creates the root at boot and the agent fills it through ordinary file
//! tools.
//!
//! The tree nests arbitrarily and its names come from the agent, so the file route
//! guards twice: a syntactic check on the path segments, then a canonicalised
//! containment check that also defeats a symlink pointing out of the drive.

use std::sync::Arc;
use std::time::SystemTime;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::foundation::server::AppState;

// ── path safety ───────────────────────────────────────────────────────────────
//
// The root and both guards live in [`crate::mind::memory::media`], because the drive
// is now addressed from two directions: these routes, and the `⟨ref: …⟩` grammar that
// resolves `drive/<path>` for the generation tools. One tree guarded two ways is one
// guard too many — the copy that gets a fix is never reliably the copy in the path
// under attack.

use crate::mind::memory::media::{content_type, drive_root as drive_dir, ext_of, resolve_in_drive};

#[cfg(test)]
use crate::mind::memory::media::{resolve_in_root, safe_rel_path};

/// A file's mtime as `2026-08-04T10:00:00Z`. Falls back to the epoch when the platform
/// has no mtime — a missing timestamp must not drop the entry from the listing.
fn rfc3339(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    chrono::DateTime::from_timestamp(secs, 0)
        .unwrap_or_else(|| chrono::DateTime::from_timestamp(0, 0).expect("unix epoch is valid"))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ── list ──────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct EntryDto {
    /// Path relative to the drive root, `/`-separated.
    path: String,
    dir: bool,
    /// File size; `0` for directories (a directory's own inode size is noise here).
    bytes: u64,
    modified: String,
    ext: String,
}

/// Walk the drive. Iterative (a stack of dirs) rather than recursive so no boxing is
/// needed; hidden entries are skipped as incidental editor/OS files. Directories are
/// emitted as entries too — an empty `drive/papers/` is a real thing the view should show.
/// A missing root yields no entries: nothing has been filed yet, which is not an error.
async fn walk_drive(root: &std::path::Path) -> std::io::Result<Vec<EntryDto>> {
    let mut out: Vec<EntryDto> = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        while let Some(ent) = rd.next_entry().await? {
            let name = ent.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let Ok(ft) = ent.file_type().await else {
                continue;
            };
            // The file endpoint refuses symlinks after canonicalisation. The listing
            // should not advertise an alias it will never serve.
            if ft.is_symlink() {
                continue;
            }
            // An entry removed mid-walk is skipped rather than failing the whole list.
            let Ok(meta) = ent.metadata().await else {
                continue;
            };
            let is_dir = ft.is_dir();
            if is_dir {
                stack.push((ent.path(), rel.clone()));
            }
            out.push(EntryDto {
                path: rel,
                dir: is_dir,
                bytes: if is_dir { 0 } else { meta.len() },
                modified: rfc3339(meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)),
                ext: if is_dir { String::new() } else { ext_of(&name) },
            });
        }
    }
    // Sorted by path, so a parent always precedes its children and the view can render
    // the tree straight from the array without re-sorting.
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// `GET /api/drive` — every file and directory under `<data_dir>/drive`, path-sorted.
/// No drive on disk means `{"entries":[]}`: a fresh install has filed nothing, and
/// that empty state is the answer, not a failure.
pub async fn get_drive(State(state): State<Arc<AppState>>) -> Response {
    match walk_drive(&drive_dir(&state.data_dir)).await {
        Ok(entries) => Json(serde_json::json!({ "entries": entries })).into_response(),
        Err(e) => err(&e.to_string()),
    }
}

// ── one file ──────────────────────────────────────────────────────────────────

/// `GET /api/drive/file/{*path}` — the raw bytes of one kept file, so the view can
/// show a PDF, an image or a note inline. Verbatim: no transcoding, no rewriting.
pub async fn get_drive_file(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Response {
    let Some(full) = resolve_in_drive(&state.data_dir, &path).await else {
        return (StatusCode::NOT_FOUND, "not found\n").into_response();
    };
    let Ok(bytes) = tokio::fs::read(&full).await else {
        return (StatusCode::NOT_FOUND, "not found\n").into_response();
    };
    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type(&path)));
    // Drive files are edited in place under a stable path, so a cached copy would go
    // stale silently.
    resp.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

/// A uniform JSON error body with a 400.
fn err(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": msg }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write `<root>/<rel>` (creating parents) — the agent filing something.
    fn write(root: &std::path::Path, rel: &str, bytes: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    #[test]
    fn rel_path_blocks_traversal() {
        assert!(safe_rel_path("projects/lease/contract.pdf"));
        assert!(!safe_rel_path("../config.db"), "no parent traversal");
        assert!(!safe_rel_path("projects/../../secret"), "no mid-path traversal");
        assert!(!safe_rel_path("a/./b"), "no dot segment");
        assert!(!safe_rel_path("/etc/passwd"), "absolute rejected via empty first segment");
        assert!(!safe_rel_path("a//b"), "no empty segment");
        assert!(!safe_rel_path(""), "empty");
    }

    #[test]
    fn extensions_are_lowercased_and_dotless() {
        assert_eq!(ext_of("contract.PDF"), "pdf");
        assert_eq!(ext_of("archive.tar.gz"), "gz");
        assert_eq!(ext_of("Makefile"), "");
        assert_eq!(content_type("projects/lease/contract.pdf"), "application/pdf");
        assert_eq!(content_type("notes/facedet.md"), "text/plain; charset=utf-8");
        assert_eq!(content_type("notes/x.unknown"), "application/octet-stream");
    }

    #[tokio::test]
    async fn walk_lists_files_and_dirs_sorted_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("drive");
        write(&root, "projects/lease/contract.pdf", b"%PDF-1.4 ...");
        write(&root, "notes/facedet.md", b"call it like ...");
        std::fs::create_dir_all(root.join("papers")).unwrap();
        write(&root, ".DS_Store", b"litter");

        let entries = walk_drive(&root).await.unwrap();
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "notes",
                "notes/facedet.md",
                "papers",
                "projects",
                "projects/lease",
                "projects/lease/contract.pdf",
            ],
            "path-sorted, dirs included, dotfiles skipped"
        );

        let pdf = entries.iter().find(|e| e.path.ends_with("contract.pdf")).unwrap();
        assert!(!pdf.dir);
        assert_eq!(pdf.ext, "pdf");
        assert_eq!(pdf.bytes, 12);
        assert!(pdf.modified.ends_with('Z'), "{}", pdf.modified);

        let papers = entries.iter().find(|e| e.path == "papers").unwrap();
        assert!(papers.dir);
        assert_eq!(papers.bytes, 0);
        assert_eq!(papers.ext, "", "a directory has no extension");
    }

    #[tokio::test]
    async fn a_missing_drive_is_the_empty_state_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let entries = walk_drive(&dir.path().join("drive")).await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn resolve_stays_inside_the_drive() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("drive");
        write(&root, "notes/facedet.md", b"exact recipe");
        // Something precious next to the drive, one level up.
        std::fs::write(dir.path().join("config.db"), b"secrets").unwrap();

        assert!(resolve_in_root(&root, "notes/facedet.md").await.is_some());
        assert!(resolve_in_root(&root, "../config.db").await.is_none(), "no traversal out");
        assert!(resolve_in_root(&root, "notes/../../config.db").await.is_none());
        assert!(resolve_in_root(&root, "notes").await.is_none(), "a directory is not a file");
        assert!(resolve_in_root(&root, "notes/missing.md").await.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn resolve_rejects_a_symlink_pointing_out_of_the_drive() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("drive");
        std::fs::create_dir_all(&root).unwrap();
        let outside = dir.path().join("config.db");
        std::fs::write(&outside, b"secrets").unwrap();
        // A path with no `..` in it that still leaves the tree — only canonicalisation
        // catches this.
        std::os::unix::fs::symlink(&outside, root.join("escape.db")).unwrap();

        assert!(resolve_in_root(&root, "escape.db").await.is_none());
    }
}
