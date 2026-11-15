use betterlite_models::model_registration::JsonObject;
use betterlite_models::tiered_pricing::{Tier, select_tier_for_input, tier_rate};
use rust_decimal::Decimal;

use super::cost_info::CostInfo;
use super::error::ReservationCostError;
use super::rates::int_of;
use super::tokens::{estimated_output_tokens, output_multiplier};

pub(crate) fn max_cost_for_info(
    body: &JsonObject,
    route: &str,
    counted: Option<i64>,
    info: &CostInfo,
) -> Result<Option<Decimal>, ReservationCostError> {
    if let Some(cost) = image_generation_cost(body, info)? {
        return Ok(Some(cost));
    }
    let Some(input_tokens) = counted.or(info.max_input_tokens) else {
        return Ok(None);
    };
    let output_tokens = estimated_output_tokens(body, route, info);
    let multiplier = output_multiplier(body);
    if let Some(tiers) = info.tiered_pricing.as_deref()
        && let Some(tier) = select_tier_for_input(tiers, input_tokens)?
    {
        return tiered_max_cost(tier, input_tokens, output_tokens, multiplier).map(Some);
    }
    flat_max_cost(info, input_tokens, output_tokens, multiplier)
}

pub(crate) fn input_cost_for_info(
    counted: Option<i64>,
    info: &CostInfo,
) -> Result<Option<Decimal>, ReservationCostError> {
    let Some(input_tokens) = counted.or(info.max_input_tokens) else {
        return Ok(None);
    };
    if let Some(tiers) = info.tiered_pricing.as_deref()
        && let Some(tier) = select_tier_for_input(tiers, input_tokens)?
    {
        let rate = tier_rate(tier.input_token.as_ref(), None)?;
        return amount(input_tokens, rate).map(Some);
    }
    let Some(rate) = info.input_cost_per_token else {
        return Ok(None);
    };
    amount(input_tokens, rate).map(Some)
}

fn image_generation_cost(
    body: &JsonObject,
    info: &CostInfo,
) -> Result<Option<Decimal>, ReservationCostError> {
    if !info.image_mode {
        return Ok(None);
    }
    let per_image = info
        .output_cost_per_image
        .unwrap_or_default()
        .checked_add(info.input_cost_per_image.unwrap_or_default())
        .ok_or(ReservationCostError::Overflow)?;
    if per_image <= Decimal::ZERO {
        return Ok(None);
    }
    let images = int_of(body.get("n")).unwrap_or(1).max(1);
    per_image
        .checked_mul(Decimal::from(images))
        .map(Some)
        .ok_or(ReservationCostError::Overflow)
}

fn tiered_max_cost(
    tier: &Tier,
    input_tokens: i64,
    output_tokens: i64,
    multiplier: i64,
) -> Result<Decimal, ReservationCostError> {
    let input_rate = tier_rate(tier.input_token.as_ref(), None)?;
    let output_rate = tier_rate(tier.output_token.as_ref(), None)?
        .max(tier_rate(tier.output_reasoning_token.as_ref(), None)?);
    let input_cost = amount(input_tokens, input_rate)?;
    let scaled_output = output_tokens
        .checked_mul(multiplier)
        .ok_or(ReservationCostError::Overflow)?;
    let output_cost = amount(scaled_output, output_rate)?;
    input_cost
        .checked_add(output_cost)
        .ok_or(ReservationCostError::Overflow)
}

fn flat_max_cost(
    info: &CostInfo,
    input_tokens: i64,
    output_tokens: i64,
    multiplier: i64,
) -> Result<Option<Decimal>, ReservationCostError> {
    let mut cost = Decimal::ZERO;
    match info.input_cost_per_token {
        Some(rate) => {
            cost = cost
                .checked_add(amount(input_tokens, rate)?)
                .ok_or(ReservationCostError::Overflow)?;
        }
        None if input_tokens > 0 => return Ok(None),
        None => {}
    }
    let output_rate = info
        .output_cost_per_token
        .unwrap_or_default()
        .max(info.output_cost_per_reasoning_token.unwrap_or_default());
    let priced_output =
        info.output_cost_per_token.is_some() || info.output_cost_per_reasoning_token.is_some();
    if !priced_output && output_tokens > 0 {
        return Ok(None);
    }
    if priced_output {
        let scaled_output = output_tokens
            .checked_mul(multiplier)
            .ok_or(ReservationCostError::Overflow)?;
        cost = cost
            .checked_add(amount(scaled_output, output_rate)?)
            .ok_or(ReservationCostError::Overflow)?;
    }
    Ok(Some(cost))
}

fn amount(tokens: i64, rate: Decimal) -> Result<Decimal, ReservationCostError> {
    Decimal::from(tokens)
        .checked_mul(rate)
        .ok_or(ReservationCostError::Overflow)
}
