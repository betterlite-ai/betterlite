use betterlite_http::error::HttpError;
use betterlite_streaming::stream_content_assembly::AssemblyError;
use serde_json::Error as JsonError;
use thiserror::Error;

use crate::endpoint_target::error::EndpointTargetError;
use crate::token_cache::error::CredentialCacheError;
use crate::vertex_interactions::error::VertexInteractionsError;

#[derive(Debug, Error)]
pub enum VertexLyriaError {
    #[error("Vertex AI model {model:?} does not declare a Lyria audio API")]
    UnknownModel { model: String },
    #[error(
        "Vertex AI {model} does not support the OpenAI parameters: {params}. To drop unsupported openai params from the call, set `litellm.drop_params = True`"
    )]
    UnsupportedParams { model: String, params: String },
    #[error(
        "Vertex AI {model} does not support response_format={format}. Supported values: {supported}. To drop unsupported openai params from the call, set `litellm.drop_params = True`"
    )]
    UnsupportedFormat {
        model: String,
        format: String,
        supported: String,
    },
    #[error("the Vertex AI access token could not be minted")]
    Mint(#[source] CredentialCacheError),
    #[error("the Vertex AI base URL is not valid")]
    ApiBase(#[source] EndpointTargetError),
    #[error("the Vertex AI interactions URL is not valid")]
    Interactions(#[source] VertexInteractionsError),
    #[error("the {field} of the Vertex AI Lyria URL cannot be a path segment")]
    Segment {
        field: &'static str,
        #[source]
        source: HttpError,
    },
    #[error("the Vertex AI Lyria response is {size} bytes, over the {limit} byte limit")]
    ResponseTooLarge { size: usize, limit: usize },
    #[error("the Vertex AI Lyria response is not valid JSON")]
    ResponseJson(#[source] JsonError),
    #[error("no generated audio found in Vertex AI {model} response")]
    MissingAudio { model: String },
    #[error("the generated audio of the Vertex AI Lyria response cannot be decoded")]
    Audio(#[source] AssemblyError),
}

impl VertexLyriaError {
    // LiteLLM raises UnsupportedParamsError (400) for the parameter checks and a bare ValueError (500) for everything else
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::UnsupportedParams { .. } | Self::UnsupportedFormat { .. } => Some(400),
            Self::UnknownModel { .. }
            | Self::Mint(_)
            | Self::ApiBase(_)
            | Self::Interactions(_)
            | Self::Segment { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson(_)
            | Self::MissingAudio { .. }
            | Self::Audio(_) => Some(500),
        }
    }
}
