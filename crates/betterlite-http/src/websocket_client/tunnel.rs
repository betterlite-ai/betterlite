use std::io::Error as IoError;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::proxy::ProxyServer;
use http::header::{HOST, PROXY_AUTHORIZATION};
use http::{HeaderValue, Method, Request};
use http_body_util::Empty;
use hyper::body::Bytes;
use hyper::client::conn::http1;
use hyper::upgrade::{self, Upgraded};
use hyper_util::rt::TokioIo;
use secrecy::ExposeSecret as _;
use tokio::net::TcpStream;
use url::Url;

use super::error::WebSocketClientError;
use super::stream::{BoxedStream, connect_tcp};

// websockets' proxy connect: one CONNECT over the proxy socket, then TLS and the upgrade run over the tunneled stream
pub(crate) async fn open_tunnel(
    url: &Url,
    proxy: &ProxyServer,
) -> Result<BoxedStream, WebSocketClientError> {
    let stream = connect_tcp(&proxy.host, proxy.port).await?;
    let request = connect_request(url, proxy)?;
    let upgraded = establish(stream, request).await?;
    Ok(Box::new(TokioIo::new(upgraded)))
}

// the CONNECT target always carries the port and the Host header drops the scheme's default, as websockets' build_host does
fn connect_request(
    url: &Url,
    proxy: &ProxyServer,
) -> Result<Request<Empty<Bytes>>, WebSocketClientError> {
    let host = url.host_str().ok_or(WebSocketClientError::Url)?;
    let port = url
        .port_or_known_default()
        .ok_or(WebSocketClientError::Url)?;
    let default_port = if url.scheme() == "wss" { 443 } else { 80 };
    let host_header = if port == default_port {
        host.to_owned()
    } else {
        format!("{host}:{port}")
    };
    let mut request = Request::builder()
        .method(Method::CONNECT)
        .uri(format!("{host}:{port}"))
        .header(HOST, host_header);
    if let Some(credentials) = &proxy.credentials {
        let plain = format!(
            "{}:{}",
            credentials.username,
            credentials.password.expose_secret()
        );
        let mut value = HeaderValue::try_from(format!("Basic {}", STANDARD.encode(plain)))
            .map_err(|_error| WebSocketClientError::Url)?;
        value.set_sensitive(true);
        request = request.header(PROXY_AUTHORIZATION, value);
    }
    request
        .body(Empty::new())
        .map_err(|_error| WebSocketClientError::Url)
}

// hyper's HTTP/1 client reads the CONNECT answer, so the status is exact and no CONNECT parser is written
async fn establish(
    stream: TokioIo<TcpStream>,
    request: Request<Empty<Bytes>>,
) -> Result<Upgraded, WebSocketClientError> {
    let (mut sender, connection) = http1::Builder::new()
        .handshake(stream)
        .await
        .map_err(|error| WebSocketClientError::Io(IoError::other(error)))?;
    let mut upgrading = connection.with_upgrades();
    tokio::pin!(upgrading);
    let mut finished = false;
    let response = {
        let mut pending = sender.send_request(request);
        tokio::pin!(pending);
        loop {
            tokio::select! {
                biased;
                response = &mut pending => break response.map_err(|error| WebSocketClientError::Io(IoError::other(error)))?,
                result = &mut upgrading, if !finished => {
                    finished = true;
                    result.map_err(|error| WebSocketClientError::Io(IoError::other(error)))?;
                }
            }
        }
    };
    if !response.status().is_success() {
        return Err(WebSocketClientError::ProxyStatus {
            status: response.status().as_u16(),
        });
    }
    drop(sender);
    if !finished {
        upgrading
            .as_mut()
            .await
            .map_err(|error| WebSocketClientError::Io(IoError::other(error)))?;
    }
    upgrade::on(response)
        .await
        .map_err(|error| WebSocketClientError::Io(IoError::other(error)))
}
