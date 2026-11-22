use rust_decimal::Decimal;
use thiserror::Error;

use crate::model_effective_fields::error::Refusal;

#[derive(Debug, Error)]
pub enum PtuPricingError {
    #[error(
        "A PTU deployment bills by reserved capacity, so {} cannot be charged on top of it. Send \
         0 or no value, or remove ptu_count and cost_per_ptu_per_hour to bill per token.",
        .priced.join(", ")
    )]
    Priced { priced: Vec<&'static str> },
    #[error("the zeroed rate {rate} is not a JSON number")]
    Rate {
        rate: Decimal,
        #[source]
        source: serde_json::Error,
    },
}

// LiteLLM refuses a priced PTU deployment with a bare HTTPException, so it carries no type or param
impl From<&PtuPricingError> for Refusal {
    fn from(error: &PtuPricingError) -> Self {
        let status = match error {
            PtuPricingError::Priced { .. } => 400_u16,
            PtuPricingError::Rate { .. } => 500_u16,
        };
        Self {
            status,
            error_type: None,
            param: None,
        }
    }
}
