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
    #[error("the dispatch answered '{function}' with the wrong reply")]
    MismatchedReply { function: &'static str },
    #[error(transparent)]
    Dispatch(#[from] DispatchError),
}

impl<DispatchError> AssistantReadDeleteError<DispatchError>
where
    DispatchError: Error + 'static,
{
    #[must_use]
    pub const fn fields(&self) -> Option<AssistantErrorFields> {
        match self {
            Self::Unsupported { .. } => Some(AssistantErrorFields {
                class: "BadRequestError",
                status: Some(400),
                prefixed: true,
            }),
            Self::InvalidFlag { class, .. } => Some(AssistantErrorFields {
                class,
                status: None,
                prefixed: false,
            }),
            Self::MissingReply { .. } | Self::MismatchedReply { .. } => {
                Some(AssistantErrorFields {
                    class: "InternalServerError",
                    status: Some(500),
                    prefixed: true,
                })
            }
            Self::Limits(_) | Self::Timeout(_) | Self::MissingAssistantId | Self::Dispatch(_) => {
                None
            }
        }
    }
}
