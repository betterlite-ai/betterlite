use std::time::TryFromFloatSecsError;

use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

use super::init::QualifireInitError;
use super::wire::BlockDetail;
use crate::guardrail_base::error::GuardrailBaseError;
use crate::guardrail_selection::error::GuardrailSelectionError;

#[derive(Debug, Error)]
pub enum QualifireError {
    #[error("the Qualifire API request timed out")]
    Timeout { source: reqwest::Error },
    #[error("the Qualifire API could not be reached")]
    Request { source: reqwest::Error },
    #[error("the Qualifire API key is not a valid header value")]
    InvalidHeader { source: InvalidHeaderValue },
    #[error("the Qualifire API returned status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the Qualifire API response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Qualifire API response is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("the Qualifire guardrail could not be written as JSON")]
    Encode { source: serde_json::Error },
    #[error("the Qualifire guardrail timeout is not a representable duration")]
    InvalidTimeout { source: TryFromFloatSecsError },
    #[error(transparent)]
    EventHook {
        #[from]
        source: GuardrailBaseError,
    },
    #[error(transparent)]
    DynamicParams {
        #[from]
        source: GuardrailSelectionError,
    },
    #[error(transparent)]
    Init {
        #[from]
        source: QualifireInitError,
    },
    #[error("the Qualifire guardrail cannot read {field} in the request messages")]
    InvalidField { field: &'static str },
    #[error("the Qualifire guardrail request option {field} has an invalid value")]
    InvalidDynamicParam { field: &'static str },
    #[error(
        "Qualifire guardrail does not support on_flagged={}; only 'block' and 'monitor' are supported.",
        quoted(.0)
    )]
    UnsupportedOnFlagged(String),
    #[error("Violated guardrail policy")]
    Blocked { detail: BlockDetail },
}

impl QualifireError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Blocked { .. } => StatusCode::BAD_REQUEST,
            Self::Timeout { .. }
            | Self::Request { .. }
            | Self::InvalidHeader { .. }
            | Self::HttpStatus { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. }
            | Self::Encode { .. }
            | Self::InvalidTimeout { .. }
            | Self::EventHook { .. }
            | Self::DynamicParams { .. }
            | Self::Init { .. }
            | Self::InvalidField { .. }
            | Self::InvalidDynamicParam { .. }
            | Self::UnsupportedOnFlagged(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

fn quoted(value: &str) -> String {
    let mut text = String::with_capacity(value.len() + 2);
    text.push('\'');
    for character in value.chars() {
        match character {
            '\\' => text.push_str("\\\\"),
            '\'' => text.push_str("\\'"),
            '\n' => text.push_str("\\n"),
            '\r' => text.push_str("\\r"),
            '\t' => text.push_str("\\t"),
            other => text.push(other),
        }
    }
    text.push('\'');
    text
}
