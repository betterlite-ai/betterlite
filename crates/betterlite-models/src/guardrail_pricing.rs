use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::IgnoredAny;
use std::collections::BTreeMap;
use std::iter::once;

use crate::tiered_pricing::Scalar;

use coercion::{
    CostMagnitude, CounterRate, coerce_bool_like, coerce_lax_magnitude, coerce_nonnegative_counter,
};

mod coercion;

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

// None means the field is uncoercible, which malforms the whole entry it came from.
fn parse_in_spend_flag(in_spend: Option<&Scalar>) -> Option<bool> {
    match in_spend {
        None | Some(Scalar::Null) => Some(true),
        Some(value) => coerce_bool_like(value),
    }
}

// Absent is a counter or scalar the hook has no price for; Malformed invalidates the whole entry it came from.
enum FieldOutcome<T> {
    Malformed,
    Absent,
    Present(T),
}

// Both billing paths share the object-shape and in_spend check; each still reads only its own field, so a forged map cannot change the other's cost.
fn parse_validated_field<F, T>(
    raw: &GuardrailInformationEntry,
    parse_field: F,
) -> Result<Option<(Option<T>, bool)>, GuardrailPricingError>
where
    F: FnOnce(&GuardrailEntryFields) -> Result<FieldOutcome<T>, GuardrailPricingError>,
{
    let GuardrailInformationEntry::Object(fields) = raw else {
        return Ok(None);
    };
    let field = match parse_field(fields)? {
        FieldOutcome::Malformed => return Ok(None),
        FieldOutcome::Absent => None,
        FieldOutcome::Present(value) => Some(value),
    };
    let Some(in_spend) = parse_in_spend_flag(fields.in_spend.as_ref()) else {
        return Ok(None);
    };
    Ok(Some((field, in_spend)))
}

struct GuardrailCostEntry {
    cost: Option<CostMagnitude>,
    in_spend: bool,
}

fn parse_scalar_cost_field(
    fields: &GuardrailEntryFields,
) -> Result<FieldOutcome<CostMagnitude>, GuardrailPricingError> {
    let present = fields
        .cost
        .as_ref()
        .filter(|value| !matches!(value, Scalar::Null));
    let Some(value) = present else {
        return Ok(FieldOutcome::Absent);
    };
    Ok(coerce_lax_magnitude(value)?.map_or(FieldOutcome::Malformed, FieldOutcome::Present))
}

// Validated apart from the by-unit path below so a forged guardrail_cost_by_unit map cannot change this entry's scalar cost.
fn parse_guardrail_cost_entry(
    raw: &GuardrailInformationEntry,
) -> Result<Option<GuardrailCostEntry>, GuardrailPricingError> {
    let parsed = parse_validated_field(raw, parse_scalar_cost_field)?;
    Ok(parsed.map(|(cost, in_spend)| GuardrailCostEntry { cost, in_spend }))
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

struct GuardrailCostByUnitEntry {
    cost_by_unit: Option<BTreeMap<String, Option<Decimal>>>,
    in_spend: bool,
}

// Parses one counter's rate, or signals that the whole map is malformed (distinct from a single unpriced counter).
enum CounterMapOutcome {
    Priced(Option<Decimal>),
    Malformed,
}

fn coerce_counter_rate(value: &Scalar) -> Result<CounterMapOutcome, GuardrailPricingError> {
    match coerce_nonnegative_counter(value)? {
        CounterRate::Unpriced => Ok(CounterMapOutcome::Priced(None)),
        CounterRate::Priced(decimal) => Ok(CounterMapOutcome::Priced(Some(decimal))),
        CounterRate::Invalid => Ok(CounterMapOutcome::Malformed),
    }
}

fn coerce_counter_map(
    counters: &BTreeMap<String, Scalar>,
) -> Result<Option<BTreeMap<String, Option<Decimal>>>, GuardrailPricingError> {
    let mut parsed = BTreeMap::new();
    for (counter, value) in counters {
        match coerce_counter_rate(value)? {
            CounterMapOutcome::Priced(magnitude) => {
                parsed.insert(counter.clone(), magnitude);
            }
            CounterMapOutcome::Malformed => return Ok(None),
        }
    }
    Ok(Some(parsed))
}

fn parse_cost_by_unit_field(
    fields: &GuardrailEntryFields,
) -> Result<FieldOutcome<BTreeMap<String, Option<Decimal>>>, GuardrailPricingError> {
    match &fields.by_unit {
        None | Some(CounterMapField::Null(())) => Ok(FieldOutcome::Absent),
        Some(CounterMapField::Map(counters)) => Ok(
            coerce_counter_map(counters)?.map_or(FieldOutcome::Malformed, FieldOutcome::Present)
        ),
        Some(CounterMapField::Other(_)) => Ok(FieldOutcome::Malformed),
    }
}

// Validated apart from the scalar entry above so a forged per-counter map can never change that entry's real spend.
fn parse_cost_by_unit_entry(
    raw: &GuardrailInformationEntry,
) -> Result<Option<GuardrailCostByUnitEntry>, GuardrailPricingError> {
    let parsed = parse_validated_field(raw, parse_cost_by_unit_field)?;
    Ok(
        parsed.map(|(cost_by_unit, in_spend)| GuardrailCostByUnitEntry {
            cost_by_unit,
            in_spend,
        }),
    )
}

// None covers an unpriced, report-only or malformed entry alike, plus any counter the hook had no price for.
pub fn billed_guardrail_cost_by_unit(
    raw: &GuardrailInformationEntry,
) -> Result<Option<BTreeMap<String, Option<Decimal>>>, GuardrailPricingError> {
    let entry = parse_cost_by_unit_entry(raw)?;
    Ok(entry
        .filter(|parsed| parsed.in_spend)
        .and_then(|parsed| parsed.cost_by_unit))
}

mod tests;
