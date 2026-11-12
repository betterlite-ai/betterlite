use betterlite_models::decisions::Json;

use super::JsonMap;
use super::source_text::source_text;

const BATCH_RETRIEVE_CALL_TYPES: [&str; 2] = ["aretrieve_batch", "retrieve_batch"];

pub fn success_minute_key<F>(
    call_type: Option<&str>,
    litellm_params: &JsonMap,
    deployment_id_counter: F,
) -> Option<String>
where
    F: FnOnce(&str) -> String,
{
    if call_type.is_some_and(|name| BATCH_RETRIEVE_CALL_TYPES.contains(&name)) {
        return None;
    }
    let Some(Json::Object(metadata)) = litellm_params.get("metadata") else {
        return None;
    };
    if metadata
        .get("model_group")
        .is_none_or(|group| matches!(group, Json::Null))
    {
        return None;
    }
    let Some(Json::Object(model_info)) = litellm_params.get("model_info") else {
        return None;
    };
    let id = model_info.get("id")?;
    if matches!(id, Json::Null) {
        return None;
    }
    Some(deployment_id_counter(source_text(id).as_str()))
}
