use std::string::FromUtf8Error;

use betterlite_redis::redis_live_client::LiveClientError;
use thiserror::Error;

use crate::redis_semantic_cache::ports::SemanticEmbedError;
use crate::semantic_cache_records::SemanticCacheRecordsError;

#[derive(Debug, Error)]
pub enum ValkeySemanticError {
    #[error("similarity_threshold must be provided, passed None")]
    MissingSimilarityThreshold,
    #[error(
        "valkey-semantic does not support cluster-mode-enabled (multi-shard) endpoints. \
         The async cluster client cannot route the FT.* search commands reliably. \
         Point it at a cluster-mode-disabled endpoint instead (a primary with replicas is fine; \
         only horizontal sharding is unsupported), or pass a single redis_url. \
         On AWS, vector search needs ElastiCache for Valkey 8.2+ on a node-based cluster."
    )]
    ClusterModeUnsupported,
    #[error(
        "Missing required Valkey configuration. Provide host and port (or VALKEY_HOST/VALKEY_PORT), \
         or pass redis_url."
    )]
    MissingConfiguration,
    #[error(transparent)]
    Embed(#[from] SemanticEmbedError),
    #[error(transparent)]
    Records(#[from] SemanticCacheRecordsError),
    #[error(transparent)]
    Backend(#[from] LiveClientError),
    #[error(
        "Valkey semantic-cache index '{index}' already exists with embedding dimension {existing}, \
         but the configured embedding model produced dimension {configured}. \
         Use a different valkey_semantic_cache_index_name or drop the existing index."
    )]
    IndexDimensionMismatch {
        index: String,
        existing: usize,
        configured: usize,
    },
    #[error("the FT.INFO reply from Valkey is malformed")]
    MalformedIndexInfo,
    #[error("the FT.SEARCH reply from Valkey is malformed")]
    MalformedSearchReply,
    #[error("the cached response could not be serialized")]
    Encode(#[source] serde_json::Error),
    #[error("a Valkey reply string is not valid UTF-8")]
    Utf8(#[from] FromUtf8Error),
    #[error("the similarity {similarity} cannot be written as a JSON number")]
    NonFiniteSimilarity { similarity: f64 },
    #[error("unexpected Valkey reply kind: {kind}")]
    UnexpectedReply { kind: &'static str },
}
