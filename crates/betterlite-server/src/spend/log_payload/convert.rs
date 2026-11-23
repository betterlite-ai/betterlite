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
