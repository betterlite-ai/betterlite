use thiserror::Error;

use crate::presidio_analysis::error::PresidioAnalysisError;

#[derive(Debug, Error)]
pub enum PresidioActionsError {
    #[error("Missing `PRESIDIO_ANALYZER_API_BASE` from environment")]
    MissingAnalyzerApiBase,
    #[error("Missing `PRESIDIO_ANONYMIZER_API_BASE` from environment")]
    MissingAnonymizerApiBase,
    #[error("request metadata must be a JSON object")]
    MetadataNotObject {
        #[source]
        source: serde_json::Error,
    },
    #[error("request metadata pii_tokens must be an object of strings")]
    PiiTokensNotObject {
        #[source]
        source: serde_json::Error,
    },
    #[error("request metadata guardrail_config must be a JSON object")]
    GuardrailConfigNotObject {
        #[source]
        source: serde_json::Error,
    },
    #[error("request metadata guardrail_config.{field} has the wrong type")]
    GuardrailConfigField {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("Presidio anonymizer request failed")]
    AnonymizerRequest {
        #[source]
        source: reqwest::Error,
    },
    #[error("Presidio anonymizer response exceeded {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("Presidio anonymizer returned HTTP {status}: {body}")]
    AnonymizerHttpStatus { status: u16, body: String },
    #[error("Presidio anonymizer returned non-JSON Content-Type '{content_type}'; body: '{body}'")]
    AnonymizerContentType { content_type: String, body: String },
    #[error("Presidio PII anonymization failed: response is not valid JSON")]
    AnonymizerResponseJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("Presidio analysis result is missing its {field}")]
    MissingSpan { field: &'static str },
    #[error("Presidio analysis span {start}..{end} is outside the text")]
    SpanOutOfBounds { start: usize, end: usize },
    #[error("Presidio analysis result has no entity type")]
    MissingEntityType,
    #[error(
        "Blocked entity detected: {entity_type} by Guardrail: {guardrail_name}. This entity is not allowed to be used in this request."
    )]
    BlockedEntity {
        entity_type: String,
        guardrail_name: String,
    },
    #[error("Presidio PII analysis failed")]
    Analysis {
        #[source]
        source: PresidioAnalysisError,
    },
    #[error("guardrail information could not be serialized")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
}

impl PresidioActionsError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::BlockedEntity { .. }
            | Self::MetadataNotObject { .. }
            | Self::PiiTokensNotObject { .. }
            | Self::GuardrailConfigNotObject { .. }
            | Self::GuardrailConfigField { .. } => 400,
            Self::Analysis { source } => source.status(),
            Self::MissingAnalyzerApiBase
            | Self::MissingAnonymizerApiBase
            | Self::AnonymizerRequest { .. }
            | Self::ResponseTooLarge { .. }
            | Self::AnonymizerHttpStatus { .. }
            | Self::AnonymizerContentType { .. }
            | Self::AnonymizerResponseJson { .. }
            | Self::MissingSpan { .. }
            | Self::SpanOutOfBounds { .. }
            | Self::MissingEntityType
            | Self::Serialize { .. } => 500,
        }
    }
}
