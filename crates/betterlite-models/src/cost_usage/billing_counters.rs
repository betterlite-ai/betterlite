use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::CostUsage;
use crate::billed_tokens::{BilledTokenCounts, BilledTokenUsage, project_billed_token_counts};
use crate::modality_cost::prompt::PromptTokensDetailsResult;

#[must_use]
pub fn billing_counters(
    usage: &CostUsage,
    prompt: Option<&PromptTokensDetailsResult>,
) -> BilledTokenCounts {
    let positive_integer = |key: &str| {
        usage
            .numeric_extras
            .get(key)
            .filter(|value| value.is_sign_positive() && value.fract() == Decimal::ZERO)
            .and_then(ToPrimitive::to_i64)
    };
    let ttl = prompt.and_then(|details| details.cache_creation_token_details);
    project_billed_token_counts(BilledTokenUsage {
        detail_reasoning: usage
            .completion_details
            .and_then(|details| details.reasoning_tokens),
        top_level_reasoning: positive_integer("reasoning_tokens"),
        detail_cache_read: prompt.map(|details| details.cache_hit_tokens),
        top_level_cache_read: positive_integer("_cache_read_input_tokens")
            .or(usage.cache_read_tokens),
        detail_cache_creation: prompt.map(|details| details.cache_creation_tokens),
        top_level_cache_creation: positive_integer("_cache_creation_input_tokens"),
        detail_cached_audio: prompt.map(|details| details.cache_hit_audio_tokens),
        detail_cache_creation_5m: ttl.map(|details| details.ephemeral_5m_input_tokens.unwrap_or(0)),
        detail_cache_creation_1h: ttl.map(|details| details.ephemeral_1h_input_tokens.unwrap_or(0)),
    })
}

mod tests;
