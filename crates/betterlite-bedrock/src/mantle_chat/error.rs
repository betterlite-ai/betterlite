use betterlite_core::api::messages_thinking::response::ResponseJsonError;
use thiserror::Error;

use crate::messages::transform::MessagesTransformError;

// The failures the Mantle Messages override adds on top of B622's transform and B962's response decode.
#[derive(Debug, Error)]
pub enum MantleMessagesError {
    #[error(transparent)]
    Transform(#[from] Box<MessagesTransformError>),
    #[error("anthropic_beta must be a list of strings")]
    BetaNotList,
    #[error("anthropic_beta[{index}] is not a string")]
    BetaItem { index: usize },
    // LiteLLM expands a truthy non-mapping usage into a dict, which raises TypeError; the provider answer is unusable.
    #[error("the Messages response usage is not an object")]
    UsageNotMapping,
    #[error(transparent)]
    Response(#[from] Box<ResponseJsonError>),
    #[error("the Messages response body does not round-trip as JSON")]
    Body(#[from] serde_json::Error),
}

impl From<MessagesTransformError> for MantleMessagesError {
    fn from(error: MessagesTransformError) -> Self {
        Self::Transform(Box::new(error))
    }
}

impl From<ResponseJsonError> for MantleMessagesError {
    fn from(error: ResponseJsonError) -> Self {
        Self::Response(Box::new(error))
    }
}

impl MantleMessagesError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Transform(error) => error.status_code(),
            Self::BetaNotList | Self::BetaItem { .. } => Some(400),
            Self::UsageNotMapping => Some(500),
            Self::Response(error) => Some(error.status_code()),
            Self::Body(_) => None,
        }
    }
}
