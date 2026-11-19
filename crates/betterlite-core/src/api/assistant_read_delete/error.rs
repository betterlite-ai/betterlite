use std::convert::Infallible;
use std::error::Error;

use thiserror::Error;

use crate::api::api_boundaries::ApiLimitError;
use crate::api::resource_sdk_settings::timeout::TimeoutError;

// the LiteLLM class a failure surfaces as, and the status its exception carries
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssistantErrorFields {
    pub class: &'static str,
    pub status: Option<u16>,
    // the OpenAI error body's type; only LiteLLM's internal error sets one on its own class
    pub error_type: Option<&'static str>,
    // litellm's own exceptions print their class before the message, a plain Exception does not
    pub prefixed: bool,
}

#[derive(Debug, Error)]
pub enum AssistantReadDeleteError<DispatchError = Infallible>
where
    DispatchError: Error + 'static,
{
    #[error(transparent)]
    Limits(#[from] ApiLimitError),
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    #[error("LiteLLM doesn't support {provider} for '{function}'. Only 'openai' is supported.")]
    Unsupported {
        provider: String,
        function: &'static str,
    },
    #[error("Invalid value passed in for {flag}. Only bool or None allowed")]
    InvalidFlag {
        flag: &'static str,
        class: &'static str,
    },
    #[error("assistant_id is required for delete_assistant")]
    MissingAssistantId,
    #[error("No response returned from '{function}'")]
    MissingReply { function: &'static str },
    #[error("the dispatch answered with the wrong reply")]
    MismatchedReply,
    #[error(transparent)]
    Dispatch(DispatchError),
}

impl<DispatchError> AssistantReadDeleteError<DispatchError>
where
    DispatchError: Error + 'static,
{
    #[must_use]
    pub const fn fields(&self) -> Option<AssistantErrorFields> {
        let (class, status, error_type, prefixed) = match self {
            Self::Unsupported { .. } => ("BadRequestError", Some(400), None, true),
            Self::InvalidFlag { class, .. } => (*class, None, None, false),
            Self::MissingReply { .. } | Self::MismatchedReply => (
                "InternalServerError",
                Some(500),
                Some("internal_server_error"),
                true,
            ),
            Self::Limits(_) | Self::Timeout(_) | Self::MissingAssistantId | Self::Dispatch(_) => {
                return None;
            }
        };
        Some(AssistantErrorFields {
            class,
            status,
            error_type,
            prefixed,
        })
    }
}
