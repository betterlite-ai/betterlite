use betterlite_models::decisions::Json as ContractJson;
use betterlite_prompts::arguments::json::Json;

use super::error::TransformError;

// B569 and B2201 read and write the decisions JSON, while the Messages request body uses the prompts JSON.
pub(crate) fn to_contract(value: &Json) -> ContractJson {
    match value {
        Json::Null => ContractJson::Null,
        Json::Bool(flag) => ContractJson::Bool(*flag),
        Json::Number(number) => ContractJson::Number(number.clone()),
        Json::Text(text) => ContractJson::Text(text.clone()),
        Json::List(items) => ContractJson::List(items.iter().map(to_contract).collect()),
        Json::Object(fields) => ContractJson::Object(
            fields
                .iter()
                .map(|(key, nested)| (key.clone(), to_contract(nested)))
                .collect(),
        ),
    }
}

// serde already knows both shapes, so the reverse direction is its own round trip.
pub(crate) fn from_contract(value: &ContractJson) -> Result<Json, TransformError> {
    serde_json::to_string(value)
        .and_then(|encoded| serde_json::from_str(&encoded))
        .map_err(TransformError::from)
}
