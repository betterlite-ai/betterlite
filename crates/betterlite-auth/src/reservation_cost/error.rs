use betterlite_models::tiered_pricing::TieredPricingError;

use crate::auth_request_models::catalog::CatalogError;
use crate::tiering::TieringError;

// Metadata lookups are swallowed by the estimator, so their failures carry no detail the caller acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ModelInfoError {
    #[error("model metadata could not be loaded")]
    Unavailable,
}

#[derive(Debug, thiserror::Error)]
pub enum ReservationCostError {
    #[error("the request's model could not be resolved: {0}")]
    Catalog(#[from] CatalogError),
    #[error("the tiered pricing table could not be decoded: {0}")]
    Table(#[from] TieringError),
    #[error("the tiered pricing table could not be applied: {0}")]
    Tier(#[from] TieredPricingError),
    #[error("the estimated cost is outside the decimal range")]
    Overflow,
}
