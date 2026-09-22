//! Serving a file off disk to an off-box client — the one way, for every route
//! that does it.
//!
//! **Three things were wrong with reading the file into a `Vec` and handing that
//! back**, and only the third is about speed:
//!
//! 1. **No byte ranges anywhere.** `tower-http` did not even carry its `fs`
//!    feature, so nothing in this server ever parsed a `Range` or answered
//!    `Accept-Ranges`. Safari opens a `<video>` by asking for `bytes=0-1` first;
//!    a server that ignores that and returns the whole file with a `200` gets a
//!    player that will not start until it has buffered everything, and a scrub
//!    bar that cannot seek at all. A hundred-megabyte video over a home uplink
//!    is not a slow video, it is a broken one.
//! 2. **The whole file went into memory before the first byte went out.** Time
//!    to first byte included reading it all off disk, and the core held one
//!    complete copy per concurrent viewer.
//! 3. **No conditional request.** Nothing emitted `Last-Modified`, so no client
//!    could ever be told "you already have this".
//!
//! [`ServeFile`] answers all three and is far better tested than a hand-rolled
//! range parser would be, which is the whole reason to reach for it: `Range`,
//! `If-Modified-Since`, `416` on an unsatisfiable range, and a streaming body.
//!
//! **What this module adds on top is the two headers a file service cannot know.**
//! Content type comes from this repo's own tables rather than a guess from the
//! extension — they carry entries a guesser does not (`heic`, `m4v`, `jsonl` as
//! text) and they are what the `⟨ref⟩` grammar agrees with. And `Cache-Control`
//! is a policy decision per route, which is now load-bearing twice over: it
//! drives the browser cache, and `immutable` is what marks a response eligible
//! to be mirrored off the machine at all (`docs/arch/topology.md` § *Content*).
//!
//! **Not used for `/views/*`**, deliberately. That route sits inside a
//! `CompressionLayer`, and a compressed `206` would carry a `Content-Range`
//! describing bytes that are not the bytes in the body. Compiled modules are
//! tens of kilobytes of text where compression is the win and ranges are never
//! asked for, so it keeps the whole-body read and this module stays out of it.

use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use tower::ServiceExt as _;
use tower_http::services::ServeFile;

/// Serve `path`, honouring whatever conditional or range headers `req` carries.
///
/// `content_type` and `cache_control` are written over whatever the file service
/// decided. `missing` is the body for a file that is not there, so each caller
/// keeps the 404 wording its own route already had.
///
/// The response is streamed: nothing here holds the file.
pub async fn serve(
    req: Request,
    path: &std::path::Path,
    content_type: &'static str,
    cache_control: &'static str,
    missing: &'static str,
) -> Response {
    // `ServeFile::new` guesses a content type from the extension and we then
    // replace it. The guess is wasted work of no measurable size, and taking the
    // guessing path keeps us off the `mime` crate as a direct dependency for a
    // value we were always going to overwrite.
    let served = match ServeFile::new(path).oneshot(req).await {
        Ok(served) => served,
        // The service's own error type is `Infallible` in this version; treat any
        // future change to that as the file being unreadable rather than
        // unwrapping on it.
        Err(_) => return (StatusCode::NOT_FOUND, missing).into_response(),
    };

    // A file that is not there, or that the range made unsatisfiable, keeps the
    // status the service chose — but a 404 gets this route's own wording, because
    // the body a caller already had is part of its contract.
    if served.status() == StatusCode::NOT_FOUND {
        return (StatusCode::NOT_FOUND, missing).into_response();
    }

    let mut resp = served.map(Body::new);
    let headers = resp.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(cache_control));
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header::{ACCEPT_RANGES, CONTENT_RANGE, RANGE};

    fn write(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("clip.mp4");
        std::fs::write(&path, bytes).expect("write");
        (dir, path)
    }

    fn get(range: Option<&str>) -> Request {
        let mut b = Request::builder().uri("/api/media/file/x/00/00-00.mp4");
        if let Some(r) = range {
            b = b.header(RANGE, r);
        }
        b.body(Body::empty()).expect("request")
    }

    async fn body_bytes(resp: Response) -> Vec<u8> {
        axum::body::to_bytes(resp.into_body(), usize::MAX).await.expect("body").to_vec()
    }

    /// The headers this module exists to impose win over whatever the file
    /// service decided — including on a `206`, where `immutable` is what marks
    /// the response mirrorable at all.
    #[tokio::test]
    async fn our_content_type_and_cache_control_win() {
        let (_d, path) = write(b"0123456789");
        let resp =
            serve(get(None), &path, "video/mp4", "private, max-age=31536000, immutable", "gone")
                .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()[CONTENT_TYPE], "video/mp4");
        assert_eq!(resp.headers()[CACHE_CONTROL], "private, max-age=31536000, immutable");
    }

    /// The point of the module: a player asking for the first two bytes gets two
    /// bytes and a `206`, not the whole file and a `200`. This is the exact probe
    /// Safari opens a `<video>` with.
    #[tokio::test]
    async fn a_range_request_is_answered_with_that_range() {
        let (_d, path) = write(b"0123456789");
        let resp = serve(get(Some("bytes=0-1")), &path, "video/mp4", "no-store", "gone").await;
        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(resp.headers()[CONTENT_RANGE], "bytes 0-1/10");
        assert_eq!(body_bytes(resp).await, b"01");
    }

    /// Seeking needs a range from the middle to work as well as one from the
    /// start — a scrub bar asks for neither end.
    #[tokio::test]
    async fn a_range_from_the_middle_seeks() {
        let (_d, path) = write(b"0123456789");
        let resp = serve(get(Some("bytes=4-6")), &path, "video/mp4", "no-store", "gone").await;
        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(body_bytes(resp).await, b"456");
    }

    /// Advertising the capability is half of it: a player that sees no
    /// `Accept-Ranges` may not try to seek even against a server that would
    /// have answered.
    #[tokio::test]
    async fn ranges_are_advertised_on_a_plain_get() {
        let (_d, path) = write(b"0123456789");
        let resp = serve(get(None), &path, "video/mp4", "no-store", "gone").await;
        assert_eq!(resp.headers()[ACCEPT_RANGES], "bytes");
    }

    /// A missing file keeps the caller's own 404 wording rather than the file
    /// service's, because that body is part of the route's contract.
    #[tokio::test]
    async fn a_missing_file_keeps_the_callers_wording() {
        let dir = tempfile::tempdir().expect("tempdir");
        let resp =
            serve(get(None), &dir.path().join("nope.mp4"), "video/mp4", "no-store", "no such media")
                .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert_eq!(body_bytes(resp).await, b"no such media");
    }
}
