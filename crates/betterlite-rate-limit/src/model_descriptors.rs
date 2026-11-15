use betterlite_auth::auth_limit_inputs::error::AuthLimitInputsError;
use betterlite_auth::resolved_key::auth::AuthError;

use crate::limiter_records::{
    ModelGroupResolver, RateLimitDescriptor, RateLimitDescriptorRateLimit, RateLimitedModel,
};

mod key_model;
mod organization;
mod project;
mod request_metadata;
mod team_model;
mod tests;

// LiteLLM builds descriptor values with an f-string, so an absent id lands in the counter name as this text
const NONE_TEXT: &str = "None";

#[derive(Debug, thiserror::Error)]
pub enum ModelDescriptorsError {
    #[error(transparent)]
    KeyLimits(#[from] AuthLimitInputsError),
    #[error(transparent)]
    RequestMetadata(#[from] AuthError),
}

pub struct ModelDescriptorBuilder {
    pub window_size: i64,
    pub resolver: Box<dyn ModelGroupResolver>,
}

impl ModelDescriptorBuilder {
    #[must_use]
    pub fn rated_model(&self, requested: Option<&str>) -> Option<RateLimitedModel> {
        let requested = requested.filter(|name| !name.is_empty())?;
        let group = self
            .resolver
            .resolve(requested)
            .filter(|group| !group.is_empty())
            .unwrap_or_else(|| requested.to_owned());
        Some(RateLimitedModel {
            requested: requested.to_owned(),
            group,
        })
    }
}

#[must_use]
const fn window_rate_limit(
    requests_per_unit: Option<i64>,
    tokens_per_unit: Option<i64>,
    window_size: i64,
) -> RateLimitDescriptorRateLimit {
    RateLimitDescriptorRateLimit {
        requests_per_unit,
        tokens_per_unit,
        max_parallel_requests: None,
        window_size: Some(window_size),
    }
}

#[must_use]
fn descriptor(
    key: &str,
    value: String,
    rate_limit: RateLimitDescriptorRateLimit,
) -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: key.to_owned(),
        value,
        rate_limit: Some(rate_limit),
    }
}
