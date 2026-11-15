use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use crate::content_text::content_to_text;
use crate::generic_guardrail_contracts::GuardrailMessage;

// B3798's content pointer takes JSON while the framework's content_to_text takes a guardrail value.
pub(super) fn content_text(content: Option<&Json>) -> String {
    let Some(content) = content else {
        return String::new();
    };
    content_to_text(&guardrail_value(content))
}

// B3798 reads messages as the framework's JSON value, so each typed message converts one for one and keeps its index.
pub(super) fn row(message: &GuardrailMessage) -> Json {
    let mut fields = Object::with_capacity(message.extra.len().saturating_add(1));
    fields.insert("role".to_owned(), Json::Text(message.role.clone()));
    for (key, value) in &message.extra {
        fields.insert(key.clone(), json_value(value));
    }
    Json::Object(fields)
}

fn json_value(value: &GuardrailValue) -> Json {
    match value {
        GuardrailValue::Null => Json::Null,
        GuardrailValue::Boolean(flag) => Json::Bool(*flag),
        GuardrailValue::SignedInteger(number) => Json::Number(Number::from(*number)),
        GuardrailValue::UnsignedInteger(number) => Json::Number(Number::from(*number)),
        GuardrailValue::Decimal(number) => {
            Number::from_f64(*number).map_or(Json::Null, Json::Number)
        }
        GuardrailValue::Text(text) => Json::Text(text.clone()),
        GuardrailValue::Array(items) => Json::List(items.iter().map(json_value).collect()),
        GuardrailValue::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), json_value(value)))
                .collect(),
        ),
    }
}

fn guardrail_value(value: &Json) -> GuardrailValue {
    match value {
        Json::Null => GuardrailValue::Null,
        Json::Bool(flag) => GuardrailValue::Boolean(*flag),
        Json::Number(number) => number_value(number),
        Json::Text(text) => GuardrailValue::Text(text.clone()),
        Json::List(items) => GuardrailValue::Array(items.iter().map(guardrail_value).collect()),
        Json::Object(fields) => GuardrailValue::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), guardrail_value(value)))
                .collect(),
        ),
    }
}

fn number_value(number: &Number) -> GuardrailValue {
    if let Some(signed) = number.as_i64() {
        return GuardrailValue::SignedInteger(signed);
    }
    if let Some(unsigned) = number.as_u64() {
        return GuardrailValue::UnsignedInteger(unsigned);
    }
    number
        .as_f64()
        .map_or(GuardrailValue::Null, GuardrailValue::Decimal)
}
