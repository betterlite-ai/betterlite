use std::error::Error as StdError;

use betterlite_prompts::anthropic::error::AnthropicPromptError;
use betterlite_prompts::media::error::MediaError;

use crate::anthropic_capability_catalog::sampling::SamplingError;
use crate::capabilities::speed::SpeedError;
use crate::option_primitives::OptionPrimitivesError;
use crate::params::error::ParamsError;
use crate::tools::ToolsError;

use super::beta_headers::BetaHeaderError;
use super::metadata::MetadataError;
use super::tool_list::ToolListError;

#[derive(Debug, thiserror::Error)]
pub enum RequestError<FileFailure>
where
    FileFailure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Media(#[from] MediaError),
    #[error("messages do not decode as chat messages")]
    Messages(#[source] serde_json::Error),
    #[error("{source}\nReceived Messages={received}")]
    Prompt {
        #[source]
        source: AnthropicPromptError<FileFailure>,
        received: String,
    },
    #[error(transparent)]
    Speed(#[from] SpeedError),
    #[error(transparent)]
    Sampling(#[from] SamplingError),
    #[error(transparent)]
    Parameters(#[from] ParamsError),
    #[error(transparent)]
    Defaults(#[from] OptionPrimitivesError),
    #[error(transparent)]
    Tools(#[from] ToolsError),
    #[error(transparent)]
    ToolList(#[from] ToolListError),
    #[error(transparent)]
    BetaHeaders(#[from] BetaHeaderError),
    #[error(transparent)]
    Metadata(#[from] MetadataError),
    #[error(transparent)]
    Convert(#[from] serde_json::Error),
}

impl<FileFailure> RequestError<FileFailure>
where
    FileFailure: StdError + Send + Sync + 'static,
{
    // LiteLLM wraps a failed message translation in a 400 AnthropicError; every other failure keeps its own status
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Media(_) | Self::Messages(_) | Self::Prompt { .. } => Some(400),
            Self::Speed(error) => error.status_code(),
            Self::Sampling(error) => error.status_code(),
            Self::Parameters(error) => error.status_code(),
            Self::Defaults(_) | Self::Convert(_) => None,
            Self::Tools(error) => error.status_code(),
            Self::ToolList(error) => Some(error.status_code()),
            Self::BetaHeaders(error) => error.status_code(),
            Self::Metadata(error) => error.status_code(),
        }
    }
}
