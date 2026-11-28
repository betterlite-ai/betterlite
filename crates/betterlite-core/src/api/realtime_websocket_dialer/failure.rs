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

    pub(super) fn into_probe(self) -> ProbeDialError {
        ProbeDialError {
            class: self.class.text().to_owned(),
            message: self.error.to_string(),
            status: self.status,
        }
    }
}

// a TLS config that will not build is an ssl error, which LiteLLM's retry loop does not catch
impl From<TlsError> for OpenFailure {
    fn from(error: TlsError) -> Self {
        Self::new(
            Class::OSError,
            DialError::Other {
                message: error.to_string(),
            },
        )
    }
}

// every WebSocketClientError a connect answers, as the exception websockets raises and the DialError LiteLLM's retry reads
pub(super) fn connect_failure(error: WebSocketClientError) -> OpenFailure {
    match error {
        WebSocketClientError::Timeout => {
            timeout_failure("timed out during opening handshake".to_owned())
        }
        WebSocketClientError::Io(source) => os_failure(source.to_string()),
        WebSocketClientError::Tls => {
            os_failure("the websocket TLS configuration is not valid".to_owned())
        }
        WebSocketClientError::Status { status } => status_failure(status),
        WebSocketClientError::ProxyStatus { status } => web_socket_failure(
            Class::InvalidProxyStatus,
            format!("proxy rejected connection: HTTP {status}"),
        ),
        WebSocketClientError::Capacity { .. } => web_socket_failure(
            Class::InvalidUpgrade,
            "the websocket upgrade answered with an oversized message".to_owned(),
        ),
        WebSocketClientError::Url => web_socket_failure(
            Class::InvalidUri,
            "the websocket URL is not usable".to_owned(),
        ),
        WebSocketClientError::KeepaliveTimeout => {
            timeout_failure("the websocket keepalive timed out".to_owned())
        }
        WebSocketClientError::Protocol => web_socket_failure(
            Class::InvalidUpgrade,
            "the websocket protocol failed".to_owned(),
        ),
    }
}

fn timeout_failure(message: String) -> OpenFailure {
    OpenFailure {
        class: Class::TimeoutError,
        status: None,
        error: DialError::Timeout { message },
    }
}

fn os_failure(message: String) -> OpenFailure {
    OpenFailure {
        class: Class::OSError,
        status: None,
        error: DialError::Os { message },
    }
}

fn web_socket_failure(class: Class, message: String) -> OpenFailure {
    OpenFailure {
        class,
        status: None,
        error: DialError::WebSocket { message },
    }
}

fn status_failure(status: u16) -> OpenFailure {
    let message = format!("server rejected WebSocket connection: HTTP {status}");
    let error = DialError::InvalidStatus {
        status_code: status,
        message,
    };
    OpenFailure::with_status(Class::InvalidStatus, status, error)
}
