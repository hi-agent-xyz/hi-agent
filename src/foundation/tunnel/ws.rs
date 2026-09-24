//! A WebSocket, read and written as a stream of bytes.
//!
//! The tunnel is a multiplexer over one connection, and a multiplexer wants a
//! byte stream. WebSocket gives message frames instead, so this is the adapter
//! between them: every write becomes one binary message, every binary message
//! read becomes bytes.
//!
//! Written here rather than pulled in as a crate because it is ninety lines and
//! the alternative was a dependency whose only job is this — and because the
//! failure modes (a partially-read message, a close mid-frame) are ours to get
//! right either way.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use futures::{AsyncRead, AsyncWrite, Future, Sink, SinkExt, Stream};
use tokio_tungstenite::tungstenite::Message;

/// How long the tunnel may hear nothing at all before it is read as dead.
///
/// **Silence is how a dead tunnel looks from this end.** A path that dies without
/// a close — the community redeployed behind its CDN, a local proxy holding its
/// own half of the socket open — leaves this side reading forever from a
/// connection nobody is on, while the community answers "asleep" for a core that
/// is running. The community's yamux pings every 10 s (`tunnelKeepAlive`,
/// `backend/internal/server/relay.go`), and those pings are binary frames that
/// cross every hop, so a live tunnel is never quiet for this long: four missed
/// pings is a gone connection, and ending the read is what sends
/// [`super::spawn`] back to redialing.
///
/// It was 90 s against pings every 30 s, and every second of it is a second a
/// request routed into the dead tunnel hangs before anything answers — measured
/// as repeated 5 s+ hangs on a flaky path. 45 s still clears the old 30 s pings,
/// so a core that updates before its community does not redial a healthy tunnel.
pub const IDLE: Duration = Duration::from_secs(45);

/// Byte-stream view of a WebSocket.
///
/// Holds the tail of the last message read, because a reader asks for the size
/// *it* wants and a message arrives at the size the sender chose; the two never
/// have to agree.
pub struct WsByteStream<S> {
    inner: S,
    /// Bytes received and not yet handed to a reader.
    pending: Bytes,
    /// Set once the peer has closed, so later reads report end-of-stream rather
    /// than an error — a closed tunnel is an event, not a fault.
    closed: bool,
    /// When the read fails for silence; pushed back by every frame that arrives.
    /// See [`IDLE`].
    idle: Pin<Box<tokio::time::Sleep>>,
}

impl<S> WsByteStream<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            pending: Bytes::new(),
            closed: false,
            idle: Box::pin(tokio::time::sleep(IDLE)),
        }
    }
}

impl<S> AsyncRead for WsByteStream<S>
where
    S: Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            if !self.pending.is_empty() {
                let n = self.pending.len().min(buf.len());
                buf[..n].copy_from_slice(&self.pending[..n]);
                self.pending = self.pending.slice(n..);
                return Poll::Ready(Ok(n));
            }
            if self.closed {
                return Poll::Ready(Ok(0));
            }
            let frame = match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Pending => {
                    if self.idle.as_mut().poll(cx).is_ready() {
                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!("nothing heard on the tunnel for {}s", IDLE.as_secs()),
                        )));
                    }
                    return Poll::Pending;
                }
                Poll::Ready(frame) => frame,
            };
            // Any frame at all — a ping included — is the far end still there.
            let deadline = tokio::time::Instant::now() + IDLE;
            self.idle.as_mut().reset(deadline);
            match frame {
                None => {
                    self.closed = true;
                    return Poll::Ready(Ok(0));
                }
                Some(Ok(Message::Binary(b))) => self.pending = Bytes::from(b),
                Some(Ok(Message::Close(_))) => {
                    self.closed = true;
                    return Poll::Ready(Ok(0));
                }
                // Ping/pong are the transport keeping itself alive and carry no
                // tunnel bytes; text has no meaning here and is ignored rather
                // than treated as a fault, so a chatty proxy cannot kill a
                // working tunnel.
                Some(Ok(_)) => continue,
                Some(Err(e)) => {
                    return Poll::Ready(Err(io::Error::other(e)));
                }
            }
        }
    }
}

