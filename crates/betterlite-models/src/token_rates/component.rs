use rust_decimal::Decimal;

use super::rates::cost_per_unit;
use super::{ModelInfo, TokenRateError};

pub fn calculate_cost_component(
    model: &ModelInfo,
    key: &str,
    usage: Option<Decimal>,
) -> Result<Decimal, TokenRateError> {
    let Some(usage_tokens) = usage else {
        return Ok(Decimal::ZERO);
    };
    if usage_tokens <= Decimal::ZERO {
        return Ok(Decimal::ZERO);
    }
    let unit_rate = cost_per_unit(model, key, Some(Decimal::ZERO))?.unwrap_or(Decimal::ZERO);
    usage_tokens
        .checked_mul(unit_rate)
        .ok_or(TokenRateError::Overflow)
}
