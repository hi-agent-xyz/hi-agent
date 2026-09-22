//! `GET /views/<path>` — serve a file from the agent's views folder on disk (where
//! `AppState.data_dir` is in scope, unlike the embed-only appearance router):
//! compiled view modules ([`crate::mind::views::ViewCompiler`] writes them under
//! `_compiled/`), images a build sub-agent downloaded, and anything else it
//! authored. Single-user and trusted, so served whole, guarded only against `..`.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::foundation::server::AppState;

/// The only traversal guard for the views tree: reject any empty or `..` segment, so
/// the joined path can't climb out of the views root. Dotfiles are allowed (harmless;
/// compiled modules live under the non-dotfile `_compiled/`).
fn safe_views_path(path: &str) -> bool {
    !path.is_empty() && path.split('/').all(|seg| !seg.is_empty() && seg != "..")
}

/// `GET /views/<path>` — serve a file from the agent's views folder: a compiled view
/// module from `_compiled/`, an image, or any artifact a build sub-agent wrote.
/// The views tree is single-user and trusted, so it's served whole; the only guard is
/// against `..` traversal out of the root.
pub async fn views_file(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Response {
    if !safe_views_path(&path) {
        return (StatusCode::NOT_FOUND, "not found\n").into_response();
    }

    let full = state.data_dir.join("views").join(&path);
    let bytes = match tokio::fs::read(&full).await {
        Ok(bytes) => bytes,
        Err(_) => return (StatusCode::NOT_FOUND, "not found\n").into_response(),
    };

    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(crate::mind::memory::media::content_type(&path)));
    // Compiled modules under _compiled/ are content-addressed → immutable; source files
    // change in place, so they must not be cached.
    //
    // **`private`, because this is the agent's own code, written for one person.**
    // Content-addressing makes a module safe to cache *forever*; it does not make
    // it safe to cache *shared*. This path is gated, so `public` would let a CDN
    // store a view it only received because a credential checked out, and then
    // serve it to a request carrying none. Same reasoning, and the same fix, as
    // the embedded assets in `appearance::serve_embedded`.
    //
    // **Their pictures under `_shots/` are kept a year but not called immutable**, because
    // the bytes at a picture's path do change: a record shot an older renderer left in
    // the wrong shape is healed in place, one pruned past the keep limit is rendered
    // again with live data, and `_shots/ref/` is re-taken on a clock and told apart only
    // by the `?v=<mtime>` its URL carries. `immutable` vouches for the *path* now,
    // because the path is what a mirror of this response is keyed on
    // (`docs/arch/topology.md` § *Content*).
    let cache = if path.starts_with("_compiled/") {
        "private, max-age=31536000, immutable"
    } else if path.starts_with("_shots/") {
        "private, max-age=31536000"
    } else {
        "no-store"
    };
    resp.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_path_blocks_traversal() {
        assert!(safe_views_path("_compiled/0a1b.mjs"));
        assert!(safe_views_path("badminton-top10/leader.jsx"));
        assert!(!safe_views_path("../secret"), "no parent traversal");
        assert!(!safe_views_path("a/../b"), "no mid-path traversal");
        assert!(!safe_views_path("a//b"), "no empty segment");
        assert!(!safe_views_path(""), "empty");
    }

    /// The views tree reads the one table, which is how a clip in a view's folder stopped
    /// being `text/plain` — the reason eight views fetched their own video as a blob
    /// instead of naming it in a `<video src>`.
    #[test]
    fn a_views_file_is_typed_by_the_one_table() {
        use crate::mind::memory::media::content_type;
        assert_eq!(content_type("x.mjs"), "application/javascript; charset=utf-8");
        assert_eq!(content_type("a/b/photo.png"), "image/png");
        assert_eq!(content_type("v.jsx"), "text/plain; charset=utf-8");
        assert_eq!(content_type("court/out/marked.mp4"), "video/mp4");
        assert_eq!(content_type("notes.bin"), "application/octet-stream");
    }
}
