use opentelemetry::trace::SpanContext;
use opentelemetry::{Array, KeyValue, StringValue, Value};
use serde_json::json;

pub(crate) fn key_values(attributes: &[(String, String)]) -> Vec<KeyValue> {
    attributes
        .iter()
        .map(|(key, value)| KeyValue::new(key.clone(), value.clone()))
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

fn attribute_value(value: &Value) -> serde_json::Value {
    match value {
        Value::Bool(flag) => json!(flag),
        Value::I64(number) => json!(number),
        Value::String(text) => json!(text.as_str()),
        Value::Array(Array::String(items)) => {
            json!(items.iter().map(StringValue::as_str).collect::<Vec<_>>())
        }
        Value::F64(number) => json!(number),
        Value::Array(_) => json!("unsupported array"),
        _ => json!("unsupported value"),
    }
}

pub(crate) fn context_parts(context: &SpanContext) -> serde_json::Value {
    let valid = context.is_valid();
    json!({
        "valid": valid,
        "trace_id": valid.then(|| context.trace_id().to_string()),
        "span_id": valid.then(|| context.span_id().to_string()),
        "is_remote": context.is_remote(),
        "sampled": context.is_sampled(),
    })
}
