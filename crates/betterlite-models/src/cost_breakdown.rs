use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Number;
use std::collections::BTreeMap;

use crate::price_fields::decimal_lexeme::{DecimalLexemeError, parse_decimal_lexeme};

#[derive(Debug, thiserror::Error)]
pub enum CostBreakdownError {
    #[error("adding the guardrail cost overflowed Decimal's representable range")]
    GuardrailCostOverflow,
}

#[derive(Deserialize)]
#[serde(try_from = "Number")]
struct ExactDecimal(Decimal);

impl TryFrom<Number> for ExactDecimal {
    type Error = DecimalLexemeError;

    fn try_from(number: Number) -> Result<Self, Self::Error> {
        let lexeme = number.to_string();
        parse_decimal_lexeme(&lexeme).map(Self)
    }
}

// service_tier, data_residency and vertex_location record the pricing basis, not what the caller asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CostBreakdown {
    pub service_tier: Option<String>,
    pub data_residency: Option<String>,
    pub vertex_location: Option<String>,
    #[serde(deserialize_with = "exact_decimal")]
    pub input_cost: Option<Decimal>,
    #[serde(deserialize_with = "exact_decimal")]
    pub cache_read_cost: Option<Decimal>,
    #[serde(deserialize_with = "exact_decimal")]
    pub cache_creation_cost: Option<Decimal>,
    #[serde(deserialize_with = "exact_decimal")]
    pub output_cost: Option<Decimal>,
    #[serde(deserialize_with = "exact_decimal")]
    pub reasoning_cost: Option<Decimal>,
    #[serde(deserialize_with = "exact_decimal")]
    pub total_cost: Option<Decimal>,
    #[serde(deserialize_with = "exact_decimal")]
    pub tool_usage_cost: Option<Decimal>,
    #[serde(deserialize_with = "exact_decimal")]
    pub guardrail_cost: Option<Decimal>,
    #[serde(deserialize_with = "exact_decimal_map")]
    pub additional_costs: Option<BTreeMap<String, Decimal>>,
    #[serde(deserialize_with = "exact_decimal")]
    pub original_cost: Option<Decimal>,
    #[serde(deserialize_with = "exact_decimal")]
    pub discount_percent: Option<Decimal>,
    #[serde(deserialize_with = "exact_decimal")]
    pub discount_amount: Option<Decimal>,
    #[serde(deserialize_with = "exact_decimal")]
    pub margin_percent: Option<Decimal>,
    #[serde(deserialize_with = "exact_decimal")]
    pub margin_fixed_amount: Option<Decimal>,
    #[serde(deserialize_with = "exact_decimal")]
    pub margin_total_amount: Option<Decimal>,
}

pub(crate) fn exact_decimal<'de, D>(deserializer: D) -> Result<Option<Decimal>, D::Error>
where
    D: Deserializer<'de>,
{
    let value: Option<ExactDecimal> = Option::deserialize(deserializer)?;
    Ok(value.map(|ExactDecimal(decimal)| decimal))
}

pub(crate) fn exact_decimal_map<'de, D>(
    deserializer: D,
) -> Result<Option<BTreeMap<String, Decimal>>, D::Error>
where
    D: Deserializer<'de>,
{
    let value: Option<BTreeMap<String, ExactDecimal>> = Option::deserialize(deserializer)?;
    Ok(value.map(|entries| {
        entries
            .into_iter()
            .map(|(key, ExactDecimal(decimal))| (key, decimal))
            .collect()
    }))
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
