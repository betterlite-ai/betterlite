use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::api::audio_file_input::AudioFileError;
use thiserror::Error;

use crate::api::soniox::prepare::error::{PrepareError, ReplyError};
use crate::api::soniox::records::error::{RecordsError, SonioxException};
use crate::api::soniox::remote::error::RemoteError;

pub const DURATION_FIELD: &str = "audio_duration_ms";

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error(transparent)]
    Prepare(#[from] PrepareError),
    #[error(transparent)]
    Provider(#[from] SonioxException),
    #[error(transparent)]
    Reply(#[from] ReplyError),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Audio(#[from] AudioFileError),
    #[error(transparent)]
    Records(#[from] RecordsError),
    #[error(transparent)]
    Remote(#[from] RemoteError),
    #[error("the Soniox endpoint {url} is not a URL this request can be sent to")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("a Soniox authenticated header cannot be sent")]
    Header(#[source] http::Error),
    #[error("the {action} request body cannot be encoded")]
    Body {
        action: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the {field} value {value} cannot be converted to seconds")]
    Duration { field: &'static str, value: String },
}

impl PipelineError {
    // A provider rejection reaches the caller the way LiteLLM raises it, whichever stage produced it
    #[must_use]
    pub const fn provider(&self) -> Option<&SonioxException> {
        match self {
            Self::Provider(error)
            | Self::Prepare(PrepareError::Auth(error))
            | Self::Remote(RemoteError::Provider(error)) => Some(error),
            Self::Prepare(_)
            | Self::Reply(_)
            | Self::Transport(_)
            | Self::Audio(_)
            | Self::Records(_)
            | Self::Remote(_)
            | Self::Url { .. }
            | Self::Header(_)
            | Self::Body { .. }
            | Self::Duration { .. } => None,
        }
    }
}
