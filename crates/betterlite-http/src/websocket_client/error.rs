use std::io::Error as IoError;

use thiserror::Error;
use tokio_tungstenite::tungstenite::error::{CapacityError, Error as TungsteniteError};

// websockets' connect failures, told apart by what a caller retries and what it reports; no text names the URL or a header
#[derive(Debug, Error)]
pub enum WebSocketClientError {
    #[error("the websocket did not open in time")]
    Timeout,
    #[error("the websocket connection failed")]
    Io(#[source] IoError),
    #[error("the websocket TLS configuration is not valid")]
    Tls,
    #[error("the websocket upgrade was answered with HTTP {status}")]
    Status { status: u16 },
    #[error("the proxy answered CONNECT with HTTP {status}")]
    ProxyStatus { status: u16 },
    #[error("the websocket message is {size} bytes, over the {max_size}-byte limit")]
    Capacity { size: usize, max_size: usize },
    #[error("the websocket URL is not usable")]
    Url,
    #[error("the websocket did not answer its keepalive ping")]
    KeepaliveTimeout,
    #[error("the websocket protocol failed")]
    Protocol,
}

impl WebSocketClientError {
    pub(crate) fn from_tungstenite(error: TungsteniteError) -> Self {
        match error {
            TungsteniteError::Io(source) => Self::Io(source),
            TungsteniteError::Tls(_) => Self::Tls,
            TungsteniteError::Capacity(CapacityError::MessageTooLong { size, max_size }) => {
                Self::Capacity { size, max_size }
            }
            TungsteniteError::Capacity(CapacityError::TooManyHeaders) => Self::Protocol,
            TungsteniteError::Url(_) => Self::Url,
            TungsteniteError::Http(response) => Self::Status {
                status: response.status().as_u16(),
            },
            TungsteniteError::ConnectionClosed
            | TungsteniteError::AlreadyClosed
            | TungsteniteError::Protocol(_)
            | TungsteniteError::WriteBufferFull(_)
            | TungsteniteError::Utf8(_)
            | TungsteniteError::AttackAttempt
            | TungsteniteError::HttpFormat(_) => Self::Protocol,
        }
    }
}
