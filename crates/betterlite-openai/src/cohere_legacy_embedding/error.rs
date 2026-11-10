use betterlite_http::proxy::transport_error::TransportError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use crate::cohere_embedding::CohereEmbeddingError;
use crate::error::ProviderError;

#[derive(Debug, Error)]
pub enum CohereLegacyEmbeddingError {
    #[error("the Cohere embeddings request could not be sent")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the Cohere embeddings endpoint is not a URL")]
    Url {
        #[source]
        source: url::ParseError,
    },
    #[error("the {name} header name cannot be carried in a request")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header value cannot be carried in a request")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the Cohere embedding request could not be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Cohere embedding reply is not JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Cohere embedding reply is not a JSON object")]
    ReplyNotAnObject,
    #[error("the Cohere embedding reply has no embeddings")]
    MissingEmbeddings,
    #[error("the Cohere embedding reply's embeddings are neither a list nor an object")]
    EmbeddingsShape,
    #[error("the Cohere embedding reply says embeddings_by_type but its embeddings are a list")]
    EmbeddingsNotGrouped,
    #[error("the Cohere billed units could not be read")]
    Usage {
        #[from]
        source: CohereEmbeddingError,
    },
    #[error("the Cohere embeddings request failed with status {}", .0.status_code)]
    Status(Box<ProviderError>),
}
