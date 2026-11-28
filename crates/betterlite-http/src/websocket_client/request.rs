use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyServer;
use http::{HeaderName, HeaderValue};
use rustls::ClientConfig;
use url::Url;

// websockets' ping_interval and ping_timeout: a ping every interval, a missing pong fails the socket after timeout
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keepalive {
    pub interval: Duration,
    pub timeout: Duration,
}

// websockets.connect's arguments this client needs: url, ordered headers, tls, message cap, open deadline, keepalive, proxy
#[derive(Debug, Clone)]
pub struct WebSocketRequest<'a> {
    pub url: Url,
    pub headers: &'a [(HeaderName, HeaderValue)],
    pub tls: Option<Arc<ClientConfig>>,
    pub max_message_size: usize,
    pub open_timeout: Duration,
    pub keepalive: Option<Keepalive>,
    pub proxy: Option<&'a ProxyServer>,
}
