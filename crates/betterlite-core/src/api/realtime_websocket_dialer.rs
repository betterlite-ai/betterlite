mod failure;
mod halves;
mod headers;
mod tests;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::{ProxyPolicy, ProxyRoute, ProxyServer};
use betterlite_config::tls::Verification;
use betterlite_http::tls::websocket_client_config;
use betterlite_http::websocket_client::connect;
use betterlite_http::websocket_client::connection::WebSocketConnection;
use betterlite_http::websocket_client::request::{Keepalive, WebSocketRequest};
use http::header::{HeaderName, HeaderValue};
use url::Url;

use crate::api::realtime_backend_transport::dial::{
    ConnectedBackend, DialRequest, RealtimeSsl, RealtimeWebSocketDialer,
};
use crate::api::realtime_backend_transport::error::DialError;
use crate::api::realtime_connection_plan::{ProbeConnection, ProbeDialError, RealtimeProbeDialer};

use self::failure::{Class, OpenFailure, connect_failure};
use self::halves::{DialedSender, DialedSource};
use self::headers::{HeaderSource, dialer_headers};

// websockets' ping_interval and ping_timeout defaults, kept by every realtime backend socket
const DEFAULT_KEEPALIVE: Keepalive = Keepalive {
    interval: Duration::from_secs(20),
    timeout: Duration::from_secs(20),
};

// websockets' default open_timeout for the health check's single handshake
const PROBE_OPEN_TIMEOUT: Duration = Duration::from_secs(10);

// The websockets client the realtime session dials through: the proxy routes LiteLLM's environment picks, and nothing else
pub struct WebSocketDialer {
    pub proxy: Arc<ProxyPolicy>,
}

// the six steps connect and probe share: the URL and ssl pair, the proxy route, the upgrade, and its error classes
struct OpenRequest<'open> {
    url: &'open str,
    headers: &'open [(HeaderName, HeaderValue)],
    ssl: &'open RealtimeSsl,
    max_size: usize,
    open_timeout: Duration,
    keepalive: Option<Keepalive>,
}

#[async_trait]
impl RealtimeWebSocketDialer for WebSocketDialer {
    async fn connect(&self, request: &DialRequest<'_>) -> Result<ConnectedBackend, DialError> {
        let headers = dialer_headers(HeaderSource::Backend(request.headers))
            .map_err(|open_failure| open_failure.error)?;
        let opened = self
            .open(OpenRequest {
                url: request.url,
                headers: &headers,
                ssl: request.ssl,
                max_size: request.max_size,
                open_timeout: request.open_timeout,
                keepalive: Some(DEFAULT_KEEPALIVE),
            })
            .await
            .map_err(|open_failure| open_failure.error)?;
        let WebSocketConnection { sender, receiver } = opened;
        let source_sender = sender.clone();
        Ok(ConnectedBackend {
            sender: Box::new(DialedSender { sender }),
            source: Some(Box::new(DialedSource {
                receiver,
                sender: source_sender,
                max_size: request.max_size,
            })),
        })
    }
}

#[async_trait]
impl RealtimeProbeDialer for WebSocketDialer {
    async fn probe(&self, connection: &ProbeConnection) -> Result<(), ProbeDialError> {
        let headers = dialer_headers(HeaderSource::Probe(&connection.headers))
            .map_err(OpenFailure::into_probe)?;
        let opened = self
            .open(OpenRequest {
                url: &connection.url,
                headers: &headers,
                ssl: &connection.ssl,
                max_size: connection.max_size,
                open_timeout: PROBE_OPEN_TIMEOUT,
                keepalive: None,
            })
            .await
            .map_err(OpenFailure::into_probe)?;
        // `async with websockets.connect(...)` closes 1000 on the way out, after the health check answered
        drop(opened.sender.close(1000, "").await);
        Ok(())
    }
}

impl WebSocketDialer {
    // websockets.connect's checks and arguments, one attempt, with the proxy websockets' get_proxy would pick
    async fn open(&self, request: OpenRequest<'_>) -> Result<WebSocketConnection, OpenFailure> {
        let url = websocket_url(request.url)?;
        let plain = url.scheme() == "ws";
        let tls = match request.ssl {
            RealtimeSsl::Shared(_) | RealtimeSsl::Insecure if plain => {
                return Err(OpenFailure::new(
                    Class::ValueError,
                    DialError::Other {
                        message: "ssl argument is incompatible with a ws:// URI".to_owned(),
                    },
                ));
            }
            RealtimeSsl::Insecure => Some(Arc::new(
                websocket_client_config(&Verification::Disabled).map_err(OpenFailure::from)?,
            )),
            RealtimeSsl::Shared(verification) => Some(Arc::new(
                websocket_client_config(verification).map_err(OpenFailure::from)?,
            )),
            RealtimeSsl::Disabled => None,
        };
        let proxy = self.proxy_target(&url)?;
        // one over the session's cap, so SizeLimitedSource still decides the exact boundary
        let limit = request.max_size.saturating_add(1);
        connect(WebSocketRequest {
            url,
            headers: request.headers,
            tls,
            max_message_size: limit,
            open_timeout: request.open_timeout,
            keepalive: request.keepalive,
            proxy: proxy.as_deref(),
        })
        .await
        .map_err(connect_failure)
    }

    // websockets' get_proxy, narrowed to the routes this client can dial and the classes a blocked one raises
    fn proxy_target(&self, url: &Url) -> Result<Option<Arc<ProxyServer>>, OpenFailure> {
        match self.proxy.websocket_route(url) {
            None => Ok(None),
            Some(ProxyRoute::Forward(server)) => Ok(Some(Arc::clone(server))),
            Some(ProxyRoute::Unsupported { scheme }) if scheme.starts_with("socks") => {
                Err(OpenFailure::new(
                    Class::ImportError,
                    DialError::Other {
                        message: "python-socks is required to use a SOCKS proxy".to_owned(),
                    },
                ))
            }
            Some(ProxyRoute::Unsupported { scheme }) => Err(OpenFailure::new(
                Class::InvalidProxy,
                DialError::Other {
                    message: format!("websockets can't connect through a {scheme} proxy"),
                },
            )),
            Some(ProxyRoute::Invalid { name }) => Err(OpenFailure::new(
                Class::InvalidProxy,
                DialError::WebSocket {
                    message: format!("the {name} proxy isn't a valid proxy"),
                },
            )),
        }
    }
}

// websockets.connect's URI check: only a ws or wss URL reaches a socket
fn websocket_url(raw: &str) -> Result<Url, OpenFailure> {
    let url = Url::parse(raw).map_err(|error| {
        OpenFailure::new(
            Class::InvalidUri,
            DialError::WebSocket {
                message: error.to_string(),
            },
        )
    })?;
    let scheme = url.scheme();
    if scheme != "ws" && scheme != "wss" {
        return Err(OpenFailure::new(
            Class::InvalidUri,
            DialError::WebSocket {
                message: format!("scheme isn't ws or wss: '{scheme}'"),
            },
        ));
    }
    Ok(url)
}
