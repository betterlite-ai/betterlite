use std::collections::BTreeSet;

use rust_decimal::Decimal;
use serde_json::Number;
use unicode_general_category::{GeneralCategory, get_general_category};

use super::custom_fields::CUSTOM_PRICING_FIELDS;
use super::shared_fields::SHARED_MODEL_INFO_FIELDS;
use crate::decisions::Json;
use crate::model_registration::JsonObject;

pub const DEPLOYMENT_SCOPED_FIELDS: &[&str] = &["off_peak_pricing"];
pub const PRICING_FIELD_EXEMPTIONS: &[&str] = &["output_vector_size"];
pub const PRICING_OVERRIDES_KEY: &str = "pricing_overrides";
pub const COST_MAP_LOOKUP_KEY: &str = "key";

#[must_use]
pub fn strip_custom_pricing_fields(model_info: &JsonObject) -> JsonObject {
    model_info
        .iter()
        .filter(|(key, _value)| !CUSTOM_PRICING_FIELDS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

#[must_use]
pub fn shared_backend_model_info(model_info: &JsonObject) -> JsonObject {
    model_info
        .iter()
        .filter(|(key, _value)| SHARED_MODEL_INFO_FIELDS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

#[must_use]
pub fn is_server_derived_pricing_key(key: &str) -> bool {
    if CUSTOM_PRICING_FIELDS.contains(&key) && !PRICING_FIELD_EXEMPTIONS.contains(&key) {
        return true;
    }
    is_above_threshold_key(key)
}

// The cost map's tiered rates end in _above_<N>_tokens or _above_<N>k_tokens, with N in any script's decimal digits.
fn is_above_threshold_key(key: &str) -> bool {
    let Some(with_suffix_k) = key.strip_suffix("_tokens") else {
        return false;
    };
    let number = with_suffix_k.strip_suffix('k').unwrap_or(with_suffix_k);
    let before_digits = number.trim_end_matches(|character: char| {
        get_general_category(character) == GeneralCategory::DecimalNumber
    });
    before_digits.len() < number.len() && before_digits.ends_with("_above_")
}

#[must_use]
pub fn without_server_derived_pricing(model_info: &JsonObject) -> JsonObject {
    model_info
        .iter()
        .filter(|(key, _value)| {
            key.as_str() != PRICING_OVERRIDES_KEY && !is_server_derived_pricing_key(key)
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

#[must_use]
pub fn echoed_cost_map_pricing_fields(model_info: &JsonObject) -> Vec<String> {
    if !model_info.contains_key(COST_MAP_LOOKUP_KEY) {
        return Vec::new();
    }
    let fields: BTreeSet<&String> = model_info
        .keys()
        .filter(|key| is_server_derived_pricing_key(key))
        .collect();
    fields.into_iter().cloned().collect()
}

#[must_use]
pub fn echoed_cost_map_fields(
    model_info: &JsonObject,
    cost_map_entries: &[&JsonObject],
) -> Vec<String> {
    if !model_info.contains_key(COST_MAP_LOOKUP_KEY) {
        return Vec::new();
    }
    let fields: BTreeSet<&String> = model_info
        .iter()
        .filter(|(key, value)| {
            cost_map_entries.iter().any(|entry| {
                entry
                    .get(*key)
                    .is_some_and(|found| same_value(found, value))
            })
        })
        .map(|(key, _value)| key)
        .collect();
    fields.into_iter().cloned().collect()
}

#[must_use]
pub fn pricing_override_fields(sources: &[&JsonObject]) -> Vec<String> {
    let fields: BTreeSet<&String> = sources
        .iter()
        .flat_map(|source| source.iter())
        .filter(|(key, value)| !matches!(value, Json::Null) && is_server_derived_pricing_key(key))
        .map(|(key, _value)| key)
        .collect();
    fields.into_iter().cloned().collect()
}

// JSON numbers compare by value, as Python compares 1 and 1.0, so a rewritten price still reads as the cost map's.
fn same_value(left: &Json, right: &Json) -> bool {
    match (left, right) {
        (Json::Number(left_number), Json::Number(right_number)) => {
            same_number(left_number, right_number)
        }
        (Json::List(left_items), Json::List(right_items)) => {
            left_items.len() == right_items.len()
                && left_items
                    .iter()
                    .zip(right_items)
                    .all(|(left_item, right_item)| same_value(left_item, right_item))
        }
        (Json::Object(left_fields), Json::Object(right_fields)) => {
            left_fields.len() == right_fields.len()
                && left_fields.iter().all(|(key, left_field)| {
                    right_fields
                        .get(key)
                        .is_some_and(|right_field| same_value(left_field, right_field))
                })
        }
        _ => left == right,
    }
}

fn same_number(left: &Number, right: &Number) -> bool {
    match (decimal_of(left), decimal_of(right)) {
        (Some(left_value), Some(right_value)) => left_value == right_value,
        _ => left == right,
    }
}

fn decimal_of(number: &Number) -> Option<Decimal> {
    let text = number.to_string();
    text.parse::<Decimal>()
        .or_else(|_error| Decimal::from_scientific(&text))
        .ok()
}
