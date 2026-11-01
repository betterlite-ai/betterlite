use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::IgnoredAny;
use std::collections::BTreeMap;

use crate::tiered_pricing::Scalar;

use super::GuardrailPricingError;
use super::coercion::{
    CostMagnitude, CounterRate, coerce_bool_like, coerce_lax_magnitude, coerce_nonnegative_counter,
};

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
    field_kind: &str,
    parse_field: F,
) -> Result<Option<(Option<T>, bool)>, GuardrailPricingError>
where
    F: FnOnce(&GuardrailEntryFields) -> Result<FieldOutcome<T>, GuardrailPricingError>,
{
    let GuardrailInformationEntry::Object(fields) = raw else {
        tracing::warn!(field_kind, "ignoring malformed guardrail_information entry");
        return Ok(None);
    };
    let field = match parse_field(fields)? {
        FieldOutcome::Malformed => {
            tracing::warn!(field_kind, "ignoring malformed guardrail_information entry");
            return Ok(None);
        }
        FieldOutcome::Absent => None,
        FieldOutcome::Present(value) => Some(value),
    };
    let Some(in_spend) = parse_in_spend_flag(fields.in_spend.as_ref()) else {
        tracing::warn!(field_kind, "ignoring malformed guardrail_information entry");
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
    let parsed = parse_validated_field(raw, "guardrail cost", parse_scalar_cost_field)?;
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
    let parsed = parse_validated_field(raw, "guardrail cost rollup", parse_cost_by_unit_field)?;
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
