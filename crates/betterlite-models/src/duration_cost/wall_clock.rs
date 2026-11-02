use rust_decimal::Decimal;

use super::error::DurationCostError;
use super::numeric::numeric_price;
use crate::price_lookup::{PriceLookup, PriceRecord};
use crate::tiered_pricing::Scalar;

const WALL_CLOCK_PRICED_MODES: [&str; 4] = ["chat", "completion", "embedding", "responses"];

// Short-circuits left to right like the source's `or` chain: a malformed lower-priority field is never inspected once an earlier one already answers true.
fn has_token_or_tiered_pricing(record: &PriceRecord) -> Result<bool, DurationCostError> {
    if numeric_price(record, "input_cost_per_token")?.unwrap_or(Decimal::ZERO) > Decimal::ZERO {
        return Ok(true);
    }
    if numeric_price(record, "output_cost_per_token")?.unwrap_or(Decimal::ZERO) > Decimal::ZERO {
        return Ok(true);
    }
    Ok(!matches!(
        record.get("tiered_pricing"),
        None | Some(Scalar::Null)
    ))
}

// A mode of the wrong type is simply not a member of the string set, same as Python's `in`.
fn bills_wall_clock_seconds(record: &PriceRecord) -> bool {
    match record.get("mode") {
        None | Some(Scalar::Null) => true,
        Some(Scalar::Text(mode)) => WALL_CLOCK_PRICED_MODES.contains(&mode.as_str()),
        Some(Scalar::Bool(_) | Scalar::Number(_) | Scalar::Array(_) | Scalar::Object(_)) => false,
    }
}

// A failed model or provider lookup, token/tiered pricing, or an ineligible mode all come back as an absent price rather than an error: only an eligible, wall-clock-only model is priced.
pub fn per_second_pricing_cost(
    model: &str,
    custom_llm_provider: Option<&str>,
    response_time_ms: Option<Decimal>,
    lookup: &dyn PriceLookup,
) -> Result<Option<(Decimal, Decimal)>, DurationCostError> {
    let Some(record) = lookup.effective_model_info(model, custom_llm_provider) else {
        return Ok(None);
    };
    if has_token_or_tiered_pricing(&record)? || !bills_wall_clock_seconds(&record) {
        return Ok(None);
    }
    // Only the winning field is coerced, so a malformed lower-priority rate never fails a request that never reads it.
    let rate = match numeric_price(&record, "cost_per_second")? {
        Some(rate) => rate,
        None => match numeric_price(&record, "input_cost_per_second")? {
            Some(rate) => rate,
            None => match numeric_price(&record, "output_cost_per_second")? {
                Some(rate) => rate,
                None => return Ok(None),
            },
        },
    };
    let seconds = response_time_ms
        .unwrap_or(Decimal::ZERO)
        .checked_div(Decimal::from(1_000_i32))
        .ok_or(DurationCostError::Overflow)?;
    let prompt_cost = rate
        .checked_mul(seconds)
        .ok_or(DurationCostError::Overflow)?;
    Ok(Some((prompt_cost, Decimal::ZERO)))
}

mod tests;
