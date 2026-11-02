use crate::tiered_pricing::TieredPricingError;
use crate::token_cost::error::TokenCostError;

#[derive(Debug, thiserror::Error)]
pub enum DurationCostError {
    #[error("model {model} was not found in the catalog")]
    UnknownModel { model: String },
    #[error("model {key} does not have input_cost_per_character or input_cost_per_token pricing")]
    NoCharacterOrTokenPricing { key: String },
    #[error("duration cost overflowed Decimal arithmetic")]
    Overflow,
    #[error(transparent)]
    Price(#[from] TieredPricingError),
    #[error(transparent)]
    TokenCost(#[from] TokenCostError),
}
