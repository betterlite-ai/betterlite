use std::io::Error as IoError;

use http::Uri;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tower_service::Service as _;

use super::error::WebSocketClientError;

// one stream type for the plain socket, the proxy tunnel and the TLS layer tokio-tungstenite adds
pub(crate) trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> AsyncStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

pub(crate) type BoxedStream = Box<dyn AsyncStream>;

// the non-blocking connect and DNS lookup for a direct socket and for the proxy the tunnel runs over
pub(crate) async fn connect_tcp(
    host: &str,
    port: u16,
) -> Result<TokioIo<TcpStream>, WebSocketClientError> {
    let target = Uri::try_from(format!("http://{host}:{port}/"))
        .map_err(|_error| WebSocketClientError::Url)?;
    let mut connector = HttpConnector::new();
    std::future::poll_fn(|context| connector.poll_ready(context))
        .await
        .map_err(|error| WebSocketClientError::Io(IoError::other(error)))?;
    connector
        .call(target)
        .await
        .map_err(|error| WebSocketClientError::Io(IoError::other(error)))
}
