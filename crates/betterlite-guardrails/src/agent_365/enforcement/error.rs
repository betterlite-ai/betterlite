use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

use crate::guardrail_base::failure::GuardrailFailure;

#[derive(Debug, Error)]
pub enum EnforcementError {
    #[error(transparent)]
    Guardrail(#[from] GuardrailFailure),
    #[error("the Agent 365 evaluation payload could not be written as JSON")]
    PayloadEncoding {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Agent 365 token is not a valid header value")]
    TokenHeader {
        #[source]
        source: InvalidHeaderValue,
    },
}
