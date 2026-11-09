use betterlite_models::decisions::Json;
use opentelemetry::{Array, KeyValue, StringValue, Value};
use serde_json::Number;

use super::wire::serialized;

// a JSON reader rounds integers past 2**53 - 1, so Python's v2 carrier sent them as strings and v4 keeps that
const JSON_SAFE_INT: i64 = 0x001f_ffff_ffff_ffff;

// litellm's _flattened_metadata: one attribute per key, or the whole non-dict value under the bare prefix
pub(crate) fn flattened_metadata(prefix: &str, metadata: Option<&Json>) -> Vec<KeyValue> {
    let Some(present) = metadata.filter(|candidate| !matches!(candidate, Json::Null)) else {
        return Vec::new();
    };
    match present {
        Json::Object(fields) => fields
            .iter()
            .filter_map(|(key, field)| {
                metadata_value(field)
                    .map(|converted| KeyValue::new(format!("{prefix}.{key}"), converted))
            })
            .collect(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            let text = serialized(present);
            vec![KeyValue::new(prefix.to_owned(), Value::from(text))]
        }
    }
}

// litellm's _string_sequence: each tag becomes text and a lone value becomes a one-entry list, an empty list becomes no attribute
#[must_use]
pub(crate) fn tags_value(tags: Option<&Json>) -> Option<Value> {
    let present = tags.filter(|candidate| !matches!(candidate, Json::Null))?;
    match present {
        Json::List(items) if items.is_empty() => None,
        Json::List(items) => Some(Value::Array(Array::String(
            items
                .iter()
                .map(serialized)
                .map(StringValue::from)
                .collect(),
        ))),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => Some(
            Value::Array(Array::String(vec![StringValue::from(serialized(present))])),
        ),
    }
}

fn metadata_value(value: &Json) -> Option<Value> {
    match value {
        Json::Null => None,
        Json::Bool(flag) => Some(Value::Bool(*flag)),
        Json::Text(text) => Some(Value::from(text.clone())),
        Json::Number(number) => Some(number_value(number)),
        Json::List(_) | Json::Object(_) => Some(Value::from(serialized(value))),
    }
}

fn number_value(number: &Number) -> Value {
    match number.as_i64() {
        Some(integer) if (-JSON_SAFE_INT..=JSON_SAFE_INT).contains(&integer) => Value::I64(integer),
        _ => Value::from(number.to_string()),
    }
}
