use super::super::utils::coercion::{OtelModelValue, as_int};
use rust_decimal::prelude::ToPrimitive as _;
use std::collections::BTreeMap;

#[must_use]
pub(super) fn float_value(value: &OtelModelValue) -> Option<f64> {
    if let OtelModelValue::Number(number) = value {
        number.as_f64()
    } else {
        value.as_decimal()?.to_f64()
    }
}

#[must_use]
pub fn cache_token_value(values: &[OtelModelValue]) -> Option<i64> {
    let mut saw_zero = false;
    let mut invalid_before_zero = false;
    for raw in values {
        if matches!(raw, OtelModelValue::Null(())) {
            continue;
        }
        let parsed = if matches!(raw, OtelModelValue::Bool(_)) {
            None
        } else {
            as_int(raw)
        };
        match parsed {
            Some(parsed_value) if parsed_value > 0 => return Some(parsed_value),
            Some(0) => saw_zero = true,
            _ if !saw_zero => invalid_before_zero = true,
            _ => {}
        }
    }
    (saw_zero && !invalid_before_zero).then_some(0)
}

#[must_use]
pub fn guardrail_mode_str(value: &OtelModelValue) -> Option<String> {
    if let OtelModelValue::Array(items) = value {
        let mut joined = String::new();
        for item in items {
            let Some(mode) = item.as_str() else {
                continue;
            };
            if !joined.is_empty() {
                joined.push(',');
            }
            joined.push_str(mode);
        }
        return (!joined.is_empty()).then_some(joined);
    }
    if value.as_object().is_some() {
        return json_or_none(value);
    }
    value.as_str().map(str::to_owned)
}

#[must_use]
pub fn total_masked_entities(value: &OtelModelValue) -> Option<i64> {
    value.as_object().map_or_else(
        || as_int(value),
        |items| {
            let total = items.values().fold(0_i64, |sum, item| {
                let count = match item {
                    OtelModelValue::Number(number) => number.as_i64(),
                    OtelModelValue::Bool(boolean) => Some(i64::from(*boolean)),
                    OtelModelValue::Null(())
                    | OtelModelValue::String(_)
                    | OtelModelValue::Array(_)
                    | OtelModelValue::Object(_) => None,
                };
                sum.saturating_add(count.unwrap_or(0))
            });
            (total != 0).then_some(total)
        },
    )
}

#[must_use]
pub fn dicts(value: &OtelModelValue) -> Vec<BTreeMap<String, OtelModelValue>> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for item in items {
        if let Some(fields) = item.as_object() {
            result.push(fields.clone());
        }
    }
    result
}

#[must_use]
pub fn json_or_none(value: &OtelModelValue) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_owned());
    }
    serde_json::to_string(value).ok()
}

#[path = "helpers/tests.rs"]
mod tests;
