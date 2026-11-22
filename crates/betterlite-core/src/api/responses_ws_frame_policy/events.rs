use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::response_id_codecs::encrypted::wrap_encrypted_content;

#[must_use]
pub fn ws_event_error(event: &IndexMap<String, Json>) -> Option<&Json> {
    if matches!(event.get("type"), Some(Json::Text(kind)) if kind == "error") {
        return event.get("error");
    }
    let Some(Json::Object(response)) = event.get("response") else {
        return None;
    };
    response.get("error")
}

// the caller picks the event types; any event with an item takes the same path
#[must_use]
pub fn wrap_item_encrypted_content(
    event: &IndexMap<String, Json>,
    litellm_metadata: &IndexMap<String, Json>,
) -> Option<IndexMap<String, Json>> {
    if litellm_metadata.get("encrypted_content_affinity_enabled") != Some(&Json::Bool(true)) {
        return None;
    }
    let Some(Json::Object(model_info)) = litellm_metadata.get("model_info") else {
        return None;
    };
    // an empty model id is still a string, so it still wraps
    let Some(Json::Text(model_id)) = model_info.get("id") else {
        return None;
    };
    let Some(Json::Object(item)) = event.get("item") else {
        return None;
    };
    let Some(Json::Text(cipher)) = item.get("encrypted_content") else {
        return None;
    };
    if cipher.is_empty() {
        return None;
    }
    let mut wrapped_item = item.clone();
    wrapped_item.insert(
        "encrypted_content".to_owned(),
        Json::Text(wrap_encrypted_content(cipher, model_id)),
    );
    let mut wrapped = event.clone();
    wrapped.insert("item".to_owned(), Json::Object(wrapped_item));
    Some(wrapped)
}
