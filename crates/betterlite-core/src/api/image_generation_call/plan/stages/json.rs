use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use std::collections::BTreeMap;

use self::back::json_back;
use super::super::super::operations::Params;

mod back;

pub(super) fn to_values(params: &IndexMap<String, Json>) -> BTreeMap<String, JsonValue> {
    params
        .iter()
        .map(|(name, value)| (name.clone(), json_value(value)))
        .collect()
}

pub(super) fn from_values(params: BTreeMap<String, JsonValue>) -> Params {
    params
        .into_iter()
        .map(|(name, value)| (name, json_back(value)))
        .collect()
}

fn json_value(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::Text(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(json_value).collect()),
        Json::Object(fields) => JsonValue::Object(
            fields
                .iter()
                .map(|(name, field)| (name.clone(), json_value(field)))
                .collect(),
        ),
    }
}
