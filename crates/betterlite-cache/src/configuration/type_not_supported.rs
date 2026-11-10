use thiserror::Error;

use crate::types::cache_type::LiteLlmCacheType;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum CacheTypeNotSupportedError {
    #[error("cache type '{}' is owned by {owner}", wire_name(*cache_type))]
    OwnedByTask {
        cache_type: LiteLlmCacheType,
        owner: &'static str,
    },
    #[error("cache type '{}' is not yet implemented", wire_name(*cache_type))]
    NotImplemented { cache_type: LiteLlmCacheType },
    #[error("cache type 'redis' with cluster mode is owned by {owner}")]
    RedisCluster { owner: &'static str },
}

#[must_use]
pub(crate) const fn wire_name(cache_type: LiteLlmCacheType) -> &'static str {
    match cache_type {
        LiteLlmCacheType::Local => "local",
        LiteLlmCacheType::Redis => "redis",
        LiteLlmCacheType::RedisSemantic => "redis-semantic",
        LiteLlmCacheType::ValkeySemantic => "valkey-semantic",
        LiteLlmCacheType::S3 => "s3",
        LiteLlmCacheType::Disk => "disk",
        LiteLlmCacheType::QdrantSemantic => "qdrant-semantic",
        LiteLlmCacheType::AzureBlob => "azure-blob",
        LiteLlmCacheType::Gcs => "gcs",
    }
}
