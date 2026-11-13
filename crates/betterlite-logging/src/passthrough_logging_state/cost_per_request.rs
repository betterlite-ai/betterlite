use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use super::usage::{PassThroughCallState, has_upstream_reported_usage};

// An upstream that reports its own cost owns the request, so its unusable value means zero
pub fn set_cost_per_request(
    state: &mut PassThroughCallState,
    kwargs_response_cost: &mut Option<f64>,
    cost_per_request: Option<Decimal>,
) {
    if has_upstream_reported_usage(state) {
        return;
    }
    let Some(amount) = cost_per_request else {
        return;
    };
    let Some(cost) = amount.to_f64() else {
        return;
    };
    *kwargs_response_cost = Some(cost);
    state.response_cost = Some(cost);
}
