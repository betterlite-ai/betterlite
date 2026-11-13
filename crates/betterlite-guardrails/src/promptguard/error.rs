use std::time::TryFromFloatSecsError;

use betterlite_env::EnvError;
use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

// LiteLLM raises every one of these as GuardrailRaisedException with status 400.
#[derive(Debug, Error)]
pub enum PromptGuardError {
    #[error(
        "PromptGuard API key is required. Set PROMPTGUARD_API_KEY in the environment or pass api_key in the guardrail config."
    )]
    MissingCredentials,
    #[error(transparent)]
    Environment {
        #[from]
        source: EnvError,
    },
    #[error("the PromptGuard timeout is not a representable duration")]
    UnrepresentableTimeout {
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("PromptGuard API unreachable (block_on_error=True): {source}")]
    Request {
        #[source]
        source: reqwest::Error,
    },
    #[error(
        "PromptGuard API unreachable (block_on_error=True): the API answered with status {status}"
    )]
    HttpStatus { status: StatusCode },
    #[error(
        "PromptGuard API unreachable (block_on_error=True): the answer is larger than {limit} bytes"
    )]
    ResponseTooLarge { limit: usize },
    #[error("PromptGuard API unreachable (block_on_error=True): the answer is not valid JSON")]
    ResponseJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("PromptGuard API unreachable (block_on_error=True): the API key is not a valid header")]
    InvalidHeader {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error(
        "Blocked by PromptGuard: {threat_type} (confidence={confidence:?}, event_id={event_id})"
    )]
    Blocked {
        threat_type: String,
        event_id: String,
        confidence: f64,
    },
}

impl PromptGuardError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::MissingCredentials
            | Self::Environment { .. }
            | Self::UnrepresentableTimeout { .. }
            | Self::Request { .. }
            | Self::HttpStatus { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. }
            | Self::InvalidHeader { .. }
            | Self::Blocked { .. } => 400,
        }
    }

    #[must_use]
    pub const fn blocked_content(&self) -> bool {
        match self {
            Self::Blocked { .. } => true,
            Self::MissingCredentials
            | Self::Environment { .. }
            | Self::UnrepresentableTimeout { .. }
            | Self::Request { .. }
            | Self::HttpStatus { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. }
            | Self::InvalidHeader { .. } => false,
        }
    }
}
