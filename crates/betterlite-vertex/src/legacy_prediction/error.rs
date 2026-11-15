use betterlite_error::facts::ProviderErrorFacts;
use betterlite_error::vertex_errors::map_vertex_exception;
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
    #[error("the Vertex prediction body is not a JSON object")]
    BodyShape,
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

// async_completion raises a Vertex 500 for every failure; async_streaming's raw google exception takes exception_type's message mapping first
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
    pub fn status_code(&self) -> u16 {
        match self {
            Self::Streaming {
                source: PredictionError::Http { message, .. },
            } => map_vertex_exception(&ProviderErrorFacts::default(), message, "vertex_ai", None)
                .map_or(500, |class| class.metadata().status_code),
            Self::Completion { .. } | Self::Streaming { .. } | Self::UnableToGenerate => 500,
        }
    }
}
