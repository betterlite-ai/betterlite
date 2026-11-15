use std::collections::BTreeMap;

use betterlite_auth::auth_limit_inputs::key_limits::{
    ModelRateLimitKey, get_key_own_model_rate_limit,
};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;

use super::ModelDescriptorsError;
use crate::limiter_records::RateLimitedModel;

// ast-grep-ignore: no-long-names
pub fn key_owns_tpm_from_request_metadata(
    request_metadata: &BTreeMap<String, JsonData>,
    model: Option<&RateLimitedModel>,
) -> Result<bool, ModelDescriptorsError> {
    let Some(rated) = model else {
        return Ok(false);
    };
    let mut fields = BTreeMap::new();
    for (field, source) in [
        ("metadata", "user_api_key_metadata"),
        ("model_max_budget", "user_api_key_model_max_budget"),
    ] {
        let value = match request_metadata.get(source) {
            None | Some(JsonData::Null) => JsonData::Object(BTreeMap::new()),
            Some(present) => present.clone(),
        };
        fields.insert(field.to_owned(), value);
    }
    let key_view = UserAPIKeyAuth::construct(AuthInput::Dict(fields))?;
    Ok(rated
        .limit_in(get_key_own_model_rate_limit(&key_view, ModelRateLimitKey::Tpm).as_ref())
        .is_some())
}
