use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

const REQUEST_FIELDS: [&str; 2] = ["metadata", "litellm_metadata"];
const AUTH_FIELD: &str = "user_api_key_auth";

// _token_in_metadata: only an object's user_api_key_auth entry decodes, and an invalid shape is no token
#[must_use]
pub fn token_in_metadata(metadata: &JsonData) -> Option<UserAPIKeyAuth> {
    let JsonData::Object(fields) = metadata else {
        return None;
    };
    let JsonData::Object(token_fields) = fields.get(AUTH_FIELD)? else {
        return None;
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(token_fields.clone())).ok()
}

// _user_api_key_auth_from_request: metadata first, then litellm_metadata, skipping a shape that does not decode
#[must_use]
pub fn user_api_key_auth_from_request(
    request: &BTreeMap<String, JsonData>,
) -> Option<UserAPIKeyAuth> {
    for field in REQUEST_FIELDS {
        if let Some(token) = request.get(field).and_then(token_in_metadata) {
            return Some(token);
        }
    }
    None
}
