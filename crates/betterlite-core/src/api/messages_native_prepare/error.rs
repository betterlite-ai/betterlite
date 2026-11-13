use thiserror::Error;

use crate::api::anthropic_beta_filter::error::BetaFilterError;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MessagesPrepareError {
    #[error("no Anthropic credential is set; set ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN")]
    MissingCredential,
    #[error(transparent)]
    Beta(#[from] BetaFilterError),
}

impl MessagesPrepareError {
    // LiteLLM raises its SDK AuthenticationError for the missing credential, and that class carries 401
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::MissingCredential => Some(401),
            Self::Beta(_) => None,
        }
    }

    #[must_use]
    pub const fn class_name(&self) -> &'static str {
        match self {
            Self::MissingCredential => "AuthenticationError",
            Self::Beta(_) => "BetaFilterError",
        }
    }
}
