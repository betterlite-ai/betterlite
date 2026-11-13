#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::settings::JsonValue;
use betterlite_guardrails::guardrail_message_scope::{Object, StreamItem};
use std::str::from_utf8;

// serde_json keeps integers as text here, so a plain parse into GuardrailValue would read them as objects.
pub(super) fn guardrail(value: JsonValue) -> GuardrailValue {
    match value {
        JsonValue::Null => GuardrailValue::Null,
        JsonValue::Bool(flag) => GuardrailValue::Boolean(flag),
        JsonValue::Number(number) => number
            .as_i64()
            .map(GuardrailValue::SignedInteger)
            .or_else(|| number.as_u64().map(GuardrailValue::UnsignedInteger))
            .or_else(|| number.as_f64().map(GuardrailValue::Decimal))
            .unwrap_or(GuardrailValue::Null),
        JsonValue::String(text) => GuardrailValue::Text(text),
        JsonValue::Array(items) => {
            GuardrailValue::Array(items.into_iter().map(guardrail).collect())
        }
        JsonValue::Object(fields) => GuardrailValue::Object(
            fields
                .into_iter()
                .map(|(key, item)| (key, guardrail(item)))
                .collect(),
        ),
    }
}

// Tests write JSON as text because the json! macro trips the disallowed-type and numeric-fallback lints.
pub(super) fn json(source: &str) -> JsonValue {
    match serde_json::from_str(source) {
        Ok(value) => value,
        Err(error) => panic!("invalid test JSON {source}: {error}"),
    }
}

pub(super) fn object(value: JsonValue) -> Object {
    let GuardrailValue::Object(fields) = guardrail(value) else {
        return Object::new();
    };
    fields
}

pub(super) fn values(value: JsonValue) -> Vec<GuardrailValue> {
    let GuardrailValue::Array(items) = guardrail(value) else {
        return Vec::new();
    };
    items
}

// The JSON a "data: ...\n\n" frame carries.
pub(super) fn frame_json(frame: &[u8]) -> JsonValue {
    let text = from_utf8(frame).unwrap();
    let body = text
        .strip_prefix("data: ")
        .unwrap()
        .strip_suffix("\n\n")
        .unwrap();
    serde_json::from_str(body).unwrap()
}

pub(super) fn stream_items(chunks: Vec<JsonValue>) -> Vec<StreamItem> {
    chunks
        .into_iter()
        .map(|chunk| StreamItem::Json(guardrail(chunk)))
        .collect()
}
