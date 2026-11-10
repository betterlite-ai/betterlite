use std::io::Read as _;

use betterlite_logging::datadog::payloads::types::DatadogPayload;
use flate2::read::GzDecoder;
use http::HeaderMap;
use serde_json::Value;

pub(super) fn body(bytes: &[u8], headers: &HeaderMap) -> Value {
    let decoded = match headers.get("content-encoding") {
        Some(value) if value == "gzip" => {
            let mut decoded = Vec::new();
            let _read = GzDecoder::new(bytes).read_to_end(&mut decoded);
            decoded
        }
        _ => bytes.to_vec(),
    };
    let Ok(parsed) = serde_json::from_slice::<Value>(&decoded) else {
        return Value::Null;
    };
    match parsed {
        Value::Array(items) => Value::Array(items.iter().map(envelope).collect()),
        other @ (Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Object(_)) => envelope(&other),
    }
}

pub(super) fn payload(payload: &DatadogPayload) -> Value {
    match serde_json::to_value(payload) {
        Ok(value) => envelope(&value),
        Err(_error) => Value::Null,
    }
}

fn envelope(value: &Value) -> Value {
    let mut compacted = compact(value);
    let Some(Value::String(message)) = value.get("message") else {
        return compacted;
    };
    let Ok(parsed) = serde_json::from_str::<Value>(message) else {
        return compacted;
    };
    if let Value::Object(fields) = &mut compacted {
        fields.insert("message".to_owned(), compact(&parsed));
    }
    compacted
}

fn compact(value: &Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(_name, field)| !field.is_null())
                .map(|(name, field)| (name.clone(), compact(field)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(compact).collect()),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
    }
}
