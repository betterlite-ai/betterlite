use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::IgnoredAny;
use std::collections::BTreeMap;
use std::iter::once;

use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::tiered_pricing::Scalar;

#[derive(Debug, thiserror::Error)]
pub enum GuardrailPricingError {
    #[error("the guardrail cost arithmetic overflowed Decimal's representable range")]
    CostOverflow,
}

// A present, even empty, Usable map still blocks the regional-then-global fallback; only Malformed or a miss falls through.
#[derive(Debug, Clone)]
pub enum GuardrailPriceLookup {
    Usable(BTreeMap<String, Decimal>),
    Malformed,
}

const BEDROCK_GUARDRAIL_PRICING_KEY: &str = "bedrock/guardrails";

// LiteLLM logs a warning and falls through on a malformed entry; here the caller gets Malformed instead of a log line.
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
            Some(GuardrailPriceLookup::Malformed) | None => {}
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
    let Some(costs) = cost_by_unit else {
        return Ok(Decimal::ZERO);
    };
    costs
        .values()
        .flatten()
        .try_fold(Decimal::ZERO, |total, cost| {
            total
                .checked_add(*cost)
                .ok_or(GuardrailPricingError::CostOverflow)
        })
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

// NonFinite exists because Pydantic accepts inf/nan spellings; billing later treats any NonFinite as zero.
#[derive(Debug, Clone, Copy)]
enum CostMagnitude {
    Finite(Decimal),
    NonFinite,
}

fn is_nonfinite_literal(text: &str) -> bool {
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    body.eq_ignore_ascii_case("inf")
        || body.eq_ignore_ascii_case("infinity")
        || body.eq_ignore_ascii_case("nan")
}

fn coerce_finite_lexeme(lexeme: &str) -> Result<Option<CostMagnitude>, GuardrailPricingError> {
    match parse_decimal_lexeme(lexeme) {
        Ok(decimal) => Ok(Some(CostMagnitude::Finite(decimal))),
        Err(_source) => Err(GuardrailPricingError::CostOverflow),
    }
}

fn coerce_lax_magnitude(value: &Scalar) -> Result<Option<CostMagnitude>, GuardrailPricingError> {
    match value {
        Scalar::Bool(flag) => Ok(Some(CostMagnitude::Finite(Decimal::from(u8::from(*flag))))),
        Scalar::Number(number) => coerce_finite_lexeme(&number.to_string()),
        Scalar::Text(text) => {
            let trimmed = text.trim();
            if is_nonfinite_literal(trimmed) {
                Ok(Some(CostMagnitude::NonFinite))
            } else if trimmed.parse::<f64>().is_ok() {
                coerce_finite_lexeme(trimmed)
            } else {
                Ok(None)
            }
        }
        Scalar::Null | Scalar::Array(_) | Scalar::Object(_) => Ok(None),
    }
}

fn coerce_bool_spelling(spelling: &str) -> Option<bool> {
    if spelling == "0"
        || spelling == "f"
        || spelling == "n"
        || spelling == "no"
        || spelling == "off"
        || spelling == "false"
    {
        return Some(false);
    }
    if spelling == "1"
        || spelling == "t"
        || spelling == "y"
        || spelling == "on"
        || spelling == "yes"
        || spelling == "true"
    {
        return Some(true);
    }
    None
}

