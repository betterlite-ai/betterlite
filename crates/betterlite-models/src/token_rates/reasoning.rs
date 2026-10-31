use rust_decimal::Decimal;

use super::rates::{cost_per_unit, service_tier_cost_key};
use super::{ModelInfo, RequestContext, TokenRateError};
use crate::off_peak::schema::OffPeakBlock;
use crate::off_peak::{self, TokenRates};
use crate::tiered_pricing::Scalar;

// Off-peak wins first, then a selected tier's own rate, then service-tier precedence.
pub(super) fn resolve_billed_reasoning_rate(
    model: &ModelInfo,
    tiered_reasoning_rate: Option<Decimal>,
    off_peak_pricing: Option<&OffPeakBlock>,
    context: &RequestContext<'_>,
    completion_base_cost: Decimal,
) -> Result<Decimal, TokenRateError> {
    if let Some(off_peak_rate) = open_off_peak_reasoning_rate(off_peak_pricing, context)? {
        return Ok(off_peak_rate);
    }
    if let Some(tiered) = tiered_reasoning_rate {
        return Ok(tiered);
    }
    resolve_reasoning_token_cost(model, context.service_tier, completion_base_cost)
}

fn open_off_peak_reasoning_rate(
    off_peak_pricing: Option<&OffPeakBlock>,
    context: &RequestContext<'_>,
) -> Result<Option<Decimal>, TokenRateError> {
    let probe = TokenRates {
        input_rate: Decimal::ZERO,
        output_rate: Decimal::ZERO,
        cache_read_rate: Decimal::ZERO,
        cache_creation_rate: Decimal::ZERO,
        reasoning_rate: None,
    };
    let resolved = off_peak::apply_off_peak_pricing(
        off_peak_pricing,
        context.moment,
        probe,
        context.database,
    )?;
    Ok(resolved.reasoning_rate)
}

// A present service output price suppresses the standard reasoning rate for the completion rate.
fn resolve_reasoning_token_cost(
    model: &ModelInfo,
    service_tier: Option<&str>,
    completion_base_cost: Decimal,
) -> Result<Decimal, TokenRateError> {
    let tier_reasoning_key = service_tier_cost_key("output_cost_per_reasoning_token", service_tier);
    if !matches!(model.get(&tier_reasoning_key), None | Some(Scalar::Null))
        && let Some(cost) = cost_per_unit(model, &tier_reasoning_key, None)?
    {
        return Ok(cost);
    }
    let tier_output_key = service_tier_cost_key("output_cost_per_token", service_tier);
    if tier_output_key != "output_cost_per_token"
        && !matches!(model.get(&tier_output_key), None | Some(Scalar::Null))
    {
        return Ok(completion_base_cost);
    }
    Ok(
        cost_per_unit(model, "output_cost_per_reasoning_token", None)?
            .unwrap_or(completion_base_cost),
    )
}