impl<S> AsyncWrite for WsByteStream<S>
where
    S: Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match Pin::new(&mut self.inner).poll_ready(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(io::Error::other(e))),
            Poll::Ready(Ok(())) => {}
        }
        match Pin::new(&mut self.inner).start_send(Message::Binary(buf.to_vec().into())) {
            Ok(()) => Poll::Ready(Ok(buf.len())),
            Err(e) => Poll::Ready(Err(io::Error::other(e))),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx).map_err(io::Error::other)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_close(cx).map_err(io::Error::other)
    }
}

/// Send a close frame, best-effort. A tunnel that is going away says so, which
/// is what lets the community mark the handle asleep instead of waiting for a
/// timeout.
pub async fn say_goodbye<S>(inner: &mut S)
where
    S: Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let _ = inner.send(Message::Close(None)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{AsyncReadExt, StreamExt};

    /// A message arrives at the size the sender chose; a reader asks for the
    /// size it wants. The tail has to survive between the two.
    #[tokio::test]
    async fn a_big_message_is_read_in_small_bites() {
        let msgs = futures::stream::iter(vec![
            Ok(Message::Binary(vec![1, 2, 3, 4, 5].into())),
            Ok(Message::Binary(vec![6, 7].into())),
        ]);
        let mut s = WsByteStream::new(msgs);

        let mut buf = [0u8; 2];
        assert_eq!(s.read(&mut buf).await.unwrap(), 2);
        assert_eq!(buf, [1, 2]);
        assert_eq!(s.read(&mut buf).await.unwrap(), 2);
        assert_eq!(buf, [3, 4]);
        assert_eq!(s.read(&mut buf).await.unwrap(), 1);
        assert_eq!(buf[0], 5);
        assert_eq!(s.read(&mut buf).await.unwrap(), 2);
        assert_eq!(buf, [6, 7]);
        assert_eq!(s.read(&mut buf).await.unwrap(), 0, "then end of stream");
    }

    /// A ping is the transport keeping itself alive. Treating it as data — or as
    /// an error — would break a tunnel that is working.
    #[tokio::test]
    async fn keepalives_are_not_tunnel_bytes() {
        let msgs = futures::stream::iter(vec![
            Ok(Message::Ping(vec![].into())),
            Ok(Message::Binary(vec![9].into())),
            Ok(Message::Pong(vec![].into())),
            Ok(Message::Close(None)),
            Ok(Message::Binary(vec![10].into())),
        ]);
        let mut s = WsByteStream::new(msgs);
        let mut buf = [0u8; 4];
        assert_eq!(s.read(&mut buf).await.unwrap(), 1);
        assert_eq!(buf[0], 9);
        assert_eq!(s.read(&mut buf).await.unwrap(), 0, "close ends it, and nothing after counts");
    }

    /// A connection that goes quiet without closing is dead, and the read has to
    /// say so — otherwise the core waits on it forever while the community
    /// answers "asleep".
    #[tokio::test(start_paused = true)]
    async fn silence_ends_the_read() {
        let msgs = futures::stream::iter(vec![Ok(Message::Binary(vec![1].into()))])
            .chain(futures::stream::pending());
        let mut s = WsByteStream::new(msgs);
        let mut buf = [0u8; 4];
        assert_eq!(s.read(&mut buf).await.unwrap(), 1);
        let err = s.read(&mut buf).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    /// A keepalive inside the window is enough: a tunnel carrying nothing but
    /// the community's pings is a live one.
    #[tokio::test(start_paused = true)]
    async fn a_ping_keeps_it_open() {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        let mut s = WsByteStream::new(rx);
        let mut buf = [0u8; 4];
        let feeder = tokio::spawn(async move {
            for _ in 0..3 {
                tokio::time::sleep(IDLE - Duration::from_secs(1)).await;
                tx.unbounded_send(Ok(Message::Ping(vec![].into()))).unwrap();
            }
            tx.unbounded_send(Ok(Message::Binary(vec![7].into()))).unwrap();
        });
        assert_eq!(s.read(&mut buf).await.unwrap(), 1, "outlived three windows of pings");
        assert_eq!(buf[0], 7);
        feeder.await.unwrap();
    }
}
