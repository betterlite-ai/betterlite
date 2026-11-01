use rust_decimal::Decimal;

use super::cache_write::calculate_cache_writing_cost;
use super::error::ModalityCostError;
use super::pricing::{price_for_key, service_tier_cost_key};
use super::prompt::PromptTokensDetailsResult;
use crate::tiered_pricing::Scalar;
use crate::token_rates::ModelInfo;
use crate::token_rates::component::calculate_cost_component;

pub struct InputCostRates {
    pub prompt_base_cost: Decimal,
    pub cache_read_cost: Decimal,
    pub cache_creation_cost: Decimal,
    pub cache_creation_cost_above_1hr: Decimal,
}

fn token_cost_key_with_fallback<'preferred>(
    model: &ModelInfo,
    preferred: &'preferred str,
) -> &'preferred str {
    if matches!(model.get(preferred), None | Some(Scalar::Null)) {
        "input_cost_per_token"
    } else {
        preferred
    }
}

pub fn calculate_input_cost(
    details: &PromptTokensDetailsResult,
    model: &ModelInfo,
    rates: &InputCostRates,
    service_tier: Option<&str>,
) -> Result<Decimal, ModalityCostError> {
    let overflow = || ModalityCostError::PromptCostOverflow;
    let mut prompt_cost = Decimal::from(details.text_tokens)
        .checked_mul(rates.prompt_base_cost)
        .ok_or_else(overflow)?;

    prompt_cost = add_cache_read_cost(prompt_cost, details, model, rates, service_tier)?;
    prompt_cost = add_modality_token_costs(prompt_cost, details, model, service_tier)?;

    let writing_cost = calculate_cache_writing_cost(
        details.cache_creation_tokens,
        details.cache_creation_token_details,
        rates.cache_creation_cost,
        rates.cache_creation_cost_above_1hr,
    )?;
    prompt_cost = prompt_cost.checked_add(writing_cost).ok_or_else(overflow)?;

    add_per_unit_components(prompt_cost, details, model)
}

fn add_modality_token_costs(
    mut prompt_cost: Decimal,
    details: &PromptTokensDetailsResult,
    model: &ModelInfo,
    service_tier: Option<&str>,
) -> Result<Decimal, ModalityCostError> {
    let overflow = || ModalityCostError::PromptCostOverflow;
    // A nonnull per-unit price, zero included, suppresses the token charge whenever the unit quantity is nonzero.
    if details.audio_length_seconds.is_zero()
        || matches!(
            model.get("input_cost_per_audio_per_second"),
            None | Some(Scalar::Null)
        )
    {
        let key = service_tier_cost_key("input_cost_per_audio_token", service_tier);
        let component =
            calculate_cost_component(model, &key, Some(Decimal::from(details.audio_tokens)))?;
        prompt_cost = prompt_cost.checked_add(component).ok_or_else(overflow)?;
    }

    if details.video_length_seconds.is_zero()
        || matches!(
            model.get("input_cost_per_video_per_second"),
            None | Some(Scalar::Null)
        )
    {
        let key = token_cost_key_with_fallback(model, "input_cost_per_video_token");
        let component =
            calculate_cost_component(model, key, Some(Decimal::from(details.video_tokens)))?;
        prompt_cost = prompt_cost.checked_add(component).ok_or_else(overflow)?;
    }

    // A nonnull per-image price suppresses image-token charging whenever image_count is also nonzero.
    if details.image_count == 0
        || matches!(model.get("input_cost_per_image"), None | Some(Scalar::Null))
    {
        let key = token_cost_key_with_fallback(model, "input_cost_per_image_token");
        let component =
            calculate_cost_component(model, key, Some(Decimal::from(details.image_tokens)))?;
        prompt_cost = prompt_cost.checked_add(component).ok_or_else(overflow)?;
    }

    Ok(prompt_cost)
}

fn add_per_unit_components(
    mut prompt_cost: Decimal,
    details: &PromptTokensDetailsResult,
    model: &ModelInfo,
) -> Result<Decimal, ModalityCostError> {
    let overflow = || ModalityCostError::PromptCostOverflow;
    for (key, quantity) in [
        (
            "input_cost_per_character",
            Decimal::from(details.character_count),
        ),
        ("input_cost_per_image", Decimal::from(details.image_count)),
        (
            "input_cost_per_video_per_second",
            details.video_length_seconds,
        ),
        (
            "input_cost_per_audio_per_second",
            details.audio_length_seconds,
        ),
        ("input_cost_per_query", Decimal::from(details.query_count)),
    ] {
        let component = calculate_cost_component(model, key, Some(quantity))?;
        prompt_cost = prompt_cost.checked_add(component).ok_or_else(overflow)?;
    }
    Ok(prompt_cost)
}

fn add_cache_read_cost(
    prompt_cost: Decimal,
    details: &PromptTokensDetailsResult,
    model: &ModelInfo,
    rates: &InputCostRates,
    service_tier: Option<&str>,
) -> Result<Decimal, ModalityCostError> {
    let overflow = || ModalityCostError::PromptCostOverflow;
    let audio_cache_read_key =
        service_tier_cost_key("cache_read_input_audio_token_cost", service_tier);
    let audio_cache_read_rate = price_for_key(model, &audio_cache_read_key)?;

    let nonaudio_cache_hit = details
        .cache_hit_tokens
        .checked_sub(details.cache_hit_audio_tokens)
        .ok_or(ModalityCostError::CacheAllocationOverflow)?;
    let mut cost = prompt_cost
        .checked_add(
            Decimal::from(nonaudio_cache_hit)
                .checked_mul(rates.cache_read_cost)
                .ok_or_else(overflow)?,
        )
        .ok_or_else(overflow)?;
    let audio_rate = audio_cache_read_rate.unwrap_or(rates.cache_read_cost);
    cost = cost
        .checked_add(
            Decimal::from(details.cache_hit_audio_tokens)
                .checked_mul(audio_rate)
                .ok_or_else(overflow)?,
        )
        .ok_or_else(overflow)?;
    Ok(cost)
}

mod tests;
