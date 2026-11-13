use betterlite_models::token_cost::error::TokenCostError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum XaiUsageError {
    #[error("xAI usage arithmetic overflowed")]
    Overflow,
    #[error(transparent)]
    Pricing(#[from] TokenCostError),
}
