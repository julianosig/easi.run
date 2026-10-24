//! A stream whose writes fail once one makes no progress for a while: a client that stops reading cannot keep what is
//! written to it, or the connection itself, for as long as its end stays open. Only a write that waits counts, so a
//! connection that writes nothing (an idle viewer stream, a request waiting for a long agent call) never fails by it.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Sleep;

/// How long a write may wait without any of it taken, on the HTTPS server and the local MCP socket.
pub const WRITE: Duration = Duration::from_secs(30);

pub struct Deadline<S> {
    stream: S,
    limit: Duration,
    /// Since when writing waits: set by the first write that cannot go on, cleared by any that does.
    stalled: Option<Pin<Box<Sleep>>>,
}

impl<S> Deadline<S> {
    pub fn new(stream: S, limit: Duration) -> Deadline<S> {
        Deadline { stream, limit, stalled: None }
    }

    /// A write's outcome, or its failure once writing has waited past the limit.
    fn timed<T>(&mut self, cx: &mut Context<'_>, polled: Poll<io::Result<T>>) -> Poll<io::Result<T>> {
        if polled.is_ready() {
            self.stalled = None;
            return polled;
        }
        let limit = self.limit;
        let stalled = self.stalled.get_or_insert_with(|| Box::pin(tokio::time::sleep(limit)));
        match stalled.as_mut().poll(cx) {
            Poll::Ready(()) => Poll::Ready(Err(io::Error::new(io::ErrorKind::TimedOut, "the peer stopped reading"))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Deadline<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Deadline<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let polled = Pin::new(&mut self.stream).poll_write(cx, buf);
        self.timed(cx, polled)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let polled = Pin::new(&mut self.stream).poll_write_vectored(cx, bufs);
        self.timed(cx, polled)
    }

    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let polled = Pin::new(&mut self.stream).poll_flush(cx);
        self.timed(cx, polled)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let polled = Pin::new(&mut self.stream).poll_shutdown(cx);
        self.timed(cx, polled)
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_write_fails_once_it_waited_past_the_limit_and_one_that_goes_on_never_does() {
        let (ours, mut theirs) = tokio::io::duplex(1024);
        let mut ours = Deadline::new(ours, WRITE);
        // A peer that reads a little every 20 seconds: each write waits, and goes on within the limit.
        let reader = tokio::spawn(async move {
            let mut piece = vec![0u8; 1024];
            for _ in 0..4 {
                tokio::time::sleep(Duration::from_secs(20)).await;
                theirs.read_exact(&mut piece).await.unwrap();
            }
            theirs
        });
        ours.write_all(&[1u8; 5 * 1024]).await.expect("each piece was taken within the limit");
        let _theirs = reader.await.unwrap();
        // Then nothing more is read.
        let started = tokio::time::Instant::now();
        let failed = ours.write_all(&[1u8; 2048]).await.unwrap_err();
        assert_eq!(failed.kind(), io::ErrorKind::TimedOut);
        assert_eq!(started.elapsed(), WRITE);
    }

    #[tokio::test(start_paused = true)]
    async fn a_stream_that_writes_nothing_never_fails() {
        let (ours, mut theirs) = tokio::io::duplex(1024);
        let mut ours = Deadline::new(ours, WRITE);
        tokio::time::sleep(WRITE * 10).await;
        ours.write_all(b"late").await.unwrap();
        let mut late = [0u8; 4];
        theirs.read_exact(&mut late).await.unwrap();
        assert_eq!(&late, b"late");
    }
}
