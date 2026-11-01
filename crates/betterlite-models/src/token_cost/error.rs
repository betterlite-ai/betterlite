use thiserror::Error;

use crate::modality_cost::error::ModalityCostError;
use crate::token_rates::TokenRateError;

#[derive(Debug, Error)]
pub enum TokenCostError {
    #[error("the model is not registered in the pricing catalog")]
    UnknownModel,
    #[error("a provider-specific geo multiplier price is invalid")]
    InvalidGeoMultiplier,
    #[error("generic token cost arithmetic overflowed")]
    Overflow,
    #[error(transparent)]
    Rate(#[from] TokenRateError),
    #[error(transparent)]
    Modality(#[from] ModalityCostError),
}
