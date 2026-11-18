use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::as_object;

pub const ACCESS_GROUP_METADATA_KEY: &str = "user_api_key_matched_model_access_groups";

// The sources get_request_model_access_groups reads: the standard payload first, then the request metadata mappings.
#[derive(Clone, Copy, Debug, Default)]
pub struct AccessGroupSources<'a> {
    pub standard_logging_object: Option<&'a JsonObject>,
    pub litellm_params: Option<&'a JsonObject>,
    // matched_model_access_groups on the live auth object, which no JSON mapping carries.
    pub auth_groups: Option<&'a Json>,
}

#[must_use]
pub fn request_model_access_groups(sources: &AccessGroupSources<'_>) -> Vec<String> {
    if let Some(payload) = sources.standard_logging_object {
        let from_payload = coerce_model_access_groups(payload.get("request_model_access_groups"));
        if !from_payload.is_empty() {
            return from_payload;
        }
    }
    let Some(params) = sources.litellm_params else {
        return Vec::new();
    };
    for name in ["metadata", "litellm_metadata"] {
        let Some(metadata) = params.get(name).and_then(as_object) else {
            continue;
        };
        let groups = groups_from_metadata(metadata, sources.auth_groups);
        if !groups.is_empty() {
            return groups;
        }
    }
    Vec::new()
}

// coerce_model_access_groups: texts out of a list, nonempty and deduped, first occurrence kept.
#[must_use]
pub fn coerce_model_access_groups(value: Option<&Json>) -> Vec<String> {
    let Some(Json::List(items)) = value else {
        return Vec::new();
    };
    let mut groups: Vec<String> = Vec::new();
    for item in items {
        let Json::Text(group) = item else {
            continue;
        };
        if group.is_empty() || groups.iter().any(|known| known == group) {
            continue;
        }
        groups.push(group.clone());
    }
    groups
}

// _model_access_groups_from_metadata: the stamped key wins over the auth object's attribute.
fn groups_from_metadata(metadata: &JsonObject, auth_groups: Option<&Json>) -> Vec<String> {
    let stamped = coerce_model_access_groups(metadata.get(ACCESS_GROUP_METADATA_KEY));
    if !stamped.is_empty() {
        return stamped;
    }
    let on_auth_object = match metadata.get("user_api_key_auth") {
        Some(Json::Object(auth)) => auth.get("matched_model_access_groups"),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => auth_groups,
    };
    coerce_model_access_groups(on_auth_object)
}

#[path = "access_groups/tests.rs"]
mod tests;
