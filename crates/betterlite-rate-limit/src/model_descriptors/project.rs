use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::auth_limit_inputs::key_limits::ModelRateLimitKey;
use betterlite_auth::auth_limit_inputs::metadata_limits::{
    MetadataAccessor, model_limit_from_metadata,
};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use crate::limiter_records::{
    PROJECT_ITPM_DESCRIPTOR_KEY, PROJECT_OTPM_DESCRIPTOR_KEY, RateLimitDescriptor,
    RateLimitDescriptorRateLimit,
};

use super::{ModelDescriptorBuilder, NONE_TEXT};

impl ModelDescriptorBuilder {
    pub fn project_model_descriptor(
        &self,
        auth: &UserAPIKeyAuth,
        requested_model: Option<&str>,
        out: &mut Vec<RateLimitDescriptor>,
    ) {
        let Some(model) = self.rated_model(requested_model) else {
            return;
        };
        let tokens = model.limit_in(
            model_limit_from_metadata(auth, MetadataAccessor::Project, ModelRateLimitKey::Tpm)
                .as_ref(),
        );
        let requests = model.limit_in(
            model_limit_from_metadata(auth, MetadataAccessor::Project, ModelRateLimitKey::Rpm)
                .as_ref(),
        );
        if requests.is_none() && tokens.is_none() {
            return;
        }
        let value = format!(
            "{}:{}",
            account_limits(auth).project_id.unwrap_or(NONE_TEXT),
            model.group
        );
        out.push(RateLimitDescriptor {
            key: "model_per_project".to_owned(),
            value,
            rate_limit: Some(RateLimitDescriptorRateLimit {
                requests_per_unit: requests,
                tokens_per_unit: tokens,
                max_parallel_requests: None,
                window_size: Some(self.window_size),
            }),
        });
    }

    pub fn project_io_descriptors(
        &self,
        auth: &UserAPIKeyAuth,
        requested_model: Option<&str>,
        out: &mut Vec<RateLimitDescriptor>,
    ) {
        let Some(model) = self.rated_model(requested_model) else {
            return;
        };
        let Some(project_id) = account_limits(auth).project_id else {
            return;
        };
        let input_tokens = model.limit_in(
            model_limit_from_metadata(auth, MetadataAccessor::Project, ModelRateLimitKey::Itpm)
                .as_ref(),
        );
        let output_tokens = model.limit_in(
            model_limit_from_metadata(auth, MetadataAccessor::Project, ModelRateLimitKey::Otpm)
                .as_ref(),
        );
        if input_tokens.is_none() && output_tokens.is_none() {
            return;
        }
        let value = format!("{project_id}:{}", model.group);
        if let Some(limit) = input_tokens {
            out.push(RateLimitDescriptor {
                key: PROJECT_ITPM_DESCRIPTOR_KEY.to_owned(),
                value: value.clone(),
                rate_limit: Some(RateLimitDescriptorRateLimit {
                    requests_per_unit: None,
                    tokens_per_unit: Some(limit),
                    max_parallel_requests: None,
                    window_size: Some(self.window_size),
                }),
            });
        }
        if let Some(limit) = output_tokens {
            out.push(RateLimitDescriptor {
                key: PROJECT_OTPM_DESCRIPTOR_KEY.to_owned(),
                value,
                rate_limit: Some(RateLimitDescriptorRateLimit {
                    requests_per_unit: None,
                    tokens_per_unit: Some(limit),
                    max_parallel_requests: None,
                    window_size: Some(self.window_size),
                }),
            });
        }
    }
}
