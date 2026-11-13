use betterlite_config::collector::CollectorAddress;
use std::future::{Future, poll_fn};
use std::io;
use std::pin::Pin;
use std::task::Poll;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpStream, UnixStream};
use tokio_util::either::Either;

pub(super) trait Connector: Send + Sync + 'static {
    type Stream: AsyncRead + AsyncWrite + Unpin + Send;
    fn open(&self) -> impl Future<Output = io::Result<Self::Stream>> + Send;
}

pub(super) struct Network {
    pub(super) address: CollectorAddress,
}

impl Connector for Network {
    type Stream = Either<UnixStream, TcpStream>;
    async fn open(&self) -> io::Result<Self::Stream> {
        match &self.address {
            CollectorAddress::Unix { path } => UnixStream::connect(path).await.map(Either::Left),
            CollectorAddress::Tcp { host, port } => TcpStream::connect((host.as_str(), *port))
                .await
                .map(Either::Right),
        }
    }
}

pub(super) async fn peer_closed<Stream>(stream: &mut Stream) -> bool
where
    Stream: AsyncRead + Unpin,
{
    let mut bytes = [0];
    let mut buffer = ReadBuf::new(&mut bytes);
    poll_fn(|context| {
        Poll::Ready(
            match Pin::new(&mut *stream).poll_read(context, &mut buffer) {
                Poll::Pending => false,
                Poll::Ready(Ok(())) => buffer.filled().is_empty(),
                Poll::Ready(Err(_)) => true,
            },
        )
    })
    .await
}
