use betterlite_bedrock_guardrails::bedrock_credentials::CredentialLoadError;
use betterlite_bedrock_guardrails::bedrock_request::PrepareError;
use betterlite_bedrock_guardrails::constants::BEDROCK_TOO_LARGE_SUBSTRINGS;

use crate::guardrail_base::failure::GuardrailFailure;

// a Reported failure either wrote its record already or deliberately wrote none, so apply_guardrail's consolidated catch logs only Unreported ones
#[derive(Debug)]
pub(super) enum ApplyError {
    Reported(GuardrailFailure),
    Unreported { status: u16, detail: String },
}

impl ApplyError {
    // only a real throttle retries: a 429 that reports an oversized input is not transient
    pub(super) fn is_throttle(&self) -> bool {
        match self {
            Self::Unreported { status, detail } => *status == 429 && !is_too_large(detail),
            Self::Reported(_) => false,
        }
    }

    pub(super) fn is_too_large(&self) -> bool {
        match self {
            Self::Unreported { detail, .. } => is_too_large(detail),
            Self::Reported(_) => false,
        }
    }
}

impl From<ApplyError> for GuardrailFailure {
    fn from(error: ApplyError) -> Self {
        match error {
            ApplyError::Reported(failure) => failure,
            ApplyError::Unreported { status, detail } => Self::Http { status, detail },
        }
    }
}

// LiteLLM's _is_input_too_large_error matches the lowered message against the same keyword list
fn is_too_large(detail: &str) -> bool {
    let lowered = detail.to_lowercase();
    BEDROCK_TOO_LARGE_SUBSTRINGS
        .iter()
        .any(|substring| lowered.contains(*substring))
}

#[derive(Debug, thiserror::Error)]
pub(super) enum ApplyRequestError {
    #[error("the Bedrock request `api_key` must be a string")]
    ApiKeyType,
    #[error("the Bedrock request body cannot be encoded")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Credentials(#[from] CredentialLoadError),
}

impl From<ApplyRequestError> for GuardrailFailure {
    fn from(error: ApplyRequestError) -> Self {
        Self::Http {
            status: 500,
            detail: error.to_string(),
        }
    }
}

// a prepare failure happens before the call LiteLLM's transport layer would log, so it carries no record
impl From<PrepareError> for ApplyError {
    fn from(error: PrepareError) -> Self {
        Self::Reported(GuardrailFailure::Http {
            status: 500,
            detail: error.to_string(),
        })
    }
}
