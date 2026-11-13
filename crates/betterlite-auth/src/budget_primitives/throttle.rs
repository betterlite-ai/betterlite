use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::prelude::ToPrimitive as _;

use crate::resolved_key::auth::UserAPIKeyAuth;

// LiteLLM's global starts as None, and only a fraction inside (0, 1] enables throttling
#[must_use]
pub fn percentage(configured: Option<f64>) -> Option<f64> {
    let value = configured?;
    if value <= 0.0_f64 || value > 1.0_f64 || !value.is_finite() {
        return None;
    }
    Some(value)
}

#[must_use]
pub fn should_throttle(token: &UserAPIKeyAuth, configured: Option<f64>) -> bool {
    let opted_in = matches!(
        token
            .view
            .token
            .metadata
            .0
            .get("throttle_on_budget_exceeded"),
        Some(JsonData::Bool(true))
    );
    let limited = token.view.token.tpm_limit.is_some() || token.view.token.rpm_limit.is_some();
    opted_in && limited && percentage(configured).is_some()
}

#[must_use]
pub fn throttled_limit(limit: Option<i64>, configured: Option<f64>) -> Option<i64> {
    let (Some(value), Some(fraction)) = (limit, configured) else {
        return limit;
    };
    // float_arithmetic is denied, and a fused multiply-add with zero is the same product
    let product = value.to_f64()?.mul_add(fraction, 0.0_f64).floor();
    let floored = product.to_i64()?;
    Some(floored.max(1))
}
