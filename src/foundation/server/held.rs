//! Responses held open — the long-polls and streams — and the two limits every hop
//! between a person and this core puts on them.
//!
//! A core reached by name is behind the community's CDN (EdgeOne), and that edge
//! measures a held response two ways. Measured 2026-09-24 against
//! `iloahz.hi-agent.xyz`:
//!
//! - **No head within 30 s is a `524`.** A long-poll that parks before answering at
//!   all — `/api/out/view?since=`, `/api/out/audio` waiting for a turn — was answered
//!   by the edge at 30.1 s, every time, with the core still holding it.
//! - **A body that sends nothing for ~15 s is cut.** `/api/out/text` sends its window
//!   and then nothing until somebody speaks; it died at 16–17 s as an HTTP/2 stream
//!   reset, and the page reconnected and fetched the whole 68 KB window again. SSE
//!   with axum's default 15 s keepalive died on a keepalive, at 15.4, 30.1 and 45.1 s:
//!   the limit and the interval were the same number, so every tick was a coin toss.
//!
//! Loopback has neither limit, and a public bind may have others; these numbers are
//! chosen to sit well inside the tightest measured one so no carrier has to be told
//! apart from another.

use std::convert::Infallible;
use std::time::Duration;

use axum::response::sse::KeepAlive;
use bytes::Bytes;
use futures::{Stream, StreamExt};

/// How long a held body may go without a byte. A third of the edge's ~15 s, so a tick
/// that crosses a slow tunnel still lands in time; a few bytes every few seconds is
/// nothing next to what the stream exists to carry.
pub const KEEPALIVE: Duration = Duration::from_secs(5);

/// How long a long-poll parks before answering "nothing yet" and letting the client
/// ask again. Under the edge's 30 s with room for the tunnel's own latency.
pub const LONG_POLL: Duration = Duration::from_secs(20);

/// An SSE keepalive at [`KEEPALIVE`], for every event stream this core serves.
pub fn sse_keep_alive() -> KeepAlive {
    KeepAlive::new().interval(KEEPALIVE)
}

/// An NDJSON body that sends an empty line whenever `lines` has been quiet for
/// [`KEEPALIVE`].
///
/// An empty line is the one thing every NDJSON reader already skips, so this needs
/// nothing from a client. Measured from the last byte rather than on a fixed clock,
/// so a stream that is talking pays nothing for it.
pub fn ndjson_keep_alive<S>(lines: S) -> impl Stream<Item = Result<Bytes, Infallible>> + Send
where
    S: Stream<Item = Result<Bytes, Infallible>> + Send + 'static,
{
    // Dropping `next()` on a timeout is safe: the pending item lives in the stream,
    // not in the future that polled it, so it is still there on the next call.
    futures::stream::unfold(Box::pin(lines), |mut lines| async move {
        match tokio::time::timeout(KEEPALIVE, lines.next()).await {
            Ok(Some(line)) => Some((line, lines)),
            Ok(None) => None,
            Err(_) => Some((Ok(Bytes::from_static(b"\n")), lines)),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Quiet is filled with blank lines, talk is passed through untouched, and the end
    /// of the stream is still the end.
    #[tokio::test(start_paused = true)]
    async fn a_quiet_stream_says_blank_lines_until_it_speaks() {
        let (tx, rx) = futures::channel::mpsc::unbounded::<Result<Bytes, Infallible>>();
        let mut body = Box::pin(ndjson_keep_alive(rx));

        tx.unbounded_send(Ok(Bytes::from_static(b"{\"a\":1}\n"))).unwrap();
        assert_eq!(body.next().await.unwrap().unwrap(), &b"{\"a\":1}\n"[..]);

        // Nothing to say for longer than the edge will wait: a blank line, twice.
        assert_eq!(body.next().await.unwrap().unwrap(), &b"\n"[..]);
        assert_eq!(body.next().await.unwrap().unwrap(), &b"\n"[..]);

        tx.unbounded_send(Ok(Bytes::from_static(b"{\"b\":2}\n"))).unwrap();
        assert_eq!(body.next().await.unwrap().unwrap(), &b"{\"b\":2}\n"[..]);

        drop(tx);
        assert!(body.next().await.is_none(), "the stream still ends");
    }

    /// A compressor in front of a held body hands each line on as it comes, rather
    /// than holding bytes until the body ends — which is what lets the text stream be
    /// compressed at all. async-compression flushes whenever its input goes pending;
    /// this pins that, because the router used to be laid out on the opposite belief.
    #[tokio::test(start_paused = true)]
    async fn a_compressed_held_body_still_arrives_line_by_line() {
        use tower::ServiceExt as _;
        use tower_http::compression::{CompressionLayer, CompressionLevel};

        let (tx, rx) = futures::channel::mpsc::unbounded::<Result<Bytes, Infallible>>();
        let rx = std::sync::Arc::new(std::sync::Mutex::new(Some(rx)));
        let app = axum::Router::new()
            .route(
                "/",
                axum::routing::get(move || {
                    let lines = rx.lock().unwrap().take().expect("one request");
                    async move {
                        axum::response::Response::builder()
                            .header("content-type", "application/x-ndjson")
                            .body(axum::body::Body::from_stream(ndjson_keep_alive(lines)))
                            .unwrap()
                    }
                }),
            )
            .layer(CompressionLayer::new().quality(CompressionLevel::Precise(6)));

        let res = app
            .oneshot(
                axum::http::Request::get("/")
                    .header("accept-encoding", "gzip")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.headers()["content-encoding"], "gzip");
        let mut body = res.into_body().into_data_stream();

        // The body stays open after this line; its bytes must not wait for the end.
        tx.unbounded_send(Ok(Bytes::from(format!("{{\"reset\":\"{}\"}}\n", "x".repeat(4096)))))
            .unwrap();
        let first = tokio::time::timeout(Duration::from_millis(100), body.next())
            .await
            .expect("the line arrives while the body is still open")
            .unwrap()
            .unwrap();
        assert!(!first.is_empty());
        assert!(first.len() < 4096, "and it is compressed: {} bytes", first.len());

        // Then the quiet, which the edge would cut unless something crosses it.
        let tick = tokio::time::timeout(KEEPALIVE * 2, body.next())
            .await
            .expect("the blank line crosses the compressor too")
            .unwrap()
            .unwrap();
        assert!(!tick.is_empty());
        drop(tx);
    }
}
