use std::io::Read as _;

use betterlite_logging::datadog::payloads::types::DatadogPayload;
use betterlite_models::decisions::Json;
use flate2::read::GzDecoder;
use http::HeaderMap;
use indexmap::IndexMap;
use serde_json::Value;

// create_dummy_standard_logging_payload sets exactly these keys; the rest of the typed payload stays absent there
const DUMMY_KEYS: [&str; 33] = [
    "id",
    "call_type",
    "stream",
    "response_cost",
    "autorouter_savings",
    "response_cost_failure_debug_info",
    "zero_cost_diagnostic",
    "status",
    "total_tokens",
    "prompt_tokens",
    "completion_tokens",
    "startTime",
    "endTime",
    "completionStartTime",
    "model_map_information",
    "model",
    "model_id",
    "model_group",
    "custom_llm_provider",
    "api_base",
    "metadata",
    "cache_hit",
    "cache_key",
    "saved_cache_cost",
    "request_tags",
    "request_model_access_groups",
    "end_user",
    "requester_ip_address",
    "messages",
    "response",
    "error_str",
    "model_parameters",
    "hidden_params",
];

pub(super) fn body(bytes: &[u8], headers: &HeaderMap, action: &str) -> Value {
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
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| envelope(item, action)).collect())
        }
        other @ (Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Object(_)) => envelope(&other, action),
    }
}

pub(super) fn payload(payload: &DatadogPayload) -> Value {
    match serde_json::to_value(payload) {
        Ok(value) => envelope(&value, ""),
        Err(_error) => Value::Null,
    }
}

pub(super) fn message(payload: &IndexMap<String, Json>) -> Value {
    match serde_json::to_value(payload) {
        Ok(value) => envelope(&value, ""),
        Err(_error) => Value::Null,
    }
}

fn envelope(value: &Value, action: &str) -> Value {
    let mut compacted = compact(value);
    let Some(Value::String(message)) = value.get("message") else {
        return compacted;
    };
    let Ok(parsed) = serde_json::from_str::<Value>(message) else {
        return compacted;
    };
    let reduced = compact(&parsed);
    let normalized = if action == "health_check" {
        project(&reduced)
    } else {
        reduced
    };
    if let Value::Object(fields) = &mut compacted {
        fields.insert("message".to_owned(), normalized);
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

fn project(value: &Value) -> Value {
    let Value::Object(fields) = value else {
        return value.clone();
    };
    let mut projected = serde_json::Map::new();
    for key in DUMMY_KEYS {
        if let Some(field) = fields.get(key) {
            projected.insert(key.to_owned(), field.clone());
        }
    }
    Value::Object(projected)
}
