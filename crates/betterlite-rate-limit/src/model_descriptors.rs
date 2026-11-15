use betterlite_auth::auth_limit_inputs::error::AuthLimitInputsError;
use betterlite_auth::resolved_key::auth::AuthError;

use crate::limiter_records::{ModelGroupResolver, RateLimitedModel};

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
        let wanted = requested.filter(|name| !name.is_empty())?;
        let group = self
            .resolver
            .resolve(wanted)
            .filter(|group| !group.is_empty())
            .unwrap_or_else(|| wanted.to_owned());
        Some(RateLimitedModel {
            requested: wanted.to_owned(),
            group,
        })
    }
}
