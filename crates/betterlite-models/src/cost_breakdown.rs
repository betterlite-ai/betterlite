use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use rust_decimal::Decimal;

type Extensions = BTreeMap<String, Decimal>;

#[derive(Debug, thiserror::Error)]
pub enum CostBreakdownError {
    #[error("adding the guardrail cost overflowed Decimal's representable range")]
    GuardrailCostOverflow,
}

// extra keeps any key outside this vocabulary, such as a provider-specific cost a newer writer stamped.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostBreakdown {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_residency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertex_location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_cost: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_cost: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_cost: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_cost: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_cost: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cost: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_usage_cost: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guardrail_cost: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_costs: Option<BTreeMap<String, Decimal>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_cost: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discount_percent: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discount_amount: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_percent: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_fixed_amount: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_total_amount: Option<Decimal>,
    #[serde(flatten)]
    pub extra: Extensions,
}

// Adds to total_cost without subtracting any prior guardrail amount, so repeated calls add again like the source does.
pub fn cost_breakdown_with_guardrail(
    cost_breakdown: Option<&CostBreakdown>,
    guardrail_cost: Decimal,
) -> Result<Option<CostBreakdown>, CostBreakdownError> {
    if guardrail_cost <= Decimal::ZERO {
        return Ok(cost_breakdown.cloned());
    }
    let mut merged = cost_breakdown.cloned().unwrap_or_default();
    let total = merged
        .total_cost
        .unwrap_or(Decimal::ZERO)
        .checked_add(guardrail_cost)
        .ok_or(CostBreakdownError::GuardrailCostOverflow)?;
    merged.guardrail_cost = Some(guardrail_cost);
    merged.total_cost = Some(total);
    Ok(Some(merged))
}

mod tests;
