use betterlite_http::tls::TlsError;
use betterlite_http::websocket_client::error::WebSocketClientError;

use crate::api::realtime_backend_transport::error::DialError;

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

// a TLS config that will not build is an ssl error, which LiteLLM's retry loop does not catch
impl From<TlsError> for OpenFailure {
    fn from(error: TlsError) -> Self {
        Self {
            class: Class::OSError,
            status: None,
            error: DialError::Other {
                message: error.to_string(),
            },
        }
    }
}

// every WebSocketClientError a connect answers, as the exception websockets raises and the DialError LiteLLM's retry reads
pub(super) fn connect_failure(error: WebSocketClientError) -> OpenFailure {
    let (class, status) = class_and_status(&error);
    OpenFailure {
        class,
        status,
        error: dial_error(error),
    }
}

// the exception class websockets raises for each failure, and the status the probe reports
fn class_and_status(error: &WebSocketClientError) -> (Class, Option<u16>) {
    match error {
        WebSocketClientError::Timeout | WebSocketClientError::KeepaliveTimeout => {
            (Class::TimeoutError, None)
        }
        WebSocketClientError::Io(_) | WebSocketClientError::Tls => (Class::OSError, None),
        WebSocketClientError::Status { status } => (Class::InvalidStatus, Some(*status)),
        WebSocketClientError::ProxyStatus { .. } => (Class::InvalidProxyStatus, None),
        WebSocketClientError::Capacity { .. } | WebSocketClientError::Protocol => {
            (Class::InvalidUpgrade, None)
        }
        WebSocketClientError::Url => (Class::InvalidUri, None),
    }
}

// the DialError the session logs and the client sees for each failed connect
fn dial_error(error: WebSocketClientError) -> DialError {
    match error {
        WebSocketClientError::Timeout => DialError::Timeout {
            message: "timed out during opening handshake".to_owned(),
        },
        WebSocketClientError::KeepaliveTimeout => DialError::Timeout {
            message: "the websocket keepalive timed out".to_owned(),
        },
        WebSocketClientError::Io(source) => DialError::Os {
            message: source.to_string(),
        },
        WebSocketClientError::Tls => DialError::Os {
            message: "the websocket TLS configuration is not valid".to_owned(),
        },
        WebSocketClientError::Status { status } => DialError::InvalidStatus {
            status_code: status,
            message: format!("server rejected WebSocket connection: HTTP {status}"),
        },
        WebSocketClientError::ProxyStatus { status } => DialError::WebSocket {
            message: format!("proxy rejected connection: HTTP {status}"),
        },
        WebSocketClientError::Capacity { .. } => DialError::WebSocket {
            message: "the websocket upgrade answered with an oversized message".to_owned(),
        },
        WebSocketClientError::Url => DialError::WebSocket {
            message: "the websocket URL is not usable".to_owned(),
        },
        WebSocketClientError::Protocol => DialError::WebSocket {
            message: "the websocket protocol failed".to_owned(),
        },
    }
}
