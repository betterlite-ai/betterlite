use betterlite_models::image_cost::error::ImageCostError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImagePricingError {
    #[error("model {model} has no cost map entry")]
    UnknownModel { model: String },
    #[error("the steps option must be a number or absent")]
    InvalidSteps,
    #[error(transparent)]
    Price(#[from] ImageCostError),
    #[error("image cost multiplication overflowed")]
    Overflow,
}
