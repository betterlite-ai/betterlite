use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::request_destination::RequestData;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::internal;
use crate::pre_call::failure::PreCallFailure;

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

pub(super) fn to_request_data(data: &JsonMap) -> RequestData {
    data.iter()
        .map(|(name, value)| (name.clone(), to_decision(value)))
        .collect()
}

// Json is serde-untagged over the same JSON values, so its Serialize impl is the one conversion to the wire types
pub(super) fn from_request_data(data: RequestData) -> Result<JsonMap, PreCallFailure> {
    let mut converted = JsonMap::new();
    for (name, value) in data {
        let wire = serde_json::to_value(value).map_err(|error| internal(error.to_string()))?;
        converted.insert(name, wire);
    }
    Ok(converted)
}
