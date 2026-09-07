//! Drive endpoints — a read-only window onto `<data_dir>/drive/`.
//!
//! `drive/` is the agent's Documents: files kept by a deliberate act, stored verbatim,
//! never digested by reflection (`docs/data-dir-layout.md`).
//! Because a drive entry is addressed *from memory* — a facet claim carries the path —
//! nothing today lists the tree for a human, so a file the agent filed is invisible
//! until someone greps the disk. These two routes are that listing.
//!
//! - `GET /api/drive?under=<dir>` — one directory's immediate children (`under` absent
//!   or empty means the drive root).
//! - `GET /api/drive/file/{*path}` — one file's raw bytes.
//!
//! **One directory, not the whole tree.** This route used to walk the drive to its
//! leaves and return every entry in one array, on the assumption that the cabinet is a
//! few shelves deep. It isn't: an agent that files a message archive grows
//! `wecom/<listener>/inbox/<yyyy>/<mm>/<dd>/<msg-id>/` — 876 files eight levels down on
//! the box this was found on, out of 949 in the drive. The view rendered every one of
//! those leaf paths as a row (there is no other shape a flat array has), and the
//! eight-second poll behind it re-walked the entire cabinet each time. Both problems
//! are the same problem, and listing one folder is the fix for both: the page is one
//! folder's worth of rows, and the read is one folder's worth of `read_dir`.
//!
//! A directory child carries `items`, the number of things directly inside it, so a
//! folder row can say how much is behind it without the walk that was just removed.
//!
//! **Read-only, deliberately.** `docs/data-dir-layout.md` records that the `drive/`
//! tree exists but its *contract* does not: graduation of a kept view into
//! `drive/projects/` is not built, and `drive/notes`/`papers` are conventions the agent
//! is expected to grow, not code paths. Adding write verbs here would invent that
//! contract from the view side. So this surface only shows what is already there;
//! [`crate::run`] creates the root at boot and the agent fills it through ordinary file
//! tools.
//!
//! The tree nests arbitrarily and its names come from the agent, so both routes guard
//! the path they are handed twice: a syntactic check on the segments, then a
//! canonicalised containment check that also defeats a symlink pointing out of the drive.

use std::sync::Arc;
use std::time::SystemTime;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::foundation::server::AppState;

// ── path safety ───────────────────────────────────────────────────────────────
//
// The root and both guards live in [`crate::mind::memory::media`], because the drive
// is now addressed from two directions: these routes, and the `⟨ref: …⟩` grammar that
// resolves `drive/<path>` for the generation tools. One tree guarded two ways is one
// guard too many — the copy that gets a fix is never reliably the copy in the path
// under attack.

use crate::mind::memory::media::{
    content_type, drive_root as drive_dir, ext_of, resolve_dir_in_drive, resolve_in_drive,
};

#[cfg(test)]
use crate::mind::memory::media::{resolve_dir_in_root, resolve_in_root, safe_rel_path};

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
    /// Path relative to the drive root, `/`-separated — not just the name, because it
    /// is what the file route and the next listing are addressed by.
    path: String,
    dir: bool,
    /// File size; `0` for directories (a directory's own inode size is noise here).
    bytes: u64,
    modified: String,
    ext: String,
    /// How many things are directly inside, for directories; `0` for files. Immediate
    /// children only — a subtree total would put the removed walk back, one level down.
    items: usize,
}

/// Which directory to list. Absent, or empty, is the drive root.
#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    under: String,
}

/// How many listable things are directly inside `dir` — the same entries [`list_dir`]
/// would return. An unreadable directory counts zero rather than failing the row it
/// belongs to; the count is a hint on a folder, not the answer to anything.
async fn count_children(dir: &std::path::Path) -> usize {
    let Ok(mut rd) = tokio::fs::read_dir(dir).await else {
        return 0;
    };
    let mut n = 0;
    while let Ok(Some(ent)) = rd.next_entry().await {
        if ent.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if ent.file_type().await.map(|ft| ft.is_symlink()).unwrap_or(true) {
            continue;
        }
        n += 1;
    }
    n
}

