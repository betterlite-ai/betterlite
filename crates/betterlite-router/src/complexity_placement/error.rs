use thiserror::Error;

use crate::affinity_claims::error::AffinityError;
use crate::model_catalog_lookup::error::LookupError;

#[derive(Debug, Error)]
pub enum TierPlacementError {
    #[error("No model configured for tier {tier} and no default_model set")]
    NoModelForTier { tier: String },
    #[error("Empty model pool for tier {tier}")]
    EmptyPool { tier: String },
    #[error("the random draw selected no model in the tier pool")]
    Draw,
    #[error(transparent)]
    Affinity(#[from] AffinityError),
    #[error(transparent)]
    Lookup(#[from] LookupError),
}
