pub mod catalog;
pub mod environment;
pub mod request;
pub mod response;
mod tests;

use std::fmt;

use indexmap::IndexMap;
use thiserror::Error;

use crate::feature_flags::FeatureFlagsError;

// The preparation of one request; values stay out of Debug because x-api-key and authorization carry secrets
#[derive(Clone, Default, Eq, PartialEq)]
pub struct AnthropicHeaders {
    pub values: IndexMap<String, String>,
}

impl fmt::Debug for AnthropicHeaders {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.values.keys().map(String::as_str).collect();
        formatter
            .debug_tuple("AnthropicHeaders")
            .field(&names)
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum HeadersError {
    #[error(
        "Missing Anthropic API Key - a call is being made to anthropic but no key is set either in the environment variables or via params. Please set `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN` in your environment vars"
    )]
    Authentication { model: String },
    #[error(transparent)]
    Features(#[from] FeatureFlagsError),
    #[error("model timestamp {seconds} is outside the UTC range")]
    Timestamp { seconds: i64 },
}

impl HeadersError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Authentication { .. } => Some(401),
            Self::Features(source) => source.status_code(),
            Self::Timestamp { .. } => None,
        }
    }
}
