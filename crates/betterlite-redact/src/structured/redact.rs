use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::{DEFAULT_REDACT_DEPTH, REDACTED, SensitiveDataMasker};

#[must_use]
pub fn mask_sensitive_keys(
    data: &Map<String, Value>,
    sensitive_fields: &BTreeSet<String>,
) -> Map<String, Value> {
    let masker = SensitiveDataMasker::new();
    data.iter()
        .map(|(key, value)| {
            let masked_value = if sensitive_fields.contains(key) && value.is_string() {
                let string = value.as_str().unwrap_or_default();
                let visible = masker.visible_prefix.saturating_add(masker.visible_suffix);
                if string.chars().count() < visible {
                    Value::String(masker.mask_char.to_string().repeat(string.chars().count()))
                } else {
                    Value::String(masker.mask_value(string))
                }
            } else {
                value.clone()
            };
            (key.clone(), masked_value)
        })
        .collect()
}

#[must_use]
pub fn redact_credentials_in_payload(data: &Value) -> Value {
    match data {
        Value::Object(object) => redact_mapping(object, 0),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            data.clone()
        }
    }
}

fn redact_mapping(data: &Map<String, Value>, depth: usize) -> Value {
    Value::Object(
        data.iter()
            .map(|(key, value)| (key.clone(), redact_entry(key, value, depth)))
            .collect(),
    )
}

fn redact_entry(key: &str, value: &Value, depth: usize) -> Value {
    if !value.is_null() && SensitiveDataMasker::new().is_sensitive_key(key, &[]) {
        return Value::String(REDACTED.to_owned());
    }
    match value {
        Value::Object(object) if depth < DEFAULT_REDACT_DEPTH => {
            redact_mapping(object, depth.saturating_add(1))
        }
        Value::Array(values) if depth < DEFAULT_REDACT_DEPTH => Value::Array(
            values
                .iter()
                .map(|item| redact_sequence_entry(item, depth.saturating_add(1)))
                .collect(),
        ),
        Value::Object(_) | Value::Array(_) => Value::String(REDACTED.to_owned()),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
    }
}

fn redact_sequence_entry(value: &Value, depth: usize) -> Value {
    match value {
        Value::Object(object) => redact_mapping(object, depth),
        Value::Array(values) if depth < DEFAULT_REDACT_DEPTH => Value::Array(
            values
                .iter()
                .map(|item| redact_sequence_entry(item, depth.saturating_add(1)))
                .collect(),
        ),
        Value::Array(_) => Value::String(REDACTED.to_owned()),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
    }
}
