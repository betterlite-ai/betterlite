use rust_decimal::Decimal;

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::modality_cost::prompt::{
    PromptTokensDetailsResult, parse_prompt_tokens_details,
};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::token_rates::component::calculate_cost_component;
use betterlite_models::token_rates::{ModelInfo, TokenRateError};

use super::error::BatchCostError;
use super::prompt_modality_usage;

// The two cache categories a batch record can price independently of its input tokens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PromptCacheRates {
    pub read: Option<Decimal>,
    pub creation: Option<Decimal>,
}

const AUDIO_RATE_KEY: &str = "input_cost_per_audio_token_batches";
const IMAGE_RATE_KEY: &str = "input_cost_per_image_token_batches";
const VIDEO_RATE_KEY: &str = "input_cost_per_video_token_batches";

// A modality without its own batch rate is charged at the selected input batch rate; an explicit zero stays free.
fn modality_rate(
    model_info: &ModelInfo,
    key: &str,
    fallback: Decimal,
) -> Result<Decimal, TokenRateError> {
    if matches!(model_info.get(key), None | Some(Scalar::Null)) {
        return Ok(fallback);
    }
    calculate_cost_component(model_info, key, Some(Decimal::ONE))
}

fn add_product(total: Decimal, count: i64, rate: Decimal) -> Result<Decimal, BatchCostError> {
    total
        .checked_add(
            Decimal::from(count)
                .checked_mul(rate)
                .ok_or(BatchCostError::Overflow)?,
        )
        .ok_or(BatchCostError::Overflow)
}

// Only cache categories with a batch rate leave the text remainder; a missing rate keeps their tokens there.
fn text_remainder_tokens(
    prompt_tokens: i64,
    details: &PromptTokensDetailsResult,
    cached_tokens: i64,
    written_tokens: i64,
) -> Result<i64, BatchCostError> {
    let remaining = prompt_tokens
        .checked_sub(details.audio_tokens)
        .and_then(|remaining| remaining.checked_sub(details.image_tokens))
        .and_then(|remaining| remaining.checked_sub(details.video_tokens))
        .and_then(|remaining| remaining.checked_sub(cached_tokens))
        .and_then(|remaining| remaining.checked_sub(written_tokens))
        .ok_or(BatchCostError::Overflow)?;
    Ok(remaining.max(0))
}

pub fn batch_prompt_cost(
    usage: &CostUsage,
    extras: &PromptExtras,
    model_info: &ModelInfo,
    input_rate: Decimal,
    cache_rates: PromptCacheRates,
) -> Result<Decimal, BatchCostError> {
    let details = parse_prompt_tokens_details(&prompt_modality_usage(usage, extras))?;
    let cached_tokens = if cache_rates.read.is_some() {
        details.cache_hit_tokens
    } else {
        0
    };
    let written_tokens = if cache_rates.creation.is_some() {
        details.cache_creation_tokens
    } else {
        0
    };
    let text_tokens =
        text_remainder_tokens(usage.prompt_tokens, &details, cached_tokens, written_tokens)?;
    let mut total = add_product(Decimal::ZERO, text_tokens, input_rate)?;
    total = add_product(
        total,
        details.audio_tokens,
        modality_rate(model_info, AUDIO_RATE_KEY, input_rate)?,
    )?;
    total = add_product(
        total,
        details.image_tokens,
        modality_rate(model_info, IMAGE_RATE_KEY, input_rate)?,
    )?;
    total = add_product(
        total,
        details.video_tokens,
        modality_rate(model_info, VIDEO_RATE_KEY, input_rate)?,
    )?;
    total = add_product(
        total,
        cached_tokens,
        cache_rates.read.unwrap_or(Decimal::ZERO),
    )?;
    add_product(
        total,
        written_tokens,
        cache_rates.creation.unwrap_or(Decimal::ZERO),
    )
}