// Mirrors Pydantic's lax bool validator: a real bool, 0/1, or one of these case-insensitive spellings.
fn coerce_bool_like(value: &Scalar) -> Option<bool> {
    match value {
        Scalar::Bool(flag) => Some(*flag),
        Scalar::Number(number) => match number.as_f64() {
            Some(0.0) => Some(false),
            Some(1.0) => Some(true),
            _ => None,
        },
        Scalar::Text(text) => coerce_bool_spelling(&text.to_ascii_lowercase()),
        Scalar::Null | Scalar::Array(_) | Scalar::Object(_) => None,
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum CounterMapField {
    Null(()),
    Map(BTreeMap<String, Scalar>),
    Other(IgnoredAny),
}

// Unknown fields are ignored by default, matching the source's extra="ignore" models.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GuardrailEntryFields {
    #[serde(rename = "guardrail_cost")]
    cost: Option<Scalar>,
    #[serde(rename = "guardrail_cost_in_spend")]
    in_spend: Option<Scalar>,
    #[serde(rename = "guardrail_cost_by_unit")]
    by_unit: Option<CounterMapField>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum GuardrailInformationEntry {
    Object(GuardrailEntryFields),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum GuardrailInformation {
    Null(()),
    Array(Vec<GuardrailInformationEntry>),
    Entry(GuardrailInformationEntry),
}

struct GuardrailCostEntry {
    cost: Option<CostMagnitude>,
    in_spend: bool,
}

// Validated apart from the by-unit path below so a forged guardrail_cost_by_unit map cannot change this entry's scalar cost.
fn parse_guardrail_cost_entry(
    raw: &GuardrailInformationEntry,
) -> Result<Option<GuardrailCostEntry>, GuardrailPricingError> {
    let GuardrailInformationEntry::Object(fields) = raw else {
        return Ok(None);
    };
    let cost = match &fields.cost {
        None | Some(Scalar::Null) => None,
        Some(value) => match coerce_lax_magnitude(value)? {
            Some(magnitude) => Some(magnitude),
            None => return Ok(None),
        },
    };
    let in_spend = match &fields.in_spend {
        None | Some(Scalar::Null) => true,
        Some(value) => match coerce_bool_like(value) {
            Some(flag) => flag,
            None => return Ok(None),
        },
    };
    Ok(Some(GuardrailCostEntry { cost, in_spend }))
}

fn billable_entry_cost(entry: &GuardrailCostEntry) -> Decimal {
    if !entry.in_spend {
        return Decimal::ZERO;
    }
    match entry.cost {
        Some(CostMagnitude::Finite(cost)) if cost > Decimal::ZERO => cost,
        Some(CostMagnitude::Finite(_) | CostMagnitude::NonFinite) | None => Decimal::ZERO,
    }
}

// A malformed entry or an uncoercible in_spend flag prices to zero by itself, preserving a valid sibling's cost.
fn validated_entry_cost(raw: &GuardrailInformationEntry) -> Result<Decimal, GuardrailPricingError> {
    let entry = parse_guardrail_cost_entry(raw)?;
    Ok(entry.map_or(Decimal::ZERO, |parsed| billable_entry_cost(&parsed)))
}

pub fn guardrail_information_cost(
    guardrail_information: &GuardrailInformation,
) -> Result<Decimal, GuardrailPricingError> {
    match guardrail_information {
        GuardrailInformation::Null(()) => Ok(Decimal::ZERO),
        GuardrailInformation::Array(entries) => {
            entries.iter().try_fold(Decimal::ZERO, |total, entry| {
                total
                    .checked_add(validated_entry_cost(entry)?)
                    .ok_or(GuardrailPricingError::CostOverflow)
            })
        }
        GuardrailInformation::Entry(entry) => validated_entry_cost(entry),
    }
}

enum CounterRate {
    Unpriced,
    Priced(Decimal),
    Invalid,
}

// Any single invalid counter invalidates the whole rollup entry rather than only that counter.
fn coerce_nonnegative_counter(value: &Scalar) -> Result<CounterRate, GuardrailPricingError> {
    if matches!(value, Scalar::Null) {
        return Ok(CounterRate::Unpriced);
    }
    Ok(match coerce_lax_magnitude(value)? {
        Some(CostMagnitude::Finite(decimal)) if !decimal.is_sign_negative() => {
            CounterRate::Priced(decimal)
        }
        Some(CostMagnitude::Finite(_) | CostMagnitude::NonFinite) | None => CounterRate::Invalid,
    })
}

struct GuardrailCostByUnitEntry {
    cost_by_unit: Option<BTreeMap<String, Option<Decimal>>>,
    in_spend: bool,
}

// Validated apart from the scalar entry above so a forged per-counter map can never change that entry's real spend.
fn parse_cost_by_unit_entry(
    raw: &GuardrailInformationEntry,
) -> Result<Option<GuardrailCostByUnitEntry>, GuardrailPricingError> {
    let GuardrailInformationEntry::Object(fields) = raw else {
        return Ok(None);
    };
    let cost_by_unit = match &fields.by_unit {
        None | Some(CounterMapField::Null(())) => None,
        Some(CounterMapField::Map(counters)) => {
            let mut parsed = BTreeMap::new();
            for (counter, value) in counters {
                let magnitude = match coerce_nonnegative_counter(value)? {
                    CounterRate::Unpriced => None,
                    CounterRate::Priced(decimal) => Some(decimal),
                    CounterRate::Invalid => return Ok(None),
                };
                parsed.insert(counter.clone(), magnitude);
            }
            Some(parsed)
        }
        Some(CounterMapField::Other(_)) => return Ok(None),
    };
    let in_spend = match &fields.in_spend {
        None | Some(Scalar::Null) => true,
        Some(value) => match coerce_bool_like(value) {
            Some(flag) => flag,
            None => return Ok(None),
        },
    };
    Ok(Some(GuardrailCostByUnitEntry {
        cost_by_unit,
        in_spend,
    }))
}

// None covers an unpriced, report-only or malformed entry alike, plus any counter the hook had no price for.
pub fn billed_guardrail_cost_by_unit(
    raw: &GuardrailInformationEntry,
) -> Result<Option<BTreeMap<String, Option<Decimal>>>, GuardrailPricingError> {
    let Some(entry) = parse_cost_by_unit_entry(raw)? else {
        return Ok(None);
    };
    Ok(if entry.in_spend {
        entry.cost_by_unit
    } else {
        None
    })
}

mod tests;
