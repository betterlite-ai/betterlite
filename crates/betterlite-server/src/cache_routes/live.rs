use std::sync::Arc;

use async_trait::async_trait;
use betterlite_cache::configuration::cache::{Backend, Cache};
use betterlite_cache::envelope::facade::Clock;
use betterlite_cache::types::cache_type::LiteLlmCacheType;
use betterlite_cache::types::ping::HealthCheckCacheParams;
use betterlite_redis::redis_cache_admin::RedisCacheAdmin;
use betterlite_redis::redis_cache_admin::info_parse::InfoValue;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use indexmap::IndexMap;

use super::admin::{CacheAdmin, CacheAdminError, RedisAdmin};
// POST /cache/settings replaces the server's cache while it runs, so a request asks for the current one.
pub trait LiveResponseCache: Send + Sync {
    fn current(&self) -> Option<Arc<dyn CacheAdmin>>;
}

// The Cache a request reads and the redis kwargs it was opened with, which the Cache does not keep.
pub struct LiveCache {
    pub cache: Arc<Cache>,
    pub redis: RedisOverrides,
}

// B8650 builds this once from Shared.response_cache; get runs on every request.
pub struct SlotCache<F> {
    pub get: F,
    pub clock: Clock,
}

impl<F> LiveResponseCache for SlotCache<F>
where
    F: Fn() -> Option<LiveCache> + Send + Sync,
{
    fn current(&self) -> Option<Arc<dyn CacheAdmin>> {
        let live = (self.get)()?;
        let cache: Arc<dyn CacheAdmin> = Arc::new(CacheHandle {
            cache: live.cache,
            redis: live.redis,
            clock: Arc::clone(&self.clock),
        });
        Some(cache)
    }
}

// The live Cache behind the cache-admin port the routes read.
pub struct CacheHandle {
    pub cache: Arc<Cache>,
    pub redis: RedisOverrides,
    pub clock: Clock,
}

#[async_trait]
impl CacheAdmin for CacheHandle {
    fn cache_type(&self) -> String {
        wire_name(self.cache.cache_type).to_owned()
    }

    fn masked_cache_params(&self) -> String {
        super::ping_params::masked_cache_params(&self.cache)
    }

    fn health_check_params(&self) -> HealthCheckCacheParams {
        super::ping_params::health_check_params(&self.cache, &self.redis)
    }

    async fn ping(&self) -> Result<bool, CacheAdminError> {
        self.cache
            .ping()
            .await
            .map(|pinged| pinged.unwrap_or(false))
            .map_err(|source| CacheAdminError::Ping { source })
    }

    async fn add_test_key(&self) -> Result<(), CacheAdminError> {
        super::test_write::add_test_key(Arc::clone(&self.cache), Arc::clone(&self.clock)).await
    }

    async fn delete_cache_keys(&self, keys: &[String]) -> Result<(), CacheAdminError> {
        self.cache
            .delete_cache_keys(keys)
            .await
            .map_err(|source| CacheAdminError::Delete { source })
    }

    fn redis_admin(&self) -> Option<Arc<dyn RedisAdmin>> {
        match &self.cache.backend {
            Backend::Redis(redis) => {
                let admin: Arc<dyn RedisAdmin> = Arc::new(LiveRedisAdmin {
                    admin: Arc::clone(&redis.admin),
                });
                Some(admin)
            }
            Backend::Memory(_) | Backend::Disk(_) | Backend::Injected(_) => None,
        }
    }
}

struct LiveRedisAdmin {
    admin: Arc<RedisCacheAdmin>,
}

impl RedisAdmin for LiveRedisAdmin {
    fn client_list(&self) -> Result<Vec<IndexMap<String, String>>, CacheAdminError> {
        self.admin
            .client_list()
            .map_err(|source| CacheAdminError::ClientList { source })
    }

    fn info(&self) -> Result<IndexMap<String, InfoValue>, CacheAdminError> {
        self.admin
            .info()
            .map_err(|source| CacheAdminError::Info { source })
    }

    fn flushall(&self) -> Result<(), CacheAdminError> {
        self.admin
            .flushall()
            .map_err(|source| CacheAdminError::Flush { source })
    }
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

mod tests;
