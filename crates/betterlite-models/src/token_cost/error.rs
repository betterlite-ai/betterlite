use thiserror::Error;

use crate::modality_cost::error::ModalityCostError;
use crate::price_fields::decimal_lexeme::DecimalLexemeError;
use crate::token_rates::TokenRateError;

#[derive(Debug, Error)]
pub enum TokenCostError {
    #[error("the model is not registered in the pricing catalog")]
    UnknownModel,
    #[error(
        "the provider-specific geo multiplier price for {geo} is not a number or numeric string"
    )]
    InvalidGeoMultiplier {
        geo: String,
        #[source]
        source: DecimalLexemeError,
    },
    #[error("the provider-specific geo multiplier price for {geo} is null, an array or an object")]
    UnsupportedGeoMultiplierShape { geo: String },
    #[error("generic token cost arithmetic overflowed")]
    Overflow,
    #[error(
        "the resolved model info declares a nonempty tiered_pricing table that was not also supplied as a decoded tier table"
    )]
    UndecodedTieredPricing,
    #[error(
        "the resolved model info declares an off_peak_pricing object that was not also supplied as a decoded off-peak block"
    )]
    UndecodedOffPeakPricing,
    #[error(transparent)]
    Rate(#[from] TokenRateError),
    #[error(transparent)]
    Modality(#[from] ModalityCostError),
}
