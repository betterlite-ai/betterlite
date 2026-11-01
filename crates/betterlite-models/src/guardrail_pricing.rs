use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::iter::once;

use crate::price_fields::decimal_lexeme::DecimalLexemeError;

mod coercion;
pub mod entries;

#[derive(Debug, thiserror::Error)]
pub enum GuardrailPricingError {
    #[error("the guardrail cost arithmetic overflowed Decimal's representable range")]
    CostOverflow,
    #[error("the guardrail cost is not a Decimal-representable number")]
    UnrepresentableCost {
        #[source]
        source: DecimalLexemeError,
    },
}

// A present, even empty, Usable map still blocks the regional-then-global fallback; only Malformed or a miss falls through.
#[derive(Debug, Clone)]
pub enum GuardrailPriceLookup {
    Usable(BTreeMap<String, Decimal>),
    Malformed,
}

const BEDROCK_GUARDRAIL_PRICING_KEY: &str = "bedrock/guardrails";

// A malformed entry warns and falls through to the next candidate key, same as LiteLLM's lookup loop.
fn bedrock_guardrail_pricing<F>(
    lookup: F,
    aws_region_name: Option<&str>,
) -> Option<BTreeMap<String, Decimal>>
where
    F: Fn(&str) -> Option<GuardrailPriceLookup>,
{
    let regional_key = aws_region_name
        .filter(|region| !region.is_empty())
        .map(|region| format!("bedrock/{region}/guardrails"));
    let candidate_keys = regional_key
        .iter()
        .map(String::as_str)
        .chain(once(BEDROCK_GUARDRAIL_PRICING_KEY));
    for key in candidate_keys {
        match lookup(key) {
            Some(GuardrailPriceLookup::Usable(prices)) => return Some(prices),
            Some(GuardrailPriceLookup::Malformed) => {
                tracing::warn!(key, "ignoring malformed guardrail pricing entry");
            }
            None => {}
        }
    }
    None
}

// None distinguishes a counter with no rate from one priced at an explicit zero, which is free.
pub fn bedrock_guardrail_cost_by_unit<F>(
    lookup: F,
    aws_region_name: Option<&str>,
    usage_units: &BTreeMap<String, i64>,
) -> Result<Option<BTreeMap<String, Option<Decimal>>>, GuardrailPricingError>
where
    F: Fn(&str) -> Option<GuardrailPriceLookup>,
{
    let Some(pricing) = bedrock_guardrail_pricing(lookup, aws_region_name) else {
        return Ok(None);
    };
    let mut costs = BTreeMap::new();
    for (counter, units) in usage_units {
        let cost = match pricing.get(counter) {
            Some(rate) => {
                let scaled = Decimal::from(*units)
                    .checked_mul(*rate)
                    .ok_or(GuardrailPricingError::CostOverflow)?;
                Some(scaled)
            }
            None => None,
        };
        costs.insert(counter.clone(), cost);
    }
    Ok(Some(costs))
}

// Unknown-priced counters are skipped by flatten here; the rollup keeps them unknown rather than zero.
pub fn guardrail_cost_total(
    cost_by_unit: Option<&BTreeMap<String, Option<Decimal>>>,
) -> Result<Decimal, GuardrailPricingError> {
    let mut total = Decimal::ZERO;
    for cost in cost_by_unit
        .into_iter()
        .flat_map(BTreeMap::values)
        .flatten()
    {
        total = total
            .checked_add(*cost)
            .ok_or(GuardrailPricingError::CostOverflow)?;
    }
    Ok(total)
}

pub const AZURE_TEXT_RECORD_UNIT: &str = "text_records";

// cost_tier=="free" bills zero; a missing price bills None instead of zero, so usage-only stays distinguishable.
pub fn azure_prompt_shield_cost(
    usage_units: &BTreeMap<String, i64>,
    cost_tier: Option<&str>,
    price_per_1000_text_records: Option<Decimal>,
) -> Result<Option<Decimal>, GuardrailPricingError> {
    if cost_tier == Some("free") {
        return Ok(Some(Decimal::ZERO));
    }
    let Some(price) = price_per_1000_text_records else {
        return Ok(None);
    };
    let count = usage_units
        .get(AZURE_TEXT_RECORD_UNIT)
        .copied()
        .unwrap_or(0);
    let total = Decimal::from(count)
        .checked_mul(price)
        .ok_or(GuardrailPricingError::CostOverflow)?;
    let scaled = total
        .checked_div(Decimal::from(1_000_i64))
        .ok_or(GuardrailPricingError::CostOverflow)?;
    Ok(Some(scaled))
}

mod tests;
