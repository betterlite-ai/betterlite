use std::convert::Infallible;
use std::error::Error;

use thiserror::Error;

use crate::api::resource_sdk_settings::timeout::TimeoutError;

// The parts of the placeholder response LiteLLM attaches to its unsupported-provider 400
pub const UNSUPPORTED_BODY: &str = "Unsupported provider";
pub const UNSUPPORTED_URL: &str = "https://github.com/BerriAI/litellm";
pub const UNSUPPORTED_MODEL: &str = "n/a";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadFunction {
    FileRetrieve,
    FileList,
}

impl ReadFunction {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::FileRetrieve => "file_retrieve",
            Self::FileList => "file_list",
        }
    }

    // retrieve's placeholder request still names create_thread, the function its branch was copied from
    #[must_use]
    pub const fn request_method(self) -> &'static str {
        match self {
            Self::FileRetrieve => "create_thread",
            Self::FileList => "file_list",
        }
    }
}

#[derive(Debug, Error)]
pub enum FileReadError<DispatchError = Infallible>
where
    DispatchError: Error + 'static,
{
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    #[error("{name} is not a provider name")]
    UnknownProvider {
        name: String,
        #[source]
        source: strum::ParseError,
    },
    #[error(
        "LiteLLM doesn't support {provider} for '{}'. Only 'openai', 'azure', 'manus', and 'anthropic' are supported.",
        .function.name()
    )]
    Unsupported {
        provider: String,
        function: ReadFunction,
    },
    #[error(transparent)]
    Dispatch(DispatchError),
}

impl<DispatchError> FileReadError<DispatchError>
where
    DispatchError: Error + 'static,
{
    // only the unsupported branch is LiteLLM's own 400; a bad timeout or provider name is a ValueError, and a provider failure keeps its own status
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Unsupported { .. } => Some(400),
            Self::Timeout(_) | Self::UnknownProvider { .. } | Self::Dispatch(_) => None,
        }
    }
}
