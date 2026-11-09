use betterlite_models::decisions::Json;
use opentelemetry::trace::{SpanId, TraceId};
use opentelemetry::{KeyValue, StringValue, Value};
use serde_json::json;

use crate::helpers::HelperError;

pub(crate) fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

pub(crate) fn parse_trace(text: &str) -> Result<TraceId, Box<HelperError>> {
    match TraceId::from_hex(text) {
        Ok(trace_id) => Ok(trace_id),
        Err(error) => Err(invalid(&error.to_string())),
    }
}

pub(crate) fn parse_span(text: &str) -> Result<SpanId, Box<HelperError>> {
    match SpanId::from_hex(text) {
        Ok(span_id) => Ok(span_id),
        Err(error) => Err(invalid(&error.to_string())),
    }
}

pub(crate) fn key_values(attributes: &[(String, Json)]) -> Vec<KeyValue> {
    attributes
        .iter()
        .map(|(key, value)| KeyValue::new(key.clone(), value_of(value)))
        .collect()
}

pub(crate) fn pairs(attributes: &[KeyValue]) -> serde_json::Value {
    serde_json::Value::Array(
        attributes
            .iter()
            .map(|pair| json!([pair.key.as_str(), attribute_value(&pair.value)]))
            .collect(),
    )
}

fn value_of(value: &Json) -> Value {
    if let Json::Bool(flag) = value {
        return Value::Bool(*flag);
    }
    if let Json::Number(number) = value {
        return number
            .as_i64()
            .map_or_else(|| Value::from(number.to_string()), Value::I64);
    }
    if let Json::Text(text) = value {
        return Value::from(text.clone());
    }
    Value::from(serde_json::to_string(value).unwrap_or_else(|_error| String::new()))
}

fn attribute_value(value: &Value) -> serde_json::Value {
    match value {
        Value::Bool(flag) => json!(flag),
        Value::I64(number) => json!(number),
        Value::F64(number) => json!(number),
        Value::String(text) => json!(text.as_str()),
        Value::Array(array) => array_value(array),
        _ => serde_json::Value::Null,
    }
}

fn array_value(array: &opentelemetry::Array) -> serde_json::Value {
    if let opentelemetry::Array::String(items) = array {
        return json!(items.iter().map(StringValue::as_str).collect::<Vec<_>>());
    }
    if let opentelemetry::Array::Bool(items) = array {
        return json!(items);
    }
    if let opentelemetry::Array::I64(items) = array {
        return json!(items);
    }
    if let opentelemetry::Array::F64(items) = array {
        return json!(items);
    }
    serde_json::Value::Null
}
