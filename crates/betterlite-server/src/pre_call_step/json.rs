use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::request_destination::RequestData;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object as JsonMap;

pub(super) fn to_decision(value: &JsonValue) -> Json {
    match value {
        JsonValue::Null => Json::Null,
        JsonValue::Bool(flag) => Json::Bool(*flag),
        JsonValue::Number(number) => Json::Number(number.clone()),
        JsonValue::String(text) => Json::Text(text.clone()),
        JsonValue::Array(items) => Json::List(items.iter().map(to_decision).collect()),
        JsonValue::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(name, field)| (name.clone(), to_decision(field)))
                .collect(),
        ),
    }
}

pub(super) fn to_wire(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::String(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(to_wire).collect()),
        Json::Object(fields) => JsonValue::Object(
            fields
                .iter()
                .map(|(name, field)| (name.clone(), to_wire(field)))
                .collect(),
        ),
    }
}

pub(super) fn to_request_data(data: &JsonMap) -> RequestData {
    data.iter()
        .map(|(name, value)| (name.clone(), to_decision(value)))
        .collect()
}

pub(super) fn from_request_data(data: RequestData) -> JsonMap {
    data.into_iter()
        .map(|(name, value)| (name, to_wire(&value)))
        .collect()
}
