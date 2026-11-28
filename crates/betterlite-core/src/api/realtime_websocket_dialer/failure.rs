use betterlite_http::tls::TlsError;
use betterlite_http::websocket_client::error::WebSocketClientError;

use crate::api::realtime_backend_transport::error::DialError;
use crate::api::realtime_connection_plan::ProbeDialError;

// the websockets or Python exception class LiteLLM classifies a failed handshake by, which decides the retry and the probe's answer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Class {
    InvalidStatus,
    InvalidProxyStatus,
    InvalidProxy,
    InvalidUri,
    InvalidUpgrade,
    TimeoutError,
    OSError,
    ValueError,
    ImportError,
}

impl Class {
    pub(super) const fn text(self) -> &'static str {
        match self {
            Self::InvalidStatus => "InvalidStatus",
            Self::InvalidProxyStatus => "InvalidProxyStatus",
            Self::InvalidProxy => "InvalidProxy",
            Self::InvalidUri => "InvalidURI",
            Self::InvalidUpgrade => "InvalidUpgrade",
            Self::TimeoutError => "TimeoutError",
            Self::OSError => "OSError",
            Self::ValueError => "ValueError",
            Self::ImportError => "ImportError",
        }
    }
}

// one failure of the six dial steps: the class the probe reports and the DialError the session logs and the client sees
pub(super) struct OpenFailure {
    pub(super) class: Class,
    pub(super) status: Option<u16>,
    pub(super) error: DialError,
}

impl OpenFailure {
    pub(super) const fn new(class: Class, error: DialError) -> Self {
        Self {
            class,
            status: None,
            error,
        }
    }

    pub(super) const fn with_status(class: Class, status: u16, error: DialError) -> Self {
        Self {
            class,
            status: Some(status),
            error,
        }
    }

    // a TLS config that will not build is an ssl error, which LiteLLM's retry loop does not catch
    pub(super) fn config(error: TlsError) -> Self {
        Self::new(
            Class::OSError,
            DialError::Other {
                message: error.to_string(),
            },
        )
    }

    pub(super) fn into_probe(self) -> ProbeDialError {
        ProbeDialError {
            class: self.class.text().to_owned(),
            message: self.error.to_string(),
            status: self.status,
        }
    }
}

// every WebSocketClientError a connect answers, as the exception websockets raises and the DialError LiteLLM's retry reads
pub(super) fn connect_failure(error: WebSocketClientError) -> OpenFailure {
    match error {
        WebSocketClientError::Timeout => OpenFailure::new(
            Class::TimeoutError,
            DialError::Timeout {
                message: "timed out during opening handshake".to_owned(),
            },
        ),
        WebSocketClientError::Io(source) => OpenFailure::new(
            Class::OSError,
            DialError::Os {
                message: source.to_string(),
            },
        ),
        WebSocketClientError::Tls => OpenFailure::new(
            Class::OSError,
            DialError::Os {
                message: "the websocket TLS configuration is not valid".to_owned(),
            },
        ),
        WebSocketClientError::Status { status } => OpenFailure::with_status(
            Class::InvalidStatus,
            status,
            DialError::InvalidStatus {
                status_code: status,
                message: format!("server rejected WebSocket connection: HTTP {status}"),
            },
        ),
        WebSocketClientError::ProxyStatus { status } => OpenFailure::new(
            Class::InvalidProxyStatus,
            DialError::WebSocket {
                message: format!("proxy rejected connection: HTTP {status}"),
            },
        ),
        WebSocketClientError::Capacity { .. } => OpenFailure::new(
            Class::InvalidUpgrade,
            DialError::WebSocket {
                message: "the websocket upgrade answered with an oversized message".to_owned(),
            },
        ),
        WebSocketClientError::Url => OpenFailure::new(
            Class::InvalidUri,
            DialError::WebSocket {
                message: "the websocket URL is not usable".to_owned(),
            },
        ),
        WebSocketClientError::KeepaliveTimeout => OpenFailure::new(
            Class::TimeoutError,
            DialError::Timeout {
                message: "the websocket keepalive timed out".to_owned(),
            },
        ),
        WebSocketClientError::Protocol => OpenFailure::new(
            Class::InvalidUpgrade,
            DialError::WebSocket {
                message: "the websocket protocol failed".to_owned(),
            },
        ),
    }
}
