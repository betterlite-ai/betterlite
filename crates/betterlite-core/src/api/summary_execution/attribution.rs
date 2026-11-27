use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;

// only a nonempty string end-user id travels as the top-level user kwarg
#[must_use]
pub fn end_user(metadata: Option<&Object>) -> Option<&str> {
    metadata?
        .get("user_api_key_end_user_id")
        .and_then(Json::as_str)
        .filter(|id| !id.is_empty())
}

// the router reads allowed_model_region from the principal's serde form, and any non-null region travels as sent
#[derive(Debug, Deserialize)]
pub(crate) struct PrincipalRegion {
    #[serde(default)]
    pub(crate) allowed_model_region: Option<String>,
}
