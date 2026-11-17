use std::io::Error as IoError;
use std::time::TryFromFloatSecsError;

use thiserror::Error;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::presidio_actions::error::PresidioActionsError;
use crate::presidio_analysis::error::PresidioAnalysisError;
use crate::presidio_response::error::PresidioResponseError;

pub(super) const INTERNAL_STATUS: u16 = 500;

#[derive(Debug, Error)]
pub enum PresidioRuntimeError {
    #[error("File not found. file_path={path}")]
    FileNotFound { path: String },
    #[error("Error decoding JSON file: {source}, file_path={path}")]
    Decode {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("An error occurred: {source}, file_path={path}")]
    Read {
        path: String,
        #[source]
        source: IoError,
    },
    #[error("the Presidio timeout of {seconds} seconds is not a valid duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("the Presidio mock_redacted_text setting does not hold a redacted response")]
    MockRedactedText {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Presidio guardrail request data could not be read or written as JSON")]
    Data {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Actions(#[from] PresidioActionsError),
    #[error(transparent)]
    Response(#[from] PresidioResponseError),
}

impl From<PresidioActionsError> for GuardrailFailure {
    fn from(error: PresidioActionsError) -> Self {
        // LiteLLM's exception message prints None for a guardrail without a name, so an empty name stays None
        match error {
            PresidioActionsError::BlockedEntity {
                entity_type,
                guardrail_name,
            } => Self::BlockedPii {
                entity_type,
                guardrail_name: (!guardrail_name.is_empty()).then_some(guardrail_name),
                status_code: 400,
            },
            PresidioActionsError::Analysis {
                source:
                    PresidioAnalysisError::GuardrailRaised {
                        guardrail_name,
                        message,
                    },
            } => Self::GuardrailRaised {
                guardrail_name: (!guardrail_name.is_empty()).then_some(guardrail_name),
                message,
                wrap_default_message: false,
                status_code: 400,
                blocked_content: false,
            },
            other @ (PresidioActionsError::MissingAnalyzerApiBase
            | PresidioActionsError::MissingAnonymizerApiBase
            | PresidioActionsError::MetadataNotObject { .. }
            | PresidioActionsError::PiiTokensNotObject { .. }
            | PresidioActionsError::GuardrailConfigNotObject { .. }
            | PresidioActionsError::GuardrailConfigField { .. }
            | PresidioActionsError::AnonymizerRequest { .. }
            | PresidioActionsError::ResponseTooLarge { .. }
            | PresidioActionsError::AnonymizerHttpStatus { .. }
            | PresidioActionsError::AnonymizerContentType { .. }
            | PresidioActionsError::AnonymizerResponseJson { .. }
            | PresidioActionsError::MissingSpan { .. }
            | PresidioActionsError::SpanOutOfBounds { .. }
            | PresidioActionsError::MissingEntityType
            | PresidioActionsError::Analysis { .. }
            | PresidioActionsError::Serialize { .. }) => Self::Http {
                status: other.status(),
                detail: other.to_string(),
            },
        }
    }
}

impl From<PresidioRuntimeError> for GuardrailFailure {
    fn from(error: PresidioRuntimeError) -> Self {
        match error {
            other @ (PresidioRuntimeError::FileNotFound { .. }
            | PresidioRuntimeError::Decode { .. }
            | PresidioRuntimeError::Read { .. }
            | PresidioRuntimeError::Timeout { .. }
            | PresidioRuntimeError::MockRedactedText { .. }
            | PresidioRuntimeError::Data { .. }) => Self::Http {
                status: INTERNAL_STATUS,
                detail: other.to_string(),
            },
            PresidioRuntimeError::Actions(actions) => Self::from(actions),
            PresidioRuntimeError::Response(response) => Self::from(response),
        }
    }
}
