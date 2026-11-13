use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::args::RawBody;

const ORIGIN_KEY: &str = "internal_call_origin";
const CLASSIFIER_ORIGIN: &str = "autorouter_classifier";
const CLASSIFIER_CALL_TYPES: [&str; 4] = ["completion", "acompletion", "responses", "aresponses"];
const SDK_ONLY_KEYS: [&str; 3] = ["extra_headers", "extra_query", "extra_body"];

#[must_use]
pub fn is_classifier_call(call_type: &str, litellm_params: &IndexMap<String, Json>) -> bool {
    CLASSIFIER_CALL_TYPES.contains(&call_type)
        && ["metadata", "litellm_metadata"].iter().any(|slot| {
            matches!(
                litellm_params.get(*slot),
                Some(Json::Object(metadata))
                    if metadata.get(ORIGIN_KEY) == Some(&Json::Text(CLASSIFIER_ORIGIN.to_owned()))
            )
        })
}

#[must_use]
pub fn classifier_input_snapshot(
    value: &RawBody,
    openai_sdk: bool,
) -> Option<IndexMap<String, Json>> {
    match value {
        RawBody::Object(fields) if openai_sdk => {
            let mut merged: IndexMap<String, Json> = fields
                .iter()
                .filter(|(name, _)| !SDK_ONLY_KEYS.contains(&name.as_str()))
                .map(|(name, item)| (name.clone(), item.clone()))
                .collect();
            if let Some(Json::Object(extra_body)) = fields.get("extra_body") {
                merged.extend(extra_body.clone());
            }
            Some(merged)
        }
        RawBody::Object(fields) => Some(fields.clone()),
        RawBody::Text(text) => match serde_json::from_str::<Json>(text) {
            Ok(Json::Object(fields)) => Some(fields),
            Ok(_) | Err(_) => None,
        },
    }
}
