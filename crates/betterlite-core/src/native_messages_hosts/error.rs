use thiserror::Error;

use crate::api::anthropic_beta_filter::error::BetaFilterError;
use crate::api::messages_native_prepare::error::MessagesPrepareError;
use crate::api::messages_native_transform::error::TransformError;

#[derive(Debug, Error)]
pub enum HostError {
    #[error(transparent)]
    Prepare(#[from] MessagesPrepareError),
    #[error(transparent)]
    Beta(#[from] BetaFilterError),
    #[error(transparent)]
    Transform(#[from] TransformError),
}

impl HostError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Prepare(error) => error.status_code(),
            Self::Beta(_) => None,
            Self::Transform(error) => Some(error.status_code()),
        }
    }

    // LiteLLM raises its SDK error classes; the class only names the failure to its callers
    #[must_use]
    pub const fn class_name(&self) -> &'static str {
        match self {
            Self::Prepare(error) => error.class_name(),
            Self::Beta(_) => "BetaFilterError",
            Self::Transform(_) => "AnthropicError",
        }
    }
}
