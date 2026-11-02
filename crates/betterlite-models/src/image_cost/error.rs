use crate::price_fields::decimal_lexeme::DecimalLexemeError;
use crate::tiered_pricing::TieredPricingError;
use crate::token_cost::error::TokenCostError;

#[derive(Debug, thiserror::Error)]
pub enum ImageCostError {
    #[error("model {model} has no cost map entry")]
    UnknownModel { model: String },
    #[error("image_response must be an ImageResponse")]
    NotImageResponse,
    #[error("no pricing information found for model {model}")]
    NoPrice { model: String },
    #[error("field {field} must be a JSON number or null")]
    WrongFieldType { field: String },
    #[error("image size {size:?} is not two Python-int-compatible components")]
    MalformedSize { size: String },
    #[error("image dimension exceeds the supported signed range")]
    DimensionOverflow,
    #[error("a counted token or image value exceeds the supported signed range")]
    CounterOverflow,
    #[error("cost multiplication overflow")]
    Overflow,
    #[error(transparent)]
    Rate(#[from] DecimalLexemeError),
    #[error(transparent)]
    Tier(#[from] TieredPricingError),
    #[error(transparent)]
    TokenCost(#[from] TokenCostError),
}
