use std::string::FromUtf8Error;

use betterlite_redis::redis_live_client::LiveClientError;
use thiserror::Error;

use super::ports::SemanticEmbedError;
use crate::semantic_cache_records::SemanticCacheRecordsError;

#[derive(Debug, Error)]
pub enum RedisSemanticError {
    #[error("similarity_threshold must be provided, passed None")]
    MissingSimilarityThreshold,
    #[error("Missing required Redis configuration: {variable}. Provide {variable} or redis_url.")]
    MissingConfiguration { variable: &'static str },
    #[error(transparent)]
    Embed(#[from] SemanticEmbedError),
    #[error(transparent)]
    Records(#[from] SemanticCacheRecordsError),
    #[error(transparent)]
    Backend(#[from] LiveClientError),
    #[error("the existing index {index} schema does not match the semantic cache schema")]
    SchemaMismatch { index: String },
    #[error("Distance must be between 0 and 2, got {distance}")]
    InvalidDistanceThreshold { distance: f64 },
    #[error("embedding vector has {actual} dimensions; the index expects {expected}")]
    DimensionMismatch { actual: usize, expected: usize },
    #[error("the FT.INFO reply from Redis is malformed")]
    MalformedIndexInfo,
    #[error("the FT.SEARCH reply from Redis is malformed")]
    MalformedSearchReply,
    #[error("the cached response could not be serialized")]
    Encode(#[source] serde_json::Error),
    #[error("a Redis reply string is not valid UTF-8")]
    Utf8(#[from] FromUtf8Error),
    #[error("the system clock reads a time before the unix epoch")]
    ClockBeforeEpoch,
    #[error("the vector distance {distance} is not a finite number")]
    NonFiniteDistance { distance: f64 },
    #[error("the similarity {similarity} cannot be written as a JSON number")]
    NonFiniteSimilarity { similarity: f64 },
    #[error("the index {index} does not exist")]
    MissingIndex { index: String },
    #[error("unexpected Redis reply kind: {kind}")]
    UnexpectedReply { kind: &'static str },
}
