use std::io::Error as IoError;
use std::time::TryFromFloatSecsError;

use betterlite_http::tls::TlsError;
use betterlite_http::websocket_client::error::WebSocketClientError;
use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;
use tokio::task::JoinError;

// the failure the model stream ended with, filled by the proxy from its own typed stream error
#[derive(Debug, Error)]
pub enum ModelStreamError {
    #[error("the upstream model request failed")]
    Http {
        #[source]
        source: reqwest::Error,
    },
    #[error("the upstream model sent a chunk that could not be parsed")]
    Parse {
        #[source]
        source: serde_json::Error,
    },
    #[error("the upstream model reported an error: {message}")]
    Provider { message: Box<str> },
}

#[derive(Debug, Error)]
pub enum CatoError {
    #[error(
        "no Cato Networks API key: set CATO_API_KEY or pass api_key to the guardrail configuration"
    )]
    MissingApiKey,
    #[error("the {name} header value for Cato Networks is not valid")]
    InvalidHeader {
        name: &'static str,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the Cato Networks API request timed out")]
    Timeout {
        #[source]
        source: reqwest::Error,
    },
    #[error("the Cato Networks API could not be reached")]
    Request {
        #[source]
        source: reqwest::Error,
    },
    #[error("the Cato Networks API answered with status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the Cato Networks API answer is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Cato Networks API answer is not valid JSON")]
    ResponseJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("{message}")]
    Blocked { message: String },
    #[error(
        "the Cato Networks analyze answer does not match the inspected request, so the request cannot be rewritten without forwarding unredacted text"
    )]
    RedactionMismatch,
    #[error("the Cato Networks TLS settings are not valid")]
    Tls {
        #[source]
        source: TlsError,
    },
    #[error("the Cato Networks timeout of {seconds} seconds is not a valid duration")]
    TimeoutSetting {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("the Cato Networks API client could not be built")]
    Client {
        #[source]
        source: reqwest::Error,
    },
    #[error("the Cato Networks upstream websocket failed")]
    WebSocket {
        #[source]
        source: WebSocketClientError,
    },
    #[error("the Cato Networks upstream websocket did not complete its handshake in time")]
    ConnectTimeout,
    #[error("the Cato Networks websocket TLS configuration is not valid")]
    WsTls {
        #[source]
        source: rustls::Error,
    },
    #[error("the Cato Networks websocket CA file could not be read")]
    WsCaFile {
        #[source]
        source: IoError,
    },
    #[error("the Cato Networks upstream websocket did not send or accept a message in time")]
    StreamTimeout,
    #[error("the Cato Networks upstream websocket message is not valid JSON")]
    StreamJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the model stream chunk is not valid JSON")]
    StreamChunkJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("Cato guardrail connection closed unexpectedly")]
    StreamClosed,
    #[error("{message}")]
    StreamBlocked { message: String },
    #[error("Cato guardrail upstream stream failed")]
    StreamSenderFailed {
        #[source]
        source: Box<Self>,
    },
    #[error("the model stream failed")]
    ModelStream {
        #[source]
        source: ModelStreamError,
    },
    #[error("the Cato Networks stream sender task failed")]
    StreamTask {
        #[source]
        source: JoinError,
    },
    #[error("the Cato Networks output redaction is not text")]
    OutputRedactionNotText,
    #[error(
        "the Cato Networks output redaction does not fit the response fragment it was read from"
    )]
    OutputRedactionMismatch,
}

impl CatoError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Blocked { .. } | Self::RedactionMismatch => StatusCode::BAD_REQUEST,
            Self::Timeout { .. }
            | Self::Request { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. }
            | Self::OutputRedactionNotText
            | Self::OutputRedactionMismatch => StatusCode::BAD_GATEWAY,
            Self::HttpStatus { status } => *status,
            // everything raised inside the streaming iterator reaches the client as a 500 error frame
            Self::MissingApiKey
            | Self::InvalidHeader { .. }
            | Self::Tls { .. }
            | Self::TimeoutSetting { .. }
            | Self::Client { .. }
            | Self::WebSocket { .. }
            | Self::ConnectTimeout
            | Self::WsTls { .. }
            | Self::WsCaFile { .. }
            | Self::StreamTimeout
            | Self::StreamJson { .. }
            | Self::StreamChunkJson { .. }
            | Self::StreamClosed
            | Self::StreamBlocked { .. }
            | Self::StreamSenderFailed { .. }
            | Self::ModelStream { .. }
            | Self::StreamTask { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub const INVALID_REQUEST_ERROR: &str = "invalid_request_error";
