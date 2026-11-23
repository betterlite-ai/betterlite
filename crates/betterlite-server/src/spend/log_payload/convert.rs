use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;

// a database column as the ordered JSON the classifier snapshots read
#[must_use]
pub fn column_json(value: &JsonData) -> Json {
    match value {
        JsonData::Null => Json::Null,
        JsonData::Bool(flag) => Json::Bool(*flag),
        JsonData::Number(number) => Json::Number(number.clone()),
        JsonData::String(text) => Json::Text(text.clone()),
        JsonData::Array(items) => Json::List(items.iter().map(column_json).collect()),
        JsonData::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), column_json(item)))
                .collect(),
        ),
    }
}

// decoded JSON back in the row type; a repeated key already kept its last value, as json.loads does
#[must_use]
pub fn row_json(value: Json) -> JsonData {
    match value {
        Json::Null => JsonData::Null,
        Json::Bool(flag) => JsonData::Bool(flag),
        Json::Number(number) => JsonData::Number(number),
        Json::Text(text) => JsonData::String(text),
        Json::List(items) => JsonData::Array(items.into_iter().map(row_json).collect()),
        Json::Object(fields) => JsonData::Object(
            fields
                .into_iter()
                .map(|(key, item)| (key, row_json(item)))
                .collect::<BTreeMap<_, _>>(),
        ),
    }
}
