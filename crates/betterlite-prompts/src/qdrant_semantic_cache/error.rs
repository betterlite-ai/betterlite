use betterlite_http::proxy::transport_error::TransportError;
use http::header::InvalidHeaderValue;
use thiserror::Error;
use url::ParseError;

use super::ports::QdrantEmbedError;
use crate::semantic_cache_records::SemanticCacheRecordsError;

#[derive(Debug, Error)]
pub enum QdrantSemanticError {
    #[error("collection_name must be provided, passed None")]
    MissingCollectionName,
    #[error("similarity_threshold must be provided, passed None")]
    MissingSimilarityThreshold,
    #[error("environment variable {variable} for {path} is not set")]
    MissingEnvironment {
        variable: String,
        path: &'static str,
    },
    #[error("Qdrant url must be provided")]
    MissingUrl,
    #[error("Quantization config must be one of 'scalar', 'binary' or 'product'")]
    InvalidQuantization,
    #[error("Error from qdrant checking if /collections exist {body}")]
    CollectionCheck { body: String },
    #[error("Error while creating new collection")]
    CreateCollection,
    #[error("the Qdrant api key cannot be carried in an api-key header")]
    InvalidApiKey(#[source] InvalidHeaderValue),
    #[error("the Qdrant request URL for {path} is not valid")]
    InvalidUrl {
        path: String,
        #[source]
        source: ParseError,
    },
    #[error("the Qdrant request to {path} failed")]
    Transport {
        path: String,
        #[source]
        source: TransportError,
    },
    #[error("the Qdrant request body could not be serialized")]
    Encode(#[source] serde_json::Error),
    #[error("the Qdrant {endpoint} response is not valid JSON: {source}")]
    Response {
        endpoint: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("Qdrant rejected the point write to {path} with status {status}: {body}")]
    PointWriteRejected {
        path: String,
        status: u16,
        body: String,
    },
    #[error("the Qdrant search hit payload is missing {field}")]
    MissingPayloadField { field: &'static str },
    #[error("the similarity {similarity} cannot be written as a JSON number")]
    NonFiniteSimilarity { similarity: f64 },
    #[error("a synchronous cache call cannot block a thread that already runs a runtime")]
    BlockingRuntime,
    #[error(transparent)]
    Embed(#[from] QdrantEmbedError),
    #[error(transparent)]
    Records(#[from] SemanticCacheRecordsError),
}
