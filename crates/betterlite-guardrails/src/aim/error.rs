use std::string::FromUtf8Error;

use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;
use tokio_tungstenite::tungstenite;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionCode {
    ContentPolicyViolation,
    BadRequest,
}

impl RejectionCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ContentPolicyViolation => "content_policy_violation",
            Self::BadRequest => "400",
        }
    }
}

#[derive(Debug, Error)]
pub enum AimError {
    #[error("the Aim guardrail has no API key: set AIM_API_KEY or configure api_key")]
    MissingSecrets,
    #[error("an Aim header is not a valid header value")]
    InvalidHeader {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the Aim request timed out")]
    Timeout {
        #[source]
        source: reqwest::Error,
    },
    #[error("the Aim request failed")]
    Request {
        #[source]
        source: reqwest::Error,
    },
    #[error("the Aim API returned status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the Aim response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Aim response is not valid JSON")]
    ResponseJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Aim request body could not be serialized")]
    RequestJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("{message}")]
    Rejected {
        message: String,
        code: RejectionCode,
    },
    #[error("the Aim websocket request is invalid")]
    WebSocketRequest {
        #[source]
        source: tungstenite::Error,
    },
    #[error("the Aim websocket connection failed")]
    Connect {
        #[source]
        source: tungstenite::Error,
    },
    #[error("the Aim websocket did not open in time")]
    ConnectTimeout,
    #[error("the Aim websocket failed")]
    WebSocket {
        #[source]
        source: tungstenite::Error,
    },
    #[error("an Aim stream frame is not UTF-8")]
    FrameUtf8 {
        #[source]
        source: FromUtf8Error,
    },
    #[error("an Aim stream frame is not valid JSON")]
    StreamJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Aim stream closed before it finished")]
    StreamClosed,
    #[error("the Aim stream waited longer than its timeout for a frame")]
    StreamTimeout,
    #[error("{message}")]
    StreamBlocked { message: String },
}
