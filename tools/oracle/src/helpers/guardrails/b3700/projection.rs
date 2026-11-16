use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::content_filter_streaming::error::StreamFilterError;
use serde_json::{Map, Value, json};

use betterlite_oracle_helper_api::error::HelperError;

const RECORDS_KEY: &str = "standard_logging_guardrail_information";
// The scan and trim durations differ between the two sides, so the case never compares them
const TIMING_FIELDS: [&str; 3] = ["start_time", "end_time", "duration"];

// The chunk fields LiteLLM's projection reads, with a null for one the chunk leaves out
pub(super) fn chunks(items: &[GuardrailValue]) -> Value {
    let mut projected = Vec::with_capacity(items.len());
    for item in items {
        projected.push(chunk(item));
    }
    Value::Array(projected)
}

fn chunk(item: &GuardrailValue) -> Value {
    let choices = if let Some(GuardrailValue::Array(entries)) = entry(item, "choices") {
        entries.iter().map(choice).collect()
    } else {
        Vec::new()
    };
    json!({ "id": entry(item, "id").map_or(Value::Null, to_json), "choices": choices })
}

fn choice(item: &GuardrailValue) -> Value {
    json!({
        "index": entry(item, "index").map_or(Value::Null, to_json),
        "finish_reason": entry(item, "finish_reason").map_or(Value::Null, to_json),
        "content": delta_content(item),
    })
}

fn delta_content(item: &GuardrailValue) -> Value {
    match entry(item, "delta") {
        Some(GuardrailValue::Object(delta)) => delta.get("content").map_or(Value::Null, to_json),
        _ => Value::Null,
    }
}

fn entry<'item>(item: &'item GuardrailValue, key: &str) -> Option<&'item GuardrailValue> {
    let GuardrailValue::Object(fields) = item else {
        return None;
    };
    fields.get(key)
}

// The first record the hook appended to request_data's metadata bucket, minus its own timings
pub(super) fn record(request_data: &BTreeMap<String, GuardrailValue>) -> Value {
    let Some(metadata) = request_data.get("metadata") else {
        return Value::Null;
    };
    let Value::Object(bucket) = to_json(metadata) else {
        return Value::Null;
    };
    let Some(Value::Array(entries)) = bucket.get(RECORDS_KEY) else {
        return Value::Null;
    };
    let Some(Value::Object(record)) = entries.first() else {
        return Value::Null;
    };
    visible_record(record)
}

fn visible_record(record: &Map<String, Value>) -> Value {
    let mut visible = Map::new();
    for (name, value) in record {
        if TIMING_FIELDS.contains(&name.as_str()) {
            continue;
        }
        visible.insert(name.clone(), without_nulls(value));
    }
    Value::Object(visible)
}

// Python drops a None wherever it sits, so the fields betterlite writes as null leave both sides
fn without_nulls(value: &Value) -> Value {
    if let Value::Object(fields) = value {
        let mut kept = Map::new();
        for (name, field) in fields {
            if !field.is_null() {
                kept.insert(name.clone(), without_nulls(field));
            }
        }
        return Value::Object(kept);
    }
    if let Value::Array(items) = value {
        return Value::Array(
            items
                .iter()
                .filter(|item| !item.is_null())
                .map(without_nulls)
                .collect(),
        );
    }
    value.clone()
}

// LiteLLM's hook raises HTTPException with the block detail; the status is the one it carries
pub(super) fn error(outcome: &Result<(), StreamFilterError>) -> Result<Value, Box<HelperError>> {
    let Some(error) = outcome.as_ref().err() else {
        return Ok(Value::Null);
    };
    match error {
        StreamFilterError::Blocked { detail } => {
            let block_detail = serde_json::to_value(detail)
                .map_err(|source| super::failure("SerializeError", source.to_string()))?;
            let status = Value::from(error.status_code().unwrap_or_default());
            Ok(json!({ "status": status, "detail": without_nulls(&block_detail) }))
        }
        StreamFilterError::Record(source) => Err(super::failure("RecordError", source.to_string())),
        StreamFilterError::Base(source) => Err(super::failure("BaseError", source.to_string())),
    }
}

fn to_json(value: &GuardrailValue) -> Value {
    match value {
        GuardrailValue::Null => Value::Null,
        GuardrailValue::Boolean(flag) => Value::Bool(*flag),
        GuardrailValue::SignedInteger(number) => Value::from(*number),
        GuardrailValue::UnsignedInteger(number) => Value::from(*number),
        GuardrailValue::Decimal(number) => Value::from(*number),
        GuardrailValue::Text(text) => Value::String(text.clone()),
        GuardrailValue::Array(items) => Value::Array(items.iter().map(to_json).collect()),
        GuardrailValue::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(name, field)| (name.clone(), to_json(field)))
                .collect(),
        ),
    }
}
