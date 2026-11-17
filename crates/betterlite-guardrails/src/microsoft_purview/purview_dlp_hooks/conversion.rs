use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

// The prompt and evaluation code reads request data as its own JSON type, so the record values convert once here.
pub(super) fn to_json(value: &GuardrailValue) -> Json {
    match value {
        GuardrailValue::Null => Json::Null,
        GuardrailValue::Boolean(flag) => Json::Bool(*flag),
        GuardrailValue::SignedInteger(number) => Json::Number(Number::from(*number)),
        GuardrailValue::UnsignedInteger(number) => Json::Number(Number::from(*number)),
        GuardrailValue::Decimal(number) => {
            Number::from_f64(*number).map_or(Json::Null, Json::Number)
        }
        GuardrailValue::Text(text) => Json::Text(text.clone()),
        GuardrailValue::Array(items) => Json::List(items.iter().map(to_json).collect()),
        GuardrailValue::Object(fields) => Json::Object(object(fields)),
    }
}

pub(super) fn object(fields: &BTreeMap<String, GuardrailValue>) -> Object {
    fields
        .iter()
        .map(|(key, value)| (key.clone(), to_json(value)))
        .collect()
}
