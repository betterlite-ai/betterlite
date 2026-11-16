use betterlite_models::decisions::Json;
use thiserror::Error;

use crate::fields::payload::records::GuardrailInformation;
use crate::fields::timestamps::TimestampError;
use crate::fields::usage::UsageError;
use crate::logging_metadata::base64::Base64Error;
use crate::logging_metadata::decode::DecodeError;
use crate::logging_metadata::error_info::ErrorInformationError;
use crate::logging_metadata::headers::HiddenParamsError;
use crate::logging_metadata::tags::TagError;

// get_standard_logging_object_payload returns None for any of these, the way its Python catch-all does
#[derive(Debug, Error)]
pub enum PayloadError {
    #[error("the {field} field must have the declared type")]
    FieldType { field: &'static str },
    #[error("the {field} record could not be read: {source}")]
    Decode {
        field: &'static str,
        #[source]
        source: DecodeError,
    },
    #[error("the logging payload could not be written as JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the request cost is not a JSON number")]
    ResponseCostType,
    #[error("the request cost is not a number: {source}")]
    ResponseCost {
        #[source]
        source: betterlite_models::price_fields::decimal_lexeme::DecimalLexemeError,
    },
    #[error("the request cost arithmetic overflowed Decimal's range")]
    CostOverflow,
    #[error("the guardrail cost could not be read: {source}")]
    GuardrailCost {
        #[source]
        source: betterlite_models::guardrail_pricing::GuardrailPricingError,
    },
    #[error("the cost breakdown could not be merged: {source}")]
    CostBreakdown {
        #[source]
        source: betterlite_models::cost_breakdown::CostBreakdownError,
    },
    #[error("the timestamps could not be cleaned: {source}")]
    Timestamps {
        #[from]
        source: TimestampError,
    },
    #[error("the usage object could not be read: {source}")]
    Usage {
        #[from]
        source: UsageError,
    },
    #[error("the messages could not be sanitized: {source}")]
    Base64 {
        #[from]
        source: Base64Error,
    },
    #[error("the request tags could not be read: {source}")]
    Tags {
        #[from]
        source: TagError,
    },
    #[error("the hidden params could not be read: {source}")]
    HiddenParams {
        #[from]
        source: HiddenParamsError,
    },
    #[error("the error information could not be read: {source}")]
    ErrorInformation {
        #[from]
        source: ErrorInformationError,
    },
    #[error("the guardrail information field {field} has the wrong type")]
    GuardrailInformation {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the cache-hit response cost could not be described")]
    CacheCost,
}

impl PayloadError {
    pub(crate) fn decode(field: &'static str, source: DecodeError) -> Self {
        Self::Decode { field, source }
    }
}

// The guardrail list is decoded where it is read, so the error names the metadata key it came from
pub(crate) fn guardrail_information(
    value: Option<&Json>,
) -> Result<Option<Vec<GuardrailInformation>>, PayloadError> {
    let Some(Json::List(_)) = value else {
        return match value {
            None | Some(Json::Null) => Ok(None),
            Some(_) => Err(PayloadError::FieldType {
                field: "standard_logging_guardrail_information",
            }),
        };
    };
    let encoded = serde_json::to_vec(value.as_ref().unwrap_or(&Json::Null))
        .map_err(|source| PayloadError::Json { source })?;
    serde_json::from_slice(&encoded)
        .map(Some)
        .map_err(|source| PayloadError::GuardrailInformation {
            field: "standard_logging_guardrail_information",
            source,
        })
}
