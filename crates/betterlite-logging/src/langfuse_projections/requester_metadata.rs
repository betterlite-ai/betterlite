use betterlite_models::decisions::Json;
use indexmap::IndexMap;

mod tests;

// litellm.integrations.langfuse.log_requester_metadata; requester keys ride nested so the allowlisted copy stays flat
#[must_use]
pub fn log_requester_metadata(metadata: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let requester_metadata = match metadata.get("requester_metadata") {
        Some(Json::Object(fields)) => fields.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => IndexMap::new(),
    };
    let mut returned = IndexMap::new();
    for (key, value) in metadata {
        if !requester_metadata.contains_key(key) {
            returned.insert(key.clone(), value.clone());
        }
    }
    returned.insert(
        "requester_metadata".to_owned(),
        Json::Object(requester_metadata),
    );
    returned
}
