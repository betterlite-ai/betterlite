use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

pub(super) fn json_to_guardrail_value(value: &Json) -> GuardrailValue {
    match value {
        Json::Null => GuardrailValue::Null,
        Json::Bool(flag) => GuardrailValue::Boolean(*flag),
        Json::Number(number) => number_to_guardrail_value(number),
        Json::Text(text) => GuardrailValue::Text(text.clone()),
        Json::List(items) => json_list_to_guardrail(items),
        Json::Object(fields) => json_object_to_guardrail(fields),
    }
}

fn json_list_to_guardrail(items: &[Json]) -> GuardrailValue {
    let mut converted = Vec::with_capacity(items.len());
    for item in items {
        converted.push(json_to_guardrail_value(item));
    }
    GuardrailValue::Array(converted)
}

fn json_object_to_guardrail(fields: &IndexMap<String, Json>) -> GuardrailValue {
    let mut converted = BTreeMap::new();
    for (key, item) in fields {
        converted.insert(key.clone(), json_to_guardrail_value(item));
    }
    GuardrailValue::Object(converted)
}

fn number_to_guardrail_value(number: &Number) -> GuardrailValue {
    if let Some(value) = number.as_i64() {
        return GuardrailValue::SignedInteger(value);
    }
    if let Some(value) = number.as_u64() {
        return GuardrailValue::UnsignedInteger(value);
    }
    if let Some(value) = number.as_f64() {
        return GuardrailValue::Decimal(value);
    }
    // GuardrailValue has no raw-number variant, so a number no Rust float holds keeps its literal text
    GuardrailValue::Text(number.to_string())
}

pub(super) fn guardrail_value_to_json(value: &GuardrailValue) -> Json {
    match value {
        GuardrailValue::Null => Json::Null,
        GuardrailValue::Boolean(flag) => Json::Bool(*flag),
        GuardrailValue::SignedInteger(number) => Json::Number(Number::from(*number)),
        GuardrailValue::UnsignedInteger(number) => Json::Number(Number::from(*number)),
        GuardrailValue::Decimal(number) => {
            Number::from_f64(*number).map_or(Json::Null, Json::Number)
        }
        GuardrailValue::Text(text) => Json::Text(text.clone()),
        GuardrailValue::Array(items) => {
            Json::List(items.iter().map(guardrail_value_to_json).collect())
        }
        GuardrailValue::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), guardrail_value_to_json(item)))
                .collect(),
        ),
    }
}