/// The immediate children of one directory. `prefix` is that directory's own
/// drive-relative path (empty at the root) and is what each entry's `path` is built
/// from. Hidden entries are skipped as incidental editor/OS files.
///
/// A missing directory yields no entries: at the root that is a fresh install having
/// filed nothing, which is not an error.
async fn list_dir(dir: &std::path::Path, prefix: &str) -> std::io::Result<Vec<EntryDto>> {
    let mut out: Vec<EntryDto> = Vec::new();
    let mut rd = match tokio::fs::read_dir(dir).await {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e),
    };
    while let Some(ent) = rd.next_entry().await? {
        let name = ent.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(ft) = ent.file_type().await else {
            continue;
        };
        // The file endpoint refuses symlinks after canonicalisation. The listing
        // should not advertise an alias it will never serve.
        if ft.is_symlink() {
            continue;
        }
        // An entry removed mid-listing is skipped rather than failing the whole list.
        let Ok(meta) = ent.metadata().await else {
            continue;
        };
        let is_dir = ft.is_dir();
        out.push(EntryDto {
            path: if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") },
            dir: is_dir,
            bytes: if is_dir { 0 } else { meta.len() },
            modified: rfc3339(meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)),
            ext: if is_dir { String::new() } else { ext_of(&name) },
            items: if is_dir { count_children(&ent.path()).await } else { 0 },
        });
    }
    // Folders first, then files, each by name — the order a folder is read in, and one
    // the view can render straight from the array without re-sorting.
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.path.cmp(&b.path)));
    Ok(out)
}

/// `GET /api/drive?under=<dir>` — one directory's immediate children, folders first.
/// No drive on disk means `{"path":"","entries":[]}`: a fresh install has filed
/// nothing, and that empty state is the answer, not a failure. A named directory that
/// is not there is a 404 — the client asked for a specific place.
pub async fn get_drive(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> Response {
    let dir = if q.under.is_empty() {
        drive_dir(&state.data_dir)
    } else {
        match resolve_dir_in_drive(&state.data_dir, &q.under).await {
            Some(p) => p,
            None => {
                return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "no such folder" })))
                    .into_response();
            }
        }
    };
    match list_dir(&dir, &q.under).await {
        Ok(entries) => Json(serde_json::json!({ "path": q.under, "entries": entries })).into_response(),
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
    async fn a_listing_is_one_folder_deep_folders_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("drive");
        write(&root, "projects/lease/contract.pdf", b"%PDF-1.4 ...");
        write(&root, "notes/facedet.md", b"call it like ...");
        write(&root, "receipt.txt", b"paid");
        std::fs::create_dir_all(root.join("papers")).unwrap();
        write(&root, ".DS_Store", b"litter");

        let top = list_dir(&root, "").await.unwrap();
        let paths: Vec<&str> = top.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["notes", "papers", "projects", "receipt.txt"],
            "folders first then files, each by name; dotfiles skipped; nothing nested"
        );

        let projects = top.iter().find(|e| e.path == "projects").unwrap();
        assert!(projects.dir);
        assert_eq!(projects.bytes, 0);
        assert_eq!(projects.ext, "", "a directory has no extension");
        assert_eq!(projects.items, 1, "the `lease` folder inside it");

        let papers = top.iter().find(|e| e.path == "papers").unwrap();
        assert_eq!(papers.items, 0, "an empty shelf is an answer, not an absence");

        // One level down, and the entry paths stay drive-relative — they are what the
        // file route and the next listing are addressed by.
        let inside = list_dir(&root.join("projects/lease"), "projects/lease").await.unwrap();
        let pdf = &inside[0];
        assert_eq!(pdf.path, "projects/lease/contract.pdf");
        assert!(!pdf.dir);
        assert_eq!(pdf.ext, "pdf");
        assert_eq!(pdf.bytes, 12);
        assert_eq!(pdf.items, 0);
        assert!(pdf.modified.ends_with('Z'), "{}", pdf.modified);
    }

    #[tokio::test]
    async fn a_missing_drive_is_the_empty_state_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let entries = list_dir(&dir.path().join("drive"), "").await.unwrap();
        assert!(entries.is_empty());
    }

    /// The listed folder is client-supplied, so it gets the file route's guards. A
    /// traversal, a file asked for as a folder, and a folder that isn't there all have
    /// to come back as nothing rather than as a listing of somewhere else.
    #[tokio::test]
    async fn the_listed_folder_stays_inside_the_drive() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("drive");
        write(&root, "notes/facedet.md", b"exact recipe");
        std::fs::create_dir_all(dir.path().join("elsewhere")).unwrap();

        assert!(resolve_dir_in_root(&root, "notes").await.is_some());
        assert!(resolve_dir_in_root(&root, "../elsewhere").await.is_none(), "no traversal out");
        assert!(resolve_dir_in_root(&root, "notes/../../elsewhere").await.is_none());
        assert!(
            resolve_dir_in_root(&root, "notes/facedet.md").await.is_none(),
            "a file is not a folder"
        );
        assert!(resolve_dir_in_root(&root, "papers").await.is_none(), "not there");
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
