pub mod connection;
pub mod error;
pub mod request;
mod stream;
mod tests;
mod tunnel;

use url::Url;

use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{Connector, client_async_tls_with_config};

use self::connection::WebSocketConnection;
use self::error::WebSocketClientError;
use self::request::WebSocketRequest;
use self::stream::{BoxedStream, connect_tcp};

// one socket a websockets 15.0.1 connect would open: no retry, one deadline over TCP, the tunnel, TLS and the upgrade
pub async fn connect(
    request: WebSocketRequest<'_>,
) -> Result<WebSocketConnection, WebSocketClientError> {
    timeout(request.open_timeout, open(&request))
        .await
        .map_err(|_elapsed| WebSocketClientError::Timeout)?
}

async fn open(request: &WebSocketRequest<'_>) -> Result<WebSocketConnection, WebSocketClientError> {
    let stream = match request.proxy {
        Some(proxy) => tunnel::open_tunnel(&request.url, proxy).await?,
        None => direct_stream(&request.url).await?,
    };
    let handshake = handshake_request(request)?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(request.max_message_size))
        .max_frame_size(Some(request.max_message_size));
    let connector = request.tls.clone().map(Connector::Rustls);
    let (socket, _response) =
        client_async_tls_with_config(handshake, stream, Some(config), connector)
            .await
            .map_err(WebSocketClientError::from_tungstenite)?;
    Ok(WebSocketConnection::new(socket, request.keepalive))
}

// a ws:// socket is plain TCP, so the TLS connector is only reached for a wss:// URL
async fn direct_stream(url: &Url) -> Result<BoxedStream, WebSocketClientError> {
    let host = url.host_str().ok_or(WebSocketClientError::Url)?;
    let port = url
        .port_or_known_default()
        .ok_or(WebSocketClientError::Url)?;
    let stream = connect_tcp(host, port).await?;
    Ok(Box::new(stream.into_inner()))
}

// tungstenite writes its own five headers first, then ours in the order they were appended
fn handshake_request(request: &WebSocketRequest<'_>) -> Result<Request, WebSocketClientError> {
    let mut handshake = request
        .url
        .as_str()
        .into_client_request()
        .map_err(|_error| WebSocketClientError::Url)?;
    let headers = handshake.headers_mut();
    for (name, value) in request.headers {
        headers.append(name.clone(), value.clone());
    }
    Ok(handshake)
}
