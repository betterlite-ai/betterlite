use rust_decimal::Decimal;

use super::error::ModalityCostError;
use crate::cost_usage::modality_projection::CacheCreationTokenDetails;

// A present TTL detail record replaces the aggregate charge entirely, even when both buckets are null or zero.
pub fn calculate_cache_writing_cost(
    cache_creation_tokens: i64,
    cache_creation_token_details: Option<CacheCreationTokenDetails>,
    cache_creation_cost: Decimal,
    cache_creation_cost_above_1hr: Decimal,
) -> Result<Decimal, ModalityCostError> {
    let overflow = || ModalityCostError::CacheWritingCostOverflow;
    match cache_creation_token_details {
        Some(details) => {
            let five_minute_cost = match details.ephemeral_5m_input_tokens {
                Some(tokens) => Decimal::from(tokens)
                    .checked_mul(cache_creation_cost)
                    .ok_or_else(overflow)?,
                None => Decimal::ZERO,
            };
            let one_hour_cost = match details.ephemeral_1h_input_tokens {
                Some(tokens) => Decimal::from(tokens)
                    .checked_mul(cache_creation_cost_above_1hr)
                    .ok_or_else(overflow)?,
                None => Decimal::ZERO,
            };
            five_minute_cost
                .checked_add(one_hour_cost)
                .ok_or_else(overflow)
        }
        None => Decimal::from(cache_creation_tokens)
            .checked_mul(cache_creation_cost)
            .ok_or_else(overflow),
    }
}

mod tests;
