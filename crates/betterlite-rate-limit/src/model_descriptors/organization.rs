use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::auth_limit_inputs::key_limits::ModelRateLimitKey;
use betterlite_auth::auth_limit_inputs::metadata_limits::{
    MetadataAccessor, model_limit_from_metadata,
};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use crate::limiter_records::RateLimitDescriptor;

use super::{ModelDescriptorBuilder, NONE_TEXT, descriptor, window_rate_limit};

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
            descriptors.push(descriptor(
                "organization",
                org_id.to_owned(),
                window_rate_limit(
                    account.organization_rpm_limit,
                    account.organization_tpm_limit,
                    self.window_size,
                ),
            ));
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
        descriptors.push(descriptor(
            "model_per_organization",
            format!("{}:{}", account.org_id.unwrap_or(NONE_TEXT), model.group),
            window_rate_limit(requests, tokens, self.window_size),
        ));
        descriptors
    }
}
