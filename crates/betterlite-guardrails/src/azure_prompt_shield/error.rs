use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_models::guardrail_pricing::GuardrailPricingError;
use indexmap::IndexMap;
use reqwest::StatusCode;
use thiserror::Error;

use crate::azure_content_safety::error::AzureContentSafetyError;
use crate::guardrail_base::error::GuardrailBaseError;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_records::error::GuardrailRecordError;

pub const ATTACK_MESSAGE: &str = "Violated Azure Prompt Shield guardrail policy";

#[derive(Debug, Error)]
pub enum PromptShieldError {
    #[error(
        "Azure Prompt Shield: '{reference}' resolves to an unset or blank environment variable"
    )]
    UnsetSecret { reference: String },
    #[error("Azure Prompt Shield: cost_tier must be 'free' or 'paid', got {value}")]
    InvalidCostTier { value: String },
    #[error(
        "Azure Prompt Shield: cost_tier 'paid' requires a positive price_per_1000_text_records"
    )]
    PaidTierWithoutPrice,
    #[error("Azure Prompt Shield: price_per_1000_text_records must be a number, got {value}")]
    PriceNotANumber { value: String },
    #[error(
        "Azure Prompt Shield: price_per_1000_text_records must be a finite, non-negative number, got {value}"
    )]
    InvalidPrice { value: String },
    #[error("Azure Prompt Shield: api_version must be a string, got {value}")]
    ApiVersionNotAString { value: String },
    #[error("Azure Prompt Shield: timeout must be a non-negative number of seconds, got {seconds}")]
    InvalidTimeout { seconds: f64 },
    #[error("{}", ATTACK_MESSAGE)]
    Attack { detection_message: String },
    #[error(transparent)]
    Transport(#[from] AzureContentSafetyError),
    #[error(transparent)]
    Base(#[from] GuardrailBaseError),
    #[error(transparent)]
    Record(#[from] GuardrailRecordError),
    #[error(transparent)]
    Pricing(#[from] GuardrailPricingError),
}

impl PromptShieldError {
    // LiteLLM raises HTTPException(400) on a detected attack and answers the transport's own status otherwise.
    #[must_use]
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Attack { .. } => StatusCode::BAD_REQUEST,
            Self::Transport(error) => error.status_code(),
            Self::UnsetSecret { .. }
            | Self::InvalidCostTier { .. }
            | Self::PaidTierWithoutPrice
            | Self::PriceNotANumber { .. }
            | Self::InvalidPrice { .. }
            | Self::ApiVersionNotAString { .. }
            | Self::InvalidTimeout { .. }
            | Self::Base(_)
            | Self::Record(_)
            | Self::Pricing(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    // The dict LiteLLM puts in HTTPException.detail; the other variants carry none.
    #[must_use]
    pub fn detail(&self) -> Option<AttackDetail<'_>> {
        match self {
            Self::Attack { detection_message } => Some(AttackDetail {
                error: ATTACK_MESSAGE,
                detection_message,
            }),
            Self::UnsetSecret { .. }
            | Self::InvalidCostTier { .. }
            | Self::PaidTierWithoutPrice
            | Self::PriceNotANumber { .. }
            | Self::InvalidPrice { .. }
            | Self::ApiVersionNotAString { .. }
            | Self::InvalidTimeout { .. }
            | Self::Transport(_)
            | Self::Base(_)
            | Self::Record(_)
            | Self::Pricing(_) => None,
        }
    }
}

impl From<PromptShieldError> for GuardrailFailure {
    fn from(error: PromptShieldError) -> Self {
        let status = error.status_code().as_u16();
        match error.detail() {
            Some(detail) => {
                let mut object = IndexMap::new();
                object.insert(
                    "error".to_owned(),
                    GuardrailValue::Text(detail.error.to_owned()),
                );
                object.insert(
                    "detection_message".to_owned(),
                    GuardrailValue::Text(detail.detection_message.to_owned()),
                );
                Self::HttpObject {
                    status,
                    detail: object,
                }
            }
            None => Self::Http {
                status,
                detail: error.to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackDetail<'detail> {
    pub error: &'static str,
    pub detection_message: &'detail str,
}

// LiteLLM formats these values with Python's repr, which is what a failing config names back.
pub(crate) fn describe_value(value: &GuardrailValue) -> String {
    match value {
        GuardrailValue::Null => "None".to_owned(),
        GuardrailValue::Boolean(true) => "True".to_owned(),
        GuardrailValue::Boolean(false) => "False".to_owned(),
        GuardrailValue::SignedInteger(number) => number.to_string(),
        GuardrailValue::UnsignedInteger(number) => number.to_string(),
        GuardrailValue::Decimal(number) => number.to_string(),
        GuardrailValue::Text(text) => format!("{text:?}"),
        GuardrailValue::Array(_) | GuardrailValue::Object(_) => {
            serde_json::to_string(value).unwrap_or_else(|_source| "?".to_owned())
        }
    }
}
