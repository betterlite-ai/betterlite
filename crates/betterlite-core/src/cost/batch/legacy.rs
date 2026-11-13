use rust_decimal::Decimal;

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::modality_cost::prompt::get_billable_input_tokens;
use betterlite_models::modality_cost::prompt::parse_prompt_tokens_details;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::token_rates::ModelInfo;
use betterlite_models::token_rates::component::calculate_cost_component;

use super::error::BatchCostError;
use super::prompt_modality_usage;

const CACHE_READ_RATE_KEY: &str = "cache_read_input_token_cost";
const CACHE_WRITE_RATE_KEY: &str = "cache_creation_input_token_cost";

// Half of the ordinary prices, with an unclamped residual and no fallback for a missing cache-read price.
pub(super) fn legacy_prompt_cost(
    usage: &CostUsage,
    extras: &PromptExtras,
    model_info: &ModelInfo,
    input_rate: Decimal,
) -> Result<Decimal, BatchCostError> {
    let modality_usage = prompt_modality_usage(usage, extras);
    let details = parse_prompt_tokens_details(&modality_usage)?;
    let base_input_tokens = get_billable_input_tokens(usage.prompt_tokens, &modality_usage)?
        .checked_sub(details.cache_creation_tokens)
        .ok_or(BatchCostError::Overflow)?;
    let base_cost = Decimal::from(base_input_tokens)
        .checked_mul(input_rate)
        .and_then(|cost| cost.checked_div(Decimal::TWO))
        .ok_or(BatchCostError::Overflow)?;
    let cache_read_cost = calculate_cost_component(
        model_info,
        CACHE_READ_RATE_KEY,
        Some(Decimal::from(details.cache_hit_tokens)),
    )?
    .checked_div(Decimal::TWO)
    .ok_or(BatchCostError::Overflow)?;
    let write_rate =
        calculate_cost_component(model_info, CACHE_WRITE_RATE_KEY, Some(Decimal::ONE))?;
    let cache_creation_rate = if write_rate.is_zero() {
        input_rate
    } else {
        write_rate
    };
    let cache_creation_cost = Decimal::from(details.cache_creation_tokens)
        .checked_mul(cache_creation_rate)
        .and_then(|cost| cost.checked_div(Decimal::TWO))
        .ok_or(BatchCostError::Overflow)?;
    base_cost
        .checked_add(cache_read_cost)
        .and_then(|total| total.checked_add(cache_creation_cost))
        .ok_or(BatchCostError::Overflow)
}
