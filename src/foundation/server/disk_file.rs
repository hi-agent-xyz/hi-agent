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
//! **Every response carries an `ETag`**, the file's length and modification time,
//! weak because a compressed and an uncompressed body share it, and `If-None-Match`
//! is answered here before the file service sees the request. That is a sharper
//! validator than `Last-Modified` — a file written twice inside a second is two
//! versions — and it is what the community's edge compares on every request for a
//! file that changes in place (`docs/arch/cache.md` § *Two ways a copy is trusted*).
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
//! to be mirrored off the machine at all (`docs/arch/cache.md`).
//!
//! **`/views/*` uses it too**, inside a `CompressionLayer` whose predicate leaves
//! alone any response carrying a `Content-Range` — a compressed `206` would
//! describe bytes that are not the bytes in its body — and any video or audio,
//! which gains nothing from it.

use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
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
    mut req: Request,
    path: &std::path::Path,
    content_type: &'static str,
    cache_control: &'static str,
    missing: &'static str,
) -> Response {
    let Ok(meta) = tokio::fs::metadata(path).await else {
        return (StatusCode::NOT_FOUND, missing).into_response();
    };
    let tag = etag(&meta);
    if req.headers().contains_key(IF_NONE_MATCH) {
        if matches(req.headers(), &tag) {
            let mut resp = StatusCode::NOT_MODIFIED.into_response();
            let headers = resp.headers_mut();
            headers.insert(ETAG, tag);
            headers.insert(CACHE_CONTROL, HeaderValue::from_static(cache_control));
            return resp;
        }
        // RFC 9110 § 13.2.2: with `If-None-Match` present, `If-Modified-Since` is
        // ignored — it is the coarser of the two, and would call a file rewritten
        // inside the same second unchanged.
        req.headers_mut().remove(IF_MODIFIED_SINCE);
    }

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
    headers.insert(ETAG, tag);
    resp
}

/// `W/"<length>-<mtime in nanoseconds>"`, both hex. A file whose modification time
/// cannot be read gets its length alone, which is weaker and still never wrong in
/// the direction that matters: two different lengths are never the same tag.
fn etag(meta: &std::fs::Metadata) -> HeaderValue {
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    HeaderValue::from_str(&format!("W/\"{:x}-{mtime:x}\"", meta.len())).expect("hex is a valid header")
}

/// Whether `If-None-Match` names `tag`, by weak comparison, or is `*`.
fn matches(headers: &HeaderMap, tag: &HeaderValue) -> bool {
    let want = tag.to_str().unwrap_or("").trim_start_matches("W/");
    headers.get_all(IF_NONE_MATCH).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(',')).any(|t| {
        let t = t.trim();
        t == "*" || t.trim_start_matches("W/") == want
    })
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

    /// A second look names the version it has, and is told it still has it — no body.
    /// A rewritten file is a new tag, so the same question gets the bytes.
    #[tokio::test]
    async fn an_etag_answers_if_none_match() {
        let (_d, path) = write(b"0123456789");
        let first = serve(get(None), &path, "video/mp4", "no-cache", "gone").await;
        let tag = first.headers()[ETAG].clone();
        assert!(tag.to_str().unwrap().starts_with("W/\"a-"), "{tag:?}");

        let again = |tag: HeaderValue| {
            let mut r = get(None);
            r.headers_mut().insert(IF_NONE_MATCH, tag);
            r
        };
        let resp = serve(again(tag.clone()), &path, "video/mp4", "no-cache", "gone").await;
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(resp.headers()[ETAG], tag);
        assert!(body_bytes(resp).await.is_empty());

        std::fs::write(&path, b"01234567890").unwrap();
        let resp = serve(again(tag.clone()), &path, "video/mp4", "no-cache", "gone").await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_ne!(resp.headers()[ETAG], tag);
    }

    /// A range carries the tag too: the edge compares it on a seek as on a first look.
    #[tokio::test]
    async fn a_range_carries_the_etag() {
        let (_d, path) = write(b"0123456789");
        let resp = serve(get(Some("bytes=0-1")), &path, "video/mp4", "no-cache", "gone").await;
        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
        assert!(resp.headers().contains_key(ETAG));
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
