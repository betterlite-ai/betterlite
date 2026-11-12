use betterlite_models::token_cost::error::TokenCostError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GooglePricingError {
    #[error("the {field} cost overflowed decimal arithmetic")]
    Overflow { field: &'static str },
    #[error(transparent)]
    Token(#[from] TokenCostError),
}
