use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;

use crate::api::summary_execution::roundtrip::convert;

// only a nonempty string end-user id travels as the top-level user kwarg
#[must_use]
pub fn end_user(metadata: Option<&Object>) -> Option<&str> {
    metadata?
        .get("user_api_key_end_user_id")
        .and_then(Json::as_str)
        .filter(|id| !id.is_empty())
}

// the router reads allowed_model_region from the principal's serde form, and any non-null region travels as sent
pub fn region_of(principal: &UserAPIKeyAuth) -> Result<Option<String>, serde_json::Error> {
    let fields: PrincipalRegion = convert(principal)?;
    Ok(fields.allowed_model_region)
}

#[derive(Debug, Deserialize)]
struct PrincipalRegion {
    #[serde(default)]
    allowed_model_region: Option<String>,
}
