use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::{
    Object, StreamItem, blocked_usage::OriginalResponse,
};
use serde_json::Value;

#[must_use]
pub fn blocked_original_response(original_response: Option<&Value>) -> OriginalResponse {
    match original_response {
        Some(Value::Array(items)) => OriginalResponse::Stream(
            items
                .iter()
                .map(|item| match item {
                    Value::String(text) => StreamItem::Bytes(text.as_bytes().to_vec()),
                    _ => StreamItem::Json(to_guardrail(item)),
                })
                .collect(),
        ),
        Some(Value::Object(fields)) => OriginalResponse::Response(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), to_guardrail(value)))
                .collect::<Object>(),
        ),
        Some(Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)) | None => {
            OriginalResponse::Absent
        }
    }
}

pub(crate) fn to_guardrail(input: &Value) -> GuardrailValue {
    match input {
        Value::Null => GuardrailValue::Null,
        Value::Bool(flag) => GuardrailValue::Boolean(*flag),
        Value::Number(number) => number.as_i64().map_or_else(
            || {
                number.as_u64().map_or_else(
                    || GuardrailValue::Decimal(number.as_f64().unwrap_or_default()),
                    GuardrailValue::UnsignedInteger,
                )
            },
            GuardrailValue::SignedInteger,
        ),
        Value::String(text) => GuardrailValue::Text(text.clone()),
        Value::Array(items) => GuardrailValue::Array(items.iter().map(to_guardrail).collect()),
        Value::Object(fields) => GuardrailValue::Object(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), to_guardrail(value)))
                .collect(),
        ),
    }
}
