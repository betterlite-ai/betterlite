use std::collections::BTreeMap;

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
    content_to_text(&content_value(content))
}

// content_to_text reads a string or a list of text parts; every other JSON shape holds no text.
fn content_value(content: &Json) -> GuardrailValue {
    match content {
        Json::Text(text) => GuardrailValue::Text(text.clone()),
        Json::List(parts) => GuardrailValue::Array(parts.iter().filter_map(text_part).collect()),
        _ => GuardrailValue::Null,
    }
}

// LiteLLM joins the {type: "text", text: ...} parts of a list and ignores every other part.
fn text_part(part: &Json) -> Option<GuardrailValue> {
    let Json::Object(fields) = part else {
        return None;
    };
    let Some(Json::Text(text)) = fields.get("text") else {
        return None;
    };
    match fields.get("type") {
        Some(Json::Text(kind)) if kind == "text" => Some(GuardrailValue::Object(BTreeMap::from([
            ("type".to_owned(), GuardrailValue::Text("text".to_owned())),
            ("text".to_owned(), GuardrailValue::Text(text.clone())),
        ]))),
        _ => None,
    }
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
