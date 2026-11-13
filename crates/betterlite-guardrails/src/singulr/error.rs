use std::str::Utf8Error;

use thiserror::Error;

// LiteLLM raises GuardrailRaisedException for every one of these, and its default status is 400
pub const STATUS_CODE: u16 = 400;

#[derive(Debug, Error)]
pub enum SingulrError {
    #[error("Singulr API returned HTTP {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("Singulr API returned HTTP {status} with a body that is not UTF-8")]
    HttpBodyEncoding {
        status: u16,
        #[source]
        source: Utf8Error,
    },
    #[error("Singulr API unreachable (block_on_error=True): {source}")]
    Transport {
        #[source]
        source: reqwest::Error,
    },
    #[error("Singulr API unreachable (block_on_error=True): the {name} header value is invalid")]
    InvalidHeader { name: &'static str },
    #[error("Singulr API returned an invalid response: {source}")]
    InvalidResponse {
        #[source]
        source: serde_json::Error,
    },
    #[error("Singulr API returned an invalid response: the answer is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("Blocked by Singulr, Blocking due to {due_to}")]
    Blocked { due_to: String },
}

impl SingulrError {
    // LiteLLM sets blocked_content only when the guardrail reached a verdict
    #[must_use]
    pub const fn blocked_content(&self) -> bool {
        matches!(self, Self::Blocked { .. })
    }

    // The body stays out of the log; LiteLLM logs the status and the exception text, not the payload
    pub fn log(&self) {
        match self {
            Self::HttpStatus { status, .. } | Self::HttpBodyEncoding { status, .. } => {
                tracing::error!(status = %status, "Singulr API returned an HTTP error status");
            }
            Self::Transport { source } => {
                tracing::error!(error = %source, "Singulr API unreachable");
            }
            Self::InvalidHeader { name } => {
                tracing::error!(header = *name, "Singulr API request header is invalid");
            }
            Self::InvalidResponse { source } => {
                tracing::error!(error = %source, "Singulr API returned an invalid response");
            }
            Self::ResponseTooLarge { limit } => {
                tracing::error!(limit = %limit, "Singulr API response is too large");
            }
            Self::Blocked { .. } => {}
        }
    }
}
