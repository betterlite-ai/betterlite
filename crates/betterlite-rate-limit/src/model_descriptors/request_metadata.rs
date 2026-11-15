use std::collections::BTreeMap;

use betterlite_auth::auth_limit_inputs::key_limits::{
    ModelRateLimitKey, get_key_own_model_rate_limit,
};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;

use crate::limiter_records::RateLimitedModel;

use super::{ModelDescriptorBuilder, ModelDescriptorsError};

impl ModelDescriptorBuilder {
    // ast-grep-ignore: no-long-names
    pub fn key_owns_tpm_from_request_metadata(
        request_metadata: &BTreeMap<String, JsonData>,
        model: Option<&RateLimitedModel>,
    ) -> Result<bool, ModelDescriptorsError> {
        let Some(model) = model else {
            return Ok(false);
        };
        let mut fields = BTreeMap::new();
        fields.insert(
            "metadata".to_owned(),
            request_metadata_value(request_metadata.get("user_api_key_metadata")),
        );
        fields.insert(
            "model_max_budget".to_owned(),
            request_metadata_value(request_metadata.get("user_api_key_model_max_budget")),
        );
        let key_view = UserAPIKeyAuth::construct(AuthInput::Dict(fields))?;
        Ok(model
            .limit_in(get_key_own_model_rate_limit(&key_view, ModelRateLimitKey::Tpm).as_ref())
            .is_some())
    }
}

fn request_metadata_value(value: Option<&JsonData>) -> JsonData {
    match value {
        None | Some(JsonData::Null) => JsonData::Object(BTreeMap::new()),
        Some(value) => value.clone(),
    }
}
