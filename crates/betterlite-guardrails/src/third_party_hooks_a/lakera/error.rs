use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

use crate::guardrail_selection::error::GuardrailSelectionError;
use crate::lakera::rejection::LakeraRejection;
use crate::third_party_hooks_a::error::ThirdPartyHookError;

// B3777's class has no error type of its own, so the v1 check's failures live here and B8666 reads
// the HTTP status from them.
#[derive(Debug, Error)]
pub enum LakeraHookError {
    #[error("the Lakera guardrail request failed")]
    Transport {
        #[source]
        source: reqwest::Error,
    },
    #[error("the Lakera guardrail answer is not valid JSON")]
    ResponseJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Lakera request could not be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Lakera authorization header is not a valid header value")]
    Header {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the Lakera request field {field} has the wrong type")]
    InvalidField { field: &'static str },
    #[error(transparent)]
    Selection(#[from] GuardrailSelectionError),
}

impl LakeraHookError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Selection(error) => match error {
                GuardrailSelectionError::TagMode { .. } => StatusCode::INTERNAL_SERVER_ERROR,
                GuardrailSelectionError::RequestedNotList
                | GuardrailSelectionError::DynamicParamsNotObject { .. } => StatusCode::BAD_REQUEST,
            },
            Self::Transport { .. }
            | Self::ResponseJson { .. }
            | Self::Encode { .. }
            | Self::Header { .. }
            | Self::InvalidField { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

// LiteLLM raises HTTPException(400, detail={"error": ..., "lakera_ai_response": response}), so the
// detail goes out as the JSON object B3777's rejection serializes to.
pub(super) fn rejection_error(rejection: &LakeraRejection) -> Result<(), ThirdPartyHookError> {
    let encoded = serde_json::to_value(&rejection.detail)
        .map_err(|source| LakeraHookError::Encode { source })?;
    let detail =
        serde_json::from_value(encoded).map_err(|source| LakeraHookError::Encode { source })?;
    Err(ThirdPartyHookError::Rejection {
        status: rejection.status,
        detail,
        openai_code: None,
    })
}
