use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::auth_limit_inputs::key_limits::{
    LimitQuery, get_key_model_rpm_limit, get_key_model_tpm_limit,
};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use crate::limiter_records::{RateLimitDescriptor, RateLimitDescriptorRateLimit};

use super::{ModelDescriptorBuilder, ModelDescriptorsError, NONE_TEXT};

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
        out.push(RateLimitDescriptor {
            key: "model_per_key".to_owned(),
            value,
            rate_limit: Some(RateLimitDescriptorRateLimit {
                requests_per_unit: requests,
                tokens_per_unit: tokens,
                max_parallel_requests: None,
                window_size: Some(self.window_size),
            }),
        });
        Ok(())
    }
}
