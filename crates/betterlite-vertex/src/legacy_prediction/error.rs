use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_tokenizer::engine::error::EngineError;
use thiserror::Error;
use url::ParseError;

use crate::model_urls::error::VertexUrlError;
use crate::token_cache::error::CredentialCacheError;

#[derive(Debug, Error)]
pub enum PredictionError {
    #[error("the Vertex prediction request failed")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the Vertex prediction response body failed")]
    Read {
        #[source]
        source: HttpError,
    },
    #[error("the Vertex prediction endpoint returned HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("the Vertex prediction response is not valid JSON")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Vertex prediction body is larger than {limit} bytes")]
    TooLarge { limit: usize },
    #[error("the Vertex prediction stream is not a JSON array")]
    StreamShape,
    #[error("the Vertex prediction stream ended inside an object")]
    StreamUnfinished,
    #[error("the Vertex prediction response has no prediction")]
    MissingPrediction,
    #[error("the Vertex prediction has no {field}")]
    MissingField { field: &'static str },
    #[error("the Vertex prediction field {field} has the wrong type")]
    WrongFieldType { field: &'static str },
    #[error("the Vertex prediction stream carries {count} outputs")]
    MultipleOutputs { count: usize },
    #[error("the Vertex prediction request URL is not valid")]
    Url {
        #[source]
        source: ParseError,
    },
    #[error("the Vertex prediction location is not valid")]
    VertexUrl {
        #[source]
        source: VertexUrlError,
    },
    #[error("the Vertex prediction request body cannot be encoded")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Vertex prediction request header is not valid")]
    Header {
        #[source]
        source: http::Error,
    },
    #[error("the Vertex prediction request option {name} is not accepted by the model call")]
    UnsupportedOption { name: String },
    #[error("the Vertex prediction request option {name} has the wrong type")]
    OptionType { name: String },
    #[error("the Vertex prediction usage metadata has no {field}")]
    MissingUsage { field: &'static str },
    #[error("the Vertex prediction token counts overflow")]
    UsageOverflow,
    #[error("the Vertex prediction message content is not text")]
    NonTextContent,
    #[error("the local tokenizer failed")]
    Encoding {
        #[source]
        source: EngineError,
    },
    #[error("the vertex credential could not be resolved")]
    Credentials {
        #[source]
        source: CredentialCacheError,
    },
    #[error("Vertex project and location are required for custom endpoint")]
    CustomEndpointContext,
    #[error("Instances are required for private endpoint")]
    MissingInstances,
}

impl PredictionError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Http { status, .. } => *status,
            Self::Transport { .. }
            | Self::Read { .. }
            | Self::Decode { .. }
            | Self::TooLarge { .. }
            | Self::StreamShape
            | Self::StreamUnfinished
            | Self::MissingPrediction
            | Self::MissingField { .. }
            | Self::WrongFieldType { .. }
            | Self::MultipleOutputs { .. }
            | Self::Url { .. }
            | Self::VertexUrl { .. }
            | Self::Serialize { .. }
            | Self::Header { .. }
            | Self::UnsupportedOption { .. }
            | Self::OptionType { .. }
            | Self::MissingUsage { .. }
            | Self::UsageOverflow
            | Self::NonTextContent
            | Self::Encoding { .. }
            | Self::Credentials { .. }
            | Self::CustomEndpointContext
            | Self::MissingInstances => 500,
        }
    }
}

// LiteLLM's async_completion turns every failure into VertexAIError(500); async_streaming lets the SDK's status through
#[derive(Debug, Error)]
pub enum LegacyPredictionError {
    #[error("the Vertex prediction failed: {source}")]
    Completion {
        #[source]
        source: PredictionError,
    },
    #[error("the Vertex prediction stream failed: {source}")]
    Streaming {
        #[source]
        source: PredictionError,
    },
    #[error("Unable to generate response")]
    UnableToGenerate,
}

impl LegacyPredictionError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Completion { .. } | Self::UnableToGenerate => 500,
            Self::Streaming { source } => source.status_code(),
        }
    }
}
