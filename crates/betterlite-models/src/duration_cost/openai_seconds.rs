use rust_decimal::Decimal;

use super::error::DurationCostError;
use super::numeric::numeric_price;
use super::video::{NoPriceLog, VideoCostRequest, resolved_video_cost};
use crate::price_lookup::{PriceLookup, PriceRecord};

fn model_info(
    model: &str,
    custom_llm_provider: Option<&str>,
    lookup: &dyn PriceLookup,
) -> Result<PriceRecord, DurationCostError> {
    // An absent provider defaults to "openai"; an explicit "" is used as given (docs/compatibility.md).
    lookup
        .effective_model_info(model, Some(custom_llm_provider.unwrap_or("openai")))
        .ok_or_else(|| DurationCostError::UnknownModel {
            model: model.to_owned(),
        })
}

// A strictly positive output rate charges the completion side, else a present input rate charges the prompt side even when it is zero or negative, else both are zero.
pub fn cost_per_second(
    model: &str,
    custom_llm_provider: Option<&str>,
    duration: Decimal,
    lookup: &dyn PriceLookup,
) -> Result<(Decimal, Decimal), DurationCostError> {
    let record = model_info(model, custom_llm_provider, lookup)?;
    if let Some(output) = numeric_price(&record, "output_cost_per_second")?
        && output > Decimal::ZERO
    {
        let completion = output
            .checked_mul(duration)
            .ok_or(DurationCostError::Overflow)?;
        return Ok((Decimal::ZERO, completion));
    }
    if let Some(input) = numeric_price(&record, "input_cost_per_second")? {
        let prompt = input
            .checked_mul(duration)
            .ok_or(DurationCostError::Overflow)?;
        return Ok((prompt, Decimal::ZERO));
    }
    Ok((Decimal::ZERO, Decimal::ZERO))
}

// A supplied model_info, including an empty map, bypasses the catalog lookup entirely.
pub fn video_generation_cost(
    request: &VideoCostRequest<'_>,
    lookup: &dyn PriceLookup,
) -> Result<Decimal, DurationCostError> {
    let record = match request.model_info {
        Some(supplied) => supplied.clone(),
        None => model_info(request.model, request.custom_llm_provider, lookup)?,
    };
    resolved_video_cost(
        &record,
        request.duration_seconds,
        request.video_resolution,
        request.model,
        NoPriceLog::Warn,
    )
}

mod tests;
