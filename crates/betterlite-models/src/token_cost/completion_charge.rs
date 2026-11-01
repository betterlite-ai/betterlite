use rust_decimal::Decimal;

use super::error::TokenCostError;
use crate::cost_usage::CostUsage;
use crate::modality_cost::completion::{CompletionTokensDetailsResult, parse_completion_details};
use crate::modality_cost::pricing::price_for_key;
use crate::token_rates::{EffectiveRates, ModelInfo};

// When text is zero and there is a positive reasoning/audio/image/video breakdown, text becomes a clamped residual; otherwise every completion token is text and is_text_tokens_total disables per-modality charging below.
pub(super) fn completion_tokens_details(
    usage: &CostUsage,
) -> Result<(CompletionTokensDetailsResult, bool), TokenCostError> {
    let Some(completion_details) = usage.completion_details else {
        return Ok((
            CompletionTokensDetailsResult {
                audio_tokens: 0,
                text_tokens: usage.completion_tokens,
                reasoning_tokens: 0,
                image_tokens: 0,
                video_tokens: 0,
            },
            true,
        ));
    };
    let mut result = parse_completion_details(usage.completion_tokens, &completion_details)?;
    if result.text_tokens != 0 {
        return Ok((result, false));
    }
    let has_breakdown = result.audio_tokens > 0
        || result.image_tokens > 0
        || result.reasoning_tokens > 0
        || result.video_tokens > 0;
    if !has_breakdown {
        result.text_tokens = usage.completion_tokens;
        return Ok((result, true));
    }
    let overflow = || TokenCostError::Overflow;
    result.text_tokens = usage
        .completion_tokens
        .checked_sub(result.reasoning_tokens)
        .and_then(|value| value.checked_sub(result.audio_tokens))
        .and_then(|value| value.checked_sub(result.image_tokens))
        .and_then(|value| value.checked_sub(result.video_tokens))
        .ok_or_else(overflow)?
        .max(0);
    Ok((result, false))
}

fn add_modality_charge(
    cost: Decimal,
    tokens: i64,
    key: &str,
    model: &ModelInfo,
    fallback: Decimal,
) -> Result<Decimal, TokenCostError> {
    let overflow = || TokenCostError::Overflow;
    if tokens <= 0 {
        return Ok(cost);
    }
    let rate = price_for_key(model, key)?.unwrap_or(fallback);
    let charge = Decimal::from(tokens)
        .checked_mul(rate)
        .ok_or_else(overflow)?;
    cost.checked_add(charge).ok_or_else(overflow)
}

// Positive audio/image/video each use their dedicated output price, or the completion rate when absent; positive reasoning uses the already-resolved billed reasoning rate.
pub(super) fn completion_cost(
    details: &CompletionTokensDetailsResult,
    is_text_tokens_total: bool,
    model: &ModelInfo,
    rates: &EffectiveRates,
) -> Result<Decimal, TokenCostError> {
    let overflow = || TokenCostError::Overflow;
    let mut cost = Decimal::from(details.text_tokens)
        .checked_mul(rates.output_rate)
        .ok_or_else(overflow)?;
    if is_text_tokens_total {
        return Ok(cost);
    }
    cost = add_modality_charge(
        cost,
        details.audio_tokens,
        "output_cost_per_audio_token",
        model,
        rates.output_rate,
    )?;
    if details.reasoning_tokens > 0 {
        let charge = Decimal::from(details.reasoning_tokens)
            .checked_mul(rates.reasoning_rate)
            .ok_or_else(overflow)?;
        cost = cost.checked_add(charge).ok_or_else(overflow)?;
    }
    cost = add_modality_charge(
        cost,
        details.image_tokens,
        "output_cost_per_image_token",
        model,
        rates.output_rate,
    )?;
    cost = add_modality_charge(
        cost,
        details.video_tokens,
        "output_cost_per_video_token",
        model,
        rates.output_rate,
    )?;
    Ok(cost)
}

mod tests;
