use betterlite_models::modality_cost::error::ModalityCostError;
use betterlite_models::resolution::ResolutionError;
use betterlite_models::token_rates::TokenRateError;

#[derive(Debug, thiserror::Error)]
pub enum BatchCostError {
    #[error("resolving the provider for the batch model failed: {source}")]
    Resolution {
        #[source]
        source: ResolutionError,
    },
    #[error(transparent)]
    Rate(#[from] TokenRateError),
    #[error(transparent)]
    Modality(#[from] ModalityCostError),
    #[error("batch cost arithmetic overflowed")]
    Overflow,
}
