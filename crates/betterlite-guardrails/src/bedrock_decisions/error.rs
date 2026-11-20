use betterlite_models::guardrail_pricing::GuardrailPricingError;
use thiserror::Error;

use crate::guardrail_records::error::GuardrailRecordError;

#[derive(Debug, Error)]
pub enum BedrockDecisionError {
    #[error("the summed guardrail usage counter {counter} does not fit in 64 bits")]
    UsageOverflow { counter: String },
    #[error("the guardrail payload could not be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Pricing(#[from] GuardrailPricingError),
    #[error(transparent)]
    Record(#[from] GuardrailRecordError),
}
