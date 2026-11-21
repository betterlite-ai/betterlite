use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::tls::Verification;
use betterlite_openai::realtime::message_cap::oversized_message_close;
use tokio::time::timeout;

use crate::api::realtime_backend_relay::source::{RealtimeBackendSource, RelayReceiveError};
use crate::api::realtime_contracts::close::{CloseFrame, ConnectionClosed};
use crate::api::realtime_contracts::error::HeaderMap;
use crate::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage};

use super::error::DialError;

// LiteLLM's defaults for the backend handshake: eight seconds per attempt, three attempts and no extra backoff
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(8);
pub const MAX_OPEN_ATTEMPTS: usize = 3;

// The SSL argument LiteLLM hands the websockets client; a wss URL plus SSL_VERIFY=false builds a checking-free context instead of plaintext
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealtimeSsl {
    Disabled,
    Insecure,
    Shared(Verification),
}

// LiteLLM keeps the shared TLS context for a wss URL with SSL_VERIFY=false, and gets an insecure one only when verify is off
#[must_use]
pub fn realtime_ssl(url: &str, verification: &Verification) -> RealtimeSsl {
    match verification {
        Verification::Disabled if url.starts_with("wss://") => RealtimeSsl::Insecure,
        Verification::Disabled => RealtimeSsl::Disabled,
        Verification::Enabled | Verification::CaFile(_) => {
            RealtimeSsl::Shared(verification.clone())
        }
    }
}

// The exact arguments `_open_realtime_backend_ws` passes to websockets.connect, kept together so a dialer can record them
pub struct DialRequest<'request> {
    pub url: &'request str,
    pub headers: &'request HeaderMap,
    pub ssl: &'request RealtimeSsl,
    pub max_size: usize,
    pub open_timeout: Duration,
}

// A resolved backend socket: the sending half the session owns, and the receiving half the relay reads when the dialer could split them
pub struct ConnectedBackend {
    pub sender: Box<dyn RealtimeBackend>,
    pub source: Option<Box<dyn RealtimeBackendSource>>,
}

// The websockets client behind the retry helper, injected so a hung or rejecting handshake is a deterministic test input
#[async_trait]
pub trait RealtimeWebSocketDialer: Send + Sync {
    async fn connect(&self, request: &DialRequest<'_>) -> Result<ConnectedBackend, DialError>;
}

// LiteLLM's `_open_realtime_backend_ws`: a fresh attempt after each hung handshake, the last captured exception once every attempt fails
pub async fn open_realtime_backend_ws<Dialer>(
    dialer: &Dialer,
    request: &DialRequest<'_>,
) -> Result<ConnectedBackend, DialError>
where
    Dialer: RealtimeWebSocketDialer + ?Sized,
{
    let mut last_error: Option<DialError> = None;
    for _attempt in 0..MAX_OPEN_ATTEMPTS {
        match timeout(OPEN_TIMEOUT, dialer.connect(request)).await {
            Ok(Ok(backend)) => return Ok(backend),
            Ok(Err(error)) if !error.is_retryable() => return Err(error),
            Ok(Err(error)) => last_error = Some(error),
            Err(_elapsed) => {
                last_error = Some(DialError::Timeout {
                    message: String::new(),
                });
            }
        }
    }
    Err(last_error.unwrap_or_else(|| DialError::Other {
        message: "the realtime backend websocket could not be opened".to_owned(),
    }))
}

// The bounded-input policy for frames the dialer let through: one over the configured cap closes the session with 1009
pub struct SizeLimitedSource {
    pub inner: Box<dyn RealtimeBackendSource>,
    pub max_message_size_bytes: usize,
}

#[async_trait]
impl RealtimeBackendSource for SizeLimitedSource {
    async fn recv(&mut self, decode: Option<bool>) -> Result<RealtimeMessage, RelayReceiveError> {
        let message = self.inner.recv(decode).await?;
        let size = match &message {
            RealtimeMessage::Text(text) => text.len(),
            RealtimeMessage::Binary(bytes) => bytes.len(),
        };
        let Some(close) = oversized_message_close(size, self.max_message_size_bytes) else {
            return Ok(message);
        };
        Err(RelayReceiveError::Closed(ConnectionClosed {
            received: Some(CloseFrame {
                code: close.code,
                reason: close.reason,
            }),
            message: format!(
                "the realtime backend message exceeds the {}-byte limit",
                self.max_message_size_bytes
            ),
        }))
    }
}
