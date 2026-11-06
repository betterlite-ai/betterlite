use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

use super::vigil_guard_transient::TransportError;

#[derive(Debug, Error)]
pub enum VigilGuardError {
    #[error("Vigil Guard {field} is required; set it in the guardrail config")]
    MissingConfig { field: &'static str },
    #[error("the Vigil Guard request header {name} is not a valid header value")]
    InvalidHeader {
        name: &'static str,
        source: InvalidHeaderValue,
    },
    #[error("the Vigil Guard request failed")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the Vigil Guard response is not valid JSON")]
    ResponseJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Vigil Guard tool call at index {index} of {len} does not exist")]
    ToolCallIndex { index: usize, len: usize },
    #[error("the Vigil Guard tool call at index {index} has no function object")]
    InvalidToolFunction { index: usize },
    #[error("Vigil Guard blocked the request: {reason}")]
    Blocked { reason: String },
}

impl VigilGuardError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Blocked { .. } => StatusCode::BAD_REQUEST,
            Self::MissingConfig { .. }
            | Self::InvalidHeader { .. }
            | Self::Transport { .. }
            | Self::ResponseJson { .. }
            | Self::ToolCallIndex { .. }
            | Self::InvalidToolFunction { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
