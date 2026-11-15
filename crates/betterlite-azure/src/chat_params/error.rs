use betterlite_openai::chat_content::ChatContentError;
use betterlite_openai::reasoning_models::ReasoningError;
use thiserror::Error;

use crate::azure_chat_base_policy::error::PolicyError;

pub const UNSUPPORTED_NONE: &str = "Azure OpenAI does not support reasoning_effort='none' for this model. Supported values are: 'low', 'medium', and 'high'. To drop this parameter, set `litellm.drop_params=True` or for proxy:\n\n`litellm_settings:\n drop_params: true`\nIssue: https://github.com/BerriAI/litellm/issues/16704";

#[derive(Debug, Error)]
pub enum ChatParamsError {
    #[error("{}", UNSUPPORTED_NONE)]
    UnsupportedNone,
    #[error(transparent)]
    Reasoning(#[from] ReasoningError),
    #[error(transparent)]
    Messages(#[from] PolicyError),
    #[error(transparent)]
    Content(#[from] ChatContentError),
}

impl ChatParamsError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::UnsupportedNone => Some(400),
            Self::Reasoning(error) => Some(error.status_code()),
            Self::Messages(error) => error.status_code(),
            Self::Content(error) => Some(error.status_code()),
        }
    }
}
