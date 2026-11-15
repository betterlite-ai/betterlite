use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::auth_limit_inputs::key_limits::{
    LimitQuery, get_key_model_rpm_limit, get_key_model_tpm_limit,
};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use crate::limiter_records::RateLimitDescriptor;

use super::{
    ModelDescriptorBuilder, ModelDescriptorsError, NONE_TEXT, descriptor, window_rate_limit,
};

impl ModelDescriptorBuilder {
    pub fn key_model_descriptor(
        &self,
        auth: &UserAPIKeyAuth,
        requested_model: Option<&str>,
        out: &mut Vec<RateLimitDescriptor>,
    ) -> Result<(), ModelDescriptorsError> {
        let Some(model) = self.rated_model(requested_model) else {
            return Ok(());
        };
        let query = LimitQuery {
            model_name: Some(model.group.as_str()),
            catalog: None,
        };
        let tokens = model.limit_in(get_key_model_tpm_limit(auth, &query)?.as_ref());
        let requests = model.limit_in(get_key_model_rpm_limit(auth, &query)?.as_ref());
        if requests.is_none() && tokens.is_none() {
            return Ok(());
        }
        let value = format!(
            "{}:{}",
            account_limits(auth).hashed_api_key.unwrap_or(NONE_TEXT),
            model.group
        );
        out.push(descriptor(
            "model_per_key",
            value,
            window_rate_limit(requests, tokens, self.window_size),
        ));
        Ok(())
    }
}
