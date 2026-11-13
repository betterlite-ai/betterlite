use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

use crate::guardrail_records::error::GuardrailRecordError;

pub const BLOCKED_MESSAGE: &str = "Violated CrowdStrike AIDR guardrail policy";
pub const MALFORMED_TRANSFORMED_MESSAGE: &str = "CrowdStrike AIDR returned a transformed response betterlite could not parse; failing closed instead of dropping the delivered redactions";

#[derive(Debug, Error)]
pub enum CrowdStrikeError {
    #[error("the {name} environment variable is not set")]
    MissingEnvironment { name: &'static str },
    #[error("the CrowdStrike AIDR API request timed out")]
    Timeout { source: reqwest::Error },
    #[error("the CrowdStrike AIDR API could not be reached")]
    Request { source: reqwest::Error },
    #[error("the CrowdStrike AIDR API key is not a valid header value")]
    InvalidHeader { source: InvalidHeaderValue },
    #[error("the CrowdStrike AIDR API returned status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the CrowdStrike AIDR API response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the CrowdStrike AIDR API response is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("the content could not be written as JSON")]
    JsonEncode { source: serde_json::Error },
    #[error("{MALFORMED_TRANSFORMED_MESSAGE}")]
    MalformedTransformed { source: serde_json::Error },
    #[error("{BLOCKED_MESSAGE}")]
    Blocked { guardrail_name: Option<String> },
    #[error("the {name} parameter has an invalid value")]
    InvalidParam { name: &'static str },
    #[error("the guardrail record could not be written")]
    Record {
        #[from]
        source: GuardrailRecordError,
    },
}

impl CrowdStrikeError {
    // LiteLLM raises HTTPException(400) for a block; every other failure becomes an unhandled 500
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Blocked { .. } => StatusCode::BAD_REQUEST,
            Self::MissingEnvironment { .. }
            | Self::Timeout { .. }
            | Self::Request { .. }
            | Self::InvalidHeader { .. }
            | Self::HttpStatus { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. }
            | Self::JsonEncode { .. }
            | Self::MalformedTransformed { .. }
            | Self::InvalidParam { .. }
            | Self::Record { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    // LiteLLM's HTTPException detail for a block, as the two keys it writes
    #[must_use]
    pub fn detail(&self) -> IndexMap<String, GuardrailValue> {
        let (message, name) = match self {
            Self::Blocked { guardrail_name } => (BLOCKED_MESSAGE, guardrail_name.clone()),
            _ => (MALFORMED_TRANSFORMED_MESSAGE, None),
        };
        let mut detail = IndexMap::new();
        detail.insert("error".to_owned(), GuardrailValue::Text(message.to_owned()));
        detail.insert(
            "guardrail_name".to_owned(),
            name.map_or(GuardrailValue::Null, GuardrailValue::Text),
        );
        detail
    }
}

// The error object LiteLLM's standard logging guardrail record keeps for a failed call
#[must_use]
pub fn error_response(error: &CrowdStrikeError) -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([(
        "error".to_owned(),
        GuardrailValue::Text(error.to_string()),
    )]))
}
