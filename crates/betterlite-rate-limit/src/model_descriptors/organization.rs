use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::auth_limit_inputs::key_limits::ModelRateLimitKey;
use betterlite_auth::auth_limit_inputs::metadata_limits::{
    MetadataAccessor, model_limit_from_metadata,
};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use crate::limiter_records::{RateLimitDescriptor, RateLimitDescriptorRateLimit};

use super::{ModelDescriptorBuilder, NONE_TEXT};

impl ModelDescriptorBuilder {
    #[must_use]
    pub fn organization_descriptors(
        &self,
        auth: &UserAPIKeyAuth,
        requested_model: Option<&str>,
    ) -> Vec<RateLimitDescriptor> {
        let account = account_limits(auth);
        let has_global_limit =
            account.organization_rpm_limit.is_some() || account.organization_tpm_limit.is_some();
        let mut descriptors = Vec::new();
        if let Some(org_id) = account.org_id
            && has_global_limit
        {
            descriptors.push(RateLimitDescriptor {
                key: "organization".to_owned(),
                value: org_id.to_owned(),
                rate_limit: Some(RateLimitDescriptorRateLimit {
                    requests_per_unit: account.organization_rpm_limit,
                    tokens_per_unit: account.organization_tpm_limit,
                    max_parallel_requests: None,
                    window_size: Some(self.window_size),
                }),
            });
        }
        let Some(model) = self.rated_model(requested_model) else {
            return descriptors;
        };
        let tokens = model.limit_in(
            model_limit_from_metadata(auth, MetadataAccessor::Organization, ModelRateLimitKey::Tpm)
                .as_ref(),
        );
        let requests = model.limit_in(
            model_limit_from_metadata(auth, MetadataAccessor::Organization, ModelRateLimitKey::Rpm)
                .as_ref(),
        );
        if requests.is_none() && tokens.is_none() {
            return descriptors;
        }
        descriptors.push(RateLimitDescriptor {
            key: "model_per_organization".to_owned(),
            value: format!("{}:{}", account.org_id.unwrap_or(NONE_TEXT), model.group),
            rate_limit: Some(RateLimitDescriptorRateLimit {
                requests_per_unit: requests,
                tokens_per_unit: tokens,
                max_parallel_requests: None,
                window_size: Some(self.window_size),
            }),
        });
        descriptors
    }
}
