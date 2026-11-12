use std::num::FpCategory;
use std::ops::Add;

use betterlite_models::decisions::{Extra, Json};
use serde_json::Number;

use super::error::MergeError;
use super::wrapper::build_prompt_tokens_details;

const DETAILS_FIELD: &str = "prompt_tokens_details";

// LiteLLM's _prompt_tokens_details_as_mapping: model_dump(exclude_none=True), so nulls are gone at every object level
#[must_use]
pub fn details_as_mapping(details: &Extra) -> Extra {
    details
        .iter()
        .filter(|(_, value)| !matches!(value, Json::Null))
        .map(|(name, value)| (name.clone(), without_nulls(value)))
        .collect()
}

fn without_nulls(value: &Json) -> Json {
    match value {
        Json::Object(members) => Json::Object(details_as_mapping(members)),
        Json::List(items) => Json::List(items.iter().map(without_nulls).collect()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => value.clone(),
    }
}

pub fn add_numbers(left: &Number, right: &Number) -> Result<Number, MergeError> {
    let overflow = MergeError::Overflow {
        field: DETAILS_FIELD,
    };
    if let (Some(first), Some(second)) = (left.as_i128(), right.as_i128()) {
        return first
            .checked_add(second)
            .and_then(Number::from_i128)
            .ok_or(overflow);
    }
    let (Some(first), Some(second)) = (left.as_f64(), right.as_f64()) else {
        return Err(overflow);
    };
    Number::from_f64(Add::add(first, second)).ok_or(overflow)
}

// LiteLLM's _merge_prompt_tokens_details: numbers add, nested objects add one level down, anything else keeps the first value that is set
pub fn merge_prompt_tokens_details(
    first: Option<&Extra>,
    second: Option<&Extra>,
) -> Result<Option<Extra>, MergeError> {
    let (left, right) = match (first, second) {
        (None, None) => return Ok(None),
        (None, Some(only)) | (Some(only), None) => return Ok(Some(only.clone())),
        (Some(left), Some(right)) => (left, right),
    };
    let merged = merge_maps(
        &details_as_mapping(left),
        &details_as_mapping(right),
        merge_field,
    )?;
    if merged.is_empty() {
        return Ok(None);
    }
    Ok(build_prompt_tokens_details(merged))
}

fn merge_maps(
    left: &Extra,
    right: &Extra,
    combine: fn(&Json, &Json) -> Result<Json, MergeError>,
) -> Result<Extra, MergeError> {
    let zero = Json::Number(Number::from(0_u8));
    let mut merged = Extra::new();
    let right_only = right.keys().filter(|name| !left.contains_key(*name));
    for name in left.keys().chain(right_only) {
        let first = left.get(name).unwrap_or(&zero);
        let second = right.get(name).unwrap_or(&zero);
        merged.insert(name.clone(), combine(first, second)?);
    }
    Ok(merged)
}

fn merge_field(first: &Json, second: &Json) -> Result<Json, MergeError> {
    match (first, second) {
        (Json::Object(left), Json::Object(right)) => {
            merge_maps(left, right, merge_scalar).map(Json::Object)
        }
        _ => merge_scalar(first, second),
    }
}

fn merge_scalar(first: &Json, second: &Json) -> Result<Json, MergeError> {
    match (first, second) {
        (Json::Number(left), Json::Number(right)) => add_numbers(left, right).map(Json::Number),
        _ => Ok(if is_unset(first) {
            second.clone()
        } else {
            first.clone()
        }),
    }
}

// Python's falsy values: the first side wins unless it is one of these
fn is_unset(value: &Json) -> bool {
    match value {
        Json::Null | Json::Bool(false) => true,
        Json::Bool(true) => false,
        Json::Number(number) => number
            .as_f64()
            .is_some_and(|float| float.classify() == FpCategory::Zero),
        Json::Text(text) => text.is_empty(),
        Json::List(items) => items.is_empty(),
        Json::Object(members) => members.is_empty(),
    }
}
