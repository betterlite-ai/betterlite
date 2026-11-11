use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub const USER_API_KEY_AUTH: &str = "user_api_key_auth";
const METADATA_KEYS: [&str; 2] = ["metadata", "litellm_metadata"];

// LiteLLM checks metadata first, then litellm_metadata, and accepts any value other than None
#[must_use]
pub fn user_api_key_auth_from_kwargs(kwargs: &IndexMap<String, Json>) -> Option<&Json> {
    METADATA_KEYS.iter().find_map(|key| match kwargs.get(*key) {
        Some(Json::Object(metadata)) => metadata
            .get(USER_API_KEY_AUTH)
            .filter(|value| !matches!(value, Json::Null)),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    })
}
