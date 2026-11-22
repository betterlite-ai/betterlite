use thiserror::Error;

use crate::adaptive_classification::error::AdaptiveClassificationError;
use crate::adaptive_classification::request_type::RequestType;
use crate::adaptive_selection::error::AdaptiveSelectionError;
use crate::bandit_math::error::BanditMathError;
use crate::complexity_placement::error::TierPlacementError;

#[derive(Debug, Error)]
pub enum AdaptivePlacementError {
    #[error("adaptive placement needs at least one non-empty tier pool")]
    NoTierPools,
    #[error("adaptive placement needs a non-empty model pool for tier {tier}")]
    EmptyTierPool { tier: String },
    #[error("adaptive placement is not enabled for this router")]
    AdaptiveDisabled,
    #[error("the adaptive bandit has no cell for {model} as {request_type}")]
    MissingCell {
        request_type: RequestType,
        model: String,
    },
    #[error("tier {tier} is not in this router's tier list")]
    UnknownTier { tier: String },
    #[error("the adaptive decision metadata field {field} is not a finite number")]
    NonFinite { field: &'static str },
    #[error(transparent)]
    Classification(#[from] AdaptiveClassificationError),
    #[error(transparent)]
    Selection(#[from] AdaptiveSelectionError),
    #[error(transparent)]
    Bandit(#[from] BanditMathError),
    #[error(transparent)]
    Placement(#[from] TierPlacementError),
}
