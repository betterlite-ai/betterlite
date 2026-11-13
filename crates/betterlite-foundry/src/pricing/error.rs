use betterlite_models::tiered_pricing::TieredPricingError;
use betterlite_models::token_cost::error::TokenCostError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PricingError {
    #[error("the model router fee entry {entry} is not in the cost map")]
    MissingFeeEntry { entry: &'static str },
    #[error("the model router fee rate for {entry} is not a valid price")]
    FeeRate {
        entry: &'static str,
        #[source]
        source: TieredPricingError,
    },
    #[error("the model router fee overflowed")]
    Overflow,
    #[error("pricing the azure_ai response model failed")]
    TokenCost(#[source] TokenCostError),
}
