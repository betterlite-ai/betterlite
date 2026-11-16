use crate::api::deepgram::listen_metadata::error::ListenMetadataError;
use crate::api::fal::pricing::FalPricingError;

#[derive(Debug, thiserror::Error)]
pub enum MediaRelayLoggingError {
    #[error("the upstream listen URL cannot be read: {0}")]
    DeepgramTarget(#[from] ListenMetadataError),
    #[error(transparent)]
    FalPricing(#[from] FalPricingError),
    #[error("the billed audio duration is beyond the decimal range")]
    Overflow,
}
