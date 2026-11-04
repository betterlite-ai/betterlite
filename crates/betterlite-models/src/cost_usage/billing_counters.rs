use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::CostUsage;
use super::error::CostUsageError;
use crate::billed_tokens::{BilledTokenCounts, BilledTokenUsage, project_billed_token_counts};
use crate::modality_cost::prompt::PromptTokensDetailsResult;

pub fn billing_counters(
    usage: &CostUsage,
    prompt: Option<&PromptTokensDetailsResult>,
) -> Result<BilledTokenCounts, CostUsageError> {
    let reasoning = usage
        .completion_details
        .and_then(|details| details.reasoning_tokens);
    let cache_read = prompt.map(|details| details.cache_hit_tokens);
    let cache_creation = prompt.map(|details| details.cache_creation_tokens);
    let ttl = prompt.and_then(|details| details.cache_creation_token_details);
    Ok(project_billed_token_counts(BilledTokenUsage {
        detail_reasoning: reasoning,
        top_level_reasoning: fallback_counter(usage, "reasoning_tokens", reasoning)?,
        detail_cache_read: cache_read,
        top_level_cache_read: fallback_counter(usage, "_cache_read_input_tokens", cache_read)?
            .or(usage.cache_read_tokens),
        detail_cache_creation: cache_creation,
        top_level_cache_creation: fallback_counter(
            usage,
            "_cache_creation_input_tokens",
            cache_creation,
        )?,
        detail_cached_audio: prompt.map(|details| details.cache_hit_audio_tokens),
        detail_cache_creation_5m: ttl.map(|details| details.ephemeral_5m_input_tokens.unwrap_or(0)),
        detail_cache_creation_1h: ttl.map(|details| details.ephemeral_1h_input_tokens.unwrap_or(0)),
    }))
}

fn fallback_counter(
    usage: &CostUsage,
    key: &str,
    detail: Option<i64>,
) -> Result<Option<i64>, CostUsageError> {
    if detail.is_some_and(|count| count != 0) {
        return Ok(None);
    }
    usage
        .numeric_extras
        .get(key)
        .filter(|value| value.is_sign_positive() && value.fract() == Decimal::ZERO)
        .map(|value| value.to_i64().ok_or(CostUsageError::CounterOverflow))
        .transpose()
}

mod tests;
