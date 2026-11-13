use std::time::TryFromFloatSecsError;

use thiserror::Error;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_records::error::GuardrailRecordError;

use super::advisory::format::FormatError;

#[derive(Debug, Error)]
pub enum LakeraV2Error {
    #[error(
        "Invalid advisory_system_message template: must include a real {{reason}} placeholder (not an escaped {{{{reason}}}}) so the LLM sees why the request was flagged."
    )]
    AdvisoryPlaceholder,
    #[error(
        "Invalid advisory_system_message template: {source}. The template must be a valid str.format() string using only the {{reason}} placeholder."
    )]
    AdvisoryFormat {
        #[source]
        source: FormatError,
    },
    #[error(
        "on_flagged='inject_system_message' requires payload=True and breakdown=True: advisory mode masks any detected PII before appending the advisory note, and that masking can only happen when Lakera's response carries both the violation breakdown and the payload location data. Without them, PII would be forwarded to the model unredacted."
    )]
    AdvisoryNeedsPayloadAndBreakdown,
    #[error("the advisory_system_message template is not a valid str.format() string")]
    AdvisorySyntax {
        #[source]
        source: FormatError,
    },
    #[error("the {field} guardrail parameter has the wrong type")]
    InvalidParam {
        field: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the guardrail timeout {seconds} is not a valid duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("the Lakera v2 guard call could not be sent")]
    Transport {
        #[source]
        source: reqwest::Error,
    },
    #[error("the Lakera v2 guard answer is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Lakera v2 guard answer is not the JSON object the guard API sends")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the guardrail invocation record could not be stored")]
    Record {
        #[source]
        source: GuardrailRecordError,
    },
    #[error("a guardrail message value could not be converted to or from JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("masking returned {found} messages for {expected} scoped messages")]
    ScopeMismatch { expected: usize, found: usize },
    #[error("a masked guardrail message carries no string content")]
    MissingContent,
    #[error("the response field {at} is missing")]
    ResponseShape { at: &'static str },
}

impl From<LakeraV2Error> for GuardrailFailure {
    fn from(error: LakeraV2Error) -> Self {
        Self::Http {
            status: 500,
            detail: error.to_string(),
        }
    }
}
