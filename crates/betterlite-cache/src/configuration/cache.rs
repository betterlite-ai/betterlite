use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache_admin::RedisAdminError;
use thiserror::Error;

use super::backend_construction::{BackendConstructionError, build_backend};
use super::callback_registration::CacheCallbacks;
use super::settings::{CacheDependencies, CacheSettings};
use crate::backend::BackendError;
use crate::backend::CacheBackend;
use crate::backend::memory::MemoryCache;
use crate::backend::memory::local::LocalMemoryOps;
use crate::disk::DiskCache;
use crate::types::cache_type::{CacheMode, LiteLlmCacheType};
use crate::types::call_type::CachingSupportedCallType;
use crate::types::semantic::SemanticCacheScope;

pub enum Backend {
    Memory(Box<MemoryCache<LocalMemoryOps>>),
    Redis(Box<RedisCache>),
    Disk(Box<DiskCache>),
    Injected(Arc<dyn CacheBackend>),
}

#[derive(Debug, Error)]
pub enum CacheLifecycleError {
    #[error("the cache ping failed")]
    Ping {
        #[source]
        source: RedisAdminError,
    },
    #[error("deleting the cache keys failed")]
    DeleteKeys {
        #[source]
        source: RedisAdminError,
    },
    #[error("the cache backend could not disconnect")]
    Disconnect {
        #[source]
        source: BackendError,
    },
}

pub struct Cache {
    pub backend: Backend,
    pub cache_type: LiteLlmCacheType,
    pub namespace: Option<String>,
    pub redis_flush_size: Option<usize>,
    pub ttl: Option<f64>,
    pub mode: CacheMode,
    pub supported_call_types: Vec<CachingSupportedCallType>,
    pub semantic_cache_scope: SemanticCacheScope,
}

impl Cache {
    pub async fn open(
        settings: CacheSettings,
        callbacks: &mut CacheCallbacks,
        environment: &(dyn EnvironmentLookup + Sync),
        dependencies: &CacheDependencies,
    ) -> Result<Self, BackendConstructionError> {
        let backend = match settings.backend.as_ref() {
            Some(injected) => Backend::Injected(Arc::clone(injected)),
            None => build_backend(&settings, environment, dependencies).await?,
        };
        callbacks.register_cache();
        let ttl = match settings.cache_type {
            LiteLlmCacheType::Local => settings.default_in_memory_ttl.or(settings.ttl),
            LiteLlmCacheType::Redis
            | LiteLlmCacheType::RedisSemantic
            | LiteLlmCacheType::ValkeySemantic => settings.default_in_redis_ttl.or(settings.ttl),
            LiteLlmCacheType::S3
            | LiteLlmCacheType::Disk
            | LiteLlmCacheType::QdrantSemantic
            | LiteLlmCacheType::AzureBlob
            | LiteLlmCacheType::Gcs => settings.ttl,
        };
        Ok(Self {
            backend,
            cache_type: settings.cache_type,
            namespace: settings.namespace,
            redis_flush_size: settings.redis_flush_size,
            ttl,
            mode: settings.mode,
            supported_call_types: settings.supported_call_types,
            semantic_cache_scope: settings.semantic_cache_scope,
        })
    }

    pub async fn ping(&self) -> Result<Option<bool>, CacheLifecycleError> {
        let Some(cache) = self.redis_cache() else {
            return Ok(None);
        };
        cache
            .admin
            .ping(None)
            .await
            .map(Some)
            .map_err(|source| CacheLifecycleError::Ping { source })
    }

    pub async fn delete_cache_keys(&self, keys: &[String]) -> Result<(), CacheLifecycleError> {
        // only RedisCache declares the operation; LiteLLM's getattr path skips every other backend
        let Some(cache) = self.redis_cache() else {
            return Ok(());
        };
        cache
            .admin
            .delete_cache_keys(keys)
            .await
            .map_err(|source| CacheLifecycleError::DeleteKeys { source })
    }

    fn redis_cache(&self) -> Option<&RedisCache> {
        match &self.backend {
            Backend::Redis(cache) => Some(cache),
            Backend::Memory(_) | Backend::Disk(_) | Backend::Injected(_) => None,
        }
    }

    pub async fn disconnect(&self) -> Result<(), CacheLifecycleError> {
        match &self.backend {
            Backend::Redis(cache) => {
                cache.admin.disconnect().await;
                Ok(())
            }
            Backend::Memory(_) | Backend::Disk(_) => Ok(()),
            Backend::Injected(backend) => backend
                .disconnect()
                .await
                .map_err(|source| CacheLifecycleError::Disconnect { source }),
        }
    }
}

// LiteLLM asks a cache whether it has async get and set before using them, and every backend here does
pub trait AsyncCacheSupport {
    fn supports_async(&self) -> bool;
}

impl AsyncCacheSupport for Cache {
    fn supports_async(&self) -> bool {
        true
    }
}
