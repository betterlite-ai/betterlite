use thiserror::Error;

use crate::tiered_pricing::TieredPricingError;
use crate::token_rates::TokenRateError;

#[derive(Debug, Error)]
pub enum ModalityCostError {
    #[error("billable input token subtraction overflowed")]
    BillableInputOverflow,
    #[error("cache allocation across modalities overflowed")]
    CacheAllocationOverflow,
    #[error("completion reasoning-removal arithmetic overflowed")]
    ReasoningRemovalOverflow,
    #[error("prompt cost accumulation overflowed")]
    PromptCostOverflow,
    #[error("cache writing cost accumulation overflowed")]
    CacheWritingCostOverflow,
    #[error(transparent)]
    Rate(#[from] TieredPricingError),
    #[error(transparent)]
    Component(#[from] TokenRateError),
}
