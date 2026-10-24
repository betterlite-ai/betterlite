use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};

pub(crate) const MAX_CONNECTIONS: usize = 1024;

pub(crate) struct BoundedListener {
    listener: TcpListener,
    permits: Arc<Semaphore>,
}

impl BoundedListener {
    pub(crate) fn new(listener: TcpListener) -> Self {
        let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        Self { listener, permits }
    }

    pub(crate) async fn accept(&self) -> io::Result<(BoundedStream, SocketAddr)> {
        loop {
            let (stream, address) = self.listener.accept().await?;
            match Arc::clone(&self.permits).try_acquire_owned() {
                Ok(permit) => {
                    return Ok((
                        BoundedStream {
                            stream,
                            _permit: permit,
                        },
                        address,
                    ));
                }
                Err(TryAcquireError::NoPermits) => {
                    // Drop excess sockets so an idle connection cannot block shutdown.
                }
                Err(TryAcquireError::Closed) => {
                    return Err(io::Error::other("connection limit is closed"));
                }
            }
        }
    }
}

pub(crate) struct BoundedStream {
    stream: TcpStream,
    _permit: OwnedSemaphorePermit,
}

impl AsyncRead for BoundedStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for BoundedStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}
