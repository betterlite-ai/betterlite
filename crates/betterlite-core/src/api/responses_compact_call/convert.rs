use betterlite_context::non_default_params::Params;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};

pub(super) fn object_from_params(params: &Params) -> Object {
    params
        .iter()
        .map(|(name, value)| (name.clone(), prompt_value(value)))
        .collect()
}

pub(super) fn decision_json(value: &JsonValue) -> Json {
    match value {
        JsonValue::Null => Json::Null,
        JsonValue::Bool(flag) => Json::Bool(*flag),
        JsonValue::Number(number) => Json::Number(number.clone()),
        JsonValue::Text(text) => Json::Text(text.clone()),
        JsonValue::Array(items) => Json::List(items.iter().map(decision_json).collect()),
        JsonValue::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), decision_json(item)))
                .collect(),
        ),
    }
}

pub(super) fn json_value(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::Text(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(json_value).collect()),
        Json::Object(fields) => JsonValue::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), json_value(item)))
                .collect(),
        ),
    }
}

pub(super) fn prompt_decision(value: &PromptJson) -> Json {
    match value {
        PromptJson::Null => Json::Null,
        PromptJson::Bool(flag) => Json::Bool(*flag),
        PromptJson::Number(number) => Json::Number(number.clone()),
        PromptJson::Text(text) => Json::Text(text.clone()),
        PromptJson::List(items) => Json::List(items.iter().map(prompt_decision).collect()),
        PromptJson::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), prompt_decision(item)))
                .collect(),
        ),
    }
}

fn prompt_value(value: &JsonValue) -> PromptJson {
    match value {
        JsonValue::Null => PromptJson::Null,
        JsonValue::Bool(flag) => PromptJson::Bool(*flag),
        JsonValue::Number(number) => PromptJson::Number(number.clone()),
        JsonValue::Text(text) => PromptJson::Text(text.clone()),
        JsonValue::Array(items) => PromptJson::List(items.iter().map(prompt_value).collect()),
        JsonValue::Object(fields) => PromptJson::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), prompt_value(item)))
                .collect(),
        ),
    }
}
