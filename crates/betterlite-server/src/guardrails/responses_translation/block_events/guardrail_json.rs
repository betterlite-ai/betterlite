use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::{
    Object,
    blocked_usage::{OriginalResponse, blocked_responses_stream_usage},
};
use serde_json::{Map, Value};

pub fn blocked_original_response(original_response: Option<&Value>) -> OriginalResponse {
    match original_response.and_then(Value::as_object) {
        Some(fields) => OriginalResponse::Response(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), to_guardrail(value)))
                .collect::<Object>(),
        ),
        None => OriginalResponse::Absent,
    }
}

pub(crate) fn to_guardrail(value: &Value) -> GuardrailValue {
    match value {
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

fn from_guardrail(value: &GuardrailValue) -> Value {
    match value {
        GuardrailValue::Null => Value::Null,
        GuardrailValue::Boolean(flag) => Value::Bool(*flag),
        GuardrailValue::SignedInteger(number) => Value::from(*number),
        GuardrailValue::UnsignedInteger(number) => Value::from(*number),
        GuardrailValue::Decimal(number) => Value::from(*number),
        GuardrailValue::Text(text) => Value::String(text.clone()),
        GuardrailValue::Array(items) => Value::Array(items.iter().map(from_guardrail).collect()),
        GuardrailValue::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), from_guardrail(value)))
                .collect(),
        ),
    }
}

pub(crate) fn usage_value(original_response: Option<&Value>) -> Value {
    let usage = blocked_responses_stream_usage(&blocked_original_response(original_response));
    let mut fields = Map::from_iter([
        ("input_tokens".to_owned(), Value::from(usage.input_tokens)),
        ("output_tokens".to_owned(), Value::from(usage.output_tokens)),
        ("total_tokens".to_owned(), Value::from(usage.total_tokens)),
    ]);
    fields.extend(
        usage
            .details
            .iter()
            .map(|(name, value)| (name.clone(), from_guardrail(value))),
    );
    Value::Object(fields)
}
