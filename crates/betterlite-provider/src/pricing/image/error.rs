use betterlite_bedrock::image_pricing::error::ImagePricingError;
use betterlite_models::image_cost::error::ImageCostError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImagePolicyError {
    #[error("betterlite has no executable adapter for the {0} image provider")]
    UnavailableAdapter(&'static str),
    #[error(transparent)]
    Image(#[from] ImageCostError),
    #[error(transparent)]
    Bedrock(#[from] ImagePricingError),
    #[error("an optional image parameter cannot be passed to the price calculator")]
    OptionalParams {
        #[source]
        source: serde_json::Error,
    },
}
