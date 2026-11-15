use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// NewSkillRequest.metadata reaches the row as the same JSON, with its keys in the row's sorted order
pub(super) fn encode(metadata: &IndexMap<String, Json>) -> BTreeMap<String, JsonData> {
    metadata
        .iter()
        .map(|(name, value)| (name.clone(), value_data(value)))
        .collect()
}

fn value_data(value: &Json) -> JsonData {
    match value {
        Json::Null => JsonData::Null,
        Json::Bool(flag) => JsonData::Bool(*flag),
        Json::Number(number) => JsonData::Number(number.clone()),
        Json::Text(text) => JsonData::String(text.clone()),
        Json::List(items) => JsonData::Array(items.iter().map(value_data).collect()),
        Json::Object(entries) => JsonData::Object(encode(entries)),
    }
}
