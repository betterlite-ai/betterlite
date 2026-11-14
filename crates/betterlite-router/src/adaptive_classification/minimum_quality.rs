use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::adaptive_classification::constants::{
    MIN_QUALITY_TIER_HEADER, MIN_QUALITY_TIER_METADATA_KEY,
};

#[must_use]
pub fn minimum_quality_tier(headers: Option<&Json>, metadata: Option<&Json>) -> Option<i64> {
    if let Some(Json::Object(fields)) = headers
        && let Some(value) = header_value(fields)
    {
        return json_integer(value);
    }
    let raw = match metadata {
        Some(Json::Object(fields)) => fields.get(MIN_QUALITY_TIER_METADATA_KEY),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    };
    raw.and_then(json_integer)
}

fn header_value(fields: &IndexMap<String, Json>) -> Option<&Json> {
    fields
        .iter()
        .find(|(name, _value)| name.eq_ignore_ascii_case(MIN_QUALITY_TIER_HEADER))
        .map(|(_name, value)| value)
}

fn json_integer(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => number_integer(number),
        Json::Text(text) => text_integer(text),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn number_integer(number: &Number) -> Option<i64> {
    if let Some(integer) = number.as_i64() {
        return Some(integer);
    }
    if let Some(whole) = number.as_u64() {
        return Some(i64::try_from(whole).unwrap_or(i64::MAX));
    }
    number.as_f64().and_then(whole_float)
}

// a value past i64 keeps its sign, so a tier comparison keeps its result
fn whole_float(value: f64) -> Option<i64> {
    if !value.is_finite() {
        return None;
    }
    let text = format!("{:.0}", value.trunc());
    match text.parse::<i64>() {
        Ok(integer) => Some(integer),
        Err(_source) => Some(if value.is_sign_negative() {
            i64::MIN
        } else {
            i64::MAX
        }),
    }
}

fn text_integer(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    match trimmed.parse::<i64>() {
        Ok(integer) => Some(integer),
        Err(_source) => Some(if trimmed.starts_with('-') {
            i64::MIN
        } else {
            i64::MAX
        }),
    }
}
