use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;

// Stored rows key JSON with BTreeMap; the deployment builders carry LiteLLM's insertion-ordered Json.
#[must_use]
pub(crate) fn json_of(value: &JsonData) -> Json {
    match value {
        JsonData::Null => Json::Null,
        JsonData::Bool(flag) => Json::Bool(*flag),
        JsonData::Number(number) => Json::Number(number.clone()),
        JsonData::String(text) => Json::Text(text.clone()),
        JsonData::Array(items) => Json::List(items.iter().map(json_of).collect()),
        JsonData::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), json_of(item)))
                .collect(),
        ),
    }
}
