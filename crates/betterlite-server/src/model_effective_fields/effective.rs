use betterlite_db::stored_cipher::DbCipher;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::decrypted_model;

const CONFIG_FIELD: &str = "complexity_router_config";
const JEV_CONFIG_FIELD: &str = "jev_classifier_config";

// the model a write leaves on the row, decrypting the stored one only when the patch omits it
#[must_use]
pub fn effective_model(
    cipher: &DbCipher,
    incoming: Option<&JsonObject>,
    existing: Option<&JsonObject>,
) -> Option<String> {
    if let Some(Json::Text(model)) = incoming.and_then(|params| params.get("model")) {
        return Some(model.clone());
    }
    decrypted_model(cipher, existing.and_then(|params| params.get("model")))
}

#[must_use]
pub fn effective_complexity_config(
    incoming: Option<&JsonObject>,
    existing: Option<&JsonObject>,
) -> Option<Json> {
    let stored = existing
        .and_then(|params| params.get(CONFIG_FIELD))
        .filter(|config| !matches!(config, Json::Null));
    let Some(supplied) = incoming
        .and_then(|params| params.get(CONFIG_FIELD))
        .filter(|config| !matches!(config, Json::Null))
    else {
        return stored.cloned();
    };
    if let (Json::Object(supplied_config), Some(Json::Object(stored_config))) = (supplied, stored)
        && let Some(merged) = jev_transport_merge(supplied_config, stored_config)
    {
        return Some(merged);
    }
    Some(supplied.clone())
}

// a jev config written without its transport keeps the stored api_base, and the stored api_key while the base is unchanged
fn jev_transport_merge(supplied: &JsonObject, stored: &JsonObject) -> Option<Json> {
    let jev = |config: &JsonObject| matches!(config.get("classifier_type"), Some(Json::Text(kind)) if kind == "jev");
    if !jev(supplied) || !jev(stored) {
        return None;
    }
    let (Some(Json::Object(supplied_jev)), Some(Json::Object(stored_jev))) =
        (supplied.get(JEV_CONFIG_FIELD), stored.get(JEV_CONFIG_FIELD))
    else {
        return None;
    };
    // LiteLLM compares with ==, so an explicit null base matches a base the row never stored
    let same_base = !supplied_jev.contains_key("api_base")
        || supplied_jev
            .get("api_base")
            .filter(|base| !matches!(base, Json::Null))
            == stored_jev
                .get("api_base")
                .filter(|base| !matches!(base, Json::Null));
    let mut merged_jev = JsonObject::new();
    for (key, value) in stored_jev {
        if key == "api_base" || (key == "api_key" && same_base) {
            merged_jev.insert(key.clone(), value.clone());
        }
    }
    for (key, value) in supplied_jev {
        merged_jev.insert(key.clone(), value.clone());
    }
    let mut merged = supplied.clone();
    merged.insert(JEV_CONFIG_FIELD.to_owned(), Json::Object(merged_jev));
    Some(Json::Object(merged))
}
