use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::adaptive_selection::error::AdaptiveSelectionError;

const NO_SPREAD_SCORE: f64 = 0.5;

pub fn normalized_cost(
    model_cost: Decimal,
    all_costs: &[Decimal],
) -> Result<f64, AdaptiveSelectionError> {
    let Some(first) = all_costs.first() else {
        return Ok(NO_SPREAD_SCORE);
    };
    let mut lowest = *first;
    let mut highest = *first;
    for cost in all_costs {
        lowest = lowest.min(*cost);
        highest = highest.max(*cost);
    }
    if highest == lowest {
        return Ok(NO_SPREAD_SCORE);
    }
    let spread = highest
        .checked_sub(lowest)
        .ok_or(AdaptiveSelectionError::CostNormalization)?;
    let offset = model_cost
        .checked_sub(lowest)
        .ok_or(AdaptiveSelectionError::CostNormalization)?;
    let ratio = offset
        .checked_div(spread)
        .ok_or(AdaptiveSelectionError::CostNormalization)?;
    let cost_score = Decimal::ONE
        .checked_sub(ratio)
        .ok_or(AdaptiveSelectionError::CostNormalization)?;
    cost_score
        .to_f64()
        .filter(|value| value.is_finite())
        .ok_or(AdaptiveSelectionError::CostNormalization)
}
