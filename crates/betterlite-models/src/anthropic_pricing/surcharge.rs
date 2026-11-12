use rust_decimal::Decimal;

use crate::cost_usage::server_tool_projection::ServerToolProjection;
use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::price_lookup::PriceRecord;
use crate::tiered_pricing::Scalar;
use crate::token_cost::GenericCostResult;
use crate::token_cost::regional::provider_geo_multiplier;

// cost_calculation.py wraps this block in try/except: a malformed multiplier leaves the generic totals it already reached.
pub(super) fn apply_surcharges(
    provider_specific_entry: Option<&PriceRecord>,
    projection: &ServerToolProjection,
    costs: &mut GenericCostResult,
) {
    let Some(entry) = provider_specific_entry.filter(|entry| !entry.is_empty()) else {
        return;
    };
    let Ok(geo_multiplier) =
        provider_geo_multiplier(Some(entry), projection.inference_geo.as_deref())
    else {
        return;
    };
    let Some(speed_multiplier) = speed_multiplier(entry, projection.speed.as_deref()) else {
        return;
    };
    if speed_multiplier != Decimal::ONE {
        let Some(scaled) = scaled(costs, speed_multiplier) else {
            return;
        };
        *costs = scaled;
    }
    if geo_multiplier != Decimal::ONE {
        let Some(scaled) = scaled(costs, geo_multiplier) else {
            return;
        };
        *costs = scaled;
    }
}

// Reads the raw `fast` value like the source's multiplication does: a numeric or Boolean value scales, anything else aborts the block.
fn speed_multiplier(entry: &PriceRecord, speed: Option<&str>) -> Option<Decimal> {
    if speed != Some("fast") {
        return Some(Decimal::ONE);
    }
    match entry.get("fast") {
        None => Some(Decimal::ONE),
        Some(Scalar::Number(number)) => parse_decimal_lexeme(&number.to_string()).ok(),
        Some(Scalar::Bool(flag)) => Some(Decimal::from(u8::from(*flag))),
        Some(Scalar::Null | Scalar::Text(_) | Scalar::Array(_) | Scalar::Object(_)) => None,
    }
}

// Both totals scale together, so a total that cannot represent the product leaves the block's earlier amount standing.
fn scaled(costs: &GenericCostResult, multiplier: Decimal) -> Option<GenericCostResult> {
    Some(GenericCostResult {
        prompt_cost: costs.prompt_cost.checked_mul(multiplier)?,
        completion_cost: costs.completion_cost.checked_mul(multiplier)?,
    })
}

mod tests;
