use crate::adaptive_classification::error::AdaptiveClassificationError;
use crate::adaptive_classification::request_type::RequestType;
use crate::bandit_math::error::BanditMathError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AdaptiveSelectionError {
    #[error(transparent)]
    Classification(#[from] AdaptiveClassificationError),
    #[error(transparent)]
    Bandit(#[from] BanditMathError),
    #[error("the adaptive bandit cannot turn a model cost into a score")]
    CostNormalization,
    #[error("AdaptiveRouter[{router_name}]: no models meet min_quality_tier={tier}")]
    NoEligibleModels { router_name: String, tier: String },
    #[error("model {model} has no {request_type} bandit cell")]
    MissingCell {
        request_type: RequestType,
        model: String,
    },
}
